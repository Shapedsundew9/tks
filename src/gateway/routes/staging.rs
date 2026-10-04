//! Staging review, approval, and rejection REST routes (WP-1.5, D-46, D-47, D-61, D-65, D-74, D-82, D-83).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::gateway::AppState;
use crate::gateway::auth::{AuthenticatedAgent, MaybeAuthenticatedAgent};
use crate::ingest::reconcile::CandidateSpanReanchor;
use crate::storage::GraphNode;
use crate::storage::envelope::validate_ancestor_path;

/// Payload for approving an ingestion job and promoting candidate drafts to `ACTIVE`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StagingApproveRequest {
    /// Ingestion job ID (optional if specified in URL path).
    pub job_id: Option<Uuid>,
    /// Optional subset of candidate node IDs to approve. If omitted, all candidate drafts for the job are approved.
    pub approved_node_ids: Option<Vec<Uuid>>,
}

/// Response returned on successful staging approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagingApproveResponse {
    pub approved_nodes: usize,
    pub purged_drafts: usize,
    pub batch_id: Uuid,
}

/// Payload for rejecting an ingestion job or specific candidate drafts.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StagingRejectRequest {
    /// Ingestion job ID (optional if specified in URL path).
    pub job_id: Option<Uuid>,
    /// Optional rejection reason for diagnostics.
    pub reason: Option<String>,
    /// Optional specific draft node IDs to reject. If omitted, all candidate drafts for the job are rejected.
    pub rejected_node_ids: Option<Vec<Uuid>>,
}

/// Response returned on staging rejection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagingRejectResponse {
    pub rejected_job_id: Uuid,
    pub purged_drafts: usize,
}

/// Response returned on node inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectNodeResponse {
    pub node: GraphNode,
    pub span_text: Option<String>,
}

/// Handler for `POST /api/v1/documents/ingest/{job_id}/approve`.
pub async fn approve_ingestion_job(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
    caller: AuthenticatedAgent,
    Json(payload): Json<StagingApproveRequest>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    execute_approve(state, job_id, payload.approved_node_ids, caller).await
}

/// Handler for backward-compatible alias `POST /api/v1/staging/approve`.
pub async fn approve_staging_alias(
    State(state): State<AppState>,
    caller: AuthenticatedAgent,
    Json(payload): Json<StagingApproveRequest>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let job_id = if let Some(jid) = payload.job_id {
        jid
    } else if let Some(ref ids) = payload.approved_node_ids {
        if let Some(first_id) = ids.first() {
            let client = state.pool.get().await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Pool error: {e}") })),
                )
            })?;
            let row = client
                .query_opt("SELECT job_id FROM graph_nodes WHERE id = $1;", &[first_id])
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": format!("Query error: {e}") })),
                    )
                })?;
            match row.and_then(|r| r.get::<_, Option<Uuid>>("job_id")) {
                Some(jid) => jid,
                None => {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(
                            serde_json::json!({ "error": "Candidate draft has no associated job_id" }),
                        ),
                    ));
                }
            }
        } else {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({ "error": "Missing 'job_id' or 'approved_node_ids' in approve payload" }),
                ),
            ));
        }
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Missing 'job_id' in approve payload" })),
        ));
    };

    execute_approve(state, job_id, payload.approved_node_ids, caller).await
}

