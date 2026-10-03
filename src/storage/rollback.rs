//! Unified administrative rollback utility (`revert_mutations`) and blast-radius containment (WP-2.3).
//!
//! Implements non-destructive rollback via compensating inverse `REVERT` events with monotonic `event_seq`,
//! automated downward dependency invalidation cascades transitioning child tasks to `NEEDS_REVERIFICATION`,
//! dry-run previews under `READ COMMITTED`, and cross-agent confirmation safety aborts (`force = true`)
//! under `pg_advisory_xact_lock(hashtext('tks_structural_mutation'))` (INV-1, INV-2, D-45, D-65, D-73, D-80).

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use deadpool_postgres::Client as DeadpoolClient;
use serde::{Deserialize, Serialize};
use tokio_postgres::Client;
use uuid::Uuid;

use crate::gateway::auth::AuthenticatedAgent;
use crate::storage::mutation::{MutationError, insert_audit_event};

/// Filter criteria for querying and targeting mutations for administrative rollback.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RevertFilter {
    /// Optional batch UUID to revert.
    pub batch_id: Option<Uuid>,
    /// Optional agent ID whose authored mutations should be reverted.
    pub agent_id: Option<String>,
    /// Optional timestamp threshold reverting mutations committed since this time.
    pub since: Option<DateTime<Utc>>,
    /// Optional monotonic sequence range `[min_event_seq, max_event_seq]` to revert.
    pub event_seq_range: Option<(i64, i64)>,
    /// If true, performs dependency sweep under READ COMMITTED and returns preview without modifying state.
    pub dry_run: Option<bool>,
    /// If true, overrides cross-agent confirmation safety aborts.
    pub force: Option<bool>,
}

/// Blast-radius preview returned during dry-run or when cross-agent confirmation is required.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevertPreview {
    /// Node UUIDs directly targeted or cascaded by the proposed rollback.
    pub affected_nodes: Vec<Uuid>,
    /// Active structural edge UUIDs that would be marked REVERTED.
    pub affected_edges: Vec<Uuid>,
    /// External agent IDs who authored dependent child entities in the invalidation subtree.
    pub cross_agent_dependencies: Vec<String>,
    /// Total count of matching audit events targeted for reversal.
    pub total_events: usize,
}

/// Result of an executed compensating rollback transaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevertResult {
    /// Newly allocated batch UUID correlating all compensating REVERT audit events.
    pub batch_id: Uuid,
    /// Total count of original audit events reverted.
    pub reverted_events: usize,
    /// Node UUIDs transitioned to SUPERSEDED or state-restored.
    pub affected_nodes: Vec<Uuid>,
    /// Child task UUIDs cascaded to NEEDS_REVERIFICATION.
    pub cascade_reverified_nodes: Vec<Uuid>,
}

/// Polymorphic execution result of `revert_mutations` (dry-run preview or committed revert).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", content = "details")]
pub enum RevertExecutionResult {
    /// Dry-run execution preview with zero committed mutations.
    DryRun(RevertPreview),
    /// Compensating transaction committed successfully.
    Reverted(RevertResult),
}

impl RevertExecutionResult {
    /// Returns true if this result represents a dry-run preview.
    #[must_use]
    pub fn is_dry_run(&self) -> bool {
        matches!(self, Self::DryRun(_))
    }

    /// Returns a reference to the inner `RevertPreview` if this is a dry-run result.
    #[must_use]
    pub fn preview(&self) -> Option<&RevertPreview> {
        match self {
            Self::DryRun(p) => Some(p),
            Self::Reverted(_) => None,
        }
    }

    /// Returns a reference to the inner `RevertResult` if this is a committed result.
    #[must_use]
    pub fn reverted(&self) -> Option<&RevertResult> {
        match self {
            Self::Reverted(r) => Some(r),
            Self::DryRun(_) => None,
        }
    }
}

/// Internal struct capturing matched audit ledger event records.
#[derive(Debug, Clone)]
struct MatchedAuditEvent {
    event_seq: i64,
    batch_id: Uuid,
    event_type: String,
    entity_id: Uuid,
    entity_type: String,
    actor_id: String,
    delta: serde_json::Value,
    snapshot: serde_json::Value,
}

/// Executes unified administrative rollback on a pooled client (`&mut deadpool_postgres::Client`).
///
/// # Errors
///
/// Returns `MutationError::ConfirmationRequired` if cross-agent dependencies exist and `force != true`.
/// Returns `MutationError` on query failure, serialization error, or lock contention.
pub async fn revert_mutations(
    client: &mut DeadpoolClient,
    filter: RevertFilter,
    actor: &AuthenticatedAgent,
) -> Result<RevertExecutionResult, MutationError> {
    revert_mutations_client(client, filter, actor).await
}

