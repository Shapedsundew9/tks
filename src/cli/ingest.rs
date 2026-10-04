//! Document ingestion CLI client (WP-1.6, D-67, TB-1).

use std::path::PathBuf;
use std::time::Duration;

use clap::Args;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};

use crate::gateway::routes::documents::{
    IngestDocumentRequest, IngestDocumentResponse, IngestionJobStatusResponse,
};
use crate::gateway::routes::staging::{StagingApproveRequest, StagingApproveResponse};

/// Arguments for `tks ingest <file> [--doc-path <path>] [--approve] [--timeout <secs>]`.
#[derive(Debug, Args, Clone)]
pub struct IngestArgs {
    /// Filesystem path to the Markdown document to ingest.
    pub file: PathBuf,

    /// Target logical document path in the repository (e.g. `specs/vision.md`).
    /// If omitted, defaults to `specs/<filename>`.
    #[arg(long)]
    pub doc_path: Option<String>,

    /// Automatically wait for AST decomposition and approve candidate drafts into ACTIVE state.
    #[arg(long, default_value_t = false)]
    pub approve: bool,

    /// Maximum time in seconds to wait for AST decomposition (default: 30).
    #[arg(long, default_value_t = 30)]
    pub timeout: u64,
}

/// Executes the `tks ingest` CLI command.
///
/// # Errors
///
/// Returns an error if reading the file, HTTP transmission, or staging approval fails.
pub async fn run_ingest(
    args: IngestArgs,
    server_url: &str,
    auth_token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let base_url = server_url.trim_end_matches('/');

    let content = tokio::fs::read_to_string(&args.file)
        .await
        .map_err(|e| format!("Failed to read file '{}': {e}", args.file.display()))?;

    let doc_path = args.doc_path.unwrap_or_else(|| {
        let file_name = args
            .file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document.md");
        format!("specs/{file_name}")
    });

    let ingest_url = format!("{base_url}/api/v1/documents/ingest");
    let payload = IngestDocumentRequest {
        doc_path: doc_path.clone(),
        content,
    };

    let resp = client
        .post(&ingest_url)
        .header(AUTHORIZATION, format!("Bearer {auth_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&payload)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_text = resp.text().await.unwrap_or_default();
        return Err(format!("Ingestion failed ({status}): {err_text}").into());
    }

    let ingest_resp: IngestDocumentResponse = resp.json().await?;
    let job_id = ingest_resp.job_id;

    println!("Ingestion Job Created:");
    println!("  Job ID:    {job_id}");
    println!("  Doc Path:  {doc_path}");
    println!("  Status:    {}", ingest_resp.status);

    if args.approve {
        eprint!("Waiting for AST decomposition...");
        let start = std::time::Instant::now();
        let max_duration = Duration::from_secs(args.timeout);
        let mut status = ingest_resp.status;

        while status != "STAGED" && start.elapsed() < max_duration {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let status_url = format!("{base_url}/api/v1/documents/ingest/{job_id}");
            let check_resp = client
                .get(&status_url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .send()
                .await?;

            if check_resp.status().is_success() {
                let info: IngestionJobStatusResponse = check_resp.json().await?;
                status = info.status;
                if status == "FAILED" {
                    eprintln!();
                    return Err(format!(
                        "Decomposition failed: {}",
                        info.error_message
                            .unwrap_or_else(|| "unknown error".to_string())
                    )
                    .into());
                }
            }
        }

        if status != "STAGED" {
            eprintln!();
            return Err(format!(
                "Timed out waiting for decomposition to reach STAGED (current: {status})"
            )
            .into());
        }

        eprintln!(" done.");
        println!("Approving candidate drafts...");

        let approve_url = format!("{base_url}/api/v1/documents/ingest/{job_id}/approve");
        let approve_payload = StagingApproveRequest {
            job_id: Some(job_id),
            approved_node_ids: None,
        };

        let approve_resp = client
            .post(&approve_url)
            .header(AUTHORIZATION, format!("Bearer {auth_token}"))
            .header(CONTENT_TYPE, "application/json")
            .json(&approve_payload)
            .send()
            .await?;

        if !approve_resp.status().is_success() {
            let s = approve_resp.status();
            let err = approve_resp.text().await.unwrap_or_default();
            return Err(format!("Approval failed ({s}): {err}").into());
        }

        let approve_result: StagingApproveResponse = approve_resp.json().await?;
        println!("Staging Approval Succeeded:");
        println!("  Approved Nodes: {}", approve_result.approved_nodes);
        println!("  Purged Drafts:  {}", approve_result.purged_drafts);
        println!("  Batch ID:       {}", approve_result.batch_id);
    }

    Ok(())
}
