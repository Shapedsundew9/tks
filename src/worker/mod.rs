//! Cooperative background worker manager orchestrating decomposition and offline vector queue loops (WP-1.4).

pub mod decomp;
pub mod embedding;

pub use decomp::{
    DEFAULT_DECOMP_POLL_INTERVAL, DecompError, JOB_TIMEOUT_SECONDS, process_one_job,
    run_decomposition_worker,
};
pub use embedding::{
    DEFAULT_EMBEDDING_POLL_INTERVAL, EMBEDDING_BATCH_SIZE, EmbeddingError, LocalEmbeddingGenerator,
    MAX_EMBEDDING_RETRIES, TARGET_VECTOR_DIM, process_embedding_batch, run_embedding_worker,
};

use deadpool_postgres::Pool;
use tokio_util::sync::CancellationToken;

use crate::storage::git::{GitReadHandle, GitWriteHandle, resolve_git_dir};

/// Cooperative background worker manager running inside the `tks serve` daemon.
pub struct WorkerManager {
    pool: Pool,
    git_handle: GitWriteHandle,
    cancel_token: CancellationToken,
    git_read: GitReadHandle,
}

impl WorkerManager {
    /// Creates a new `WorkerManager` instance.
    #[must_use]
    pub fn new(pool: Pool, git_handle: GitWriteHandle, cancel_token: CancellationToken) -> Self {
        let git_read = GitReadHandle::new(resolve_git_dir());
        Self {
            pool,
            git_handle,
            cancel_token,
            git_read,
        }
    }

    /// Configures a custom `GitReadHandle` (useful for integration tests).
    #[must_use]
    pub fn with_git_read(mut self, git_read: GitReadHandle) -> Self {
        self.git_read = git_read;
        self
    }

    /// Accessor for underlying database connection pool.
    #[must_use]
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Accessor for Git write handle.
    #[must_use]
    pub fn git_handle(&self) -> &GitWriteHandle {
        &self.git_handle
    }

    /// Accessor for cancellation token.
    #[must_use]
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel_token
    }

    /// Accessor for Git read handle.
    #[must_use]
    pub fn git_read(&self) -> &GitReadHandle {
        &self.git_read
    }

    /// Spawns the cooperative background workers and returns a join handle.
    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.run().await;
        })
    }

    /// Runs the cooperative worker loops until cancelled.
    pub async fn run(self) {
        let decomp_handle = tokio::spawn(run_decomposition_worker(
            self.pool.clone(),
            self.git_read.clone(),
            self.cancel_token.clone(),
        ));

        let embed_handle = tokio::spawn(run_embedding_worker(
            self.pool.clone(),
            self.cancel_token.clone(),
        ));

        let _ = tokio::join!(decomp_handle, embed_handle);
    }
}

/// Spawns the cooperative background worker manager and returns a `JoinHandle`.
pub async fn start_worker_manager(
    pool: Pool,
    git_handle: GitWriteHandle,
    cancel_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    WorkerManager::new(pool, git_handle, cancel_token).start()
}
