//! Administrative governance, rollback, and reverification CLI client (WP-2.5, D-73, D-76, D-80).

use clap::{Args, Subcommand};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

/// Subcommands for administrative governance and mutation lifecycle operations.
#[derive(Debug, Subcommand)]
pub enum AdminSubcommand {
    /// Revert mutations matching structured filter criteria with optional dry-run preview.
    Revert(AdminRevertArgs),
    /// Explicitly reverify degraded nodes whose upstream dependencies have been updated or restored.
    Reverify(AdminReverifyArgs),
}

/// Arguments for `tks admin revert [--batch-id <id>] [--agent-id <id>] [--since <iso8601>] [--dry-run] [--force]`.
#[derive(Debug, Args)]
pub struct AdminRevertArgs {
    /// Optional batch UUID to revert.
    #[arg(long)]
    pub batch_id: Option<Uuid>,

    /// Optional authoring agent identifier whose mutations should be reverted.
    #[arg(long)]
    pub agent_id: Option<String>,

    /// Optional ISO 8601 timestamp threshold (reverts mutations committed since this time).
    #[arg(long)]
    pub since: Option<chrono::DateTime<chrono::Utc>>,

    /// Optional starting event_seq for range filter.
    #[arg(long)]
    pub event_seq_start: Option<i64>,

    /// Optional ending event_seq for range filter.
    #[arg(long)]
    pub event_seq_end: Option<i64>,

    /// Perform a simulated rollback under READ COMMITTED without modifying data.
    #[arg(long)]
    pub dry_run: bool,

    /// Force execution when cross-agent dependencies exist.
    #[arg(long)]
    pub force: bool,
}

/// Arguments for `tks admin reverify <id_or_key> --rationale <text> [--reparent-to <new_parent>]`.
#[derive(Debug, Args)]
pub struct AdminReverifyArgs {
    /// Polymorphic identifier of the node to reverify (UUID or canonical node_key).
    pub id: String,

    /// Operational rationale explaining why the node has been verified.
    #[arg(long)]
    pub rationale: String,

    /// Optional new parent UUID or node_key to re-anchor the node to before verification.
    #[arg(long)]
    pub reparent_to: Option<String>,
}

#[derive(Serialize)]
struct RevertMutationsPayload {
    batch_id: Option<Uuid>,
    agent_id: Option<String>,
    since: Option<chrono::DateTime<chrono::Utc>>,
    event_seq_range: Option<[i64; 2]>,
    dry_run: Option<bool>,
    force: Option<bool>,
}

#[derive(Serialize)]
struct ReverifyNodePayload {
    rationale: String,
    updated_attributes: Option<Value>,
}

