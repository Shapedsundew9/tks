//! Verification and contract drift detection CLI client (WP-4.5, TB-12.3).

use clap::{Args, Subcommand};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;

use crate::cli::task::TestRunArgs;

/// Subcommands for verification operations.
#[derive(Debug, Subcommand)]
pub enum VerificationSubcommand {
    /// Ingest a pre-merge CI test run execution result onto active tasks.
    TestRun(TestRunArgs),
    /// Inspect code-side contract drift against active requirement specifications.
    CheckDrift(CheckDriftArgs),
}

/// Arguments for `tks verification check-drift [--diff <diff>] [--diff-file <path>] [--task-ids <ids>]`.
#[derive(Debug, Args)]
pub struct CheckDriftArgs {
    /// Raw Git diff string or '-' for standard input.
    #[arg(long)]
    pub diff: Option<String>,

    /// Path to a git diff file.
    #[arg(long)]
    pub diff_file: Option<String>,

    /// Optional target task UUIDs.
    #[arg(long, value_delimiter = ',')]
    pub task_ids: Option<Vec<String>>,
}

/// Executes the `tks verification` CLI subcommand.
///
/// # Errors
///
/// Returns an error if the HTTP request fails or server returns an error.
pub async fn run_verification(
    subcmd: VerificationSubcommand,
    server_url: &str,
    auth_token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let base_url = server_url.trim_end_matches('/');

    match subcmd {
        VerificationSubcommand::TestRun(args) => {
            crate::cli::task::run_test_run_inner(&client, base_url, auth_token, args).await
        }

        VerificationSubcommand::CheckDrift(args) => {
            let diff_content = if let Some(path) = args.diff_file {
                std::fs::read_to_string(&path)
                    .map_err(|e| format!("Failed to read diff file '{path}': {e}"))?
            } else if let Some(diff_str) = args.diff {
                if diff_str == "-" {
                    use std::io::Read;
                    let mut buffer = String::new();
                    std::io::stdin().read_to_string(&mut buffer)?;
                    buffer
                } else if std::path::Path::new(&diff_str).exists() {
                    std::fs::read_to_string(&diff_str)
                        .map_err(|e| format!("Failed to read diff file '{diff_str}': {e}"))?
                } else {
                    diff_str
                }
            } else {
                return Err("Either --diff or --diff-file must be provided".into());
            };

            let url = format!("{base_url}/api/v1/verification/check-drift");
            let payload = serde_json::json!({
                "diff": diff_content,
                "task_ids": args.task_ids.unwrap_or_default(),
            });

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
                eprintln!("Failed to check contract drift ({status}): {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let result: Value = resp.json().await?;
            let status = result
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN");
            let touched = result
                .get("touched_files")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let contract_files = result
                .get("contract_files")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);

            println!("Code-Side Contract Drift Inspection:");
            println!("  Status:         {}", status);
            println!("  Touched Files:  {}", touched);
            println!("  Contract Files: {}", contract_files);

            if let Some(anomalies) = result
                .get("anomalies")
                .and_then(|v| v.as_array())
                .filter(|a| !a.is_empty())
            {
                eprintln!("\nDRIFT ANOMALIES DETECTED ({}):", anomalies.len());
                for a in anomalies {
                    let file = a
                        .get("file")
                        .and_then(|v| v.as_str())
                        .unwrap_or("<unknown>");
                    let msg = a.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    eprintln!("  - [{}]: {}", file, msg);
                }
                return Err(format!(
                    "Contract drift detected: {} unattributed modification(s)",
                    anomalies.len()
                )
                .into());
            }

            println!(
                "Clean: All modified contracts are attributed to active requirements or tasks."
            );
            Ok(())
        }
    }
}
