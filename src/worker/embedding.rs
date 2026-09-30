//! Offline local vector queue daemon worker (WP-1.4, D-48, D-77, D-82).
//!
//! Claims `PENDING` records in `node_embeddings` with `scheduled_at <= clock_timestamp() FOR UPDATE SKIP LOCKED`,
//! executes local CPU vector inference via `fastembed` (`all-MiniLM-L6-v2`, 384 dimensions),
//! updates `COMPLETED` vector, and calculates exponential retry backoff on failures.

use deadpool_postgres::Pool;
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[cfg(feature = "vector-spike")]
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

/// Expected vector dimensionality for `all-MiniLM-L6-v2` per Decision D-77.
pub const TARGET_VECTOR_DIM: usize = 384;

/// Maximum retry attempts before marking an embedding job as permanently FAILED.
pub const MAX_EMBEDDING_RETRIES: i32 = 5;

/// Batch size for claiming pending vector embedding records.
pub const EMBEDDING_BATCH_SIZE: i64 = 10;

/// Default polling interval when the vector queue is empty.
pub const DEFAULT_EMBEDDING_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Errors occurring during local vector embedding worker execution.
#[derive(Debug)]
pub enum EmbeddingError {
    Database(tokio_postgres::Error),
    Pool(deadpool_postgres::PoolError),
    InitializationFailed(String),
    InferenceFailed(String),
    DimensionMismatch { expected: usize, actual: usize },
}

impl fmt::Display for EmbeddingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(e) => write!(f, "database error: {e}"),
            Self::Pool(e) => write!(f, "connection pool error: {e}"),
            Self::InitializationFailed(msg) => {
                write!(f, "embedding model initialization failed: {msg}")
            }
            Self::InferenceFailed(msg) => write!(f, "embedding inference failed: {msg}"),
            Self::DimensionMismatch { expected, actual } => {
                write!(
                    f,
                    "unexpected vector dimension: expected {expected}, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for EmbeddingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(e) => Some(e),
            Self::Pool(e) => Some(e),
            _ => None,
        }
    }
}

impl From<tokio_postgres::Error> for EmbeddingError {
    fn from(e: tokio_postgres::Error) -> Self {
        Self::Database(e)
    }
}

impl From<deadpool_postgres::PoolError> for EmbeddingError {
    fn from(e: deadpool_postgres::PoolError) -> Self {
        Self::Pool(e)
    }
}

/// Embedded vector generator wrapping `fastembed` for CPU execution.
pub struct LocalEmbeddingGenerator {
    #[cfg(feature = "vector-spike")]
    model: TextEmbedding,
    expected_dim: usize,
}

impl LocalEmbeddingGenerator {
    /// Initialize with the default Phase 1 embedding model: `all-MiniLM-L6-v2` (384-d).
    #[cfg(feature = "vector-spike")]
    pub fn new() -> Result<Self, EmbeddingError> {
        let options = InitOptions::new(EmbeddingModel::AllMiniLML6V2)
            .with_show_download_progress(false)
            .with_max_length(256);
        let model = TextEmbedding::try_new(options)
            .map_err(|e| EmbeddingError::InitializationFailed(e.to_string()))?;
        Ok(Self {
            model,
            expected_dim: TARGET_VECTOR_DIM,
        })
    }

    /// Fallback initialization when `vector-spike` feature is disabled.
    #[cfg(not(feature = "vector-spike"))]
    pub fn new() -> Result<Self, EmbeddingError> {
        Ok(Self {
            expected_dim: TARGET_VECTOR_DIM,
        })
    }

    /// Expected embedding vector dimensionality (384).
    #[must_use]
    pub fn vector_dim(&self) -> usize {
        self.expected_dim
    }

    /// Embed a batch of text chunks, returning a vector of vectors.
    #[cfg(feature = "vector-spike")]
    pub fn embed_batch<S: AsRef<str> + Send + Sync>(
        &self,
        texts: &[S],
        batch_size: Option<usize>,
    ) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let text_vec: Vec<&str> = texts.iter().map(|s| s.as_ref()).collect();
        let embeddings = self
            .model
            .embed(text_vec, batch_size)
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;

        for emb in &embeddings {
            if emb.len() != self.expected_dim {
                return Err(EmbeddingError::DimensionMismatch {
                    expected: self.expected_dim,
                    actual: emb.len(),
                });
            }
        }

        Ok(embeddings)
    }

    /// Fallback batch embedding when `vector-spike` feature is disabled.
    #[cfg(not(feature = "vector-spike"))]
    pub fn embed_batch<S: AsRef<str> + Send + Sync>(
        &self,
        texts: &[S],
        _batch_size: Option<usize>,
    ) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let mut results = Vec::with_capacity(texts.len());
        for _ in texts {
            let mut v = vec![0.0f32; self.expected_dim];
            v[0] = 1.0; // Unit vector
            results.push(v);
        }
        Ok(results)
    }
}

/// Runs the continuous vector queue worker loop with cooperative cancellation.
pub async fn run_embedding_worker(pool: Pool, cancel_token: CancellationToken) {
    let embedder = match LocalEmbeddingGenerator::new() {
        Ok(generator) => generator,
        Err(e) => {
            tracing::error!("Failed to initialize local embedding generator: {e}");
            return;
        }
    };

    let mut empty_ticks = 0u32;

    loop {
        if cancel_token.is_cancelled() {
            tracing::info!("Embedding worker shutting down gracefully.");
            break;
        }

        match process_embedding_batch(&pool, &embedder).await {
            Ok(true) => {
                empty_ticks = 0;
                tokio::task::yield_now().await;
            }
            Ok(false) => {
                empty_ticks = empty_ticks.saturating_add(1);
                let backoff_ms = (100 + (empty_ticks as u64 * 50)).min(500);
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        tracing::info!("Embedding worker received cancellation.");
                        break;
                    }
                    _ = tokio::time::sleep(Duration::from_millis(backoff_ms)) => {}
                }
            }
            Err(e) => {
                tracing::error!("Error in embedding worker tick: {e}");
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        break;
                    }
                    _ = tokio::time::sleep(DEFAULT_EMBEDDING_POLL_INTERVAL) => {}
                }
            }
        }
    }
}