/// Executes the `tks admin` CLI subcommand.
///
/// # Errors
///
/// Returns an error if the HTTP request fails or server returns an error.
pub async fn run_admin(
    subcmd: AdminSubcommand,
    server_url: &str,
    auth_token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let base_url = server_url.trim_end_matches('/');

    match subcmd {
        AdminSubcommand::Revert(args) => {
            let url = format!("{base_url}/api/v1/admin/revert-mutations");
            let event_seq_range = match (args.event_seq_start, args.event_seq_end) {
                (Some(s), Some(e)) => Some([s, e]),
                _ => None,
            };

            let payload = RevertMutationsPayload {
                batch_id: args.batch_id,
                agent_id: args.agent_id,
                since: args.since,
                event_seq_range,
                dry_run: if args.dry_run { Some(true) } else { None },
                force: if args.force { Some(true) } else { None },
            };

            let resp = client
                .post(&url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .header(CONTENT_TYPE, "application/json")
                .json(&payload)
                .send()
                .await?;

            let status = resp.status();
            if status == reqwest::StatusCode::CONFLICT {
                // Check if ERR_CONFIRMATION_REQUIRED
                let err_val: Value = resp.json().await.unwrap_or_default();
                let err_code = err_val.get("error").and_then(|v| v.as_str()).unwrap_or("");
                if err_code == "ERR_CONFIRMATION_REQUIRED" {
                    eprintln!(
                        "Rollback halted: Confirmation required (ERR_CONFIRMATION_REQUIRED)."
                    );
                    eprintln!("Cross-agent dependent tasks detected across subtrees.");
                    if let Some(preview) = err_val.get("preview") {
                        if let Some(cross_deps) = preview.get("cross_agent_dependencies") {
                            eprintln!("  Dependent Agents: {cross_deps}");
                        }
                        if let Some(nodes) =
                            preview.get("affected_nodes").and_then(|v| v.as_array())
                        {
                            eprintln!("  Affected Nodes:   {} nodes", nodes.len());
                        }
                    }
                    eprintln!("To force compensating transaction execution, re-run with --force.");
                    return Err(
                        "ERR_CONFIRMATION_REQUIRED: cross-agent dependencies require --force"
                            .into(),
                    );
                }
                eprintln!("Rollback conflict ({status}): {err_val}");
                return Err(format!("Server returned {status}: {err_val}").into());
            }

            if !status.is_success() {
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to revert mutations ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let result: Value = resp.json().await?;
            let status_str = result.get("status").and_then(|v| v.as_str()).unwrap_or("");

            if status_str == "PREVIEW" {
                println!("Rollback Dry-Run Preview:");
                println!(
                    "  Total Events:             {}",
                    result.get("total_events").unwrap_or(&Value::from(0))
                );
                if let Some(nodes) = result.get("affected_nodes").and_then(|v| v.as_array()) {
                    println!("  Affected Nodes:           {} nodes", nodes.len());
                    for n in nodes {
                        println!("    - {n}");
                    }
                }
                if let Some(edges) = result.get("affected_edges").and_then(|v| v.as_array()) {
                    println!("  Affected Edges:           {} edges", edges.len());
                }
                if let Some(deps) = result
                    .get("cross_agent_dependencies")
                    .and_then(|v| v.as_array())
                {
                    println!("  Cross-Agent Dependencies: {} agents", deps.len());
                    for d in deps {
                        println!("    - {d}");
                    }
                }
            } else {
                println!("Rollback Execution Succeeded:");
                println!(
                    "  Batch ID:                 {}",
                    result.get("batch_id").unwrap_or(&Value::Null)
                );
                println!(
                    "  Reverted Events:          {}",
                    result.get("reverted_events").unwrap_or(&Value::from(0))
                );
                if let Some(nodes) = result.get("affected_nodes").and_then(|v| v.as_array()) {
                    println!("  Affected Nodes:           {} nodes", nodes.len());
                }
                if let Some(cascades) = result
                    .get("cascade_reverified_nodes")
                    .and_then(|v| v.as_array())
                {
                    println!("  Cascade Reverified Nodes: {} nodes", cascades.len());
                    for c in cascades {
                        println!("    - {c}");
                    }
                }
            }
        }

        AdminSubcommand::Reverify(args) => {
            let url = format!("{base_url}/api/v1/nodes/{}/reverify", args.id);
            let updated_attributes = args
                .reparent_to
                .map(|p| serde_json::json!({ "reparent_to": p }));

            let payload = ReverifyNodePayload {
                rationale: args.rationale,
                updated_attributes,
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
                eprintln!("Failed to reverify node ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let result: Value = resp.json().await?;
            println!("Node Reverification Succeeded:");
            println!(
                "  Node ID:         {}",
                result.get("node_id").unwrap_or(&Value::Null)
            );
            println!(
                "  Status:          {}",
                result.get("status").unwrap_or(&Value::from("ACTIVE"))
            );
            println!(
                "  Staleness Score: {}",
                result.get("staleness_score").unwrap_or(&Value::from(0.0))
            );
            if let Some(batch_id) = result.get("batch_id") {
                println!("  Batch ID:        {batch_id}");
            }
            if let Some(event_seq) = result.get("event_seq") {
                println!("  Event Seq:       {event_seq}");
            }
        }
    }

    Ok(())
}
