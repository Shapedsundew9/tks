//! Terminal staging review CLI client (WP-1.6, D-83, TB-5).

use clap::{Args, Subcommand};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use uuid::Uuid;

use crate::gateway::routes::documents::IngestionJobStatusResponse;
use crate::gateway::routes::staging::{
    CandidateNodeResponse, InspectNodeResponse, StagingApproveRequest, StagingApproveResponse,
    StagingRejectRequest, StagingRejectResponse,
};

/// Subcommands for candidate draft review and staging promotion/rejection.
#[derive(Debug, Subcommand)]
pub enum StagingSubcommand {
    /// List candidate draft nodes for an ingestion job.
    List(StagingListArgs),
    /// Inspect node metadata and verbatim document span from Git ODB.
    Inspect(StagingInspectArgs),
    /// Approve an ingestion job, promoting candidate drafts to ACTIVE.
    Approve(StagingApproveArgs),
    /// Reject an ingestion job or specific candidate drafts.
    Reject(StagingRejectArgs),
}

/// Arguments for `tks staging list <job_id> [--triage-anomalies]`.
#[derive(Debug, Args)]
pub struct StagingListArgs {
    /// Ingestion job ID to list candidate drafts for.
    pub job_id: Uuid,

    /// Sort by contradiction risk and confidence ascending for anomaly exception triage.
    #[arg(long)]
    pub triage_anomalies: bool,
}

/// Arguments for `tks staging inspect <node_id>`.
#[derive(Debug, Args)]
pub struct StagingInspectArgs {
    /// Polymorphic node identifier (UUID or canonical key).
    pub node_id: String,
}

/// Arguments for `tks staging approve <job_id> [--only <id,...> | --exclude <id,...>] [--confirm-omissions]`.
#[derive(Debug, Args)]
pub struct StagingApproveArgs {
    /// Ingestion job ID to approve.
    pub job_id: Uuid,

    /// Optional comma-separated list of candidate node IDs to approve.
    #[arg(long, value_delimiter = ',')]
    pub only: Option<Vec<Uuid>>,

    /// Optional comma-separated list of candidate node IDs to exclude from approval.
    #[arg(long, value_delimiter = ',')]
    pub exclude: Option<Vec<Uuid>>,

    /// Confirm deprecation/omission of active nodes omitted in this revision.
    #[arg(long)]
    pub confirm_omissions: bool,
}

/// Arguments for `tks staging reject [--job-id <job_id> | <node_id>]`.
#[derive(Debug, Args)]
pub struct StagingRejectArgs {
    /// Candidate draft node ID to reject (positional).
    pub node_id: Option<String>,

    /// Ingestion job ID to reject.
    #[arg(long)]
    pub job_id: Option<Uuid>,

    /// Optional rejection reason for diagnostics.
    #[arg(long)]
    pub reason: Option<String>,
}

