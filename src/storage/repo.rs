//! Core database repository implementing connection acquisition, advisory locks,
//! and polymorphic lookup (WP-1.2).

use deadpool_postgres::Pool;
use tokio_postgres::Client;
use uuid::Uuid;

use crate::db;
use crate::gateway::auth::AuthenticatedAgent;
use crate::storage::governance::TaskStatus;
use crate::storage::mutation::{
    DraftMutationResult, DraftProposalResult, ElaboratedTaskResult, MutationError, TaskUpdateResult,
};
use crate::storage::reverify::ReverifyResult;
use crate::storage::rollback::{RevertExecutionResult, RevertFilter};
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

    /// Begins a structural mutation transaction on `client` holding the global advisory lock (C-19, D-66).
    ///
    /// # Errors
    ///
    /// Returns `MutationError` if starting the transaction or acquiring the advisory lock fails.
    pub async fn begin_structural_mutation<'a>(
        &self,
        client: &'a mut Client,
    ) -> Result<
        crate::storage::mutation::StructuralMutationTx<'a>,
        crate::storage::mutation::MutationError,
    > {
        crate::storage::mutation::begin_structural_mutation(client).await
    }

    /// Executes an operation inside a structural mutation transaction with the global advisory lock held.
    ///
    /// The transaction is automatically committed upon success or rolled back on error.
    ///
    /// # Errors
    ///
    /// Returns `MutationError` if connection acquisition, transaction start, lock acquisition,
    /// closure execution, or commit fails.
    pub async fn execute_structural_mutation<T, F>(
        &self,
        op: F,
    ) -> Result<T, crate::storage::mutation::MutationError>
    where
        F: for<'tx> FnOnce(
            &'tx mut crate::storage::mutation::StructuralMutationTx<'_>,
        ) -> futures_util::future::BoxFuture<
            'tx,
            Result<T, crate::storage::mutation::MutationError>,
        >,
    {
        let mut client = self.get_client().await?;
        let mut tx = crate::storage::mutation::begin_structural_mutation(&mut client).await?;
        let res = op(&mut tx).await;
        match res {
            Ok(val) => {
                tx.commit().await?;
                Ok(val)
            }
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    /// Updates leaf attributes of `node_id` under a native row-level lock (`SELECT ... FOR UPDATE`),
    /// without holding the global advisory lock (D-30).
    ///
    /// # Errors
    ///
    /// Returns `MutationError` if node is not found or database update fails.
    pub async fn update_leaf_attributes_locked(
        &self,
        node_id: Uuid,
        payload: &crate::storage::mutation::NodeMutationPayload,
        actor_id: &str,
        actor_type: &str,
        token_fingerprint: &str,
        batch_id: Option<Uuid>,
    ) -> Result<(GraphNode, i64), crate::storage::mutation::MutationError> {
        let mut client = self.get_client().await?;
        crate::storage::mutation::update_leaf_attributes_locked(
            &mut client,
            node_id,
            payload,
            actor_id,
            actor_type,
            token_fingerprint,
            batch_id,
        )
        .await
    }

    /// Validates Invariant INV-1 ancestor paths for candidate node IDs.
    ///
    /// # Errors
    ///
    /// Returns `StorageError` if validation CTE fails.
    pub async fn validate_ancestor_path(
        &self,
        client: &(impl tokio_postgres::GenericClient + ?Sized),
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

    /// Assembles the complete context envelope including topological ancestors, constraints,
    /// and vector neighbors (TB-6, DEC-0.8, §6.1).
    ///
    /// # Errors
    ///
    /// Returns `StorageError` if node is not found, not visible, or query fails.
    pub async fn assemble_context_envelope(
        &self,
        target_id: Uuid,
        depth: u32,
        include_drafts_for: Option<&str>,
    ) -> Result<TopologicalEnvelope, StorageError> {
        let client = self.get_client().await?;
        crate::storage::envelope::assemble_context_envelope(
            &client,
            target_id,
            depth,
            include_drafts_for,
        )
        .await
    }

    /// Pathway 1: Autonomously elaborates an execution task under an `AUTONOMOUS_ELABORATION` parent node.
    ///
    /// # Errors
    ///
    /// Returns `MutationError` if parent is not found, locked, lacks active requirement ancestors, or SQL fails.
    pub async fn elaborate_task(
        &self,
        parent_id: &str,
        title: &str,
        content: Option<&str>,
        attributes: Option<serde_json::Value>,
        actor: &AuthenticatedAgent,
    ) -> Result<ElaboratedTaskResult, MutationError> {
        let mut client = self.get_client().await?;
        crate::storage::mutation::elaborate_task(
            &mut client,
            parent_id,
            title,
            content,
            attributes,
            actor,
        )
        .await
    }

    /// Pathway 2: Updates status of an active execution task under native row-level lock.
    ///
    /// # Errors
    ///
    /// Returns `MutationError` if task is not found, locked, not active, or SQL fails.
    pub async fn update_task_status(
        &self,
        task_id: &str,
        status: TaskStatus,
        notes: Option<&str>,
        actor: &AuthenticatedAgent,
    ) -> Result<TaskUpdateResult, MutationError> {
        let mut client = self.get_client().await?;
        crate::storage::mutation::update_task_status(&mut client, task_id, status, notes, actor)
            .await
    }

    /// Pathway 3: Proposes a candidate normative draft replacing an active requirement/specification.
    ///
    /// # Errors
    ///
    /// Returns `MutationError` if target is not found, locked, or SQL fails.
    pub async fn propose_normative_draft(
        &self,
        target_id: &str,
        title: Option<&str>,
        content: &str,
        attributes: Option<serde_json::Value>,
        actor: &AuthenticatedAgent,
    ) -> Result<DraftProposalResult, MutationError> {
        let mut client = self.get_client().await?;
        crate::storage::mutation::propose_normative_draft(
            &mut client,
            target_id,
            title,
            content,
            attributes,
            actor,
        )
        .await
    }

    /// Pathway 4: Mutates an unapproved candidate draft entity in-place, appending to draft revisions.
    ///
    /// # Errors
    ///
    /// Returns `MutationError` if draft is not found, not in DRAFT state, violates caller isolation, or SQL fails.
    pub async fn mutate_draft_entity(
        &self,
        draft_id: &str,
        patch: serde_json::Value,
        actor: &AuthenticatedAgent,
    ) -> Result<DraftMutationResult, MutationError> {
        let mut client = self.get_client().await?;
        crate::storage::mutation::mutate_draft_entity(&mut client, draft_id, patch, actor).await
    }

    /// Unified administrative rollback utility executing compensating transactions (WP-2.3).
    ///
    /// # Errors
    ///
    /// Returns `MutationError::ConfirmationRequired` if cross-agent dependencies exist and `force != true`.
    /// Returns `MutationError` on query failure, serialization error, or lock contention.
    pub async fn revert_mutations(
        &self,
        filter: RevertFilter,
        actor: &AuthenticatedAgent,
    ) -> Result<RevertExecutionResult, MutationError> {
        let mut client = self.get_client().await?;
        crate::storage::rollback::revert_mutations(&mut client, filter, actor).await
    }

    /// Explicit reverification interface for stale and degraded graph nodes (WP-2.3, D-76).
    ///
    /// # Errors
    ///
    /// Returns `MutationError::NotFound` if `node_id` cannot be resolved.
    /// Returns `MutationError::DependencyInactive` if any direct upstream parent is not ACTIVE.
    /// Returns `MutationError::InvalidAncestorPath` if the node lacks an unbroken path to an active requirement root.
    /// Returns `MutationError` on query failure, serialization error, or lock contention.
    pub async fn reverify_node(
        &self,
        node_id: &str,
        rationale: &str,
        updated_attributes: Option<serde_json::Value>,
        actor: &AuthenticatedAgent,
    ) -> Result<ReverifyResult, MutationError> {
        let mut client = self.get_client().await?;
        crate::storage::reverify::reverify_node(
            &mut client,
            node_id,
            rationale,
            updated_attributes,
            actor,
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
