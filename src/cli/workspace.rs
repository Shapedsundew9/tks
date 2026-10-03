//! Multi-agent workspace CLI client (WP-3.5, PHASE3-003, D-14, D-53).

use clap::{Args, Subcommand};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Subcommands for multi-agent workspace lifecycle and merge operations.
#[derive(Debug, Subcommand)]
pub enum WorkspaceSubcommand {
    /// Create a new branch-isolated workspace container snapshotting the current substrate state.
    Create(CreateWorkspaceArgs),
    /// List workspace containers owned by the calling agent or all active workspaces.
    List(ListWorkspacesArgs),
    /// Inspect a workspace container, snapshot version, and candidate draft entities.
    Inspect(InspectWorkspaceArgs),
    /// Merge a workspace container into the live substrate under canonical lock serialization.
    Merge(MergeWorkspaceArgs),
    /// Rebase a workspace container branch with live substrate state.
    Rebase(RebaseWorkspaceArgs),
    /// Discard a workspace container and mark candidate drafts as discarded.
    Discard(DiscardWorkspaceArgs),
}

/// Arguments for `tks workspace create --name <name> [--json]`.
#[derive(Debug, Args)]
pub struct CreateWorkspaceArgs {
    /// Human-readable name for the branch workspace.
    #[arg(long)]
    pub name: String,

    /// Output raw JSON response instead of human-readable summary.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `tks workspace list [--json]`.
#[derive(Debug, Args)]
pub struct ListWorkspacesArgs {
    /// Output raw JSON response instead of human-readable summary.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `tks workspace inspect <workspace_id> [--json]`.
#[derive(Debug, Args)]
pub struct InspectWorkspaceArgs {
    /// Unique identifier (UUID) of the workspace to inspect.
    pub workspace_id: Uuid,

    /// Output raw JSON response instead of human-readable summary.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `tks workspace merge <workspace_id> [--auto-reparent] [--json]`.
#[derive(Debug, Args)]
pub struct MergeWorkspaceArgs {
    /// Unique identifier (UUID) of the workspace to merge.
    pub workspace_id: Uuid,

    /// Enable deterministic auto-reparenting when parent requirements advanced cleanly.
    #[arg(long)]
    pub auto_reparent: bool,

    /// Output raw JSON response instead of human-readable summary.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `tks workspace rebase <workspace_id> [--auto-reparent] [--json]`.
#[derive(Debug, Args)]
pub struct RebaseWorkspaceArgs {
    /// Unique identifier (UUID) of the workspace to rebase.
    pub workspace_id: Uuid,

    /// Enable deterministic auto-reparenting when parent requirements advanced cleanly.
    #[arg(long)]
    pub auto_reparent: bool,

    /// Output raw JSON response instead of human-readable summary.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `tks workspace discard <workspace_id> [--json]`.
#[derive(Debug, Args)]
pub struct DiscardWorkspaceArgs {
    /// Unique identifier (UUID) of the workspace to discard.
    pub workspace_id: Uuid,

