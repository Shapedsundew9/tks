//! Background cooperative invalidation worker task processing asynchronous invalidation sweeps (WP-3.1, PHASE3-002).
//!
//! Processes deferred or large-scale invalidation sweeps cooperatively from an asynchronous channel
//! without stalling HTTP/MCP client responses.

use std::fmt;

use deadpool_postgres::Pool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::gateway::auth::AuthenticatedAgent;
use crate::storage::cascade::{CascadeInvalidationResult, trigger_downward_invalidation};
use crate::storage::mutation::MutationError;

/// Deferred or large-scale invalidation job dispatched to `CascadeWorker`.
#[derive(Debug, Clone)]
pub struct CascadeJob {
    /// Root node identifier whose modification or degradation triggered the invalidation sweep.
    pub root_node_id: Uuid,
    /// Human- or agent-readable rationale describing why the invalidation occurred.
    pub reason: String,
    /// Calling agent identity initiating the cascade.
    pub caller: AuthenticatedAgent,
}

/// Errors originating during background invalidation worker execution.
#[derive(Debug)]
pub enum CascadeWorkerError {
    Pool(deadpool_postgres::PoolError),
    Mutation(MutationError),
}

impl fmt::Display for CascadeWorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pool(e) => write!(f, "Connection pool error: {e}"),
            Self::Mutation(e) => write!(f, "Cascade mutation error: {e}"),
        }
    }
}

impl std::error::Error for CascadeWorkerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Pool(e) => Some(e),
            Self::Mutation(e) => Some(e),
        }
    }
}

impl From<deadpool_postgres::PoolError> for CascadeWorkerError {
    fn from(e: deadpool_postgres::PoolError) -> Self {
        Self::Pool(e)
    }
}

impl From<MutationError> for CascadeWorkerError {
    fn from(e: MutationError) -> Self {
        Self::Mutation(e)
    }
}

/// Processes a single deferred cascade invalidation job.
///
/// # Errors
///
/// Returns `CascadeWorkerError` on connection pool or cascade execution failure.
pub async fn process_cascade_job(
    pool: &Pool,
    job: &CascadeJob,
) -> Result<CascadeInvalidationResult, CascadeWorkerError> {
    let mut client = pool.get().await?;
    let result =
        trigger_downward_invalidation(&mut client, job.root_node_id, &job.reason, &job.caller)
            .await?;
    Ok(result)
}

/// Runs the continuous cooperative invalidation worker loop with cooperative cancellation.
pub async fn run_cascade_worker(
    pool: Pool,
    mut rx: tokio::sync::mpsc::Receiver<CascadeJob>,
    cancel_token: CancellationToken,
) {
    tracing::info!("Cascade invalidation background worker started.");

    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                tracing::info!("Cascade invalidation worker received cancellation.");
                break;
            }
            job_opt = rx.recv() => {
                match job_opt {
                    Some(job) => {
                        tracing::debug!(
                            root_node_id = %job.root_node_id,
                            reason = %job.reason,
                            "Processing background cascade invalidation job"
                        );
                        match process_cascade_job(&pool, &job).await {
                            Ok(res) => {
                                tracing::info!(
                                    root_node_id = %res.root_node_id,
                                    invalidated_count = res.invalidated_nodes.len(),
                                    max_depth = res.max_depth,
                                    batch_id = %res.batch_id,
                                    "Successfully processed background cascade invalidation job"
                                );
                            }
                            Err(err) => {
                                tracing::error!(
                                    root_node_id = %job.root_node_id,
                                    error = %err,
                                    "Background cascade invalidation job failed"
                                );
                            }
                        }
                        tokio::task::yield_now().await;
                    }
                    None => {
                        tracing::info!("Cascade worker job channel closed, terminating loop.");
                        break;
                    }
                }
            }
        }
    }
}