/// Executes unified administrative rollback on a raw client (`&mut tokio_postgres::Client`).
///
/// # Errors
///
/// Returns `MutationError::ConfirmationRequired` if cross-agent dependencies exist and `force != true`.
/// Returns `MutationError` on query failure, serialization error, or lock contention.
pub async fn revert_mutations_client(
    client: &mut Client,
    filter: RevertFilter,
    actor: &AuthenticatedAgent,
) -> Result<RevertExecutionResult, MutationError> {
    // 1. Validate filter criteria
    if filter.batch_id.is_none()
        && filter.agent_id.is_none()
        && filter.since.is_none()
        && filter.event_seq_range.is_none()
    {
        return Err(MutationError::Other(
            "At least one revert filter criterion (batch_id, agent_id, since, or event_seq_range) must be specified".to_string(),
        ));
    }

    let is_dry_run = filter.dry_run.unwrap_or(false);
    let force = filter.force.unwrap_or(false);

    // 2. Query target audit ledger events matching the filter
    let mut conditions = vec!["event_type != 'REVERT'".to_string()];
    let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = Vec::new();

    if let Some(batch_id) = filter.batch_id {
        params.push(Box::new(batch_id));
        conditions.push(format!("batch_id = ${}", params.len()));
    }
    if let Some(agent_id) = &filter.agent_id {
        params.push(Box::new(agent_id.clone()));
        conditions.push(format!("actor_id = ${}", params.len()));
    }
    if let Some(since) = filter.since {
        params.push(Box::new(since));
        conditions.push(format!("created_at >= ${}", params.len()));
    }
    if let Some((min_seq, max_seq)) = filter.event_seq_range {
        params.push(Box::new(min_seq));
        conditions.push(format!("event_seq >= ${}", params.len()));
        params.push(Box::new(max_seq));
        conditions.push(format!("event_seq <= ${}", params.len()));
    }

    let query_sql = format!(
        "SELECT event_seq, batch_id, event_type, entity_id, entity_type, \
                actor_id, actor_type, token_fingerprint, delta, snapshot \
         FROM audit_ledger \
         WHERE {} \
         ORDER BY event_seq DESC;",
        conditions.join(" AND ")
    );

    let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
        .iter()
        .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();

    let rows = client.query(&query_sql, &param_refs).await?;

    if rows.is_empty() {
        if is_dry_run {
            return Ok(RevertExecutionResult::DryRun(RevertPreview {
                affected_nodes: vec![],
                affected_edges: vec![],
                cross_agent_dependencies: vec![],
                total_events: 0,
            }));
        }
        return Ok(RevertExecutionResult::Reverted(RevertResult {
            batch_id: Uuid::new_v4(),
            reverted_events: 0,
            affected_nodes: vec![],
            cascade_reverified_nodes: vec![],
        }));
    }

    let mut matched_events = Vec::with_capacity(rows.len());
    let mut targeted_actors = HashSet::new();
    let mut targeted_node_ids = HashSet::new();
    let mut targeted_edge_ids = HashSet::new();
    let mut creation_node_ids = HashSet::new();

    for row in rows {
        let ev = MatchedAuditEvent {
            event_seq: row.get("event_seq"),
            batch_id: row.get("batch_id"),
            event_type: row.get("event_type"),
            entity_id: row.get("entity_id"),
            entity_type: row.get("entity_type"),
            actor_id: row.get("actor_id"),
            delta: row.get("delta"),
            snapshot: row.get("snapshot"),
        };
        targeted_actors.insert(ev.actor_id.clone());
        if ev.entity_type == "EDGE" {
            targeted_edge_ids.insert(ev.entity_id);
        } else {
            targeted_node_ids.insert(ev.entity_id);
            if matches!(
                ev.event_type.as_str(),
                "TASK_ELABORATED" | "NODE_INSERTED" | "STAGING_APPROVED"
            ) {
                creation_node_ids.insert(ev.entity_id);
            }
        }
        matched_events.push(ev);
    }

    // 3. Discover active outgoing edges for newly created nodes being reverted
    let targeted_node_vec: Vec<Uuid> = targeted_node_ids.iter().copied().collect();
    let creation_node_vec: Vec<Uuid> = creation_node_ids.iter().copied().collect();
    let mut affected_edge_ids = targeted_edge_ids.clone();

    if !creation_node_vec.is_empty() {
        let incident_edges = client
            .query(
                "SELECT edge_id, from_node_id, to_node_id, created_by \
                 FROM graph_edges \
                 WHERE from_node_id = ANY($1) \
                   AND lifecycle_state = 'ACTIVE';",
                &[&creation_node_vec],
            )
            .await?;

        for edge_row in incident_edges {
            let edge_id: Uuid = edge_row.get("edge_id");
            affected_edge_ids.insert(edge_id);
        }
    }

    // 4. Downward recursive dependency sweep to discover active dependent child entities
    let mut cascade_reverified_nodes = Vec::new();
    let mut cross_agent_dependencies = HashSet::new();

    if !targeted_node_vec.is_empty() {
        let dep_cte_query = "
            WITH RECURSIVE dep_tree AS (
                SELECT
                    e.from_node_id AS child_id,
                    e.to_node_id AS parent_id,
                    e.edge_id,
                    1 AS depth,
                    ARRAY[e.to_node_id, e.from_node_id] AS visited
                FROM graph_edges e
                WHERE e.to_node_id = ANY($1)
                  AND e.lifecycle_state = 'ACTIVE'
                  AND e.edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM')

                UNION ALL

                SELECT
                    e.from_node_id AS child_id,
                    e.to_node_id AS parent_id,
                    e.edge_id,
                    dt.depth + 1,
                    dt.visited || e.from_node_id
                FROM graph_edges e
                JOIN dep_tree dt ON dt.child_id = e.to_node_id
                WHERE e.lifecycle_state = 'ACTIVE'
                  AND e.edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM')
                  AND NOT (e.from_node_id = ANY(dt.visited))
                  AND dt.depth < 20
            )
            SELECT DISTINCT dt.child_id, n.node_key, n.title, n.node_type, n.created_by
            FROM dep_tree dt
            JOIN graph_nodes n ON n.id = dt.child_id
            WHERE n.lifecycle_state = 'ACTIVE'
              AND NOT (dt.child_id = ANY($1));
        ";

        let dep_rows = client.query(dep_cte_query, &[&targeted_node_vec]).await?;
        for row in dep_rows {
            let child_id: Uuid = row.get("child_id");
            let created_by: String = row.get("created_by");

            cascade_reverified_nodes.push(child_id);

            // Cross-agent detection: child task authored by an agent other than targeted actors
            if !targeted_actors.contains(&created_by) && created_by != actor.agent_id {
                cross_agent_dependencies.insert(created_by);
            }
        }
    }

    // Sort and deduplicate preview lists
    let mut all_affected_nodes: Vec<Uuid> = targeted_node_ids
        .iter()
        .copied()
        .chain(cascade_reverified_nodes.iter().copied())
        .collect();
    all_affected_nodes.sort();
    all_affected_nodes.dedup();

    let mut affected_edges_vec: Vec<Uuid> = affected_edge_ids.into_iter().collect();
    affected_edges_vec.sort();

    let mut cross_agent_deps_vec: Vec<String> = cross_agent_dependencies.into_iter().collect();
    cross_agent_deps_vec.sort();

    cascade_reverified_nodes.sort();
    cascade_reverified_nodes.dedup();

    let preview = RevertPreview {
        affected_nodes: all_affected_nodes,
        affected_edges: affected_edges_vec.clone(),
        cross_agent_dependencies: cross_agent_deps_vec,
        total_events: matched_events.len(),
    };

    // 5. Handle dry-run inspection under READ COMMITTED
    if is_dry_run {
        return Ok(RevertExecutionResult::DryRun(preview));
    }

    // 6. Safety confirmation verification: halt if cross-agent dependencies exist and force != true
    if !preview.cross_agent_dependencies.is_empty() && !force {
        return Err(MutationError::ConfirmationRequired(Box::new(preview)));
    }

    // 7. Execute compensating rollback transaction under global advisory lock
    let tx = client.transaction().await?;

    // Acquire global transaction advisory lock FIRST (C-19, D-66)
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtext('tks_structural_mutation'));",
        &[],
    )
    .await?;

    let rollback_batch_id = Uuid::new_v4();

    // (a) Record compensating REVERT audit events for matched operations
    for ev in &matched_events {
        let delta = serde_json::json!({
            "reverted_event_seq": ev.event_seq,
            "reverted_batch_id": ev.batch_id,
            "reverted_event_type": ev.event_type,
            "compensation_action": "REVERT"
        });

        insert_audit_event(
            &tx,
            rollback_batch_id,
            "REVERT",
            ev.entity_id,
            &ev.entity_type,
            &actor.agent_id,
            &actor.actor_type,
            &actor.token_fingerprint(),
            delta,
            ev.snapshot.clone(),
        )
        .await?;
    }

    // (b) Transition targeted nodes:
    // Creation events transition live nodes to SUPERSEDED.
    // In-place update events restore prior attributes/status without superseding.
    let mut superseded_node_ids = Vec::new();

    for ev in &matched_events {
        match ev.event_type.as_str() {
            "TASK_ELABORATED" | "NODE_INSERTED" | "STAGING_APPROVED" => {
                superseded_node_ids.push(ev.entity_id);
            }
            "TASK_STATUS_UPDATE" => {
                if let Some(prev_status) = ev
                    .delta
                    .get("previous_status")
                    .and_then(serde_json::Value::as_str)
                {
                    tx.execute(
                        "UPDATE graph_nodes \
                         SET attributes = jsonb_set(attributes, '{execution_status}', to_jsonb($1::text)) \
                         WHERE id = $2;",
                        &[&prev_status, &ev.entity_id],
                    )
                    .await?;
                }
            }
            "LEAF_ATTRIBUTE_UPDATE" => {
                if let Some(prior_attrs) = ev.snapshot.get("attributes") {
                    tx.execute(
                        "UPDATE graph_nodes \
                         SET attributes = $1 \
                         WHERE id = $2;",
                        &[&prior_attrs, &ev.entity_id],
                    )
                    .await?;
                }
            }
            _ => {
                // Default: transition to SUPERSEDED if active node
                if ev.entity_type != "EDGE" {
                    superseded_node_ids.push(ev.entity_id);
                }
            }
        }
    }

    if !superseded_node_ids.is_empty() {
        tx.execute(
            "UPDATE graph_nodes \
             SET lifecycle_state = 'SUPERSEDED' \
             WHERE id = ANY($1) AND lifecycle_state = 'ACTIVE';",
            &[&superseded_node_ids],
        )
        .await?;
    }

    // (c) Transition associated edges to REVERTED
    if !affected_edges_vec.is_empty() {
        tx.execute(
            "UPDATE graph_edges \
             SET lifecycle_state = 'REVERTED' \
             WHERE edge_id = ANY($1) AND lifecycle_state = 'ACTIVE';",
            &[&affected_edges_vec],
        )
        .await?;

        for edge_id in &affected_edges_vec {
            let edge_delta = serde_json::json!({
                "action": "EDGE_REVERTED",
                "rollback_batch_id": rollback_batch_id
            });
            insert_audit_event(
                &tx,
                rollback_batch_id,
                "REVERT",
                *edge_id,
                "EDGE",
                &actor.agent_id,
                &actor.actor_type,
                &actor.token_fingerprint(),
                edge_delta,
                serde_json::Value::Null,
            )
            .await?;
        }
    }

    // (d) Cascading transition active child tasks to NEEDS_REVERIFICATION
    if !cascade_reverified_nodes.is_empty() {
        tx.execute(
            "UPDATE graph_nodes \
             SET lifecycle_state = 'NEEDS_REVERIFICATION', \
                 attributes = jsonb_set(attributes, '{staleness_reason}', '\"Upstream dependency was reverted\"'::jsonb) \
             WHERE id = ANY($1) AND lifecycle_state = 'ACTIVE';",
            &[&cascade_reverified_nodes],
        )
        .await?;

        for child_id in &cascade_reverified_nodes {
            let cascade_delta = serde_json::json!({
                "lifecycle_state": "NEEDS_REVERIFICATION",
                "triggered_by_rollback_batch": rollback_batch_id,
                "staleness_reason": "Upstream dependency was reverted"
            });
            insert_audit_event(
                &tx,
                rollback_batch_id,
                "CASCADE_NEEDS_REVERIFICATION",
                *child_id,
                "TASK",
                &actor.agent_id,
                &actor.actor_type,
                &actor.token_fingerprint(),
                cascade_delta,
                serde_json::Value::Null,
            )
            .await?;
        }
    }

    tx.commit().await?;

    let targeted_nodes_list: Vec<Uuid> = targeted_node_ids.into_iter().collect();

    Ok(RevertExecutionResult::Reverted(RevertResult {
        batch_id: rollback_batch_id,
        reverted_events: matched_events.len(),
        affected_nodes: targeted_nodes_list,
        cascade_reverified_nodes,
    }))
}
