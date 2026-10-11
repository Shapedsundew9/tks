//! Release operations CLI client (WP-4.5, TB-12.3, D-113, D-120).

use clap::{Args, Subcommand};
use reqwest::header::AUTHORIZATION;
use serde_json::Value;

/// Subcommands for release operations.
#[derive(Debug, Subcommand)]
pub enum ReleaseSubcommand {
    /// Inspect release readiness for pre-merge PR or post-merge milestone release.
    Readiness(ReleaseReadinessArgs),
}

/// Arguments for `tks release readiness [--scope <pre-merge|release>] [--pr-commit-sha <sha>] [--task-ids <ids>] [--milestone <tag>]`.
#[derive(Debug, Args)]
pub struct ReleaseReadinessArgs {
    /// Readiness scope: "pre-merge" or "release" (default: "pre-merge").
    #[arg(long, default_value = "pre-merge")]
    pub scope: String,

    /// Candidate pull request commit SHA (required for scope=pre-merge).
    #[arg(long)]
    pub pr_commit_sha: Option<String>,

    /// Comma-separated task UUIDs.
    #[arg(long, value_delimiter = ',')]
    pub task_ids: Option<Vec<String>>,

    /// Milestone release tag (for scope=release).
    #[arg(long)]
    pub milestone: Option<String>,
}

/// Executes the `tks release` CLI subcommand.
///
/// # Errors
///
/// Returns an error if the HTTP request fails or server returns an error.
pub async fn run_release(
    subcmd: ReleaseSubcommand,
    server_url: &str,
    auth_token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let base_url = server_url.trim_end_matches('/');

    match subcmd {
        ReleaseSubcommand::Readiness(args) => {
            let mut url = format!("{base_url}/api/v1/release/readiness?scope={}", args.scope);
            if let Some(ref sha) = args.pr_commit_sha {
                url.push_str(&format!("&pr_commit_sha={sha}"));
            }
            if let Some(ref tasks) = args.task_ids {
                url.push_str(&format!("&task_ids={}", tasks.join(",")));
            }
            if let Some(ref tag) = args.milestone {
                url.push_str(&format!("&milestone={tag}"));
            }

            let resp = client
                .get(&url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to check release readiness ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let result: Value = resp.json().await?;
            let status = result
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN");
            let scope = result
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or(&args.scope);

            println!("Release Readiness Inspection (scope: {scope}):");
            println!("  Status: {}", status);

            if let Some(unverified) = result
                .get("unverified_task_ids")
                .and_then(|v| v.as_array())
                .filter(|a| !a.is_empty())
            {
                println!("  Unverified Task IDs: {:?}", unverified);
            }
            if let Some(uncommitted) = result
                .get("uncommitted_tasks")
                .and_then(|v| v.as_array())
                .filter(|a| !a.is_empty())
            {
                println!("  Uncommitted Task IDs: {:?}", uncommitted);
            }
            if let Some(satisfied) = result
                .get("satisfied_requirements")
                .and_then(|v| v.as_array())
                .filter(|a| !a.is_empty())
            {
                println!("  Satisfied Requirements: {:?}", satisfied);
            }
            if let Some(pct) = result.get("readiness_percentage").and_then(|v| v.as_f64()) {
                println!("  Readiness: {:.1}%", pct);
            }

            if status == "BLOCKED" {
                return Err("Release readiness is BLOCKED".into());
            }
        }
    }

    Ok(())
}
