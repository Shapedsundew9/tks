//! Document ingestion and job polling REST routes (WP-1.5, D-67, LD-6, TB-1).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::gateway::AppState;
use crate::gateway::auth::AuthenticatedAgent;
use crate::storage::GraphNode;
use crate::storage::git::normalize_doc_path;

/// Payload for submitting a Markdown document for ingestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestDocumentRequest {
    /// Relative persistent document path (e.g. `specs/vision.md`).
    pub doc_path: String,
    /// Raw Markdown document content.
    pub content: String,
}

/// Response returned when a document is accepted for ingestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestDocumentResponse {
    /// Unique identifier of the created ingestion job.
    pub job_id: Uuid,
    /// Initial lifecycle status (`QUEUED`).
    pub status: String,
}

/// Detailed status and candidate nodes for an ingestion job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionJobStatusResponse {
    pub job_id: Uuid,
    pub doc_path: String,
    pub document_hash: String,
    pub status: String,
    pub error_message: Option<String>,
    pub retry_count: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub nodes: Vec<GraphNode>,
}

/// Handles `POST /api/v1/documents/ingest`.
///
/// 1. Validates and normalizes `doc_path`.
/// 2. Performs automatic supersession sweep of prior open jobs at `doc_path` (D-67).
/// 3. Synchronously commits document content to bare Git repository via Git write actor.
/// 4. Inserts a `QUEUED` ingestion job record.
/// 5. Returns `202 Accepted` with `job_id`.
pub async fn ingest_document(
    State(state): State<AppState>,
    caller: AuthenticatedAgent,
    Json(payload): Json<IngestDocumentRequest>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let doc_path = normalize_doc_path(&payload.doc_path).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Invalid document path: {e}") })),
        )
    })?;

    // Step 1: Automatic supersession sweep inside a transaction (D-67, LD-6).
    let mut client = state.pool.get().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Pool error: {e}") })),
        )
    })?;

    let tx = client.transaction().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Transaction error: {e}") })),
        )
    })?;

    // Find any open jobs for this doc_path
    let open_rows = tx
        .query(
            "SELECT job_id FROM ingestion_jobs \
             WHERE doc_path = $1 AND status IN ('QUEUED', 'PROCESSING', 'STAGED') \
             FOR UPDATE;",
            &[&doc_path],
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Query error: {e}") })),
            )
        })?;

    let superseded_job_ids: Vec<Uuid> = open_rows.iter().map(|r| r.get("job_id")).collect();

    if !superseded_job_ids.is_empty() {
        tracing::info!(
            "Superseding {} prior open jobs for path '{}'",
            superseded_job_ids.len(),
            doc_path
        );

        // Mark open jobs as SUPERSEDED
        tx.execute(
            "UPDATE ingestion_jobs \
             SET status = 'SUPERSEDED', updated_at = clock_timestamp() \
             WHERE job_id = ANY($1);",
            &[&superseded_job_ids],
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Supersede update error: {e}") })),
            )
        })?;

        // Purge candidate draft nodes associated with superseded jobs
        tx.execute(
            "DELETE FROM graph_nodes \
             WHERE job_id = ANY($1) AND lifecycle_state = 'DRAFT';",
            &[&superseded_job_ids],
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Draft purge error: {e}") })),
            )
        })?;
    }

    // Step 2: Synchronous Git commit onto refs/heads/specs via dedicated write actor (TB-1).
    let commit_res = state
        .git_write
        .commit_document(&doc_path, &payload.content)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Git commit error: {e}") })),
            )
        })?;

    // Step 3: Insert new QUEUED job record.
    let job_id = Uuid::new_v4();
    tx.execute(
        "INSERT INTO ingestion_jobs (job_id, doc_path, document_hash, status) \
         VALUES ($1, $2, $3, 'QUEUED');",
        &[&job_id, &doc_path, &commit_res.blob_hash],
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Insert job error: {e}") })),
        )
    })?;

    tx.commit().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Commit transaction error: {e}") })),
        )
    })?;

    tracing::info!(
        "Ingestion job {job_id} accepted for '{}' by agent '{}'",
        doc_path,
        caller.agent_id
    );

    let resp = IngestDocumentResponse {
        job_id,
        status: "QUEUED".to_string(),
    };

    Ok((StatusCode::ACCEPTED, Json(resp)).into_response())
}

/// Handles `GET /api/v1/documents/ingest/{job_id}`.
pub async fn get_ingestion_job(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let client = state.pool.get().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Pool error: {e}") })),
        )
    })?;

    let row_opt = client
        .query_opt(
            "SELECT job_id, doc_path, document_hash, status, error_message, retry_count, \
             created_at, updated_at \
             FROM ingestion_jobs \
             WHERE job_id = $1;",
            &[&job_id],
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Query error: {e}") })),
            )
        })?;

    let row = match row_opt {
        Some(r) => r,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("Ingestion job '{job_id}' not found") })),
            ));
        }
    };

    // Query candidate draft nodes directly from graph_nodes without joins (D-71)
    let node_rows = client
        .query(
            "SELECT id, node_key, node_type, title, content, lifecycle_state, \
             governance_policy, created_by, job_id, doc_path, doc_hash, \
             byte_start, byte_end, attributes \
             FROM graph_nodes \
             WHERE job_id = $1 \
             ORDER BY byte_start ASC NULLS LAST;",
            &[&job_id],
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Nodes query error: {e}") })),
            )
        })?;

    let nodes: Vec<GraphNode> = node_rows.iter().map(GraphNode::from_row).collect();

    let response = IngestionJobStatusResponse {
        job_id: row.get("job_id"),
        doc_path: row.get("doc_path"),
        document_hash: row.get("document_hash"),
        status: row.get("status"),
        error_message: row.get("error_message"),
        retry_count: row.get("retry_count"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        nodes,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}
