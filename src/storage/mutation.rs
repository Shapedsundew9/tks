//! Transactional graph mutation primitives, strict lock acquisition hierarchy,
//! DAG cycle detection, and audit logging (WP-2.1).
//!
//! Enforces:
//! - Strict lock hierarchy (C-19, D-66): global transaction advisory lock
//!   `pg_advisory_xact_lock(hashtext('tks_structural_mutation'))` must be acquired
//!   *before* any row-level locks on `graph_nodes` or `graph_edges` for structural mutations.
//! - In-database DAG cycle prevention (D-8, D-20): recursive CTE walks upward across
//!   `FULFILLS`, `CONSTRAINED_BY`, and `DERIVED_FROM` edges to verify acyclicity.
//! - Native PostgreSQL row-level locking (`SELECT ... FOR UPDATE`) for concurrent leaf attribute updates (D-30).
//! - Invariant INV-1 ancestor path validation CTE for active requirement roots.
//! - Monotonic `event_seq` assignment and atomic `batch_id UUID` correlation in `audit_ledger` (INV-2, D-45, D-65).

use serde::{Deserialize, Serialize};
use tokio_postgres::Client;
use uuid::Uuid;

use crate::storage::{GraphEdge, GraphNode, StorageError};

/// SQL advisory lock query for structural graph mutations.
pub const STRUCTURAL_ADVISORY_LOCK_SQL: &str =
    "SELECT pg_advisory_xact_lock(hashtext('tks_structural_mutation'));";

/// Comprehensive error taxonomy for transactional graph mutations.
#[derive(Debug)]
pub enum MutationError {
    /// Directed edge creation would create a directed cycle in structural relationships.
    CycleDetected { from_id: Uuid, to_id: Uuid },
    /// Entity with specified identifier was not found.
    NotFound(String),
    /// Invariant INV-1 violated: node does not have an unbroken directed path to an active requirement.
    InvalidAncestorPath(String),
    /// Per-node governance policy rejects proposed operation.
    GovernanceRejected(String),
    /// Lock acquisition failure or contention timeout.
    LockFailure(String),
    /// Underlying PostgreSQL database driver error.
    Database(tokio_postgres::Error),
    /// JSON serialization or deserialization error.
    Serialization(serde_json::Error),
    /// Connection pool acquisition error.
    Pool(deadpool_postgres::PoolError),
    /// General storage mutation error.
    Other(String),
}

impl MutationError {
    /// Returns the standardized error code string.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::CycleDetected { .. } => "ERR_GRAPH_CYCLE_DETECTED",
            Self::NotFound(_) => "ERR_NOT_FOUND",
            Self::InvalidAncestorPath(_) => "ERR_INVALID_ANCESTOR_PATH",
            Self::GovernanceRejected(_) => "ERR_GOVERNANCE_REJECTED",
            Self::LockFailure(_) => "ERR_LOCK_FAILURE",
            Self::Database(_) => "ERR_DATABASE",
            Self::Serialization(_) => "ERR_SERIALIZATION",
            Self::Pool(_) => "ERR_POOL",
            Self::Other(_) => "ERR_MUTATION_FAILED",
        }
    }
}

impl std::fmt::Display for MutationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CycleDetected { from_id, to_id } => {
                write!(
                    f,
                    "ERR_GRAPH_CYCLE_DETECTED: directed edge from {from_id} to {to_id} would create a directed cycle in structural relationships"
                )
            }
            Self::NotFound(msg) => write!(f, "ERR_NOT_FOUND: {msg}"),
            Self::InvalidAncestorPath(msg) => write!(f, "ERR_INVALID_ANCESTOR_PATH: {msg}"),
            Self::GovernanceRejected(msg) => write!(f, "ERR_GOVERNANCE_REJECTED: {msg}"),
            Self::LockFailure(msg) => write!(f, "ERR_LOCK_FAILURE: {msg}"),
            Self::Database(e) => write!(f, "Database error: {e}"),
            Self::Serialization(e) => write!(f, "Serialization error: {e}"),
            Self::Pool(e) => write!(f, "Connection pool error: {e}"),
            Self::Other(msg) => write!(f, "Mutation error: {msg}"),
        }
    }
}

