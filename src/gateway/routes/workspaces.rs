//! REST route handlers for multi-agent workspace containers (WP-3.2, PHASE3-003).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::gateway::AppState;
use crate::gateway::auth::AuthenticatedAgent;
use crate::gateway::routes::mutation::mutation_error_to_response;
use crate::storage::workspace::{
    create_workspace_with_attributes, discard_workspace, elaborate_in_workspace, get_workspace,
    get_workspace_edges, get_workspace_nodes, list_workspaces,
};

/// Request payload for creating a workspace container.
#[derive(Debug, Deserialize)]
pub struct CreateWorkspacePayload {
    pub name: Option<String>,
    pub workspace_name: Option<String>,
    pub attributes: Option<Value>,
}

/// Response payload for workspace container creation.
#[derive(Debug, Serialize)]
pub struct WorkspaceCreatedResponse {
    pub workspace_id: Uuid,
    pub status: String,
    pub base_event_seq: i64,
    pub workspace_name: String,
    pub owner_agent: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub attributes: Value,
}

/// Request payload for candidate task elaboration inside a workspace.
#[derive(Debug, Deserialize)]
pub struct ElaborateWorkspaceTaskPayload {
    pub parent_id: Option<String>,
    pub parent_node_id: Option<String>,
    pub title: Option<String>,
    pub content: Option<String>,
    pub attributes: Option<Value>,
}

/// `POST /api/v1/workspaces`
///
/// Registers a new isolated branch workspace snapshotting the current substrate state.
pub async fn create_workspace_route(
    State(state): State<AppState>,
    actor: AuthenticatedAgent,
    Json(payload): Json<CreateWorkspacePayload>,
) -> impl IntoResponse {
    let mut client = match state.pool.get().await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "ERR_POOL",
                    "message": format!("Connection pool error: {e}")
                })),
            );
        }
    };

    let name = payload
        .name
        .or(payload.workspace_name)
        .unwrap_or_else(|| format!("workspace-{}", Uuid::new_v4()));

    match create_workspace_with_attributes(&mut client, &name, payload.attributes, &actor).await {
        Ok(ws) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "workspace_id": ws.id,
                "status": ws.status,
                "base_event_seq": ws.base_event_seq,
                "workspace_name": ws.workspace_name,
                "owner_agent": ws.owner_agent,
                "created_at": ws.created_at,
                "attributes": ws.attributes,
            })),
        ),
        Err(err) => mutation_error_to_response(err),
    }
}

/// `GET /api/v1/workspaces/{id}`
///
/// Inspects a workspace container, its base snapshot version, and elaborated candidate entities.
pub async fn inspect_workspace_route(
    State(state): State<AppState>,
    actor: AuthenticatedAgent,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut client = match state.pool.get().await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "ERR_POOL",
                    "message": format!("Connection pool error: {e}")
                })),
            );
        }
    };

    match get_workspace(&mut client, id).await {
        Ok(Some(ws)) => {
            // Boundary isolation check (INV-7): only owner or admin can inspect workspace
            if ws.owner_agent != actor.agent_id
                && actor.actor_type != "HUMAN"
                && actor.agent_id != "system"
            {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": "ERR_NOT_FOUND",
                        "message": format!("Workspace '{id}' not found")
                    })),
                );
            }

            let candidate_nodes = get_workspace_nodes(&client, id).await.unwrap_or_default();
            let candidate_edges = get_workspace_edges(&client, id).await.unwrap_or_default();

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": ws.id,
                    "workspace_id": ws.id,
                    "workspace_name": ws.workspace_name,
                    "owner_agent": ws.owner_agent,
                    "base_event_seq": ws.base_event_seq,
                    "status": ws.status,
                    "created_at": ws.created_at,
                    "attributes": ws.attributes,
                    "candidate_nodes": candidate_nodes,
                    "candidate_edges": candidate_edges,
                })),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "ERR_NOT_FOUND",
                "message": format!("Workspace '{id}' not found")
            })),
        ),
        Err(err) => mutation_error_to_response(err),
    }
}

/// `GET /api/v1/workspaces`
///
/// Lists workspace containers owned by the calling agent (or all workspaces if human supervisor).
pub async fn list_workspaces_route(
    State(state): State<AppState>,
    actor: AuthenticatedAgent,
) -> impl IntoResponse {
    let mut client = match state.pool.get().await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "ERR_POOL",
                    "message": format!("Connection pool error: {e}")
                })),
            );
        }
    };

    let owner_filter = if actor.actor_type == "HUMAN" || actor.agent_id == "system" {
        None
    } else {
        Some(actor.agent_id.as_str())
    };

    match list_workspaces(&mut client, owner_filter).await {
        Ok(workspaces) => (StatusCode::OK, Json(serde_json::json!(workspaces))),
        Err(err) => mutation_error_to_response(err),
    }
}

/// `DELETE /api/v1/workspaces/{id}`
///
/// Discards a workspace container and marks all associated candidate draft nodes and edges as discarded.
pub async fn discard_workspace_route(
    State(state): State<AppState>,
    actor: AuthenticatedAgent,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut client = match state.pool.get().await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "ERR_POOL",
                    "message": format!("Connection pool error: {e}")
                })),
            );
        }
    };

    match discard_workspace(&mut client, id, &actor).await {
        Ok(ws) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "workspace_id": ws.id,
                "status": ws.status,
            })),
        ),
        Err(err) => mutation_error_to_response(err),
    }
}

/// `POST /api/v1/workspaces/{id}/subtasks` (and alias `/api/v1/workspaces/{id}/elaborate`)
///
/// Autonomously elaborates a candidate execution task inside a branch workspace container without taking
/// the global structural mutation advisory lock.
pub async fn elaborate_workspace_task_route(
    State(state): State<AppState>,
    actor: AuthenticatedAgent,
    Path(id): Path<Uuid>,
    Json(payload): Json<ElaborateWorkspaceTaskPayload>,
) -> impl IntoResponse {
    let mut client = match state.pool.get().await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "ERR_POOL",
                    "message": format!("Connection pool error: {e}")
                })),
            );
        }
    };

    let parent_id = match payload.parent_id.or(payload.parent_node_id) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "ERR_BAD_REQUEST",
                    "message": "Missing required field 'parent_id' or 'parent_node_id'"
                })),
            );
        }
    };

    let title = match payload.title {
        Some(t) => t,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "ERR_BAD_REQUEST",
                    "message": "Missing required field 'title'"
                })),
            );
        }
    };

    match elaborate_in_workspace(
        &mut client,
        id,
        &parent_id,
        &title,
        payload.content.as_deref(),
        payload.attributes,
        &actor,
    )
    .await
    {
        Ok(res) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "status": "COMMITTED",
                "workspace_id": id,
                "task_id": res.task_id,
                "node_id": res.task_id,
                "batch_id": res.batch_id,
                "event_seq": res.event_seq,
                "lifecycle_state": res.lifecycle_state,
                "governance_policy": res.governance_policy,
                "node_key": res.node_key,
                "node": res.node,
            })),
        ),
        Err(err) => mutation_error_to_response(err),
    }
}
