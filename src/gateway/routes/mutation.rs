//! REST route handlers for node mutations, subtasks, task status updates,
//! administrative rollbacks, and node reverification (WP-2.4, D-57, D-63, D-73, D-76).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::gateway::AppState;
use crate::gateway::auth::{AuthenticatedAgent, MaybeAuthenticatedAgent};
use crate::storage::governance::TaskStatus;
use crate::storage::mutation::{
    MutationError, elaborate_task, propose_normative_draft, update_task_status,
};
use crate::storage::reverify::reverify_node;
use crate::storage::rollback::{RevertExecutionResult, RevertFilter, revert_mutations};
use crate::storage::workspace::elaborate_in_workspace;

/// Converts a `MutationError` into a structured Axum HTTP response with appropriate status code.
pub fn mutation_error_to_response(err: MutationError) -> (StatusCode, Json<Value>) {
    match err {
        MutationError::NotFound(msg) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "ERR_NOT_FOUND",
                "message": msg
            })),
        ),
        MutationError::CycleDetected { from_id, to_id } => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "ERR_GRAPH_CYCLE_DETECTED",
                "message": format!(
                    "Directed edge from {from_id} to {to_id} would create a directed cycle in structural relationships"
                ),
                "from_id": from_id,
                "to_id": to_id
            })),
        ),
        MutationError::GovernanceLocked(msg) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "ERR_GOVERNANCE_LOCKED",
                "message": msg
            })),
        ),
        MutationError::GovernanceRejected(msg) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "ERR_GOVERNANCE_REJECTED",
                "message": msg
            })),
        ),
        MutationError::InvalidAncestorPath(msg) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "ERR_INVALID_ANCESTOR_PATH",
                "message": msg
            })),
        ),
        MutationError::ConfirmationRequired(preview) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "ERR_CONFIRMATION_REQUIRED",
                "message": "Cross-agent dependent tasks detected; confirmation with force=true required",
                "preview": preview
            })),
        ),
        MutationError::DependencyInactive(msg) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "ERR_DEPENDENCY_INACTIVE",
                "message": msg
            })),
        ),
        MutationError::LockFailure(msg) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "ERR_LOCK_FAILURE",
                "message": msg
            })),
        ),
        MutationError::Database(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "ERR_DATABASE",
                "message": e.to_string()
            })),
        ),
        MutationError::Serialization(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "ERR_SERIALIZATION",
                "message": e.to_string()
            })),
        ),
        MutationError::Pool(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "ERR_POOL",
                "message": e.to_string()
            })),
        ),
        MutationError::Other(msg) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "ERR_MUTATION_FAILED",
                "message": msg
            })),
        ),
    }
}

/// Request payload for `POST /api/v1/nodes/mutate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMutateRequest {
    pub target_node_id: Option<String>,
    pub node_id: Option<String>,
    pub target_id: Option<String>,
    pub mutation_type: String,
    pub proposed_title: Option<String>,
    pub title: Option<String>,
    pub proposed_content: Option<String>,
    pub content: Option<String>,
    pub proposed_attributes: Option<Value>,
    pub attributes: Option<Value>,
    pub edge_type: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<Uuid>,
}

/// Request payload for `POST /api/v1/nodes/{id}/subtasks`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSubtaskRequest {
    pub title: String,
    pub content: Option<String>,
    pub attributes: Option<Value>,
    #[serde(default)]
    pub workspace_id: Option<Uuid>,
}

/// Request payload for `PATCH /api/v1/nodes/{id}/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: String,
    pub notes: Option<String>,
}

/// Request payload for `POST /api/v1/admin/revert-mutations`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RevertMutationsRequest {
    pub batch_id: Option<Uuid>,
    pub agent_id: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub event_seq_range: Option<(i64, i64)>,
    pub dry_run: Option<bool>,
    pub force: Option<bool>,
}

/// Request payload for `POST /api/v1/nodes/{id}/reverify`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReverifyNodeRequest {
    pub rationale: String,
    pub updated_attributes: Option<Value>,
}