impl std::error::Error for MutationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(e) => Some(e),
            Self::Serialization(e) => Some(e),
            Self::Pool(e) => Some(e),
            _ => None,
        }
    }
}

impl From<tokio_postgres::Error> for MutationError {
    fn from(e: tokio_postgres::Error) -> Self {
        Self::Database(e)
    }
}

impl From<serde_json::Error> for MutationError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization(e)
    }
}

impl From<deadpool_postgres::PoolError> for MutationError {
    fn from(e: deadpool_postgres::PoolError) -> Self {
        Self::Pool(e)
    }
}

impl From<StorageError> for MutationError {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::Database(err) => Self::Database(err),
            StorageError::Pool(err) => Self::Pool(err),
            StorageError::NotFound(msg) => Self::NotFound(msg),
            StorageError::LockAcquisitionFailed(msg) => Self::LockFailure(msg),
            StorageError::Serialization(err) => Self::Serialization(err),
            other => Self::Other(other.to_string()),
        }
    }
}

impl From<MutationError> for StorageError {
    fn from(e: MutationError) -> Self {
        match e {
            MutationError::Database(err) => Self::Database(err),
            MutationError::Pool(err) => Self::Pool(err),
            MutationError::Serialization(err) => Self::Serialization(err),
            MutationError::NotFound(msg) => Self::NotFound(msg),
            MutationError::LockFailure(msg) => Self::LockAcquisitionFailed(msg),
            MutationError::CycleDetected { from_id, to_id } => Self::InvalidState(format!(
                "ERR_GRAPH_CYCLE_DETECTED: cycle between {from_id} and {to_id}"
            )),
            other => Self::InvalidState(other.to_string()),
        }
    }
}

/// Payload for updating node attributes, title, or content.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeMutationPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<serde_json::Value>,
}

/// Payload for creating or updating a directed structural property graph edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeMutationPayload {
    pub from_id: Uuid,
    pub to_id: Uuid,
    pub edge_type: String,
    pub created_by: String,
    #[serde(default = "default_edge_lifecycle_state")]
    pub lifecycle_state: String,
}

fn default_edge_lifecycle_state() -> String {
    "ACTIVE".to_string()
}

/// Result returned from successful mutation operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationResult {
    pub entity_id: Uuid,
    pub batch_id: Uuid,
    pub event_seq: i64,
    pub status: String,
}

/// Encapsulates an active PostgreSQL transaction holding the global structural mutation advisory lock:
/// `SELECT pg_advisory_xact_lock(hashtext('tks_structural_mutation'));` (C-19, D-66).
///
/// In PostgreSQL, advisory transaction locks are automatically released when the transaction terminates
/// via commit or rollback.
pub struct StructuralMutationTx<'a> {
    tx: tokio_postgres::Transaction<'a>,
}

impl<'a> StructuralMutationTx<'a> {
    /// Creates a new `StructuralMutationTx` wrapping an active transaction that holds the advisory lock.
    #[must_use]
    pub fn new(tx: tokio_postgres::Transaction<'a>) -> Self {
        Self { tx }
    }