/// Executes the `tks staging` CLI subcommand.
///
/// # Errors
///
/// Returns an error if HTTP request fails or server returns an error.
pub async fn run_staging(
    subcmd: StagingSubcommand,
    server_url: &str,
    auth_token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let base_url = server_url.trim_end_matches('/');

    match subcmd {
        StagingSubcommand::List(args) => {
            let candidates_url = format!(
                "{base_url}/api/v1/staging/{}/candidates?triage_anomalies={}",
                args.job_id, args.triage_anomalies
            );

            let resp = client
                .get(&candidates_url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to fetch candidates {status}: {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let candidates: Vec<CandidateNodeResponse> = resp.json().await?;

            if args.triage_anomalies {
                println!("Staging Anomaly Triage (Job: {}):", args.job_id);
                println!("  Candidates ({}):", candidates.len());
                for node in &candidates {
                    let key = node.node_key.as_deref().unwrap_or("<no-key>");
                    let title = node.title.as_deref().unwrap_or("<no-title>");
                    let start = node.byte_start.unwrap_or(0);
                    let end = node.byte_end.unwrap_or(0);
                    let modality = node
                        .extraction_metadata
                        .get("modality")
                        .and_then(|v| v.as_str())
                        .unwrap_or("SHALL");
                    let conf = node
                        .extraction_metadata
                        .get("confidence")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0);
                    let risk = node
                        .extraction_metadata
                        .get("contradiction_risk")
                        .and_then(|v| v.as_str())
                        .unwrap_or("LOW");

                    println!(
                        "    [{}] {} | {} | \"{}\" | bytes: {start}..{end} [{}] [CONF: {:.2}] [CONTRADICTION RISK: {}]",
                        node.id,
                        key,
                        node.node_type,
                        title,
                        modality.to_uppercase(),
                        conf,
                        risk.to_uppercase()
                    );
                    if conf < 0.70 {
                        println!(
                            "      ⚠️  ADVISORY: Low extraction confidence ({:.2}) - supervisor verification required",
                            conf
                        );
                    }
                }
            } else {
                let doc_path = candidates
                    .first()
                    .and_then(|c| c.doc_path.as_deref())
                    .unwrap_or("<unknown>");
                println!("Staging Candidates (Job: {}):", args.job_id);
                println!("  Document Path: {doc_path}");
                println!("  Candidates ({}):", candidates.len());

                for node in &candidates {
                    let key = node.node_key.as_deref().unwrap_or("<no-key>");
                    let title = node.title.as_deref().unwrap_or("<no-title>");
                    let start = node.byte_start.unwrap_or(0);
                    let end = node.byte_end.unwrap_or(0);
                    let modality = node
                        .extraction_metadata
                        .get("modality")
                        .and_then(|v| v.as_str())
                        .unwrap_or("SHALL");
                    let conf = node
                        .extraction_metadata
                        .get("confidence")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0);
                    let risk = node
                        .extraction_metadata
                        .get("contradiction_risk")
                        .and_then(|v| v.as_str())
                        .unwrap_or("LOW");

                    let is_child = node.parent_anchor.is_some();
                    let prefix = if is_child { "      └─" } else { "    " };

                    println!(
                        "{} [{}] {} | {} | \"{}\" | bytes: {start}..{end} [{}] [CONF: {:.2}] [CONTRADICTION RISK: {}]",
                        prefix,
                        node.id,
                        key,
                        node.node_type,
                        title,
                        modality.to_uppercase(),
                        conf,
                        risk.to_uppercase()
                    );
                    if conf < 0.70 {
                        let adv_prefix = if is_child { "         " } else { "      " };
                        println!(
                            "{}⚠️  ADVISORY: Low extraction confidence ({:.2}) - supervisor verification required",
                            adv_prefix, conf
                        );
                    }
                }
            }
        }

        StagingSubcommand::Inspect(args) => {
            let url = format!("{base_url}/api/v1/staging/inspect/{}", args.node_id);

            let resp = client
                .get(&url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to inspect node {status}: {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let inspect_info: InspectNodeResponse = resp.json().await?;
            let node = &inspect_info.node;

            println!("Node Inspection: {}", node.id);
            if let Some(key) = &node.node_key {
                println!("  Node Key:          {key}");
            }
            println!("  Node Type:         {}", node.node_type);
            println!(
                "  Title:             {}",
                node.title.as_deref().unwrap_or("<no-title>")
            );
            println!("  Lifecycle State:   {}", node.lifecycle_state);
            println!("  Governance Policy: {}", node.governance_policy);
            println!("  Created By:        {}", node.created_by);
            if let Some(doc_path) = &node.doc_path {
                println!("  Doc Path:          {doc_path}");
            }
            if let Some(doc_hash) = &node.doc_hash {
                println!("  Doc Hash:          {doc_hash}");
            }
            if let (Some(start), Some(end)) = (node.byte_start, node.byte_end) {
                println!("  Byte Span:         {start}..{end}");
            }
            if let Some(content) = &node.content {
                println!("  Content:           {content}");
            }

            if let Some(span_text) = &inspect_info.span_text {
                println!("\n--- Verbatim Document Span (Git ODB) ---");
                println!("{span_text}");
                println!("----------------------------------------\n");
            }
        }

        StagingSubcommand::Approve(args) => {
            let mut approved_node_ids = args.only;

            if let Some(exclude_ids) = args.exclude {
                // Fetch candidate nodes for the job first
                let url = format!("{base_url}/api/v1/documents/ingest/{}", args.job_id);
                let resp = client
                    .get(&url)
                    .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                    .send()
                    .await?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let err_text = resp.text().await.unwrap_or_default();
                    eprintln!("Failed to fetch job for exclusion filter {status}: {err_text}");
                    return Err(format!("Server returned {status}: {err_text}").into());
                }

                let job_info: IngestionJobStatusResponse = resp.json().await?;
                let remaining_ids: Vec<Uuid> = job_info
                    .nodes
                    .iter()
                    .map(|n| n.id)
                    .filter(|id| !exclude_ids.contains(id))
                    .collect();

                approved_node_ids = Some(remaining_ids);
            }

            let approve_url = format!("{base_url}/api/v1/documents/ingest/{}/approve", args.job_id);
            let payload = StagingApproveRequest {
                job_id: Some(args.job_id),
                approved_node_ids,
                confirm_omissions: if args.confirm_omissions {
                    Some(true)
                } else {
                    None
                },
                confirmed_omitted_node_ids: None,
                review_duration_seconds: None,
            };

            let resp = client
                .post(&approve_url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .header(CONTENT_TYPE, "application/json")
                .json(&payload)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to approve staging {status}: {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let result: StagingApproveResponse = resp.json().await?;
            println!("Staging Approval Succeeded:");
            println!("  Approved Nodes: {}", result.approved_nodes);
            println!("  Purged Drafts:  {}", result.purged_drafts);
            println!("  Batch ID:       {}", result.batch_id);
        }

        StagingSubcommand::Reject(args) => {
            let reject_url = format!("{base_url}/api/v1/staging/reject");

            let (job_id, rejected_node_ids) = match (args.job_id, args.node_id) {
                (Some(jid), None) => (Some(jid), None),
                (jid_opt, Some(nid_str)) => {
                    let node_uuid = if let Ok(u) = Uuid::parse_str(&nid_str) {
                        u
                    } else {
                        // Inspect node to find UUID
                        let inspect_url = format!("{base_url}/api/v1/staging/inspect/{nid_str}");
                        let resp = client
                            .get(&inspect_url)
                            .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                            .send()
                            .await?;
                        if !resp.status().is_success() {
                            return Err(format!("Node '{nid_str}' not found for rejection").into());
                        }
                        let inspect_info: InspectNodeResponse = resp.json().await?;
                        inspect_info.node.id
                    };
                    (jid_opt, Some(vec![node_uuid]))
                }
                (None, None) => {
                    eprintln!(
                        "Error: Either --job-id <job_id> or <node_id> must be specified for staging reject."
                    );
                    return Err("Missing target for staging reject".into());
                }
            };

            let payload = StagingRejectRequest {
                job_id,
                reason: args.reason,
                rejected_node_ids,
                ..Default::default()
            };

            let resp = client
                .post(&reject_url)
                .header(AUTHORIZATION, format!("Bearer {auth_token}"))
                .header(CONTENT_TYPE, "application/json")
                .json(&payload)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("Failed to reject staging {status}: {err_text}");
                return Err(format!("Server returned {status}: {err_text}").into());
            }

            let result: StagingRejectResponse = resp.json().await?;
            println!("Staging Rejection Succeeded:");
            println!("  Rejected Job ID: {}", result.rejected_job_id);
            println!("  Purged Drafts:   {}", result.purged_drafts);
        }
    }

    Ok(())
}