/// Handler for `POST /api/v1/nodes/mutate`.
pub async fn mutate_node(
    State(state): State<AppState>,
    caller: AuthenticatedAgent,
    Json(payload): Json<NodeMutateRequest>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let target = payload
        .target_node_id
        .or(payload.node_id)
        .or(payload.target_id)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Missing required field 'target_node_id' or 'node_id'"
                })),
            )
        })?;

    let mut client = state
        .pool
        .get()
        .await
        .map_err(|e| mutation_error_to_response(MutationError::Pool(e)))?;

    let mut_type_upper = payload.mutation_type.to_uppercase();

    if mut_type_upper.contains("TASK")
        || mut_type_upper.contains("SUBTASK")
        || mut_type_upper.contains("ELABORAT")
    {
        let title = payload
            .proposed_title
            .or(payload.title)
            .or_else(|| {
                payload
                    .proposed_attributes
                    .as_ref()
                    .or(payload.attributes.as_ref())
                    .and_then(|a| a.get("title"))
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            })
            .unwrap_or_else(|| "Subtask".to_string());

        let mut attrs = payload
            .proposed_attributes
            .or(payload.attributes)
            .unwrap_or_else(|| serde_json::json!({}));
        if !attrs.is_object() {
            attrs = serde_json::json!({});
        }
        if let Some(et) = payload.edge_type {
            attrs["edge_type"] = serde_json::json!(et);
        }

        let content = payload.proposed_content.or(payload.content);

        let res = if let Some(ws_id) = payload.workspace_id {
            elaborate_in_workspace(
                &mut client,
                ws_id,
                &target,
                &title,
                content.as_deref(),
                Some(attrs),
                &caller,
            )
            .await
            .map_err(mutation_error_to_response)?
        } else {
            elaborate_task(
                &mut client,
                &target,
                &title,
                content.as_deref(),
                Some(attrs),
                &caller,
            )
            .await
            .map_err(mutation_error_to_response)?
        };

        Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({
                "status": "COMMITTED",
                "node_id": res.task_id,
                "task_id": res.task_id,
                "batch_id": res.batch_id,
                "event_seq": res.event_seq,
                "lifecycle_state": res.lifecycle_state,
                "governance_policy": res.governance_policy,
                "node_key": res.node_key
            })),
        )
            .into_response())
    } else if mut_type_upper.contains("STATUS") {
        let status_str = payload
            .proposed_attributes
            .as_ref()
            .or(payload.attributes.as_ref())
            .and_then(|a| a.get("execution_status").or_else(|| a.get("status")))
            .and_then(|v| v.as_str())
            .or(payload.proposed_content.as_deref())
            .or(payload.content.as_deref())
            .unwrap_or("IN_PROGRESS");

        let task_status = status_str
            .parse::<TaskStatus>()
            .map_err(MutationError::GovernanceRejected)
            .map_err(mutation_error_to_response)?;

        let notes = payload
            .proposed_attributes
            .as_ref()
            .or(payload.attributes.as_ref())
            .and_then(|a| a.get("notes"))
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .or(payload.proposed_content)
            .or(payload.content);

        let res = update_task_status(&mut client, &target, task_status, notes.as_deref(), &caller)
            .await
            .map_err(mutation_error_to_response)?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "COMMITTED",
                "node_id": res.task_id,
                "task_id": res.task_id,
                "batch_id": res.batch_id,
                "event_seq": res.event_seq,
                "execution_status": res.execution_status
            })),
        )
            .into_response())
    } else {
        // Normative draft proposal (Pathway 3)
        let title = payload.proposed_title.or(payload.title);
        let content_str = payload
            .proposed_content
            .or(payload.content)
            .unwrap_or_default();
        let attrs = payload.proposed_attributes.or(payload.attributes);

        let res = propose_normative_draft(
            &mut client,
            &target,
            title.as_deref(),
            &content_str,
            attrs,
            &caller,
        )
        .await
        .map_err(mutation_error_to_response)?;

        let batch_id = res.batch_id.unwrap_or(res.draft_id);

        Ok((
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "status": "PENDING_REVIEW",
                "node_id": res.draft_id,
                "draft_id": res.draft_id,
                "target_id": res.target_id,
                "batch_id": batch_id,
                "lifecycle_state": res.lifecycle_state,
                "governance_policy": res.governance_policy
            })),
        )
            .into_response())
    }
}

