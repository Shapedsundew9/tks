//! Document decomposition background worker (WP-1.4, TB-7).
//!
//! Claims `QUEUED` or timed-out `PROCESSING` jobs from `ingestion_jobs`,
//! reads Git blobs via `GitReadHandle`, executes CommonMark AST decomposition
//! and 3-tier document reconciliation, writes candidate `DRAFT` nodes and edges,
//! and executes conditional status transition to `STAGED` with atomic draft purge.

use deadpool_postgres::Pool;
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::ingest::parser::parse_markdown;
use crate::ingest::reconcile::{
    get_node_ast_anchor, reconcile_reingestion_with_options, stage_reanchoring_metadata,
};
use crate::storage::GraphNode;
use crate::storage::git::{GitError, GitReadHandle};

/// Timeout threshold in seconds after which a `PROCESSING` job is considered stuck and reclaimed.
pub const JOB_TIMEOUT_SECONDS: i64 = 180;

/// Default polling interval when the ingestion job queue is empty.
pub const DEFAULT_DECOMP_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Errors occurring during document decomposition worker execution.
#[derive(Debug)]
pub enum DecompError {
    Database(tokio_postgres::Error),
    Pool(deadpool_postgres::PoolError),
    Git(GitError),
    Ingest(crate::ingest::IngestError),
    Serialization(serde_json::Error),
}

impl fmt::Display for DecompError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(e) => write!(f, "database error: {e}"),
            Self::Pool(e) => write!(f, "connection pool error: {e}"),
            Self::Git(e) => write!(f, "git storage error: {e}"),
            Self::Ingest(e) => write!(f, "ingestion error: {e}"),
            Self::Serialization(e) => write!(f, "serialization error: {e}"),
        }
    }
}

impl std::error::Error for DecompError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(e) => Some(e),
            Self::Pool(e) => Some(e),
            Self::Git(e) => Some(e),
            Self::Ingest(e) => Some(e),
            Self::Serialization(e) => Some(e),
        }
    }
}

impl From<tokio_postgres::Error> for DecompError {
    fn from(e: tokio_postgres::Error) -> Self {
        Self::Database(e)
    }
}

impl From<deadpool_postgres::PoolError> for DecompError {
    fn from(e: deadpool_postgres::PoolError) -> Self {
        Self::Pool(e)
    }
}

impl From<GitError> for DecompError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

impl From<crate::ingest::IngestError> for DecompError {
    fn from(e: crate::ingest::IngestError) -> Self {
        Self::Ingest(e)
    }
}

impl From<serde_json::Error> for DecompError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization(e)
    }
}

/// Runs the continuous decomposition worker loop with cooperative cancellation.
pub async fn run_decomposition_worker(
    pool: Pool,
    git_read: GitReadHandle,
    cancel_token: CancellationToken,
) {
    let mut empty_ticks = 0u32;

    loop {
        if cancel_token.is_cancelled() {
            tracing::info!("Decomposition worker shutting down gracefully.");
            break;
        }

        match process_one_job(&pool, &git_read).await {
            Ok(true) => {
                empty_ticks = 0;
                // Yield to allow other tasks to progress before processing next job immediately
                tokio::task::yield_now().await;
            }
            Ok(false) => {
                empty_ticks = empty_ticks.saturating_add(1);
                let backoff_ms = (100 + (empty_ticks as u64 * 50)).min(500);
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        tracing::info!("Decomposition worker received cancellation.");
                        break;
                    }
                    _ = tokio::time::sleep(Duration::from_millis(backoff_ms)) => {}
                }
            }
            Err(e) => {
                tracing::error!("Error in decomposition worker tick: {e}");
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        break;
                    }
                    _ = tokio::time::sleep(DEFAULT_DECOMP_POLL_INTERVAL) => {}
                }
            }
        }
    }
}

