//! Unified Axum HTTP, REST, and Model Context Protocol (MCP) server gateway (WP-1.5, D-19, D-53).

pub mod auth;
pub mod elicitation_ui;
pub mod error;
pub mod explorer;
pub mod mcp;
pub mod routes;
pub mod server;

use std::time::Duration;

use axum::extract::FromRef;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use deadpool_postgres::Pool;
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

pub use auth::{AuthState, AuthenticatedAgent};
pub use error::InvalidEdgeTargetEnvelope;
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
    pub elicitation: crate::elicitation::ElicitationStore,
    pub cancel_token: CancellationToken,
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
        let elicitation = crate::elicitation::ElicitationStore::new(
            crate::elicitation::ElicitationConfig::default(),
        );
        let cancel_token = CancellationToken::new();
        Self {
            pool,
            storage,
            git_write,
            git_read,
            auth,
            event_bus,
            elicitation,
            cancel_token,
        }
    }

    /// Sets custom `GraphEventBus` on `AppState`.
    #[must_use]
    pub fn with_event_bus(mut self, event_bus: crate::storage::GraphEventBus) -> Self {
        self.event_bus = event_bus;
        self
    }

    /// Sets custom `ElicitationStore` on `AppState`.
    #[must_use]
    pub fn with_elicitation(mut self, elicitation: crate::elicitation::ElicitationStore) -> Self {
        self.elicitation = elicitation;
        self
    }

    /// Sets custom `CancellationToken` on `AppState`.
    #[must_use]
    pub fn with_cancel_token(mut self, cancel_token: CancellationToken) -> Self {
        self.cancel_token = cancel_token;
        self
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(state: &AppState) -> Self {
        state.auth.clone()
    }
}

impl FromRef<AppState> for crate::elicitation::ElicitationStore {
    fn from_ref(state: &AppState) -> Self {
        state.elicitation.clone()
    }
}

/// Loopback access-control guard middleware (D-EA-10, §6.2).
///
/// Enforces that elicitation routes are accessible only via loopback Host
/// headers, rejects cross-origin state-changing mutations, and avoids permissive CORS.
pub async fn loopback_guard_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let host_header = match req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
    {
        Some(h) => h,
        None => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "ERR_LOOPBACK_REQUIRED",
                    "message": "Host header is required and must resolve to loopback"
                })),
            )
                .into_response();
        }
    };

    let hostname = if host_header.starts_with('[') {
        host_header.split(']').next().map(|s| &s[1..]).unwrap_or("")
    } else {
        host_header.split(':').next().unwrap_or("")
    };

    let is_loopback = hostname == "localhost" || hostname == "127.0.0.1" || hostname == "::1";
    if !is_loopback {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "ERR_LOOPBACK_REQUIRED",
                "message": format!("Access forbidden for host '{hostname}'; loopback address required")
            })),
        )
            .into_response();
    }

    let method = req.method();
    let is_state_changing = matches!(
        *method,
        axum::http::Method::POST
            | axum::http::Method::PUT
            | axum::http::Method::PATCH
            | axum::http::Method::DELETE
    );

    if is_state_changing && let Some(origin) = req.headers().get(axum::http::header::ORIGIN) {
        let origin_str = match origin.to_str() {
            Ok(s) => s,
            Err(_) => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "ERR_INVALID_ORIGIN",
                        "message": "Invalid Origin header encoding"
                    })),
                )
                    .into_response();
            }
        };

        let expected_origin = format!("http://{host_header}");
        if origin_str != expected_origin {
            return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "ERR_CROSS_ORIGIN_FORBIDDEN",
                        "message": format!(
                            "Cross-origin request rejected: expected '{expected_origin}', got '{origin_str}'"
                        )
                    })),
                )
                    .into_response();
        }
    }

    next.run(req).await
}

