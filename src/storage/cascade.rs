//! Automated invalidation cascade engine and recursive downward dependency sweep (WP-3.1, PHASE3-002).
//!
//! Implements:
//! - Recursive downward CTE queries traversing downstream dependency relationships
//!   (`FULFILLS`, `CONSTRAINED_BY`, `DERIVED_FROM`) starting from any modified, superseded,
//!   or reverted root specification node (INV-1, TB-6).
//! - Progressive `staleness_score = 1.0 / (depth as f64)` propagation stamped in node attributes
//!   alongside `invalidated_by = root_node_id` and `staleness_reason` (D-73, D-76).
//! - Advisory lock serialization under `pg_advisory_xact_lock(hashtext('tks_structural_mutation'))` (C-19, D-66).
//! - Batch mutation transitioning active descendant nodes to `NEEDS_REVERIFICATION`.
//! - Monotonic append-only audit records with `CASCADE_INVALIDATED` and correlated `batch_id` (INV-2, D-4, D-45).
//! - Discrete change event emission onto PostgreSQL notification channel `tks_graph_events` (TB-1).

use deadpool_postgres::Client as DeadpoolClient;
use serde::{Deserialize, Serialize};
use tokio_postgres::Client;
use uuid::Uuid;

use crate::gateway::auth::AuthenticatedAgent;
use crate::storage::event_bus::{GraphChangeEvent, GraphEventBus};
use crate::storage::mutation::MutationError;

/// SQL query executing root existence check, advisory lock acquisition, recursive downward
/// invalidation, node attribute updates, audit ledger logging, and PostgreSQL notification
/// emission in a single unified round-trip.
const DOWNWARD_INVALIDATION_SQL: &str = "
WITH RECURSIVE root_check AS (
    SELECT id, pg_advisory_xact_lock(hashtext('tks_structural_mutation')) AS lock
    FROM graph_nodes
    WHERE id = $1
),
dep_tree AS (
    -- Base case: active downstream edges where to_node_id = root_node_id
    SELECT
        e.from_node_id AS child_id,
        1 AS depth,
        ARRAY[e.to_node_id, e.from_node_id] AS visited
    FROM graph_edges e
    JOIN root_check rc ON rc.id = e.to_node_id
    WHERE e.lifecycle_state = 'ACTIVE'
      AND e.edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM')

    UNION ALL

    -- Recursive case: downstream active edges from child_id
    SELECT
        e.from_node_id AS child_id,
        dt.depth + 1 AS depth,
        dt.visited || e.from_node_id AS visited
    FROM graph_edges e
    JOIN dep_tree dt ON dt.child_id = e.to_node_id
    WHERE e.lifecycle_state = 'ACTIVE'
      AND e.edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM')
      AND NOT (e.from_node_id = ANY(dt.visited))
      AND dt.depth < 100
),
distinct_descendants AS (
    SELECT
        dt.child_id,
        MIN(dt.depth) AS depth,
        (1.0 / MIN(dt.depth)::float8) AS staleness_score
    FROM dep_tree dt
    JOIN graph_nodes n ON n.id = dt.child_id
    WHERE n.lifecycle_state = 'ACTIVE'
      AND dt.child_id != $1
    GROUP BY dt.child_id
),
updated_nodes AS (
    UPDATE graph_nodes gn
    SET lifecycle_state = 'NEEDS_REVERIFICATION',
        attributes = gn.attributes || jsonb_build_object(
            'staleness_score', dd.staleness_score,
            'invalidated_by', $1::text,
            'staleness_reason', $2::text
        )
    FROM distinct_descendants dd
    WHERE gn.id = dd.child_id
    RETURNING gn.id, gn.node_type, gn.node_key, gn.attributes, dd.depth, dd.staleness_score
),
inserted_audit AS (
    INSERT INTO audit_ledger (
        batch_id,
        event_type,
        entity_id,
        entity_type,
        actor_id,
        actor_type,
        token_fingerprint,
        delta,
        snapshot,
        created_at
    )
    SELECT
        $3::uuid AS batch_id,
        'CASCADE_INVALIDATED' AS event_type,
        un.id AS entity_id,
        un.node_type AS entity_type,
        $4::text AS actor_id,
        $5::text AS actor_type,
        $6::text AS token_fingerprint,
        jsonb_build_object(
            'lifecycle_state', 'NEEDS_REVERIFICATION',
            'previous_lifecycle_state', 'ACTIVE',
            'staleness_score', un.staleness_score,
            'invalidated_by', $1::text,
            'staleness_reason', $2::text,
            'depth', un.depth
        ) AS delta,
        jsonb_build_object(
            'id', un.id,
            'node_type', un.node_type,
            'node_key', un.node_key,
            'attributes', un.attributes
        ) AS snapshot,
        clock_timestamp() AS created_at
    FROM updated_nodes un
    RETURNING event_seq, batch_id, event_type, entity_id, entity_type, actor_id, created_at
)
SELECT
    EXISTS(SELECT 1 FROM root_check) AS root_found,
    coalesce(
        (
            SELECT array_agg(ia.entity_id ORDER BY ia.event_seq ASC)
            FROM inserted_audit ia
            WHERE pg_notify(
                'tks_graph_events',
                json_build_object(
                    'event_seq', ia.event_seq,
                    'batch_id', ia.batch_id,
                    'event_type', ia.event_type,
                    'entity_id', ia.entity_id,
                    'entity_type', ia.entity_type,
                    'actor_id', ia.actor_id,
                    'timestamp', ia.created_at
                )::text
            ) IS NOT NULL
        ),
        '{}'::uuid[]
    ) AS invalidated_nodes,
    coalesce((SELECT max(un.depth) FROM updated_nodes un), 0)::int4 AS max_depth,
    coalesce((SELECT max(ia.event_seq) FROM inserted_audit ia), 0)::int8 AS event_seq;
