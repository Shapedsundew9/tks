//! Route handler for release readiness inspection (WP-4.5, TB-12.3, D-113, D-120).

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::gateway::AppState;

/// Query parameters for `GET /api/v1/release/readiness` (D-113, D-120).
#[derive(Debug, Deserialize)]
pub struct ReleaseReadinessQuery {
    /// Evaluation scope: "pre-merge" or "release" (default: "pre-merge").
    pub scope: Option<String>,
    /// Pull request or candidate commit SHA (required for scope=pre-merge).
    pub pr_commit_sha: Option<String>,
    /// Comma-separated list of target task UUIDs.
    pub task_ids: Option<String>,
    /// Release milestone tag (for scope=release).
    pub milestone: Option<String>,
    /// Optional root requirement identifier.
    pub requirement_root_id: Option<Uuid>,
}

/// Handler for `GET /api/v1/release/readiness` (TB-12.3, D-113, D-120).
pub async fn get_release_readiness(
    State(state): State<AppState>,
    Query(query): Query<ReleaseReadinessQuery>,
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

    let scope = query
        .scope
        .as_deref()
        .map(str::to_lowercase)
        .unwrap_or_else(|| "pre-merge".to_string());

    if scope == "pre-merge" {
        evaluate_pre_merge_readiness(&**client, query).await
    } else if scope == "release" {
        evaluate_milestone_release_readiness(&**client, query).await
    } else {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "code": "ERR_INVALID_SCOPE",
                "message": format!("Invalid readiness scope '{scope}', expected 'pre-merge' or 'release'")
            })),
        )
    }
}

/// Evaluates pull request readiness prior to merge (D-113, D-120).
async fn evaluate_pre_merge_readiness(
    client: &(impl tokio_postgres::GenericClient + ?Sized),
    query: ReleaseReadinessQuery,
) -> (StatusCode, Json<serde_json::Value>) {
    let pr_sha = match query.pr_commit_sha.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => s,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "code": "ERR_INVALID_REQUEST",
                    "message": "Query parameter 'pr_commit_sha' is required for scope=pre-merge"
                })),
            );
        }
    };

    // Determine target task UUIDs
    let mut target_task_ids = Vec::new();

    if let Some(ref raw_ids) = query.task_ids {
        for part in raw_ids.split(',') {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                continue;
            }
            match Uuid::parse_str(trimmed) {
                Ok(id) => target_task_ids.push(id),
                Err(_) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "code": "ERR_INVALID_UUID",
                            "message": format!("Invalid task UUID in task_ids: '{trimmed}'")
                        })),
                    );
                }
            }
        }
    } else {
        // Fallback: scan active tasks touching pr_commit_sha in vcs_commits or pre_merge_verification (D-120)
        let rows = match client
            .query(
                "SELECT id FROM graph_nodes \
                 WHERE lifecycle_state = 'ACTIVE' \
                   AND node_type = 'TASK' \
                   AND ( \
                       (attributes->'vcs_commits') @> to_jsonb($1::text) \
                       OR EXISTS ( \
                           SELECT 1 FROM jsonb_array_elements(COALESCE(attributes->'pre_merge_verification', '[]'::jsonb)) elem \
                           WHERE elem->>'pr_commit_sha' = $1 \
                       ) \
                   );",
                &[&pr_sha],
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "code": "ERR_DATABASE",
                        "message": format!("Failed to query matching tasks: {e}")
                    })),
                );
            }
        };

        for row in rows {
            target_task_ids.push(row.get("id"));
        }

        if target_task_ids.is_empty() {
            return (
                StatusCode::OK,
                Json(json!({
                    "scope": "pre-merge",
                    "status": "BLOCKED",
                    "unverified_task_ids": [],
                    "message": format!("No active tasks associated with candidate PR commit SHA '{pr_sha}'")
                })),
            );
        }
    }

    if target_task_ids.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "code": "ERR_EMPTY_TASK_LIST",
                "message": "At least one task ID is required for pre-merge readiness evaluation"
            })),
        );
    }

    let mut unverified_task_ids = Vec::new();
    let mut verified_task_ids = Vec::new();

    for task_id in &target_task_ids {
        let row_opt = match client
            .query_opt(
                "SELECT attributes, lifecycle_state, node_type FROM graph_nodes WHERE id = $1;",
                &[task_id],
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "code": "ERR_DATABASE",
                        "message": format!("Failed to inspect task {task_id}: {e}")
                    })),
                );
            }
        };

        let is_verified = match row_opt {
            Some(row) => {
                let state: String = row.get("lifecycle_state");
                let ntype: String = row.get("node_type");
                if (state != "ACTIVE" && state != "NEEDS_REVERIFICATION") || ntype != "TASK" {
                    false
                } else {
                    let attrs: serde_json::Value = row.get("attributes");
                    attrs
                        .get("pre_merge_verification")
                        .and_then(|v| v.as_array())
                        .map(|runs| {
                            runs.iter().any(|run| {
                                run.get("pr_commit_sha")
                                    .and_then(|s| s.as_str())
                                    .is_some_and(|sha| sha == pr_sha)
                                    && (run
                                        .get("passed")
                                        .and_then(|b| b.as_bool())
                                        .unwrap_or(false)
                                        || run
                                            .get("status")
                                            .and_then(|s| s.as_str())
                                            .is_some_and(|s| s.eq_ignore_ascii_case("PASSED")))
                            })
                        })
                        .unwrap_or(false)
                }
            }
            None => false,
        };

        if is_verified {
            verified_task_ids.push(*task_id);
        } else {
            unverified_task_ids.push(*task_id);
        }
    }

    let status = if unverified_task_ids.is_empty() && !target_task_ids.is_empty() {
        "READY"
    } else {
        "BLOCKED"
    };

    (
        StatusCode::OK,
        Json(json!({
            "scope": "pre-merge",
            "status": status,
            "pr_commit_sha": pr_sha,
            "unverified_task_ids": unverified_task_ids,
            "verified_task_ids": verified_task_ids,
        })),
    )
}

