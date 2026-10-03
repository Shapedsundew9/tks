//! Command-line interface modules (WP-1.6).

pub mod admin;
pub mod explorer;
pub mod identity;
pub mod mcp_stdio;
pub mod serve;
pub mod staging;
pub mod task;
pub mod workspace;

pub use admin::{AdminReverifyArgs, AdminRevertArgs, AdminSubcommand, run_admin};
pub use explorer::{ExplorerServeArgs, ExplorerSubcommand, run_explorer};
pub use identity::{CreateIdentityArgs, IdentitySubcommand, RevokeIdentityArgs, run_identity};
pub use mcp_stdio::{McpStdioArgs, run_mcp_stdio};
pub use serve::{ServeArgs, run_serve};
pub use staging::{
    StagingApproveArgs, StagingInspectArgs, StagingListArgs, StagingRejectArgs, StagingSubcommand,
    run_staging,
};
pub use task::{CreateTaskArgs, ListTasksArgs, TaskSubcommand, UpdateTaskArgs, run_task};
pub use workspace::{
    CreateWorkspaceArgs, DiscardWorkspaceArgs, InspectWorkspaceArgs, ListWorkspacesArgs,
    MergeWorkspaceArgs, RebaseWorkspaceArgs, WorkspaceSubcommand, run_workspace,
};

/// Resolves the server base URL from arguments, environment variables, or defaults.
#[must_use]
pub fn resolve_server_url(arg: Option<&str>) -> String {
    arg.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            std::env::var("TKS_SERVER_URL")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "http://127.0.0.1:8080".to_string())
}

/// Resolves the bearer auth token from arguments, environment variables, or dev seed default.
#[must_use]
pub fn resolve_auth_token(arg: Option<&str>) -> String {
    arg.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            std::env::var("TKS_DEV_TOKEN")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            std::env::var("TKS_AUTH_TOKEN")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "tks_dev_token".to_string())
}