    /// Output raw JSON response instead of human-readable summary.
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct CreateWorkspacePayload<'a> {
    name: &'a str,
}

#[derive(Deserialize, Serialize)]
struct WorkspaceItemResponse {
    id: Option<Uuid>,
    workspace_id: Option<Uuid>,
    workspace_name: Option<String>,
    owner_agent: Option<String>,
    base_event_seq: Option<i64>,
    status: Option<String>,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    attributes: Option<Value>,
    candidate_nodes: Option<Vec<Value>>,
    candidate_edges: Option<Vec<Value>>,
}

#[derive(Serialize)]
struct PromotePayload {
    auto_reparent: bool,
}

#[derive(Deserialize, Serialize)]
struct PromotionResponse {
    status: String,
    workspace_id: Uuid,
    batch_id: Uuid,
    promoted_nodes: usize,
    promoted_edges: usize,
    event_seq: i64,
}

#[derive(Deserialize, Serialize)]
struct RebaseResponse {
    status: String,
    workspace_id: Uuid,
    old_base_event_seq: i64,
    new_base_event_seq: i64,
    reparented_tasks: usize,
}

#[derive(Deserialize, Serialize)]
struct DiscardResponse {
    workspace_id: Uuid,
    status: String,
}

/// Executes the `tks workspace` CLI subcommand.
///
/// # Errors
///
/// Returns an error if the HTTP request fails or the server returns an error.
pub async fn run_workspace(
    subcmd: WorkspaceSubcommand,
    server_url: &str,
    auth_token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let base_url = server_url.trim_end_matches('/');

    match subcmd {
        WorkspaceSubcommand::Create(args) => {
            let url = format!("{base_url}/api/v1/workspaces");
            let payload = CreateWorkspacePayload { name: &args.name };

            let resp = client
                .post(&url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .header(CONTENT_TYPE, "application/json")
                .json(&payload)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to create workspace ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let result: Value = resp.json().await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                let ws_id = result
                    .get("workspace_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let ws_name = result
                    .get("workspace_name")
                    .and_then(Value::as_str)
                    .unwrap_or(&args.name);
                let status = result
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("ACTIVE");
                let base_seq = result
                    .get("base_event_seq")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let owner = result
                    .get("owner_agent")
                    .and_then(Value::as_str)
                    .unwrap_or("");

                println!("Workspace created successfully:");
                println!("  workspace_id:    {ws_id}");
                println!("  workspace_name:  {ws_name}");
                println!("  status:          {status}");
                println!("  base_event_seq:  {base_seq}");
                println!("  owner_agent:     {owner}");
            }
        }

        WorkspaceSubcommand::List(args) => {
            let url = format!("{base_url}/api/v1/workspaces");
            let resp = client
                .get(&url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to list workspaces ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let items: Vec<WorkspaceItemResponse> = resp.json().await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                println!("Workspaces ({}):", items.len());
                for item in items {
                    let id = item
                        .workspace_id
                        .or(item.id)
                        .map(|u| u.to_string())
                        .unwrap_or_else(|| "<unknown>".to_string());
                    let name = item
                        .workspace_name
                        .unwrap_or_else(|| "<unnamed>".to_string());
                    let status = item.status.unwrap_or_else(|| "ACTIVE".to_string());
                    let owner = item.owner_agent.unwrap_or_else(|| "unknown".to_string());
                    let base_seq = item.base_event_seq.unwrap_or(0);
                    println!(
                        "  [{id}] {name} | {status} | owner: {owner} (base_event_seq: {base_seq})"
                    );
                }
            }
        }

        WorkspaceSubcommand::Inspect(args) => {
            let url = format!("{base_url}/api/v1/workspaces/{}", args.workspace_id);
            let resp = client
                .get(&url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to inspect workspace ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let item: WorkspaceItemResponse = resp.json().await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&item)?);
            } else {
                let id = item
                    .workspace_id
                    .or(item.id)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| args.workspace_id.to_string());
                let name = item
                    .workspace_name
                    .unwrap_or_else(|| "<unnamed>".to_string());
                let status = item.status.unwrap_or_else(|| "ACTIVE".to_string());
                let owner = item.owner_agent.unwrap_or_else(|| "unknown".to_string());
                let base_seq = item.base_event_seq.unwrap_or(0);
                let node_count = item.candidate_nodes.as_ref().map(Vec::len).unwrap_or(0);
                let edge_count = item.candidate_edges.as_ref().map(Vec::len).unwrap_or(0);

                println!("Workspace Details:");
                println!("  workspace_id:    {id}");
                println!("  workspace_name:  {name}");
                println!("  status:          {status}");
                println!("  owner_agent:     {owner}");
                println!("  base_event_seq:  {base_seq}");
                println!("  candidate_nodes: {node_count}");
                println!("  candidate_edges: {edge_count}");
            }
        }

        WorkspaceSubcommand::Merge(args) => {
            let url = format!("{base_url}/api/v1/workspaces/{}/promote", args.workspace_id);
            let payload = PromotePayload {
                auto_reparent: args.auto_reparent,
            };

            let resp = client
                .post(&url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .header(CONTENT_TYPE, "application/json")
                .json(&payload)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to merge workspace ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let result: PromotionResponse = resp.json().await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("Workspace merged successfully:");
                println!("  workspace_id:    {}", result.workspace_id);
                println!("  status:          {}", result.status);
                println!("  batch_id:        {}", result.batch_id);
                println!("  promoted_nodes:  {}", result.promoted_nodes);
                println!("  promoted_edges:  {}", result.promoted_edges);
                println!("  event_seq:       {}", result.event_seq);
            }
        }

        WorkspaceSubcommand::Rebase(args) => {
            let url = format!("{base_url}/api/v1/workspaces/{}/rebase", args.workspace_id);
            let payload = PromotePayload {
                auto_reparent: args.auto_reparent,
            };

            let resp = client
                .post(&url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .header(CONTENT_TYPE, "application/json")
                .json(&payload)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to rebase workspace ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let result: RebaseResponse = resp.json().await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("Workspace rebased successfully:");
                println!("  workspace_id:        {}", result.workspace_id);
                println!("  status:              {}", result.status);
                println!("  old_base_event_seq:  {}", result.old_base_event_seq);
                println!("  new_base_event_seq:  {}", result.new_base_event_seq);
                println!("  reparented_tasks:    {}", result.reparented_tasks);
            }
        }

        WorkspaceSubcommand::Discard(args) => {
            let url = format!("{base_url}/api/v1/workspaces/{}", args.workspace_id);
            let resp = client
                .delete(&url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to discard workspace ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let result: DiscardResponse = resp.json().await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("Workspace discarded successfully:");
                println!("  workspace_id:  {}", result.workspace_id);
                println!("  status:        {}", result.status);
            }
        }
    }

    Ok(())
}