    /// Borrows the inner transaction as a reference.
    #[must_use]
    pub fn client(&self) -> &tokio_postgres::Transaction<'a> {
        &self.tx
    }

    /// Borrows the inner transaction as a mutable reference.
    pub fn client_mut(&mut self) -> &mut tokio_postgres::Transaction<'a> {
        &mut self.tx
    }

    /// Consumes the wrapper and returns the inner `tokio_postgres::Transaction`.
    #[must_use]
    pub fn into_inner(self) -> tokio_postgres::Transaction<'a> {
        self.tx
    }

    /// Commits the PostgreSQL transaction, releasing the advisory lock and all row locks.
    ///
    /// # Errors
    ///
    /// Returns `MutationError::Database` if transaction commit fails.
    pub async fn commit(self) -> Result<(), MutationError> {
        self.tx.commit().await.map_err(MutationError::Database)
    }

    /// Rolls back the PostgreSQL transaction, releasing the advisory lock and discarding changes.
    ///
    /// # Errors
    ///
    /// Returns `MutationError::Database` if transaction rollback fails.
    pub async fn rollback(self) -> Result<(), MutationError> {
        self.tx.rollback().await.map_err(MutationError::Database)
    }
}

impl<'a> std::ops::Deref for StructuralMutationTx<'a> {
    type Target = tokio_postgres::Transaction<'a>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl<'a> std::ops::DerefMut for StructuralMutationTx<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.tx
    }
}

/// Transaction lifecycle helper: begins a PostgreSQL transaction on `client` and immediately
/// acquires the global structural mutation advisory lock before yielding the transaction handle (C-19, D-66).
///
/// # Errors
///
/// Returns `MutationError::Database` if beginning the transaction or acquiring the advisory lock fails.
pub async fn begin_structural_mutation<'a>(
    client: &'a mut Client,
) -> Result<StructuralMutationTx<'a>, MutationError> {
    let tx = client.transaction().await?;
    tx.execute(STRUCTURAL_ADVISORY_LOCK_SQL, &[]).await?;
    Ok(StructuralMutationTx::new(tx))
}

/// Checks whether adding a directed edge from `from_id` to `to_id` would create a directed cycle
/// in the structural relationship graph (`FULFILLS`, `CONSTRAINED_BY`, `DERIVED_FROM`).
///
/// In TKS, all structural edges point upward from child to parent/governing requirement (D-12, D-50, D-64).
/// Adding an upward edge `from_id -> to_id` introduces a cycle if and only if there is already an upward path
/// from `to_id` reaching `from_id`, or if `from_id == to_id`.
///
/// Evaluates active and candidate (`DRAFT`) edges in `graph_edges`, ignoring superseded or reverted edges.
///
/// # Errors
///
/// Returns `StorageError::Database` if the recursive CTE query execution fails.
pub async fn check_dag_cycle(
    client: &(impl tokio_postgres::GenericClient + ?Sized),
    from_id: Uuid,
    to_id: Uuid,
) -> Result<bool, StorageError> {
    // 1. Self-loop is an immediate cycle
    if from_id == to_id {
        return Ok(true);
    }

    // 2. Recursive CTE starting at to_id walking upward across structural relationships
    let query = "
        WITH RECURSIVE upward_walk AS (
            SELECT
                e.to_node_id AS current_id,
                1 AS depth,
                ARRAY[$1, e.to_node_id] AS visited
            FROM graph_edges e
            WHERE e.from_node_id = $1
              AND e.edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM')
              AND e.lifecycle_state IN ('ACTIVE', 'DRAFT')
            UNION ALL
            SELECT
                e.to_node_id AS current_id,
                uw.depth + 1,
                uw.visited || e.to_node_id
            FROM upward_walk uw
            JOIN graph_edges e ON e.from_node_id = uw.current_id
            WHERE e.edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM')
              AND e.lifecycle_state IN ('ACTIVE', 'DRAFT')
              AND NOT (e.to_node_id = ANY(uw.visited))
              AND uw.depth < 100
        )
        SELECT EXISTS (
            SELECT 1 FROM upward_walk WHERE current_id = $2
        ) AS has_cycle;
    ";

    let row = client.query_one(query, &[&to_id, &from_id]).await?;
    let has_cycle: bool = row.get("has_cycle");
    Ok(has_cycle)
}