/// Core staging approval execution logic inside a single transaction with advisory locks.
async fn execute_approve(
    state: AppState,
    job_id: Uuid,
    explicit_approved_node_ids: Option<Vec<Uuid>>,
    caller: AuthenticatedAgent,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
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

    // Step 1: Enforce strict lock acquisition hierarchy (D-66, LD-5).
    // Acquire global transaction advisory lock before any row updates.
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtext('tks_structural_mutation'));",
        &[],
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Advisory lock error: {e}") })),
        )
    })?;

    // Step 2: Fetch job row and document path.
    let job_row = tx
        .query_opt(
            "SELECT doc_path, status, attributes FROM ingestion_jobs WHERE job_id = $1 FOR UPDATE;",
            &[&job_id],
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Query job error: {e}") })),
            )
        })?;

    let job_row = match job_row {
        Some(r) => r,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("Ingestion job '{job_id}' not found") })),
            ));
        }
    };

    let doc_path: String = job_row.get("doc_path");
    let status: String = job_row.get("status");
    let job_attributes: serde_json::Value = job_row.get("attributes");

    if status == "APPROVED" {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": format!("Job '{job_id}' is already approved") })),
        ));
    }

    // Acquire secondary document advisory lock
    tx.execute("SELECT pg_advisory_xact_lock(hashtext($1));", &[&doc_path])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Doc lock error: {e}") })),
            )
        })?;

    // Step 3: Identify candidate draft nodes for this job.
    let candidate_rows = tx
        .query(
            "SELECT id, node_key, title, content, attributes \
             FROM graph_nodes \
             WHERE job_id = $1 AND lifecycle_state = 'DRAFT';",
            &[&job_id],
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Query candidate drafts error: {e}") })),
            )
        })?;

    let all_candidate_ids: Vec<Uuid> = candidate_rows.iter().map(|r| r.get("id")).collect();

    let (approved_node_ids, purged_count) = if let Some(only_ids) = explicit_approved_node_ids {
        // Granular approval: purge unapproved candidate drafts (D-46).
        let purge_rows = tx
            .query(
                "DELETE FROM graph_nodes \
                 WHERE job_id = $1 AND lifecycle_state = 'DRAFT' AND id != ALL($2) \
                 RETURNING id;",
                &[&job_id, &only_ids],
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Purge drafts error: {e}") })),
                )
            })?;

        let approved: Vec<Uuid> = all_candidate_ids
            .into_iter()
            .filter(|id| only_ids.contains(id))
            .collect();

        (approved, purge_rows.len())
    } else {
        (all_candidate_ids, 0)
    };

    // Step 4: Validate Invariant INV-1 ancestor paths for candidate nodes (D-34, DEC-1.4).
    if !approved_node_ids.is_empty() {
        let is_valid = validate_ancestor_path(&*tx, &approved_node_ids, false)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Ancestor validation CTE error: {e}") })),
                )
            })?;

        if !is_valid {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": "Invariant INV-1 violated: candidate nodes lack unbroken upward path to an active requirement root"
                })),
            ));
        }
        // Rebind tx from client_ref
        // Since validate_ancestor_path takes &mut Client and deadpool Transaction derefs to Client, tx is retained.
    }

    let batch_id = Uuid::new_v4();

    // Step 5: Apply candidate span re-anchoring in place on matching active graph_nodes (D-74, TB-7.5).
    if let Some(candidate_reanchors) = job_attributes
        .get("candidate_reanchoring")
        .and_then(|v| v.as_array())
    {
        for item in candidate_reanchors {
            if let Ok(reanchor) = serde_json::from_value::<CandidateSpanReanchor>(item.clone()) {
                // Update active node embedded spans in place
                tx.execute(
                    "UPDATE graph_nodes \
                     SET doc_hash = $1, byte_start = $2, byte_end = $3 \
                     WHERE id = $4 AND lifecycle_state = 'ACTIVE';",
                    &[
                        &reanchor.doc_hash,
                        &reanchor.new_byte_start,
                        &reanchor.new_byte_end,
                        &reanchor.target_node_id,
                    ],
                )
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": format!("Update span error: {e}") })),
                    )
                })?;

                // Append SPAN_REANCHORED audit event to audit_ledger
                let delta = serde_json::json!({
                    "doc_hash": reanchor.doc_hash,
                    "old_byte_start": reanchor.old_byte_start,
                    "old_byte_end": reanchor.old_byte_end,
                    "new_byte_start": reanchor.new_byte_start,
                    "new_byte_end": reanchor.new_byte_end,
                    "ast_anchor": reanchor.ast_anchor,
                });

                tx.execute(
                    "INSERT INTO audit_ledger ( \
                        batch_id, event_type, entity_id, entity_type, \
                        actor_id, actor_type, token_fingerprint, delta, snapshot \
                     ) VALUES ($1, 'SPAN_REANCHORED', $2, 'GRAPH_NODE', $3, $4, $5, $6, $7);",
                    &[
                        &batch_id,
                        &reanchor.target_node_id,
                        &caller.agent_id,
                        &caller.actor_type,
                        &caller.agent_id,
                        &delta,
                        &delta,
                    ],
                )
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": format!("Audit reanchor error: {e}") })),
                    )
                })?;
            }
        }
    }

    // Step 6: Supersede replaced active nodes and purge obsolete embeddings (D-47, D-55, D-59).
    for row in &candidate_rows {
        let node_id: Uuid = row.get("id");
        if !approved_node_ids.contains(&node_id) {
            continue;
        }

        let attrs: serde_json::Value = row.get("attributes");
        if let Some(replaces_id) = attrs
            .get("replaces_node_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
        {
            // Supersede previous active node
            tx.execute(
                "UPDATE graph_nodes SET lifecycle_state = 'SUPERSEDED' WHERE id = $1;",
                &[&replaces_id],
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Supersede node error: {e}") })),
                )
            })?;

            // Purge obsolete embeddings for superseded nodes (D-59)
            tx.execute(
                "DELETE FROM node_embeddings WHERE node_id = $1;",
                &[&replaces_id],
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Purge embeddings error: {e}") })),
                )
            })?;
        }
    }

    // Step 7: Dual-endpoint active presence on draft edges and orphan edge purge (D-47).
    tx.execute(
        "UPDATE graph_edges \
         SET lifecycle_state = 'ACTIVE' \
         WHERE lifecycle_state = 'DRAFT' \
           AND (from_node_id IN (SELECT id FROM graph_nodes WHERE lifecycle_state = 'ACTIVE') OR from_node_id = ANY($1)) \
           AND (to_node_id IN (SELECT id FROM graph_nodes WHERE lifecycle_state = 'ACTIVE') OR to_node_id = ANY($1));",
        &[&approved_node_ids],
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Edge promotion error: {e}") })),
        )
    })?;

    tx.execute(
        "DELETE FROM graph_edges \
         WHERE lifecycle_state = 'DRAFT';",
        &[],
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Orphan draft edge purge error: {e}") })),
        )
    })?;

    // Step 8: Squash draft revisions into draft_evolution_summary (D-61).
    let mut total_revisions_squashed = 0;
    for row in &candidate_rows {
        let node_id: Uuid = row.get("id");
        if !approved_node_ids.contains(&node_id) {
            continue;
        }

        let attrs: serde_json::Value = row.get("attributes");
        if let Some(revs) = attrs.get("draft_revisions").and_then(|v| v.as_array()) {
            total_revisions_squashed += revs.len();
        }
    }

    let draft_evolution_summary = serde_json::json!({
        "approved_by": caller.agent_id,
        "actor_type": caller.actor_type,
        "node_count": approved_node_ids.len(),
        "revisions_squashed": total_revisions_squashed,
        "timestamp": chrono::Utc::now(),
    });

    // Step 8.5: Enforce canonical node_key uniqueness before active promotion (DEC-1.17, D-58).
    // (a) Clear node_key on candidate drafts if already active in graph_nodes outside this batch.
    tx.execute(
        "UPDATE graph_nodes \
         SET node_key = NULL \
         WHERE id = ANY($1) \
           AND node_key IS NOT NULL \
           AND node_key IN (SELECT node_key FROM graph_nodes WHERE lifecycle_state = 'ACTIVE' AND node_key IS NOT NULL AND id != ALL($1));",
        &[&approved_node_ids],
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Dedup active keys error: {e}") })),
        )
    })?;

    // (b) Clear duplicate node_keys within the batch itself, keeping only the first node for each key.
    tx.execute(
        "UPDATE graph_nodes g \
         SET node_key = NULL \
         WHERE g.id = ANY($1) \
           AND g.node_key IS NOT NULL \
           AND g.id NOT IN ( \
               SELECT DISTINCT ON (node_key) id \
               FROM graph_nodes \
               WHERE id = ANY($1) AND node_key IS NOT NULL \
               ORDER BY node_key, byte_start ASC NULLS LAST, id ASC \
           );",
        &[&approved_node_ids],
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Dedup batch keys error: {e}") })),
        )
    })?;

    // Step 9: Promote approved candidate nodes to ACTIVE, defaulting UNCLASSIFIED to SPECIFICATION (D-61, D-62).
    tx.execute(
        "UPDATE graph_nodes \
         SET lifecycle_state = 'ACTIVE', \
             node_type = CASE WHEN node_type = 'UNCLASSIFIED' THEN 'SPECIFICATION' ELSE node_type END, \
             attributes = attributes - 'draft_revisions' \
         WHERE id = ANY($1);",
        &[&approved_node_ids],
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!(
                    "Promote nodes error: {} (detail: {:?}, constraint: {:?}, message: {})",
                    e,
                    e.as_db_error().and_then(|d| d.detail()),
                    e.as_db_error().and_then(|d| d.constraint()),
                    e.as_db_error().map(|d| d.message()).unwrap_or("")
                )
            })),
        )
    })?;

    // Step 10: Enqueue vector embeddings strictly for promoted ACTIVE nodes (D-82).
    for row in &candidate_rows {
        let node_id: Uuid = row.get("id");
        if !approved_node_ids.contains(&node_id) {
            continue;
        }

        let title: Option<String> = row.get("title");
        let content: Option<String> = row.get("content");
        let combined_text = format!(
            "{} {}",
            title.as_deref().unwrap_or(""),
            content.as_deref().unwrap_or("")
        );

        let mut hasher = Sha256::new();
        hasher.update(combined_text.as_bytes());
        let content_hash = format!("{:x}", hasher.finalize());

        tx.execute(
            "INSERT INTO node_embeddings (node_id, content_hash, status, scheduled_at) \
             VALUES ($1, $2, 'PENDING', clock_timestamp()) \
             ON CONFLICT (node_id) DO UPDATE SET \
                content_hash = EXCLUDED.content_hash, \
                status = 'PENDING', \
                scheduled_at = clock_timestamp();",
            &[&node_id, &content_hash],
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Enqueue embeddings error: {e}") })),
            )
        })?;
    }

    // Step 11: Write canonical APPROVED audit event into audit_ledger (D-65).
    let approved_delta = serde_json::json!({
        "approved_node_ids": approved_node_ids,
        "purged_draft_count": purged_count,
    });
    let snapshot = serde_json::json!({
        "job_id": job_id,
        "doc_path": doc_path,
        "approved_count": approved_node_ids.len(),
    });

    let audit_row = tx
        .query_one(
            "INSERT INTO audit_ledger ( \
            batch_id, event_type, entity_id, entity_type, \
            actor_id, actor_type, token_fingerprint, delta, snapshot, draft_evolution_summary \
         ) VALUES ($1, 'APPROVED', $2, 'INGESTION_JOB', $3, $4, $5, $6, $7, $8) \
         RETURNING event_seq, created_at;",
            &[
                &batch_id,
                &job_id,
                &caller.agent_id,
                &caller.actor_type,
                &caller.agent_id,
                &approved_delta,
                &snapshot,
                &draft_evolution_summary,
            ],
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Audit approved error: {e}") })),
            )
        })?;

    let event_seq: i64 = audit_row.get("event_seq");
    let timestamp: chrono::DateTime<chrono::Utc> = audit_row.get("created_at");

    // Step 12: Transition ingestion_jobs status to APPROVED.
    tx.execute(
        "UPDATE ingestion_jobs \
         SET status = 'APPROVED', updated_at = clock_timestamp() \
         WHERE job_id = $1;",
        &[&job_id],
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Update job status error: {e}") })),
        )
    })?;

    tx.commit().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Commit error: {e}") })),
        )
    })?;

    tracing::info!(
        "Approved job {job_id} with {} nodes (purged {} drafts, batch: {batch_id})",
        approved_node_ids.len(),
        purged_count
    );

    // Publish STAGING_APPROVED event to event bus so connected Web Explorer clients reload via SSE
    let first_node_id = approved_node_ids.first().copied().unwrap_or(job_id);
    state
        .event_bus
        .publish(crate::storage::event_bus::GraphChangeEvent {
            event_seq,
            batch_id,
            event_type: "STAGING_APPROVED".to_string(),
            entity_id: first_node_id,
            entity_type: "INGESTION_JOB".to_string(),
            actor_id: caller.agent_id.clone(),
            timestamp,
        });

    let resp = StagingApproveResponse {
        approved_nodes: approved_node_ids.len(),
        purged_drafts: purged_count,
        batch_id,
    };

    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Handler for `DELETE /api/v1/documents/ingest/{job_id}`.