/// Handler for `POST /api/v1/nodes/{id}/subtasks`.
pub async fn create_subtask_route(
    State(state): State<AppState>,
    Path(parent_id): Path<String>,
    caller: AuthenticatedAgent,
    Json(payload): Json<CreateSubtaskRequest>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let mut client = state
        .pool
        .get()
        .await
        .map_err(|e| mutation_error_to_response(MutationError::Pool(e)))?;

    let res = if let Some(ws_id) = payload.workspace_id {
        elaborate_in_workspace(
            &mut client,
            ws_id,
            &parent_id,
            &payload.title,
            payload.content.as_deref(),
            payload.attributes,
            &caller,
        )
        .await
        .map_err(mutation_error_to_response)?
    } else {
        elaborate_task(
            &mut client,
            &parent_id,
            &payload.title,
            payload.content.as_deref(),
            payload.attributes,
            &caller,
        )
        .await
        .map_err(mutation_error_to_response)?
    };

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "task_id": res.task_id,
            "node_key": res.node_key,
            "status": res.lifecycle_state,
            "lifecycle_state": res.lifecycle_state,
            "batch_id": res.batch_id,
            "event_seq": res.event_seq,
            "governance_policy": res.governance_policy
        })),
    )
        .into_response())
}

/// Handler for `PATCH /api/v1/nodes/{id}/status`.
pub async fn update_status_route(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    caller: AuthenticatedAgent,
    Json(payload): Json<UpdateStatusRequest>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let task_status = payload
        .status
        .parse::<TaskStatus>()
        .map_err(MutationError::GovernanceRejected)
        .map_err(mutation_error_to_response)?;

    let mut client = state
        .pool
        .get()
        .await
        .map_err(|e| mutation_error_to_response(MutationError::Pool(e)))?;

    let res = update_task_status(
        &mut client,
        &node_id,
        task_status,
        payload.notes.as_deref(),
        &caller,
    )
    .await
    .map_err(mutation_error_to_response)?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "UPDATED",
            "task_id": res.task_id,
            "execution_status": res.execution_status,
            "batch_id": res.batch_id,
            "event_seq": res.event_seq
        })),
    )
        .into_response())
}

/// Handler for `POST /api/v1/admin/revert-mutations`.
pub async fn revert_mutations_route(
    State(state): State<AppState>,
    caller: AuthenticatedAgent,
    Json(payload): Json<RevertMutationsRequest>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let filter = RevertFilter {
        batch_id: payload.batch_id,
        agent_id: payload.agent_id,
        since: payload.since,
        event_seq_range: payload.event_seq_range,
        dry_run: payload.dry_run,
        force: payload.force,
    };

    let mut client = state
        .pool
        .get()
        .await
        .map_err(|e| mutation_error_to_response(MutationError::Pool(e)))?;

    let result = revert_mutations(&mut client, filter, &caller)
        .await
        .map_err(mutation_error_to_response)?;

    match result {
        RevertExecutionResult::DryRun(preview) => Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "PREVIEW",
                "reverted_events": 0,
                "total_events": preview.total_events,
                "affected_nodes": preview.affected_nodes,
                "affected_edges": preview.affected_edges,
                "cross_agent_dependencies": preview.cross_agent_dependencies,
                "preview": preview
            })),
        )
            .into_response()),
        RevertExecutionResult::Reverted(reverted) => Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "REVERTED",
                "batch_id": reverted.batch_id,
                "reverted_events": reverted.reverted_events,
                "affected_nodes": reverted.affected_nodes,
                "cascade_reverified_nodes": reverted.cascade_reverified_nodes
            })),
        )
            .into_response()),
    }
}

/// Handler for `POST /api/v1/nodes/{id}/reverify`.
pub async fn reverify_node_route(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    caller: AuthenticatedAgent,
    Json(payload): Json<ReverifyNodeRequest>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let mut client = state
        .pool
        .get()
        .await
        .map_err(|e| mutation_error_to_response(MutationError::Pool(e)))?;

    let res = reverify_node(
        &mut client,
        &node_id,
        &payload.rationale,
        payload.updated_attributes,
        &caller,
    )
    .await
    .map_err(mutation_error_to_response)?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ACTIVE",
            "node_id": res.node_id,
            "staleness_score": res.staleness_score,
            "batch_id": res.batch_id,
            "event_seq": res.event_seq
        })),
    )
        .into_response())
}

