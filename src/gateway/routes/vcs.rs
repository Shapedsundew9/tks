//! Route handler for VCS merge commit materialization (WP-4.5, TB-12.2, D-108).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::gateway::AppState;
use crate::gateway::auth::AuthenticatedAgent;
use crate::gateway::error::InvalidEdgeTargetEnvelope;
use crate::storage::mutation::{CodeCommitPayload, MutationError};

fn default_author() -> String {
    "system".to_string()
}

/// Request payload for materializing merge commits (TB-12.2, D-108).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VcsCommitPayload {
    pub commit_sha: String,
    #[serde(default = "default_author")]
    pub author: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub merged_at: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub task_ids: Vec<String>,
    #[serde(default)]
    pub implemented_task_ids: Vec<String>,
}

/// Handler for `POST /api/v1/vcs/commits` (TB-12.2, D-108).
pub async fn materialize_commit(
    State(state): State<AppState>,
    auth: AuthenticatedAgent,
    Json(payload): Json<VcsCommitPayload>,
) -> impl IntoResponse {
    let client = match state.storage.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "code": "ERR_DATABASE",
                    "message": format!("Database connection error: {e}")
                })),
            );
        }
    };

    let mut raw_task_ids = payload.task_ids;
    if raw_task_ids.is_empty() {
        raw_task_ids = payload.implemented_task_ids;
    }

    if raw_task_ids.is_empty() {
        let (repairs, candidates) =
            crate::storage::mutation::fetch_target_candidates(&**client).await;
        let envelope = InvalidEdgeTargetEnvelope::new(
            "task_ids",
            "[]",
            "Edge endpoint is not a registered node UUID: at least one task_id required",
            repairs,
            candidates,
        );
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::to_value(&envelope).unwrap_or_default()),
        );
    }

    // Pre-validate that all task IDs parse as valid UUIDs (INV-8, INV-9, D-96, D-97)
    let mut parsed_task_ids = Vec::with_capacity(raw_task_ids.len());
    for raw_id in &raw_task_ids {
        match Uuid::parse_str(raw_id.trim()) {
            Ok(uuid) => parsed_task_ids.push(uuid),
            Err(_) => {
                let (repairs, candidates) =
                    crate::storage::mutation::fetch_target_candidates(&**client).await;
                let envelope = InvalidEdgeTargetEnvelope::new(
                    "task_ids",
                    raw_id,
                    "Edge endpoint is not a registered node UUID",
                    repairs,
                    candidates,
                );
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::to_value(&envelope).unwrap_or_default()),
                );
            }
        }
    }

    let message = payload
        .message
        .unwrap_or_else(|| format!("Merge commit {}", payload.commit_sha));
    let timestamp = payload.timestamp.or(payload.merged_at);

    let commit_payload = CodeCommitPayload {
        commit_sha: payload.commit_sha.clone(),
        author: payload.author,
        message,
        timestamp,
        branch: payload.branch,
        task_ids: parsed_task_ids.clone(),
    };

    match state
        .storage
        .materialize_code_commit(
            &commit_payload,
            &auth.agent_id,
            &auth.actor_type,
            &auth.token_fingerprint(),
        )
        .await
    {
        Ok(result) => (
            StatusCode::CREATED,
            Json(json!({
                "status": "MATERIALIZED",
                "commit_id": result.commit_node.id,
                "commit_sha": payload.commit_sha,
                "implemented_task_ids": parsed_task_ids,
                "event_seq": result.event_seq,
            })),
        ),
        Err(MutationError::InvalidEdgeTarget {
            invalid_field,
            invalid_value,
            message,
            suggested_repairs,
            valid_target_candidates,
        }) => {
            let envelope = InvalidEdgeTargetEnvelope::new(
                invalid_field,
                invalid_value,
                message,
                suggested_repairs,
                valid_target_candidates,
            );
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::to_value(&envelope).unwrap_or_default()),
            )
        }
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "code": e.code(),
                "message": format!("Failed to materialize code commit: {e:?}")
            })),
        ),
    }
}