pub async fn reject_ingestion_job(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
    caller: AuthenticatedAgent,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    execute_reject(state, job_id, None, caller).await
}

/// Handler for backward-compatible alias `POST /api/v1/staging/reject`.
pub async fn reject_staging_alias(
    State(state): State<AppState>,
    caller: AuthenticatedAgent,
    Json(payload): Json<StagingRejectRequest>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let job_id = if let Some(jid) = payload.job_id {
        jid
    } else if let Some(ids) = &payload.rejected_node_ids {
        if let Some(first_id) = ids.first() {
            let client = state.pool.get().await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Pool error: {e}") })),
                )
            })?;
            let row = client
                .query_opt("SELECT job_id FROM graph_nodes WHERE id = $1;", &[first_id])
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": format!("Query error: {e}") })),
                    )
                })?;
            match row.and_then(|r| r.get::<_, Option<Uuid>>("job_id")) {
                Some(jid) => jid,
                None => {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(
                            serde_json::json!({ "error": "Could not determine job_id for draft node" }),
                        ),
                    ));
                }
            }
        } else {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({ "error": "Missing job_id or rejected_node_ids in reject payload" }),
                ),
            ));
        }
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Missing 'job_id' in reject payload" })),
        ));
    };

    execute_reject(state, job_id, payload.rejected_node_ids, caller).await
}