/// Query parameters for listing execution tasks.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ListTasksQuery {
    pub parent_id: Option<String>,
    pub status: Option<String>,
    pub limit: Option<u32>,
}

/// Handler for `GET /api/v1/tasks`.
pub async fn list_tasks_route(
    State(state): State<AppState>,
    _caller: MaybeAuthenticatedAgent,
    Query(params): Query<ListTasksQuery>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let client = state
        .pool
        .get()
        .await
        .map_err(|e| mutation_error_to_response(MutationError::Pool(e)))?;

    let limit = params.limit.unwrap_or(50).clamp(1, 100) as i64;

    let parent_uuid = if let Some(ref p) = params.parent_id {
        if let Ok(u) = Uuid::parse_str(p) {
            Some(u)
        } else {
            let row = client
                .query_opt(
                    "SELECT id FROM graph_nodes WHERE node_key = $1 LIMIT 1;",
                    &[p],
                )
                .await
                .map_err(|e| mutation_error_to_response(MutationError::Database(e)))?;
            row.map(|r| r.get::<_, Uuid>("id"))
        }
    } else {
        None
    };

    let rows = if let Some(pid) = parent_uuid {
        if let Some(ref s) = params.status {
            client
                .query(
                    "SELECT g.id, g.node_key, g.node_type, g.title, g.content, g.lifecycle_state, \
                            g.governance_policy, g.created_by, g.attributes \
                     FROM graph_nodes g \
                     JOIN graph_edges e ON e.from_node_id = g.id \
                     WHERE g.node_type = 'TASK' \
                       AND e.to_node_id = $1 \
                       AND (g.lifecycle_state = $2 OR g.attributes->>'execution_status' = $2) \
                     ORDER BY g.id DESC \
                     LIMIT $3;",
                    &[&pid, s, &limit],
                )
                .await
        } else {
            client
                .query(
                    "SELECT g.id, g.node_key, g.node_type, g.title, g.content, g.lifecycle_state, \
                            g.governance_policy, g.created_by, g.attributes \
                     FROM graph_nodes g \
                     JOIN graph_edges e ON e.from_node_id = g.id \
                     WHERE g.node_type = 'TASK' \
                       AND e.to_node_id = $1 \
                     ORDER BY g.id DESC \
                     LIMIT $2;",
                    &[&pid, &limit],
                )
                .await
        }
    } else if let Some(ref s) = params.status {
        client
            .query(
                "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                        governance_policy, created_by, attributes \
                 FROM graph_nodes \
                 WHERE node_type = 'TASK' \
                   AND (lifecycle_state = $1 OR attributes->>'execution_status' = $1) \
                 ORDER BY id DESC \
                 LIMIT $2;",
                &[s, &limit],
            )
            .await
    } else {
        client
            .query(
                "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                        governance_policy, created_by, attributes \
                 FROM graph_nodes \
                 WHERE node_type = 'TASK' \
                 ORDER BY id DESC \
                 LIMIT $1;",
                &[&limit],
            )
            .await
    }
    .map_err(|e| mutation_error_to_response(MutationError::Database(e)))?;

    let tasks: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            let id: Uuid = r.get("id");
            let node_key: Option<String> = r.get("node_key");
            let title: Option<String> = r.get("title");
            let content: Option<String> = r.get("content");
            let lifecycle_state: String = r.get("lifecycle_state");
            let governance_policy: String = r.get("governance_policy");
            let created_by: String = r.get("created_by");
            let attrs: Value = r.get("attributes");
            let exec_status = attrs
                .get("execution_status")
                .and_then(|v| v.as_str())
                .unwrap_or(if lifecycle_state == "ACTIVE" {
                    "OPEN"
                } else {
                    &lifecycle_state
                });

            serde_json::json!({
                "id": id,
                "node_key": node_key,
                "node_type": "TASK",
                "title": title,
                "content": content,
                "lifecycle_state": lifecycle_state,
                "execution_status": exec_status,
                "governance_policy": governance_policy,
                "created_by": created_by,
                "attributes": attrs,
            })
        })
        .collect();

    Ok((StatusCode::OK, Json(tasks)).into_response())
}
