//! Route handlers for pre-merge verification test runs and contract drift detection (WP-4.5, TB-12.1).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::gateway::AppState;
use crate::gateway::auth::AuthenticatedAgent;
use crate::ingest::drift::check_contract_drift;

/// Payload describing a pre-merge CI test run result (D-107, TB-12.1).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TestRunPayload {
    pub test_suite: String,
    #[serde(default)]
    pub passed: Option<bool>,
    #[serde(default)]
    pub status: Option<String>,
    pub pr_commit_sha: String,
    #[serde(default)]
    pub target_task_ids: Vec<Uuid>,
    #[serde(default)]
    pub task_id: Option<Uuid>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub manifest_hash: Option<String>,
    #[serde(default)]
    pub log_summary: Option<String>,
}

/// Payload for inspecting code-side contract drift across diff changes.
#[derive(Debug, Deserialize)]
pub struct CheckDriftPayload {
    pub diff: String,
    #[serde(default)]
    pub task_ids: Vec<Uuid>,
}

/// Handler for `POST /api/v1/verification/test-run` (TB-12.1, D-107).
pub async fn record_test_run(
    State(state): State<AppState>,
    auth: AuthenticatedAgent,
    Json(payload): Json<TestRunPayload>,
) -> impl IntoResponse {
    let passed = payload
        .passed
        .or_else(|| {
            payload
                .status
                .as_ref()
                .map(|s| s.eq_ignore_ascii_case("PASSED"))
        })
        .unwrap_or(false);

    let mut target_task_ids = payload.target_task_ids;
    if target_task_ids.is_empty() {
        target_task_ids.extend(payload.task_id);
    }

    if target_task_ids.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "code": "ERR_INVALID_REQUEST",
                "message": "At least one target task ID must be provided in target_task_ids"
            })),
        );
    }

    let test_run_metadata = json!({
        "test_suite": payload.test_suite,
        "passed": passed,
        "pr_commit_sha": payload.pr_commit_sha,
        "duration_ms": payload.duration_ms,
        "manifest_hash": payload.manifest_hash,
        "log_summary": payload.log_summary,
        "recorded_at": chrono::Utc::now().to_rfc3339(),
    });

    let mut recorded_tasks = Vec::new();

    for task_id in target_task_ids {
        match state
            .storage
            .record_pre_merge_verification(
                task_id,
                &test_run_metadata,
                &auth.agent_id,
                &auth.actor_type,
                &auth.token_fingerprint(),
            )
            .await
        {
            Ok(node) => recorded_tasks.push(node.id),
            Err(e) => {
                let code = e.code();
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "code": code,
                        "message": format!("Failed to record verification for task {task_id}: {e:?}")
                    })),
                );
            }
        }
    }

    (
        StatusCode::OK,
        Json(json!({
            "status": "RECORDED",
            "test_suite": payload.test_suite,
            "passed": passed,
            "pr_commit_sha": payload.pr_commit_sha,
            "recorded_task_ids": recorded_tasks,
        })),
    )
}

/// Handler for `POST /api/v1/verification/check-drift`.
pub async fn check_drift_route(
    State(state): State<AppState>,
    _auth: AuthenticatedAgent,
    Json(payload): Json<CheckDriftPayload>,
) -> impl IntoResponse {
    let client = match state.storage.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "ERR_DATABASE",
                    "message": format!("Database connection error: {e}")
                })),
            );
        }
    };

    match check_contract_drift(&**client, &payload.diff, &payload.task_ids).await {
        Ok(res) => (StatusCode::OK, Json(json!(res))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": "ERR_DATABASE",
                "message": format!("Failed to evaluate contract drift: {e}")
            })),
        ),
    }
}