/// Helper asserting that adding edge `from_id -> to_id` does not create a cycle.
///
/// # Errors
///
/// Returns `MutationError::CycleDetected` if a directed cycle is detected, or `MutationError::Database` on query failure.
pub async fn assert_no_dag_cycle(
    client: &(impl tokio_postgres::GenericClient + ?Sized),
    from_id: Uuid,
    to_id: Uuid,
) -> Result<(), MutationError> {
    if check_dag_cycle(client, from_id, to_id).await? {
        Err(MutationError::CycleDetected { from_id, to_id })
    } else {
        Ok(())
    }
}

/// Inserts a directed structural edge between `from_id` and `to_id`, enforcing DAG cycle prevention.
///
/// If `edge_type` is one of `FULFILLS`, `CONSTRAINED_BY`, or `DERIVED_FROM`, executes `check_dag_cycle`
/// prior to insertion. If a cycle is detected, returns `Err(MutationError::CycleDetected)` and leaves
/// the graph unaltered.
///
/// # Errors
///
/// - Returns `MutationError::NotFound` if either `from_id` or `to_id` does not exist in `graph_nodes`.
/// - Returns `MutationError::CycleDetected` if the edge would introduce a directed cycle.
/// - Returns `MutationError::Database` on query or constraint violations.
pub async fn insert_structural_edge(
    client: &(impl tokio_postgres::GenericClient + ?Sized),
    from_id: Uuid,
    to_id: Uuid,
    edge_type: &str,
    created_by: &str,
    state: &str,
) -> Result<GraphEdge, MutationError> {
    let edge_type_norm = edge_type.trim().to_uppercase();
    let state_norm = state.trim().to_uppercase();

    // 1. Verify existence of both endpoints
    let from_exists = client
        .query_opt("SELECT id FROM graph_nodes WHERE id = $1;", &[&from_id])
        .await?;
    if from_exists.is_none() {
        return Err(MutationError::NotFound(format!(
            "Source node does not exist: {from_id}"
        )));
    }

    let to_exists = client
        .query_opt("SELECT id FROM graph_nodes WHERE id = $1;", &[&to_id])
        .await?;
    if to_exists.is_none() {
        return Err(MutationError::NotFound(format!(
            "Target node does not exist: {to_id}"
        )));
    }

    // 2. Cycle prevention check for structural edge types
    if is_structural_edge_type(&edge_type_norm) {
        assert_no_dag_cycle(client, from_id, to_id).await?;
    }

    // 3. Insert edge into graph_edges
    let row = client
        .query_one(
            "INSERT INTO graph_edges (from_node_id, to_node_id, edge_type, created_by, lifecycle_state) \
             VALUES ($1, $2, $3, $4, $5) \
             RETURNING edge_id, from_node_id, to_node_id, edge_type, created_by, lifecycle_state;",
            &[&from_id, &to_id, &edge_type_norm, &created_by, &state_norm],
        )
        .await?;

    Ok(GraphEdge::from_row(&row))
}

/// Returns true if `edge_type` participates in structural requirement lineage and DAG cycle checks.
#[must_use]
pub fn is_structural_edge_type(edge_type: &str) -> bool {
    matches!(edge_type, "FULFILLS" | "CONSTRAINED_BY" | "DERIVED_FROM")
}

/// Writes an event record to `audit_ledger` with generated or supplied `batch_id`, monotonic `event_seq`,
/// actor attributes, delta, and snapshot (D-45, D-65).
///
/// # Errors
///
/// Returns `StorageError::Database` if query fails.
#[allow(clippy::too_many_arguments)]
pub async fn insert_audit_event(
    client: &(impl tokio_postgres::GenericClient + ?Sized),
    batch_id: Uuid,
    event_type: &str,
    entity_id: Uuid,
    entity_type: &str,
    actor_id: &str,
    actor_type: &str,
    token_fingerprint: &str,
    delta: serde_json::Value,
    snapshot: serde_json::Value,
) -> Result<i64, StorageError> {
    insert_audit_event_full(
        client,
        batch_id,
        event_type,
        entity_id,
        entity_type,
        actor_id,
        actor_type,
        token_fingerprint,
        delta,
        snapshot,
        None,
    )
    .await
}