/// Helper struct for claimed embedding task.
#[derive(Debug)]
struct ClaimedEmbedding {
    node_id: Uuid,
    retry_count: i32,
}

/// Claims and processes a batch of up to 10 `PENDING` records from `node_embeddings`.
///
/// Returns:
/// - `Ok(true)` if records were claimed and processed.
/// - `Ok(false)` if no records were due for processing.
/// - `Err(e)` on database failure.
pub async fn process_embedding_batch(
    pool: &Pool,
    embedder: &LocalEmbeddingGenerator,
) -> Result<bool, EmbeddingError> {
    let mut client = pool.get().await?;

    // Step 1: Batch-claim up to 10 PENDING records in a transaction with FOR UPDATE SKIP LOCKED
    let tx = client.transaction().await?;

    let claim_query = "SELECT node_id, content_hash, retry_count \
         FROM node_embeddings \
         WHERE status = 'PENDING' AND scheduled_at <= clock_timestamp() \
         ORDER BY scheduled_at \
         LIMIT $1 \
         FOR UPDATE SKIP LOCKED;";

    let rows = tx.query(claim_query, &[&EMBEDDING_BATCH_SIZE]).await?;

    if rows.is_empty() {
        tx.rollback().await?;
        return Ok(false);
    }

    let mut claimed: Vec<ClaimedEmbedding> = Vec::with_capacity(rows.len());
    let mut node_ids: Vec<Uuid> = Vec::with_capacity(rows.len());

    for row in &rows {
        let node_id: Uuid = row.get("node_id");
        let retry_count: i32 = row.get("retry_count");
        node_ids.push(node_id);
        claimed.push(ClaimedEmbedding {
            node_id,
            retry_count,
        });
    }

    // Transition claimed records to PROCESSING
    tx.execute(
        "UPDATE node_embeddings \
         SET status = 'PROCESSING', updated_at = clock_timestamp() \
         WHERE node_id = ANY($1);",
        &[&node_ids],
    )
    .await?;

    tx.commit().await?;

    // Step 2: Fetch corresponding text from graph_nodes
    let node_rows = client
        .query(
            "SELECT id, title, content, node_key \
             FROM graph_nodes \
             WHERE id = ANY($1);",
            &[&node_ids],
        )
        .await?;

    let mut node_text_map: HashMap<Uuid, String> = HashMap::new();
    for row in &node_rows {
        let id: Uuid = row.get("id");
        let title: Option<String> = row.get("title");
        let content: Option<String> = row.get("content");
        let node_key: Option<String> = row.get("node_key");

        let text = match (&title, &content) {
            (Some(t), Some(c)) if !t.is_empty() && !c.is_empty() => format!("{t}\n{c}"),
            (Some(t), _) if !t.is_empty() => t.clone(),
            (_, Some(c)) if !c.is_empty() => c.clone(),
            _ => node_key.unwrap_or_else(|| "Requirement".to_string()),
        };
        node_text_map.insert(id, text);
    }

    // Prepare batch texts for inference in the exact order of claimed nodes
    let texts: Vec<String> = claimed
        .iter()
        .map(|item| {
            node_text_map
                .get(&item.node_id)
                .cloned()
                .unwrap_or_else(|| "Requirement".to_string())
        })
        .collect();

    // Step 3: Execute local CPU vector inference via fastembed
    let inference_result = embedder.embed_batch(&texts, Some(EMBEDDING_BATCH_SIZE as usize));

    match inference_result {
        Ok(embeddings) => {
            // Step 4a: Update node_embeddings with COMPLETED vectors
            for (item, emb) in claimed.iter().zip(embeddings) {
                let vec_str = format!(
                    "[{}]",
                    emb.iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                );

                client
                    .execute(
                        "UPDATE node_embeddings \
                         SET embedding = ($1::text)::vector, status = 'COMPLETED', error_message = NULL, updated_at = clock_timestamp() \
                         WHERE node_id = $2;",
                        &[&vec_str, &item.node_id],
                    )
                    .await?;
            }
            tracing::info!(
                "Generated embeddings for {} nodes successfully",
                claimed.len()
            );
        }
        Err(e) => {
            // Step 4b: Handle failure with exponential retry backoff (D-48, §5.2)
            tracing::warn!("Embedding inference failed for batch: {e}; applying retry backoff");
            let err_msg = e.to_string();

            for item in &claimed {
                if item.retry_count >= MAX_EMBEDDING_RETRIES {
                    client
                        .execute(
                            "UPDATE node_embeddings \
                             SET status = 'FAILED', error_message = $2, updated_at = clock_timestamp() \
                             WHERE node_id = $1;",
                            &[&item.node_id, &err_msg],
                        )
                        .await?;
                } else {
                    client
                        .execute(
                            "UPDATE node_embeddings \
                             SET status = 'PENDING', \
                                 retry_count = retry_count + 1, \
                                 scheduled_at = clock_timestamp() + (INTERVAL '1 second' * POWER(2, retry_count)), \
                                 error_message = $2, \
                                 updated_at = clock_timestamp() \
                             WHERE node_id = $1;",
                            &[&item.node_id, &err_msg],
                        )
                        .await?;
                }
            }
        }
    }

    Ok(true)
}