";

/// Result of an automated downward invalidation sweep.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CascadeInvalidationResult {
    /// Identifier of the root node whose mutation triggered the cascade.
    pub root_node_id: Uuid,
    /// List of descendant node UUIDs transitioned to `NEEDS_REVERIFICATION`.
    pub invalidated_nodes: Vec<Uuid>,
    /// Maximum topological distance traversed in the invalidation sweep.
    pub max_depth: u32,
    /// Shared batch UUID correlating all compensating `CASCADE_INVALIDATED` audit events.
    pub batch_id: Uuid,
    /// Monotonic sequence number of the final audit event emitted in the cascade.
    pub event_seq: i64,
}

/// Calculates progressive staleness score based on topological distance `depth`.
///
/// Formula: `staleness_score = 1.0 / (depth as f64)`.
#[must_use]
pub fn calculate_staleness_score(depth: u32) -> f64 {
    1.0 / (depth.max(1) as f64)
}

/// Triggers recursive downward invalidation using a pooled connection (`&mut deadpool_postgres::Client`).
///
/// Standard contract function for WP-3.1.
///
/// # Errors
///
/// Returns `MutationError::NotFound` if `root_node_id` does not exist in `graph_nodes`.
/// Returns `MutationError::Database` on query execution error or lock contention.
pub async fn trigger_downward_invalidation(
    client: &mut DeadpoolClient,
    root_node_id: Uuid,
    reason: &str,
    caller: &AuthenticatedAgent,
) -> Result<CascadeInvalidationResult, MutationError> {
    trigger_downward_invalidation_client(client.as_mut(), root_node_id, reason, caller).await
}

/// Triggers recursive downward invalidation using a raw client (`&mut tokio_postgres::Client`).
///
/// # Errors
///
/// Returns `MutationError::NotFound` if `root_node_id` does not exist in `graph_nodes`.
/// Returns `MutationError::Database` on query execution error or lock contention.
pub async fn trigger_downward_invalidation_client(
    client: &mut Client,
    root_node_id: Uuid,
    reason: &str,
    caller: &AuthenticatedAgent,
) -> Result<CascadeInvalidationResult, MutationError> {
    trigger_downward_invalidation_with_bus(client, root_node_id, reason, caller, None).await
}

/// Triggers recursive downward invalidation with optional in-memory broadcast bus notification.
///
/// # Errors
///
/// Returns `MutationError::NotFound` if `root_node_id` does not exist in `graph_nodes`.
/// Returns `MutationError::Database` on query execution error or lock contention.
pub async fn trigger_downward_invalidation_with_bus(
    client: &mut Client,
    root_node_id: Uuid,
    reason: &str,
    caller: &AuthenticatedAgent,
    bus: Option<&GraphEventBus>,
) -> Result<CascadeInvalidationResult, MutationError> {
    let batch_id = Uuid::new_v4();

    // Begin transaction
    let tx = client
        .transaction()
        .await
        .map_err(MutationError::Database)?;

    // Configure local transaction for high-throughput in-memory commit response (SLA-2)
    let _ = tx.execute("SET LOCAL synchronous_commit = off;", &[]).await;

    // Execute atomic CTE invalidation and notification query
    let row = tx
        .query_one(
            DOWNWARD_INVALIDATION_SQL,
            &[
                &root_node_id,
                &reason,
                &batch_id,
                &caller.agent_id,
                &caller.actor_type,
                &caller.token_fingerprint(),
            ],
        )
        .await
        .map_err(MutationError::Database)?;

    let root_found: bool = row.get(0);
    if !root_found {
        let _ = tx.rollback().await;
        return Err(MutationError::NotFound(format!(
            "Root node '{root_node_id}' not found"
        )));
    }

    let invalidated_nodes: Vec<Uuid> = row.get(1);
    let max_depth: i32 = row.get(2);
    let event_seq: i64 = row.get(3);

    // Commit atomic batch mutation
    tx.commit().await.map_err(MutationError::Database)?;

    // Broadcast to in-memory bus if supplied
    if let Some(event_bus) = bus {
        let now = chrono::Utc::now();
        for &node_id in &invalidated_nodes {
            event_bus.publish(GraphChangeEvent {
                event_seq,
                batch_id,
                event_type: "CASCADE_INVALIDATED".to_string(),
                entity_id: node_id,
                entity_type: "TASK".to_string(),
                actor_id: caller.agent_id.clone(),
                timestamp: now,
            });
        }
    }

    Ok(CascadeInvalidationResult {
        root_node_id,
        invalidated_nodes,
        max_depth: max_depth as u32,
        batch_id,
        event_seq,
    })
}