/// Core staging rejection execution logic (D-83).
async fn execute_reject(
    state: AppState,
    job_id: Uuid,
    specific_node_ids: Option<Vec<Uuid>>,
    caller: AuthenticatedAgent,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
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

    let purged_count = if let Some(ids) = specific_node_ids {
        // Specific draft rejection
        let rows = tx
            .query(
                "DELETE FROM graph_nodes \
                 WHERE id = ANY($1) AND lifecycle_state = 'DRAFT' \
                 RETURNING id;",
                &[&ids],
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Purge drafts error: {e}") })),
                )
            })?;
        rows.len()
    } else {
        // Full job rejection: purge all candidate drafts for job_id
        let rows = tx
            .query(
                "DELETE FROM graph_nodes \
                 WHERE job_id = $1 AND lifecycle_state = 'DRAFT' \
                 RETURNING id;",
                &[&job_id],
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Purge drafts error: {e}") })),
                )
            })?;

        // Update ingestion_jobs status to REJECTED
        tx.execute(
            "UPDATE ingestion_jobs \
             SET status = 'REJECTED', updated_at = clock_timestamp() \
             WHERE job_id = $1;",
            &[&job_id],
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Update job error: {e}") })),
            )
        })?;

        rows.len()
    };

    tx.commit().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Commit error: {e}") })),
        )
    })?;

    tracing::info!(
        "Rejected job {job_id} by '{}' (purged {} drafts)",
        caller.agent_id,
        purged_count
    );

    let resp = StagingRejectResponse {
        rejected_job_id: job_id,
        purged_drafts: purged_count,
    };

    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Handles `GET /api/v1/nodes/{id}` and `GET /api/v1/staging/inspect/{id}`.