/// Attempts to claim and process a single `QUEUED` or timed-out `PROCESSING` job.
///
/// Returns:
/// - `Ok(true)` if a job was claimed and processed.
/// - `Ok(false)` if no jobs were pending.
/// - `Err(e)` on unhandled database or infrastructure error.
pub async fn process_one_job(pool: &Pool, git_read: &GitReadHandle) -> Result<bool, DecompError> {
    let mut client = pool.get().await?;

    // Step 1: Claim a job in a transaction using FOR UPDATE SKIP LOCKED.
    // Prioritize timed-out PROCESSING jobs (> 180s) over QUEUED jobs.
    let tx = client.transaction().await?;

    let claim_query = "SELECT job_id, doc_path, document_hash, status \
         FROM ingestion_jobs \
         WHERE status = 'QUEUED' \
            OR (status = 'PROCESSING' AND updated_at < clock_timestamp() - (INTERVAL '1 second' * $1)) \
         ORDER BY \
            CASE WHEN status = 'PROCESSING' THEN 0 ELSE 1 END, \
            created_at ASC \
         LIMIT 1 \
         FOR UPDATE SKIP LOCKED;";

    let row_opt = tx
        .query_opt(claim_query, &[&(JOB_TIMEOUT_SECONDS as f64)])
        .await?;

    let (job_id, doc_path, document_hash, is_timeout_reclaim) = match row_opt {
        Some(row) => {
            let job_id: Uuid = row.get("job_id");
            let doc_path: String = row.get("doc_path");
            let document_hash: String = row.get("document_hash");
            let status: String = row.get("status");
            let is_timeout = status == "PROCESSING";

            if is_timeout {
                // TB-7.6 & D-79: On reclaiming timed-out PROCESSING jobs (> 180s),
                // atomically purge any partial candidate draft nodes before re-executing.
                tx.execute(
                    "DELETE FROM graph_nodes WHERE job_id = $1 AND lifecycle_state = 'DRAFT';",
                    &[&job_id],
                )
                .await?;

                tx.execute(
                    "UPDATE ingestion_jobs \
                     SET status = 'PROCESSING', retry_count = retry_count + 1, updated_at = clock_timestamp() \
                     WHERE job_id = $1;",
                    &[&job_id],
                )
                .await?;
            } else {
                tx.execute(
                    "UPDATE ingestion_jobs \
                     SET status = 'PROCESSING', updated_at = clock_timestamp() \
                     WHERE job_id = $1 AND status = 'QUEUED';",
                    &[&job_id],
                )
                .await?;
            }

            (job_id, doc_path, document_hash, is_timeout)
        }
        None => {
            tx.rollback().await?;
            return Ok(false);
        }
    };

    tx.commit().await?;

    if is_timeout_reclaim {
        tracing::warn!("Reclaimed timed-out ingestion job {job_id} for path '{doc_path}'");
    } else {
        tracing::info!("Claimed ingestion job {job_id} for path '{doc_path}'");
    }

    // Step 2: Load document content from Git via GitReadHandle.
    let content = match git_read.read_blob(&document_hash).await {
        Ok(text) => text,
        Err(e) => {
            tracing::error!("Failed to load git blob '{document_hash}' for job {job_id}: {e}");
            let client = pool.get().await?;
            client
                .execute(
                    "UPDATE ingestion_jobs \
                     SET status = 'FAILED', error_message = $2, updated_at = clock_timestamp() \
                     WHERE job_id = $1;",
                    &[&job_id, &e.to_string()],
                )
                .await?;
            return Ok(true);
        }
    };

    // Step 3: Execute Stage 1 mechanical AST decomposition.
    let decomp = match parse_markdown(&doc_path, &content) {
        Ok(res) => res,
        Err(e) => {
            tracing::error!("AST decomposition failed for job {job_id}: {e}");
            let client = pool.get().await?;
            client
                .execute(
                    "UPDATE ingestion_jobs \
                     SET status = 'FAILED', error_message = $2, updated_at = clock_timestamp() \
                     WHERE job_id = $1;",
                    &[&job_id, &e.to_string()],
                )
                .await?;
            return Ok(true);
        }
    };

    // Step 4: Query existing active nodes for doc_path to perform 3-tier document reconciliation.
    let active_rows = client
        .query(
            "SELECT id, node_key, node_type, title, content, lifecycle_state, \
             governance_policy, created_by, job_id, doc_path, doc_hash, \
             byte_start, byte_end, attributes \
             FROM graph_nodes \
             WHERE doc_path = $1 AND lifecycle_state = 'ACTIVE';",
            &[&doc_path],
        )
        .await?;

    let existing_active_nodes: Vec<GraphNode> =
        active_rows.iter().map(GraphNode::from_row).collect();

    // Step 5: Execute 3-tier document reconciliation.
    let plan = reconcile_reingestion_with_options(
        &decomp.chunks,
        &existing_active_nodes,
        Some(&doc_path),
        Some(&document_hash),
    );

    // Step 6: Write candidate DRAFT nodes and hierarchy edges in a transaction.
    let tx = client.transaction().await?;

    let mut anchor_to_id: HashMap<String, Uuid> = HashMap::new();

    // Index active nodes by AST anchor so structural edges pointing to active parent headings resolve
    for active_node in &existing_active_nodes {
        if let Some(anchor) = get_node_ast_anchor(active_node) {
            anchor_to_id.insert(anchor.to_string(), active_node.id);
        }
    }

    // Insert candidate DRAFT nodes.
    // Ensure candidate draft nodes NEVER enqueue embeddings (WP-1.4 Task 2).
    for draft in &plan.new_draft_nodes {
        let node_id = Uuid::new_v4();
        anchor_to_id.insert(draft.ast_anchor.clone(), node_id);

        tx.execute(
            "INSERT INTO graph_nodes ( \
                id, node_key, node_type, title, content, lifecycle_state, \
                governance_policy, created_by, job_id, doc_path, doc_hash, \
                byte_start, byte_end, attributes \
             ) VALUES ($1, $2, $3, $4, $5, 'DRAFT', $6, 'system', $7, $8, $9, $10, $11, $12);",
            &[
                &node_id,
                &draft.node_key,
                &draft.node_type,
                &draft.title,
                &draft.content,
                &draft.governance_policy,
                &job_id,
                &draft.doc_path,
                &draft.doc_hash,
                &draft.byte_start,
                &draft.byte_end,
                &draft.attributes,
            ],
        )
        .await?;
    }

    // Insert candidate DRAFT hierarchy edges (`DERIVED_FROM`)
    for edge in &decomp.edges {
        if let (Some(&from_id), Some(&to_id)) = (
            anchor_to_id.get(&edge.from_anchor),
            anchor_to_id.get(&edge.to_anchor),
        ) {
            let edge_id = Uuid::new_v4();
            tx.execute(
                "INSERT INTO graph_edges ( \
                    edge_id, from_node_id, to_node_id, edge_type, created_by, lifecycle_state \
                 ) VALUES ($1, $2, $3, $4, 'system', 'DRAFT') \
                 ON CONFLICT DO NOTHING;",
                &[&edge_id, &from_id, &to_id, &edge.edge_type],
            )
            .await?;
        }
    }

    // Step 7: Enforce worker concurrency hardening (TB-7.6, D-79).
    // Record candidate re-anchoring metadata in ingestion_jobs attributes (TB-7.5).
    let reanchoring_metadata = stage_reanchoring_metadata(&plan.reanchored_spans);
    let job_attributes = serde_json::json!({
        "candidate_reanchoring": reanchoring_metadata,
    });

    // Execute conditional terminal status transition:
    // UPDATE ingestion_jobs SET status = 'STAGED', updated_at = NOW() WHERE job_id = $1 AND status = 'PROCESSING';
    let rows_updated = tx
        .execute(
            "UPDATE ingestion_jobs \
             SET status = 'STAGED', attributes = $2, updated_at = clock_timestamp() \
             WHERE job_id = $1 AND status = 'PROCESSING';",
            &[&job_id, &job_attributes],
        )
        .await?;

    if rows_updated == 0 {
        // Zero rows updated indicates the job was cancelled or superseded during processing.
        // Immediately abort and purge candidate drafts to eliminate zombie residue.
        tracing::warn!(
            "Job {job_id} was superseded or cancelled; purging candidate drafts immediately"
        );
        tx.execute(
            "DELETE FROM graph_nodes WHERE job_id = $1 AND lifecycle_state = 'DRAFT';",
            &[&job_id],
        )
        .await?;
    }

    tx.commit().await?;

    tracing::info!(
        "Decomposition complete for job {job_id}: {} drafts staged, {} spans re-anchored, status = STAGED",
        plan.new_draft_nodes.len(),
        plan.reanchored_spans.len()
    );

    Ok(true)
}