/// Full audit event insert including optional `draft_evolution_summary` JSONB.
///
/// # Errors
///
/// Returns `StorageError::Database` if query fails.
#[allow(clippy::too_many_arguments)]
pub async fn insert_audit_event_full(
    client: &(impl tokio_postgres::GenericClient + ?Sized),
    batch_id: Uuid,
    event_type: &str,
    entity_id: Uuid,
    entity_type: &str,
    actor_id: &str,
    actor_type: &str,
    token_fingerprint: &str,
    delta: serde_json::Value,
    snapshot: serde_json::Value,
    draft_evolution_summary: Option<serde_json::Value>,
) -> Result<i64, StorageError> {
    let row = client
        .query_one(
            "INSERT INTO audit_ledger ( \
                 batch_id, event_type, entity_id, entity_type, \
                 actor_id, actor_type, token_fingerprint, \
                 delta, snapshot, draft_evolution_summary \
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             RETURNING event_seq;",
            &[
                &batch_id,
                &event_type,
                &entity_id,
                &entity_type,
                &actor_id,
                &actor_type,
                &token_fingerprint,
                &delta,
                &snapshot,
                &draft_evolution_summary,
            ],
        )
        .await?;

    let event_seq: i64 = row.get("event_seq");
    Ok(event_seq)
}

/// Executes a leaf attribute update for `node_id` under native row-level lock (`SELECT ... FOR UPDATE`),
/// without acquiring the global structural advisory lock (D-30).
///
/// Takes a mutable reference to an open PostgreSQL connection `client`, starts a transaction,
/// locks the target row with `SELECT ... FOR UPDATE`, applies updates to attributes, title, or content,
/// records a discrete audit event in `audit_ledger`, and commits the transaction.
///
/// # Errors
///
/// - Returns `MutationError::NotFound` if `node_id` does not exist.
/// - Returns `MutationError::Database` on SQL errors.
pub async fn update_leaf_attributes_locked(
    client: &mut Client,
    node_id: Uuid,
    payload: &NodeMutationPayload,
    actor_id: &str,
    actor_type: &str,
    token_fingerprint: &str,
    batch_id: Option<Uuid>,
) -> Result<(GraphNode, i64), MutationError> {
    let tx = client.transaction().await?;

    // 1. Acquire row-level lock without holding global advisory lock (D-30)
    let row_opt = tx
        .query_opt(
            "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                    governance_policy, created_by, job_id, doc_path, doc_hash, \
                    byte_start, byte_end, attributes \
             FROM graph_nodes \
             WHERE id = $1 \
             FOR UPDATE;",
            &[&node_id],
        )
        .await?;

    let row = match row_opt {
        Some(r) => r,
        None => {
            return Err(MutationError::NotFound(format!(
                "Node not found for leaf update: {node_id}"
            )));
        }
    };

    let existing_node = GraphNode::from_row(&row);

    // 2. Merge progressive JSONB attributes (C-5)
    let mut merged_attributes = existing_node.attributes.clone();
    if let Some(patch) = &payload.attributes {
        if let (Some(base_obj), Some(patch_obj)) =
            (merged_attributes.as_object_mut(), patch.as_object())
        {
            for (k, v) in patch_obj {
                base_obj.insert(k.clone(), v.clone());
            }
        } else {
            merged_attributes = patch.clone();
        }
    }

    let updated_title = payload.title.as_deref().or(existing_node.title.as_deref());
    let updated_content = payload
        .content
        .as_deref()
        .or(existing_node.content.as_deref());

    // 3. Update row in place
    let updated_row = tx
        .query_one(
            "UPDATE graph_nodes \
             SET title = $2, content = $3, attributes = $4 \
             WHERE id = $1 \
             RETURNING id, node_key, node_type, title, content, lifecycle_state, \
                       governance_policy, created_by, job_id, doc_path, doc_hash, \
                       byte_start, byte_end, attributes;",
            &[
                &node_id,
                &updated_title,
                &updated_content,
                &merged_attributes,
            ],
        )
        .await?;

    let updated_node = GraphNode::from_row(&updated_row);

    // 4. Record audit ledger entry with monotonic event_seq and batch correlation
    let effective_batch_id = batch_id.unwrap_or_else(Uuid::new_v4);
    let delta = serde_json::json!({
        "title": payload.title,
        "content": payload.content,
        "attributes": payload.attributes,
    });
    let snapshot = serde_json::to_value(&updated_node)?;

    let event_seq = insert_audit_event(
        &tx,
        effective_batch_id,
        "LEAF_ATTRIBUTE_UPDATE",
        node_id,
        &updated_node.node_type,
        actor_id,
        actor_type,
        token_fingerprint,
        delta,
        snapshot,
    )
    .await?;

    // 5. Commit transaction
    tx.commit().await?;

    Ok((updated_node, event_seq))
}