/// Evaluates post-merge release milestone readiness via recursive CTE traversal (D-113, TB-12.3).
async fn evaluate_milestone_release_readiness(
    client: &(impl tokio_postgres::GenericClient + ?Sized),
    query: ReleaseReadinessQuery,
) -> (StatusCode, Json<serde_json::Value>) {
    let milestone_tag = query.milestone.as_deref().unwrap_or("latest");

    // 1. Identify active requirement roots in scope
    let req_rows = if let Some(root_id) = query.requirement_root_id {
        match client
            .query(
                "SELECT id, node_key, title FROM graph_nodes \
                 WHERE id = $1 AND node_type = 'REQUIREMENT' AND lifecycle_state = 'ACTIVE';",
                &[&root_id],
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "code": "ERR_DATABASE",
                        "message": format!("Failed to query root requirement: {e}")
                    })),
                );
            }
        }
    } else {
        // Query active requirements (matching milestone attribute if tagged, otherwise all active requirements)
        let tagged_rows: Vec<tokio_postgres::Row> = client
            .query(
                "SELECT id, node_key, title FROM graph_nodes \
                 WHERE node_type = 'REQUIREMENT' AND lifecycle_state = 'ACTIVE' \
                   AND (attributes->>'milestone') = $1;",
                &[&milestone_tag],
            )
            .await
            .unwrap_or_default();

        if !tagged_rows.is_empty() {
            tagged_rows
        } else {
            match client
                .query(
                    "SELECT id, node_key, title FROM graph_nodes \
                     WHERE node_type = 'REQUIREMENT' AND lifecycle_state = 'ACTIVE';",
                    &[],
                )
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "code": "ERR_DATABASE",
                            "message": format!("Failed to query active requirements: {e}")
                        })),
                    );
                }
            }
        }
    };

    let mut satisfied_requirements = Vec::new();
    let mut uncommitted_tasks = Vec::new();
    let mut total_tasks_count = 0usize;
    let mut committed_tasks_count = 0usize;

    for req_row in &req_rows {
        let req_id: Uuid = req_row.get("id");
        let req_key: Option<String> = req_row.get("node_key");
        let req_display = req_key.unwrap_or_else(|| req_id.to_string());

        // 2. Downward recursive walk to find all child TASK nodes
        let task_rows = match client
            .query(
                "WITH RECURSIVE downward_tasks AS ( \
                     SELECT \
                         e.from_node_id AS child_id, \
                         n.node_type AS child_type \
                     FROM graph_edges e \
                     JOIN graph_nodes n ON n.id = e.from_node_id AND n.lifecycle_state = 'ACTIVE' \
                     WHERE e.to_node_id = $1 AND e.lifecycle_state = 'ACTIVE' \
                     UNION ALL \
                     SELECT \
                         e.from_node_id AS child_id, \
                         n.node_type AS child_type \
                     FROM downward_tasks dt \
                     JOIN graph_edges e ON e.to_node_id = dt.child_id AND e.lifecycle_state = 'ACTIVE' \
                     JOIN graph_nodes n ON n.id = e.from_node_id AND n.lifecycle_state = 'ACTIVE' \
                     WHERE dt.child_type != 'TASK' \
                 ) \
                 SELECT DISTINCT dt.child_id AS task_id \
                 FROM downward_tasks dt \
                 WHERE dt.child_type = 'TASK';",
                &[&req_id],
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "code": "ERR_DATABASE",
                        "message": format!("Failed to find tasks for requirement {req_id}: {e}")
                    })),
                );
            }
        };

        let mut req_all_committed = true;

        for task_row in &task_rows {
            total_tasks_count += 1;
            let task_id: Uuid = task_row.get("task_id");

            // 3. Verify terminating upward IMPLEMENTS edge from canonical CODE_COMMIT node (D-108, TB-12.3)
            let is_committed_row = match client
                .query_one(
                    "SELECT EXISTS ( \
                         SELECT 1 FROM graph_edges e \
                         JOIN graph_nodes c ON c.id = e.from_node_id \
                         WHERE e.to_node_id = $1 \
                           AND e.edge_type = 'IMPLEMENTS' \
                           AND e.lifecycle_state = 'ACTIVE' \
                           AND c.node_type = 'CODE_COMMIT' \
                           AND c.lifecycle_state = 'ACTIVE' \
                     ) AS is_committed;",
                    &[&task_id],
                )
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "code": "ERR_DATABASE",
                            "message": format!("Failed to check commit for task {task_id}: {e}")
                        })),
                    );
                }
            };

            let is_committed: bool = is_committed_row.get("is_committed");
            if is_committed {
                committed_tasks_count += 1;
            } else {
                req_all_committed = false;
                if !uncommitted_tasks.contains(&task_id) {
                    uncommitted_tasks.push(task_id);
                }
            }
        }

        if req_all_committed && !task_rows.is_empty() {
            satisfied_requirements.push(req_display);
        }
    }

    let readiness_percentage = if total_tasks_count == 0 {
        if !req_rows.is_empty() { 100.0 } else { 0.0 }
    } else {
        (committed_tasks_count as f64 / total_tasks_count as f64) * 100.0
    };

    let status = if uncommitted_tasks.is_empty()
        && (!satisfied_requirements.is_empty() || total_tasks_count == 0)
    {
        "READY"
    } else {
        "BLOCKED"
    };

    (
        StatusCode::OK,
        Json(json!({
            "scope": "release",
            "milestone": milestone_tag,
            "status": status,
            "satisfied_requirements": satisfied_requirements,
            "uncommitted_tasks": uncommitted_tasks,
            "total_tasks": total_tasks_count,
            "committed_tasks": committed_tasks_count,
            "readiness_percentage": readiness_percentage,
        })),
    )
}
