//! Core database repository implementing connection acquisition, advisory locks,
//! and polymorphic lookup (WP-1.2).

use deadpool_postgres::Pool;
use tokio_postgres::Client;
use uuid::Uuid;

use crate::db;
use crate::storage::{GraphNode, SearchResultNode, StorageError, TopologicalEnvelope};

/// Core storage repository wrapping a PostgreSQL connection pool (`deadpool-postgres`).
#[derive(Clone)]
pub struct StorageRepo {
    pool: Pool,
}

impl StorageRepo {
    /// Creates a new `StorageRepo` wrapping an existing connection pool.
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Alias for `StorageRepo::new(pool)`.
    #[must_use]
    pub fn from_pool(pool: Pool) -> Self {
        Self::new(pool)
    }

    /// Creates a new `StorageRepo` by initializing a connection pool from `database_url`.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::BuildPool` if pool initialization fails.
    pub fn from_database_url(database_url: &str) -> Result<Self, StorageError> {
        let pool =
            db::create_pool(database_url).map_err(|e| StorageError::BuildPool(e.to_string()))?;
        Ok(Self::new(pool))
    }

    /// Creates a new `StorageRepo` resolving the database URL from environment or container defaults.
    ///
    /// # Errors
    ///
    /// Returns `StorageError` if pool creation fails.
    pub fn from_env() -> Result<Self, StorageError> {
        let database_url = db::resolve_database_url();
        Self::from_database_url(&database_url)
    }

    /// Returns a reference to the underlying connection pool.
    #[must_use]
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Acquires a connection client from the pool.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Pool` if connection checkout fails.
    pub async fn get_client(&self) -> Result<deadpool_postgres::Client, StorageError> {
        self.pool.get().await.map_err(StorageError::from)
    }

    /// Acquires the global structural mutation transaction advisory lock on `client`.
    ///
    /// # Errors
    ///
    /// Returns `StorageError` if lock acquisition query fails.
    pub async fn acquire_structural_lock(&self, client: &mut Client) -> Result<(), StorageError> {
        acquire_structural_lock(client).await
    }

    /// Validates Invariant INV-1 ancestor paths for candidate node IDs.
    ///
    /// # Errors
    ///
    /// Returns `StorageError` if validation CTE fails.
    pub async fn validate_ancestor_path(
        &self,
        client: &mut Client,
        node_ids: &[Uuid],
        allow_draft_parents: bool,
    ) -> Result<bool, StorageError> {
        crate::storage::envelope::validate_ancestor_path(client, node_ids, allow_draft_parents)
            .await
    }

    /// Polymorphically resolves a node from identifier (UUID or canonical `node_key`).
    ///
    /// # Errors
    ///
    /// Returns `StorageError` on database errors.
    pub async fn lookup_node_polymorphic(
        &self,
        identifier: &str,
    ) -> Result<Option<GraphNode>, StorageError> {
        let client = self.get_client().await?;
        lookup_node_polymorphic(&client, identifier).await
    }

    /// Queries active requirements using sanitized full-text search against `search_tsv`.
    ///
    /// # Errors
    ///
    /// Returns `StorageError` on database errors.
    pub async fn query_active_requirements(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SearchResultNode>, StorageError> {
        let client = self.get_client().await?;
        crate::storage::search::query_active_requirements(&client, query, limit).await
    }

    /// Assembles the topological context envelope for `target_id`.
    ///
    /// # Errors
    ///
    /// Returns `StorageError` if node is not found, not visible, or query fails.
    pub async fn assemble_topological_envelope(
        &self,
        target_id: Uuid,
        depth: u32,
        include_drafts_for: Option<&str>,
    ) -> Result<TopologicalEnvelope, StorageError> {
        let client = self.get_client().await?;
        crate::storage::envelope::assemble_topological_envelope(
            &client,
            target_id,
            depth,
            include_drafts_for,
        )
        .await
    }
}

/// Acquires the global transaction advisory lock:
/// `SELECT pg_advisory_xact_lock(hashtext('tks_structural_mutation'));`
///
/// In PostgreSQL, `pg_advisory_xact_lock` is automatically released when the transaction ends.
///
/// # Errors
///
/// Returns `StorageError::Database` if query execution fails.
pub async fn acquire_structural_lock(client: &mut Client) -> Result<(), StorageError> {
    client
        .execute(
            "SELECT pg_advisory_xact_lock(hashtext('tks_structural_mutation'));",
            &[],
        )
        .await?;
    Ok(())
}

/// Polymorphic node resolution:
/// - If `identifier` parses as a UUID, queries by `id = $1`.
/// - Otherwise, queries by `node_key = $1 AND lifecycle_state = 'ACTIVE'` (TB-7).
///
/// # Errors
///
/// Returns `StorageError::Database` if query fails.
pub async fn lookup_node_polymorphic(
    client: &Client,
    identifier: &str,
) -> Result<Option<GraphNode>, StorageError> {
    let identifier = identifier.trim();
    if identifier.is_empty() {
        return Ok(None);
    }

    if let Ok(uuid) = Uuid::parse_str(identifier) {
        let row_opt = client
            .query_opt(
                "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                 governance_policy, created_by, job_id, doc_path, doc_hash, \
                 byte_start, byte_end, attributes \
                 FROM graph_nodes \
                 WHERE id = $1;",
                &[&uuid],
            )
            .await?;
        Ok(row_opt.map(|r| GraphNode::from_row(&r)))
    } else {
        let row_opt = client
            .query_opt(
                "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                 governance_policy, created_by, job_id, doc_path, doc_hash, \
                 byte_start, byte_end, attributes \
                 FROM graph_nodes \
                 WHERE node_key = $1 AND lifecycle_state = 'ACTIVE';",
                &[&identifier],
            )
            .await?;
        Ok(row_opt.map(|r| GraphNode::from_row(&r)))
    }
}
