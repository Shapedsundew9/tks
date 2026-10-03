//! Unified Axum HTTP, REST, and Model Context Protocol (MCP) server gateway (WP-1.5, D-19, D-53).

pub mod auth;
pub mod mcp;
pub mod routes;
pub mod server;

use std::time::Duration;

use axum::extract::FromRef;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use deadpool_postgres::Pool;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

pub use auth::{AuthState, AuthenticatedAgent};
pub use server::run_server;

use crate::storage::StorageRepo;
use crate::storage::git::{GitReadHandle, GitWriteHandle};

/// Shared gateway application state accessible to all Axum route handlers.
#[derive(Clone)]
pub struct AppState {
    pub pool: Pool,
    pub storage: StorageRepo,
    pub git_write: GitWriteHandle,
    pub git_read: GitReadHandle,
    pub auth: AuthState,
    pub event_bus: crate::storage::GraphEventBus,
}

impl AppState {
    /// Creates a new `AppState` instance.
    #[must_use]
    pub fn new(
        pool: Pool,
        storage: StorageRepo,
        git_write: GitWriteHandle,
        git_read: GitReadHandle,
    ) -> Self {
        let auth = AuthState::new(pool.clone());
        let event_bus = crate::storage::GraphEventBus::default();
        Self {
            pool,
            storage,
            git_write,
            git_read,
            auth,
            event_bus,
        }
    }

    /// Sets custom `GraphEventBus` on `AppState`.
    #[must_use]
    pub fn with_event_bus(mut self, event_bus: crate::storage::GraphEventBus) -> Self {
        self.event_bus = event_bus;
        self
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(state: &AppState) -> Self {
        state.auth.clone()
    }
}

/// Builds and configures the unified Axum router with REST endpoints, MCP tools, and middleware.
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Health check endpoint (Architecture §8 Observability)
        .route("/health", get(health_check))
        // Document ingestion routes (TB-1, D-67)
        .route(
            "/api/v1/documents/ingest",
            post(routes::documents::ingest_document),
        )
        .route(
            "/api/v1/documents/ingest/{job_id}",
            get(routes::documents::get_ingestion_job),
        )
        // Staging review, approval, and rejection routes (D-83)
        .route(
            "/api/v1/documents/ingest/{job_id}/approve",
            post(routes::staging::approve_ingestion_job),
        )
        .route(
            "/api/v1/documents/ingest/{job_id}",
            delete(routes::staging::reject_ingestion_job),
        )
        // Staging backward-compatible aliases (D-83)
        .route(
            "/api/v1/staging/approve",
            post(routes::staging::approve_staging_alias),
        )
        .route(
            "/api/v1/staging/reject",
            post(routes::staging::reject_staging_alias),
        )
        // Node inspection routes (WP-1.6, D-83)
        .route("/api/v1/nodes/{id}", get(routes::staging::inspect_node))
        .route(
            "/api/v1/staging/inspect/{id}",
            get(routes::staging::inspect_node),
        )
        // Identity management routes (TB-5, D-49)
        .route(
            "/api/v1/identities",
            post(routes::identities::create_identity),
        )
        .route(
            "/api/v1/identities/{id}/revoke",
            post(routes::identities::revoke_identity),
        )
        // Node mutation and execution task routes (WP-2.4, D-57, D-63)
        .route("/api/v1/nodes/mutate", post(routes::mutation::mutate_node))
        .route(
            "/api/v1/nodes/{id}/subtasks",
            post(routes::mutation::create_subtask_route),
        )
        .route(
            "/api/v1/nodes/{id}/status",
            patch(routes::mutation::update_status_route),
        )
        .route("/api/v1/tasks", get(routes::mutation::list_tasks_route))
        // Administrative rollback and node reverification routes (WP-2.4, D-73, D-76)
        .route(
            "/api/v1/admin/revert-mutations",
            post(routes::mutation::revert_mutations_route),
        )
        .route(
            "/api/v1/nodes/{id}/reverify",
            post(routes::mutation::reverify_node_route),
        )
        // Multi-agent workspace routes (WP-3.2, PHASE3-003)
        .route(
            "/api/v1/workspaces",
            post(routes::workspaces::create_workspace_route)
                .get(routes::workspaces::list_workspaces_route),
        )
        .route(
            "/api/v1/workspaces/{id}",
            get(routes::workspaces::inspect_workspace_route)
                .delete(routes::workspaces::discard_workspace_route),
        )
        .route(
            "/api/v1/workspaces/{id}/subtasks",
            post(routes::workspaces::elaborate_workspace_task_route),
        )
        .route(
            "/api/v1/workspaces/{id}/elaborate",
            post(routes::workspaces::elaborate_workspace_task_route),
        )
        // Model Context Protocol (MCP) endpoints over HTTP/SSE and direct JSON-RPC (D-19, TB-4)
        .route("/mcp/sse", get(mcp::handle_mcp_sse))
        .route("/mcp/message", post(mcp::handle_mcp_message))
        .route("/mcp", post(mcp::handle_mcp_rpc))
        // Layer configurations: tracing, CORS, per-request timeout (Architecture §6.2)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .with_state(state)
}

/// Health check handler verifying server status and PostgreSQL connectivity.
async fn health_check(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    match state.pool.get().await {
        Ok(client) => match client.simple_query("SELECT 1;").await {
            Ok(_) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "healthy",
                    "database": "connected",
                    "service": "tks"
                })),
            ),
            Err(e) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "degraded",
                    "database": format!("Query failed: {e}"),
                    "service": "tks"
                })),
            ),
        },
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "unhealthy",
                "database": format!("Pool error: {e}"),
                "service": "tks"
            })),
        ),
    }
}