/// Builds and configures the unified Axum router with REST endpoints, MCP tools, and middleware.
pub fn create_router(state: AppState) -> Router {
    let general_routes = Router::new()
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
        // Staging review, approval, and rejection routes (D-83, WP-4.3)
        .route(
            "/api/v1/staging/{job_id}/candidates",
            get(routes::staging::get_staging_candidates),
        )
        .route(
            "/api/v1/documents/ingest/{job_id}/candidates",
            get(routes::staging::get_staging_candidates),
        )
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
        // Pre-merge verification and CI test run routes (WP-4.5, TB-12.1, D-107)
        .route(
            "/api/v1/verification/test-run",
            post(routes::verification::record_test_run),
        )
        .route(
            "/api/v1/verification/check-drift",
            post(routes::verification::check_drift_route),
        )
        // VCS merge commit materialization routes (WP-4.5, TB-12.2, D-108)
        .route("/api/v1/vcs/commits", post(routes::vcs::materialize_commit))
        // Disambiguated release readiness routes (WP-4.5, TB-12.3, D-113, D-120)
        .route(
            "/api/v1/release/readiness",
            get(routes::release::get_release_readiness),
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
        .route(
            "/api/v1/workspaces/{id}/promote",
            post(routes::workspaces::promote_workspace_route),
        )
        .route(
            "/api/v1/workspaces/{id}/merge",
            post(routes::workspaces::promote_workspace_route),
        )
        .route(
            "/api/v1/workspaces/{id}/rebase",
            post(routes::workspaces::rebase_workspace_route),
        )
        .route(
            "/api/v1/workspaces/{id}/analyze",
            get(routes::workspaces::analyze_workspace_merge_route),
        )
        .route(
            "/api/v1/workspaces/{id}/merge-preview",
            get(routes::workspaces::analyze_workspace_merge_route),
        )
        // Model Context Protocol (MCP) endpoints over HTTP/SSE and direct JSON-RPC (D-19, TB-4)
        .route("/mcp/sse", get(mcp::handle_mcp_sse))
        .route("/mcp/message", post(mcp::handle_mcp_message))
        .route("/mcp", post(mcp::handle_mcp_rpc))
        // Real-Time Substrate Web Explorer routes (WP-3.4, PHASE3-001)
        .route("/explorer", get(explorer::serve_explorer_index))
        .route("/explorer/", get(explorer::serve_explorer_index))
        .route("/explorer/index.html", get(explorer::serve_explorer_index))
        .route("/explorer/app.js", get(explorer::serve_explorer_js))
        .route("/explorer/style.css", get(explorer::serve_explorer_css))
        .route("/explorer/cytoscape.min.js", get(explorer::serve_cytoscape))
        .route("/explorer/dagre.min.js", get(explorer::serve_dagre))
        .route(
            "/explorer/cytoscape-dagre.min.js",
            get(explorer::serve_cytoscape_dagre),
        )
        // Web Explorer REST and SSE routes (WP-3.4)
        .route(
            "/api/v1/explorer/graph",
            get(routes::explorer::get_explorer_graph),
        )
        .route(
            "/api/v1/explorer/events",
            get(routes::explorer::stream_explorer_events),
        )
        .layer(CorsLayer::permissive());

    // Elicitation REST, SSE, UI, and ASR asset routes (D-130..D-134, D-EA-10, §6.2, §6.3, §6.7)
    // Strictly loopback-guarded, excluded from permissive CORS, with hardened security headers
    let elicitation_routes = Router::new()
        // Elicitation Studio UI
        .route(
            "/explorer/elicitation",
            get(elicitation_ui::serve_elicitation_index),
        )
        .route(
            "/explorer/elicitation/",
            get(elicitation_ui::serve_elicitation_index),
        )
        .route(
            "/explorer/elicitation/index.html",
            get(elicitation_ui::serve_elicitation_index),
        )
        .route(
            "/explorer/elicitation/elicitation.js",
            get(elicitation_ui::serve_elicitation_js),
        )
        .route(
            "/explorer/elicitation/asr-worker.js",
            get(elicitation_ui::serve_asr_worker_js),
        )
        .route(
            "/explorer/elicitation/audio-worklet.js",
            get(elicitation_ui::serve_audio_worklet_js),
        )
        .route(
            "/explorer/elicitation/elicitation.css",
            get(elicitation_ui::serve_elicitation_css),
        )
        // ASR Asset Serving (whitelisted via lock manifest)
        .route(
            "/explorer/asr/{version}/{file}",
            get(elicitation_ui::serve_asr_asset),
        )
        // Elicitation REST & SSE API
        .route(
            "/api/v1/elicitation/capabilities",
            get(routes::elicitation::get_capabilities),
        )
        .route(
            "/api/v1/elicitation/facilitator",
            put(routes::elicitation::put_facilitator_backend),
        )
        .route(
            "/api/v1/elicitation/sessions",
            post(routes::elicitation::create_session),
        )
        .route(
            "/api/v1/elicitation/sessions/{elicitation_id}",
            get(routes::elicitation::get_session).delete(routes::elicitation::delete_session),
        )
        .route(
            "/api/v1/elicitation/sessions/{elicitation_id}/turns",
            post(routes::elicitation::add_turn),
        )
        .route(
            "/api/v1/elicitation/sessions/{elicitation_id}/turns/{number}",
            patch(routes::elicitation::patch_turn).delete(routes::elicitation::delete_turn),
        )
        .route(
            "/api/v1/elicitation/sessions/{elicitation_id}/facilitator/proposals",
            post(routes::elicitation::request_facilitator_proposal),
        )
        .route(
            "/api/v1/elicitation/sessions/{elicitation_id}/events",
            get(routes::elicitation::stream_session_events),
        )
        .route(
            "/api/v1/elicitation/sessions/{elicitation_id}/close",
            post(routes::elicitation::close_session),
        )
        .route(
            "/api/v1/elicitation/sessions/{elicitation_id}/reopen",
            post(routes::elicitation::reopen_session),
        )
        .route(
            "/api/v1/elicitation/sessions/{elicitation_id}/transcript",
            get(routes::elicitation::get_transcript),
        )
        .layer(axum::middleware::from_fn(
            elicitation_ui::elicitation_security_headers_middleware,
        ))
        .layer(axum::middleware::from_fn(loopback_guard_middleware));

    general_routes
        .merge(elicitation_routes)
        // Layer configurations: tracing, per-request timeout (Architecture §6.2)
        .layer(TraceLayer::new_for_http())
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