///
/// Resolves a node polymorphically (by UUID or canonical key), and if document span
/// coordinates are present, fetches verbatim text from Git ODB.
pub async fn inspect_node(
    State(state): State<AppState>,
    Path(id_str): Path<String>,
    _caller: MaybeAuthenticatedAgent,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let client = state.pool.get().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Pool error: {e}") })),
        )
    })?;

    let node_opt = if let Ok(uuid) = Uuid::parse_str(&id_str) {
        let row_opt = client
            .query_opt(
                "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                 governance_policy, created_by, job_id, doc_path, doc_hash, \
                 byte_start, byte_end, attributes \
                 FROM graph_nodes \
                 WHERE id = $1;",
                &[&uuid],
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Query error: {e}") })),
                )
            })?;
        row_opt.map(|r| GraphNode::from_row(&r))
    } else {
        let row_opt = client
            .query_opt(
                "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                 governance_policy, created_by, job_id, doc_path, doc_hash, \
                 byte_start, byte_end, attributes \
                 FROM graph_nodes \
                 WHERE node_key = $1 \
                 ORDER BY CASE WHEN lifecycle_state = 'ACTIVE' THEN 0 ELSE 1 END \
                 LIMIT 1;",
                &[&id_str],
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Query error: {e}") })),
                )
            })?;
        row_opt.map(|r| GraphNode::from_row(&r))
    };

    let node = match node_opt {
        Some(n) => n,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("Node '{id_str}' not found") })),
            ));
        }
    };

    let span_text = if let (Some(doc_hash), Some(start), Some(end)) =
        (&node.doc_hash, node.byte_start, node.byte_end)
    {
        if start >= 0 && end >= start {
            state
                .git_read
                .read_blob_span(doc_hash, start as usize, end as usize)
                .await
                .ok()
        } else {
            None
        }
    } else {
        None
    };

    let resp = InspectNodeResponse { node, span_text };
    Ok((StatusCode::OK, Json(resp)).into_response())
}