/// Polymorphic node resolution helper: accepts either UUID string or canonical `node_key`.
///
/// - If `identifier` parses as a UUID, looks up `id = $1`.
/// - Otherwise, looks up `node_key = $1 AND lifecycle_state = 'ACTIVE'` (TB-7, D-58).
///
/// # Errors
///
/// Returns `MutationError::NotFound` if the node cannot be resolved.
pub async fn resolve_node_polymorphic(
    client: &(impl tokio_postgres::GenericClient + ?Sized),
    identifier: &str,
) -> Result<GraphNode, MutationError> {
    let identifier = identifier.trim();
    if identifier.is_empty() {
        return Err(MutationError::NotFound(
            "Empty identifier supplied".to_string(),
        ));
    }

    let node_opt = if let Ok(uuid) = Uuid::parse_str(identifier) {
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
        row_opt.map(|r| GraphNode::from_row(&r))
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
        row_opt.map(|r| GraphNode::from_row(&r))
    };

    match node_opt {
        Some(node) => Ok(node),
        None => Err(MutationError::NotFound(format!(
            "Node not found by polymorphic identifier: {identifier}"
        ))),
    }
}

/// Validates Invariant INV-1 ancestor paths for candidate or active nodes, ensuring that every
/// non-requirement node reaches an authorized active requirement root across upward structural edges.
///
/// # Errors
///
/// Returns `MutationError::Database` if query fails.
pub async fn validate_ancestor_path(
    client: &(impl tokio_postgres::GenericClient + ?Sized),
    node_ids: &[Uuid],
    allow_draft_parents: bool,
) -> Result<bool, MutationError> {
    crate::storage::envelope::validate_ancestor_path(client, node_ids, allow_draft_parents)
        .await
        .map_err(MutationError::from)
}

/// Asserts Invariant INV-1 ancestor paths, returning `Err(MutationError::InvalidAncestorPath)` if invalid.
///
/// # Errors
///
/// - Returns `MutationError::InvalidAncestorPath` if any node does not trace to an active requirement.
/// - Returns `MutationError::Database` on query execution error.
pub async fn assert_ancestor_path(
    client: &(impl tokio_postgres::GenericClient + ?Sized),
    node_ids: &[Uuid],
    allow_draft_parents: bool,
) -> Result<(), MutationError> {
    let is_valid = validate_ancestor_path(client, node_ids, allow_draft_parents).await?;
    if !is_valid {
        Err(MutationError::InvalidAncestorPath(format!(
            "One or more nodes in {node_ids:?} lack an unbroken directed path to an active requirement root (INV-1)"
        )))
    } else {
        Ok(())
    }
}
