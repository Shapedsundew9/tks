//! Three-way topological merge analysis, conflict resolution heuristics,
//! and atomic batch promotion for multi-agent workspaces (WP-3.3, PHASE3-003).
//!
//! Analyzes divergence between a workspace branch and the live substrate since `base_event_seq`.
//! Formulates a structured conflict taxonomy:
//! - Structural cycle formations (`MergeConflict::CycleDetected`)
//! - Parent requirement supersessions or rollbacks (`MergeConflict::ParentSuperseded`)
//! - Canonical `node_key` collisions (`MergeConflict::KeyCollision`)
//!
//! Implements automated resolution strategies:
//! - Fast-forward promotion for disjoint subtrees
//! - Deterministic auto-reparenting when parent requirements advanced cleanly
//! - Explicit rebase requirements on structural conflicts
//! - Atomic promotion under the canonical lock hierarchy (`pg_advisory_xact_lock` -> row locks)
//!   promoting candidate entities into `ACTIVE` state with monotonic `event_seq` and shared `batch_id`
//!   (INV-1, INV-2, INV-5, INV-7, C-18, C-19, D-8, D-32, D-45, D-57, D-65, D-81).

use serde::{Deserialize, Serialize};
use sha2::Digest;
use tokio_postgres::Client;
use uuid::Uuid;

use crate::gateway::auth::AuthenticatedAgent;
use crate::storage::envelope::validate_ancestor_path;
use crate::storage::governance::GovernancePolicy;
use crate::storage::mutation::{MutationError, STRUCTURAL_ADVISORY_LOCK_SQL, insert_audit_event};
use crate::storage::workspace::{WorkspaceRecord, get_workspace_edges, get_workspace_nodes};
use crate::storage::{GraphEdge, GraphNode};

/// Categorized conflict encountered during three-way workspace merge analysis or promotion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum MergeConflict {
    /// Directed candidate edge would introduce a directed cycle in structural relationships.
    CycleDetected { from_id: Uuid, to_id: Uuid },
    /// Parent requirement was superseded, reverted, or degraded in the live substrate.
    ParentSuperseded {
        task_id: Uuid,
        old_parent_id: Uuid,
        reason: String,
    },
    /// Candidate node key collides with an existing active node in the substrate.
    KeyCollision {
        node_key: String,
        conflicting_node_id: Uuid,
    },
}

impl std::fmt::Display for MergeConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CycleDetected { from_id, to_id } => {
                write!(
                    f,
                    "CycleDetected: directed edge from {from_id} to {to_id} would create a directed cycle in structural relationships"
                )
            }
            Self::ParentSuperseded {
                task_id,
                old_parent_id,
                reason,
            } => {
                write!(
                    f,
                    "ParentSuperseded: task {task_id} parent {old_parent_id} is superseded: {reason}"
                )
            }
            Self::KeyCollision {
                node_key,
                conflicting_node_id,
            } => {
                write!(
                    f,
                    "KeyCollision: node key '{node_key}' collides with active node {conflicting_node_id}"
                )
            }
        }
    }
}

/// Preview returned from three-way workspace merge analysis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MergePreview {
    pub can_fast_forward: bool,
    pub candidate_nodes: Vec<Uuid>,
    pub candidate_edges: Vec<Uuid>,
    pub conflicts: Vec<MergeConflict>,
}

/// Outcome of an atomic workspace promotion transaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromotionResult {
    pub status: String,
    pub batch_id: Uuid,
    pub promoted_nodes: usize,
    pub promoted_edges: usize,
    pub event_seq: i64,
}

/// Outcome of a workspace synchronization rebase transaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RebaseResult {
    pub status: String,
    pub workspace_id: Uuid,
    pub old_base_event_seq: i64,
    pub new_base_event_seq: i64,
    pub reparented_tasks: usize,
}

/// Analyzes divergence between a workspace branch and the live substrate, detecting cycles,
/// parent supersessions, and node key collisions (WP-3.3, PHASE3-003).
///
/// # Errors
///
/// - Returns `MutationError::NotFound` if workspace is missing.
/// - Returns `MutationError::GovernanceRejected` if workspace is not in `ACTIVE` status.
/// - Returns `MutationError::Database` on SQL failure.
pub async fn analyze_workspace_merge(
    client: &mut deadpool_postgres::Client,
    workspace_id: Uuid,
) -> Result<MergePreview, MutationError> {
    analyze_workspace_merge_client(client, workspace_id).await
}

/// Variant of `analyze_workspace_merge` operating on a standard `tokio_postgres::Client`.
///
/// # Errors
///
/// Returns `MutationError` on query failure or if workspace does not exist or is inactive.
pub async fn analyze_workspace_merge_client(
    client: &Client,
    workspace_id: Uuid,
) -> Result<MergePreview, MutationError> {
    // 1. Verify workspace exists and is active
    let ws_row = client
        .query_opt(
            "SELECT id, workspace_name, owner_agent, base_event_seq, status, created_at, attributes \
             FROM workspaces WHERE id = $1;",
            &[&workspace_id],
        )
        .await?;

    let ws = ws_row
        .map(|r| WorkspaceRecord::from_row(&r))
        .ok_or_else(|| MutationError::NotFound(format!("Workspace '{workspace_id}' not found")))?;

    if ws.status != "ACTIVE" {
        return Err(MutationError::GovernanceRejected(format!(
            "Workspace '{workspace_id}' is not ACTIVE (status: '{}')",
            ws.status
        )));
    }

    // 2. Fetch candidate nodes and edges
    let candidate_nodes_list = get_workspace_nodes(client, workspace_id)
        .await
        .map_err(MutationError::from)?;
    let candidate_node_ids: Vec<Uuid> = candidate_nodes_list.iter().map(|n| n.id).collect();

    let candidate_edges_list = get_workspace_edges(client, workspace_id)
        .await
        .map_err(MutationError::from)?;
    let candidate_edge_ids: Vec<Uuid> = candidate_edges_list.iter().map(|e| e.edge_id).collect();

    let mut conflicts = Vec::new();

    // 3. Detect canonical node_key collisions against active substrate nodes
    let key_collision_rows = client
        .query(
            "SELECT cn.node_key, gn.id AS conflicting_id \
             FROM graph_nodes cn \
             JOIN graph_nodes gn ON gn.node_key = cn.node_key AND gn.id != cn.id \
             WHERE (cn.attributes->>'workspace_id') = $1 \
               AND cn.lifecycle_state = 'DRAFT' \
               AND cn.node_key IS NOT NULL \
               AND gn.lifecycle_state = 'ACTIVE';",
            &[&workspace_id.to_string()],
        )
        .await?;

    for row in key_collision_rows {
        let node_key: String = row.get("node_key");
        let conflicting_node_id: Uuid = row.get("conflicting_id");
        conflicts.push(MergeConflict::KeyCollision {
            node_key,
            conflicting_node_id,
        });
    }

    // 4. Inspect external parent nodes of candidate edges for supersession / degradation
    for edge in &candidate_edges_list {
        if candidate_node_ids.contains(&edge.to_node_id) {
            // Intra-workspace candidate edge
            continue;
        }

        let parent_row_opt = client
            .query_opt(
                "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                        governance_policy, created_by, job_id, doc_path, doc_hash, \
                        byte_start, byte_end, attributes \
                 FROM graph_nodes WHERE id = $1;",
                &[&edge.to_node_id],
            )
            .await?;

        match parent_row_opt {
            None => {
                conflicts.push(MergeConflict::ParentSuperseded {
                    task_id: edge.from_node_id,
                    old_parent_id: edge.to_node_id,
                    reason: format!("Parent node '{}' not found", edge.to_node_id),
                });
            }
            Some(row) => {
                let parent = GraphNode::from_row(&row);
                if parent.lifecycle_state != "ACTIVE" {
                    // Check audit ledger for specific modification event committed since base_event_seq
                    let audit_row_opt = client
                        .query_opt(
                            "SELECT event_seq, event_type FROM audit_ledger \
                             WHERE entity_id = $1 AND event_seq > $2 \
                             ORDER BY event_seq DESC LIMIT 1;",
                            &[&parent.id, &ws.base_event_seq],
                        )
                        .await?;

                    let reason = if let Some(ar) = audit_row_opt {
                        let ev_seq: i64 = ar.get("event_seq");
                        let ev_type: String = ar.get("event_type");
                        format!(
                            "Parent node '{}' is in state '{}' (audit event #{ev_seq} {ev_type})",
                            parent.id, parent.lifecycle_state
                        )
                    } else {
                        format!(
                            "Parent node '{}' is in state '{}'",
                            parent.id, parent.lifecycle_state
                        )
                    };

                    conflicts.push(MergeConflict::ParentSuperseded {
                        task_id: edge.from_node_id,
                        old_parent_id: parent.id,
                        reason,
                    });
                }
            }
        }
    }

    // 5. Detect cycle formation across simulated combined graph (active edges + candidate edges)
    let cycle_conflicts = detect_cycle_conflicts(client, workspace_id).await?;
    conflicts.extend(cycle_conflicts);

    // 6. Fast-forward is possible if zero conflicts exist
    let can_fast_forward = conflicts.is_empty();

    Ok(MergePreview {
        can_fast_forward,
        candidate_nodes: candidate_node_ids,
        candidate_edges: candidate_edge_ids,
        conflicts,
    })
}

/// Runs a recursive CTE simulating candidate edges against current live active edges
/// to detect any circular paths emerging in structural relationships (D-8, INV-1).
async fn detect_cycle_conflicts(
    client: &(impl tokio_postgres::GenericClient + ?Sized),
    workspace_id: Uuid,
) -> Result<Vec<MergeConflict>, MutationError> {
    let ws_id_str = workspace_id.to_string();

    let query = "
        WITH RECURSIVE combined_edges AS (
            SELECT from_node_id, to_node_id
            FROM graph_edges
            WHERE lifecycle_state = 'ACTIVE'
              AND edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM')
            UNION ALL
            SELECT from_node_id, to_node_id
            FROM graph_edges
            WHERE (attributes->>'workspace_id') = $1
              AND lifecycle_state = 'DRAFT'
              AND edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM')
        ),
        candidate_edges AS (
            SELECT from_node_id, to_node_id
            FROM graph_edges
            WHERE (attributes->>'workspace_id') = $1
              AND lifecycle_state = 'DRAFT'
              AND edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM')
        ),
        cycle_walk AS (
            SELECT
                ce.from_node_id,
                ce.to_node_id,
                ce.from_node_id AS target_id,
                ce.to_node_id AS current_id,
                ARRAY[ce.to_node_id] AS visited,
                1 AS depth,
                (ce.from_node_id = ce.to_node_id) AS is_cycle
            FROM candidate_edges ce
            UNION ALL
            SELECT
                cw.from_node_id,
                cw.to_node_id,
                cw.target_id,
                e.to_node_id AS current_id,
                cw.visited || e.to_node_id,
                cw.depth + 1,
                (e.to_node_id = cw.target_id) AS is_cycle
            FROM cycle_walk cw
            JOIN combined_edges e ON e.from_node_id = cw.current_id
            WHERE NOT cw.is_cycle
              AND NOT (e.to_node_id = ANY(cw.visited))
              AND cw.depth < 50
        )
        SELECT DISTINCT from_node_id, to_node_id
        FROM cycle_walk
        WHERE is_cycle = true;
    ";

    let rows = client.query(query, &[&ws_id_str]).await?;
    let mut conflicts = Vec::with_capacity(rows.len());

    for row in rows {
        let from_id: Uuid = row.get("from_node_id");
        let to_id: Uuid = row.get("to_node_id");
        conflicts.push(MergeConflict::CycleDetected { from_id, to_id });
    }

    Ok(conflicts)
}

/// Recursively traverses the replacement lineage to find an active replacement node
/// for a superseded requirement node (DEC-3.7).
///
/// First inspects `attributes->>'replaces_node_id'` chain for active descendants.
/// If not found, falls back to an active node matching `old_node_key`.
pub async fn find_active_parent_replacement(
    client: &(impl tokio_postgres::GenericClient + ?Sized),
    old_parent_id: Uuid,
    old_node_key: Option<&str>,
) -> Result<Option<GraphNode>, MutationError> {
    let old_id_str = old_parent_id.to_string();

    // 1. Recursive lineage traversal following replaces_node_id
    let recursive_query = "
        WITH RECURSIVE replacement_chain AS (
            SELECT id, node_key, node_type, title, content, lifecycle_state, \
                   governance_policy, created_by, job_id, doc_path, doc_hash, \
                   byte_start, byte_end, attributes, 1 AS depth \
            FROM graph_nodes \
            WHERE (attributes->>'replaces_node_id') = $1 \
            UNION ALL \
            SELECT gn.id, gn.node_key, gn.node_type, gn.title, gn.content, gn.lifecycle_state, \
                   gn.governance_policy, gn.created_by, gn.job_id, gn.doc_path, gn.doc_hash, \
                   gn.byte_start, gn.byte_end, gn.attributes, rc.depth + 1 \
            FROM replacement_chain rc \
            JOIN graph_nodes gn ON (gn.attributes->>'replaces_node_id') = rc.id::text \
            WHERE rc.depth < 10 \
        ) \
        SELECT id, node_key, node_type, title, content, lifecycle_state, \
               governance_policy, created_by, job_id, doc_path, doc_hash, \
               byte_start, byte_end, attributes \
        FROM replacement_chain \
        WHERE lifecycle_state = 'ACTIVE' \
        ORDER BY depth DESC \
        LIMIT 1;
    ";

    let row_opt = client.query_opt(recursive_query, &[&old_id_str]).await?;
    if let Some(row) = row_opt {
        return Ok(Some(GraphNode::from_row(&row)));
    }

    // 2. Fallback: match by canonical node_key in active state
    if let Some(key) = old_node_key {
        let key_trimmed = key.trim();
        if !key_trimmed.is_empty() {
            let row_opt = client
                .query_opt(
                    "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                            governance_policy, created_by, job_id, doc_path, doc_hash, \
                            byte_start, byte_end, attributes \
                     FROM graph_nodes \
                     WHERE node_key = $1 AND lifecycle_state = 'ACTIVE' AND id != $2 \
                     LIMIT 1;",
                    &[&key_trimmed, &old_parent_id],
                )
                .await?;

            if let Some(row) = row_opt {
                return Ok(Some(GraphNode::from_row(&row)));
            }
        }
    }

    Ok(None)
}

/// Executes atomic promotion of a workspace branch into the live substrate under canonical lock
/// serialization (`pg_advisory_xact_lock` -> row locks) (INV-1, INV-2, INV-5, C-19).
///
/// Promotes all candidate nodes and edges from `DRAFT` to `ACTIVE`, marks the workspace container
/// as `MERGED`, and commits an atomic `PROMOTION` audit ledger entry stamped with monotonic
/// `event_seq` and a shared transaction correlation `batch_id`.
///
/// # Errors
///
/// - Returns `MutationError::NotFound` if workspace is missing.
/// - Returns `MutationError::GovernanceRejected` if workspace is inactive or caller is unauthorized.
/// - Returns `MutationError::MergeConflict` if topological cycles, unresolvable supersessions, or key collisions occur.
/// - Returns `MutationError::InvalidAncestorPath` if promoted tasks lack an unbroken path to an active requirement (INV-1).
/// - Returns `MutationError::Database` on SQL failure.
pub async fn promote_workspace(
    client: &mut deadpool_postgres::Client,
    workspace_id: Uuid,
    auto_reparent: bool,
    actor: &AuthenticatedAgent,
) -> Result<PromotionResult, MutationError> {
    promote_workspace_client(client, workspace_id, auto_reparent, actor).await
}

/// Variant of `promote_workspace` operating on a standard `tokio_postgres::Client`.
///
/// # Errors
///
/// Returns `MutationError` on validation or persistence error.
pub async fn promote_workspace_client(
    client: &mut Client,
    workspace_id: Uuid,
    auto_reparent: bool,
    actor: &AuthenticatedAgent,
) -> Result<PromotionResult, MutationError> {
    let tx = client.transaction().await?;

    // 1. Strict lock hierarchy (C-19, D-66): global transaction advisory lock FIRST
    tx.execute(STRUCTURAL_ADVISORY_LOCK_SQL, &[]).await?;

    // 2. Fetch and lock workspace record
    let ws_row = tx
        .query_opt(
            "SELECT id, workspace_name, owner_agent, base_event_seq, status, created_at, attributes \
             FROM workspaces WHERE id = $1 FOR UPDATE;",
            &[&workspace_id],
        )
        .await?;

    let ws = ws_row
        .map(|r| WorkspaceRecord::from_row(&r))
        .ok_or_else(|| MutationError::NotFound(format!("Workspace '{workspace_id}' not found")))?;

    if ws.status != "ACTIVE" {
        return Err(MutationError::GovernanceRejected(format!(
            "Workspace '{workspace_id}' is not ACTIVE (status: '{}')",
            ws.status
        )));
    }

    if ws.owner_agent != actor.agent_id && actor.actor_type != "HUMAN" && actor.agent_id != "system"
    {
        return Err(MutationError::GovernanceRejected(format!(
            "Agent '{}' is not authorized to promote workspace owned by '{}'",
            actor.agent_id, ws.owner_agent
        )));
    }

    let ws_id_str = workspace_id.to_string();

    // 3. Fetch candidate nodes with row lock
    let node_rows = tx
        .query(
            "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                    governance_policy, created_by, job_id, doc_path, doc_hash, \
                    byte_start, byte_end, attributes \
             FROM graph_nodes \
             WHERE (attributes->>'workspace_id') = $1 AND lifecycle_state = 'DRAFT' \
             FOR UPDATE;",
            &[&ws_id_str],
        )
        .await?;

    let candidate_nodes: Vec<GraphNode> = node_rows.iter().map(GraphNode::from_row).collect();
    let candidate_node_ids: Vec<Uuid> = candidate_nodes.iter().map(|n| n.id).collect();

    // 4. Fetch candidate edges with row lock
    let edge_rows = tx
        .query(
            "SELECT edge_id, from_node_id, to_node_id, edge_type, created_by, lifecycle_state, attributes \
             FROM graph_edges \
             WHERE (attributes->>'workspace_id') = $1 AND lifecycle_state = 'DRAFT' \
             FOR UPDATE;",
            &[&ws_id_str],
        )
        .await?;

    let candidate_edges: Vec<GraphEdge> = edge_rows.iter().map(GraphEdge::from_row).collect();
    let candidate_edge_ids: Vec<Uuid> = candidate_edges.iter().map(|e| e.edge_id).collect();

    let mut conflicts = Vec::new();

    // 5. Check canonical node_key collisions against active substrate nodes
    let key_collision_rows = tx
        .query(
            "SELECT cn.node_key, gn.id AS conflicting_id \
             FROM graph_nodes cn \
             JOIN graph_nodes gn ON gn.node_key = cn.node_key AND gn.id != cn.id \
             WHERE (cn.attributes->>'workspace_id') = $1 \
               AND cn.lifecycle_state = 'DRAFT' \
               AND cn.node_key IS NOT NULL \
               AND gn.lifecycle_state = 'ACTIVE';",
            &[&ws_id_str],
        )
        .await?;

    for row in key_collision_rows {
        let node_key: String = row.get("node_key");
        let conflicting_node_id: Uuid = row.get("conflicting_id");
        conflicts.push(MergeConflict::KeyCollision {
            node_key,
            conflicting_node_id,
        });
    }

    // 6. Inspect external parents and apply auto-reparenting if requested
    for edge in &candidate_edges {
        if candidate_node_ids.contains(&edge.to_node_id) {
            // Intra-workspace edge
            continue;
        }

        let parent_row_opt = tx
            .query_opt(
                "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                        governance_policy, created_by, job_id, doc_path, doc_hash, \
                        byte_start, byte_end, attributes \
                 FROM graph_nodes WHERE id = $1 FOR UPDATE;",
                &[&edge.to_node_id],
            )
            .await?;

        match parent_row_opt {
            None => {
                conflicts.push(MergeConflict::ParentSuperseded {
                    task_id: edge.from_node_id,
                    old_parent_id: edge.to_node_id,
                    reason: format!("Parent node '{}' not found", edge.to_node_id),
                });
            }
            Some(p_row) => {
                let parent = GraphNode::from_row(&p_row);
                if parent.lifecycle_state != "ACTIVE" {
                    if auto_reparent {
                        let replacement_opt = find_active_parent_replacement(
                            &tx,
                            parent.id,
                            parent.node_key.as_deref(),
                        )
                        .await?;

                        if let Some(rep) = replacement_opt {
                            let rep_policy = rep.governance_policy.parse::<GovernancePolicy>().ok();
                            if rep_policy == Some(GovernancePolicy::AutonomousElaboration) {
                                // Auto-reparent child upward edge to newest active parent revision
                                tx.execute(
                                    "UPDATE graph_edges SET to_node_id = $1 WHERE edge_id = $2;",
                                    &[&rep.id, &edge.edge_id],
                                )
                                .await?;

                                let rep_id_str = rep.id.to_string();
                                // Update candidate task parent_node_id attribute
                                tx.execute(
                                    "UPDATE graph_nodes \
                                     SET attributes = jsonb_set(attributes, '{parent_node_id}', to_jsonb($1::text)) \
                                     WHERE id = $2;",
                                    &[&rep_id_str, &edge.from_node_id],
                                )
                                .await?;
                            } else {
                                conflicts.push(MergeConflict::ParentSuperseded {
                                    task_id: edge.from_node_id,
                                    old_parent_id: parent.id,
                                    reason: format!(
                                        "Active replacement '{}' has policy '{}', expected 'AUTONOMOUS_ELABORATION'",
                                        rep.id, rep.governance_policy
                                    ),
                                });
                            }
                        } else {
                            conflicts.push(MergeConflict::ParentSuperseded {
                                task_id: edge.from_node_id,
                                old_parent_id: parent.id,
                                reason: format!(
                                    "Parent node '{}' is in state '{}' with no active replacement in lineage",
                                    parent.id, parent.lifecycle_state
                                ),
                            });
                        }
                    } else {
                        conflicts.push(MergeConflict::ParentSuperseded {
                            task_id: edge.from_node_id,
                            old_parent_id: parent.id,
                            reason: format!(
                                "Parent node '{}' is in state '{}'",
                                parent.id, parent.lifecycle_state
                            ),
                        });
                    }
                }
            }
        }
    }

    // 7. Verify DAG cycle freedom with reparented edges
    let cycle_conflicts = detect_cycle_conflicts(&tx, workspace_id).await?;
    conflicts.extend(cycle_conflicts);

    // 8. If conflicts exist, abort transaction and return structured conflict error
    if !conflicts.is_empty() {
        return Err(MutationError::MergeConflict(conflicts));
    }

    // 9. Validate Invariant INV-1 ancestor paths for candidate nodes (D-34, INV-1)
    if !candidate_node_ids.is_empty() {
        let is_valid = validate_ancestor_path(&tx, &candidate_node_ids, false).await?;
        if !is_valid {
            return Err(MutationError::InvalidAncestorPath(
                "Promoted workspace nodes lack an unbroken path to an active requirement root (INV-1)".to_string(),
            ));
        }
    }

    // 10. Promote candidate nodes and edges from DRAFT to ACTIVE
    tx.execute(
        "UPDATE graph_nodes SET lifecycle_state = 'ACTIVE' \
         WHERE (attributes->>'workspace_id') = $1 AND lifecycle_state = 'DRAFT';",
        &[&ws_id_str],
    )
    .await?;

    tx.execute(
        "UPDATE graph_edges SET lifecycle_state = 'ACTIVE' \
         WHERE (attributes->>'workspace_id') = $1 AND lifecycle_state = 'DRAFT';",
        &[&ws_id_str],
    )
    .await?;

    // 11. Transition workspace container status to MERGED
    tx.execute(
        "UPDATE workspaces SET status = 'MERGED' WHERE id = $1;",
        &[&workspace_id],
    )
    .await?;

    // 12. Enqueue embeddings for newly active nodes into node_embeddings
    for node in &candidate_nodes {
        let combined_text = format!(
            "{} {}",
            node.title.as_deref().unwrap_or(""),
            node.content.as_deref().unwrap_or("")
        );
        let mut hasher = sha2::Sha256::new();
        hasher.update(combined_text.as_bytes());
        let content_hash = format!("{:x}", hasher.finalize());

        tx.execute(
            "INSERT INTO node_embeddings (node_id, content_hash, status, scheduled_at) \
             VALUES ($1, $2, 'PENDING', clock_timestamp()) \
             ON CONFLICT (node_id) DO UPDATE SET \
                content_hash = EXCLUDED.content_hash, \
                status = 'PENDING', \
                scheduled_at = clock_timestamp();",
            &[&node.id, &content_hash],
        )
        .await?;
    }

    // 13. Write atomic PROMOTION audit event with monotonic event_seq and batch_id
    let batch_id = Uuid::new_v4();
    let delta = serde_json::json!({
        "workspace_id": workspace_id,
        "status": "MERGED",
        "promoted_nodes": candidate_node_ids.len(),
        "promoted_edges": candidate_edge_ids.len(),
        "auto_reparent": auto_reparent,
    });
    let snapshot = serde_json::json!({
        "workspace_id": workspace_id,
        "promoted_node_ids": candidate_node_ids,
        "promoted_edge_ids": candidate_edge_ids,
    });

    let event_seq = insert_audit_event(
        &tx,
        batch_id,
        "PROMOTION",
        workspace_id,
        "WORKSPACE",
        &actor.agent_id,
        &actor.actor_type,
        &actor.token_fingerprint(),
        delta,
        snapshot,
    )
    .await?;

    tx.commit().await?;

    Ok(PromotionResult {
        status: "MERGED".to_string(),
        batch_id,
        promoted_nodes: candidate_node_ids.len(),
        promoted_edges: candidate_edge_ids.len(),
        event_seq,
    })
}

/// Synchronizes a workspace container branch with live substrate state (rebase).
///
/// If `auto_reparent == true`, updates candidate upward edges whose parents advanced cleanly
/// in the live substrate to point to the newest active parent revision.
/// Updates the workspace's `base_event_seq` to the latest event sequence in `audit_ledger`,
/// logging an audit entry.
///
/// # Errors
///
/// - Returns `MutationError::NotFound` if workspace is missing.
/// - Returns `MutationError::GovernanceRejected` if workspace is not active or caller is unauthorized.
/// - Returns `MutationError::MergeConflict` if unresolved conflicts remain.
/// - Returns `MutationError::Database` on SQL failure.
pub async fn sync_workspace_rebase(
    client: &mut deadpool_postgres::Client,
    workspace_id: Uuid,
    auto_reparent: bool,
    actor: &AuthenticatedAgent,
) -> Result<RebaseResult, MutationError> {
    sync_workspace_rebase_client(client, workspace_id, auto_reparent, actor).await
}

/// Variant of `sync_workspace_rebase` operating on a standard `tokio_postgres::Client`.
///
/// # Errors
///
/// Returns `MutationError` on validation or persistence error.
pub async fn sync_workspace_rebase_client(
    client: &mut Client,
    workspace_id: Uuid,
    auto_reparent: bool,
    actor: &AuthenticatedAgent,
) -> Result<RebaseResult, MutationError> {
    let tx = client.transaction().await?;

    // 1. Structural advisory lock
    tx.execute(STRUCTURAL_ADVISORY_LOCK_SQL, &[]).await?;

    // 2. Fetch and lock workspace record
    let ws_row = tx
        .query_opt(
            "SELECT id, workspace_name, owner_agent, base_event_seq, status, created_at, attributes \
             FROM workspaces WHERE id = $1 FOR UPDATE;",
            &[&workspace_id],
        )
        .await?;

    let ws = ws_row
        .map(|r| WorkspaceRecord::from_row(&r))
        .ok_or_else(|| MutationError::NotFound(format!("Workspace '{workspace_id}' not found")))?;

    if ws.status != "ACTIVE" {
        return Err(MutationError::GovernanceRejected(format!(
            "Workspace '{workspace_id}' is not ACTIVE (status: '{}')",
            ws.status
        )));
    }

    if ws.owner_agent != actor.agent_id && actor.actor_type != "HUMAN" && actor.agent_id != "system"
    {
        return Err(MutationError::GovernanceRejected(format!(
            "Agent '{}' is not authorized to rebase workspace owned by '{}'",
            actor.agent_id, ws.owner_agent
        )));
    }

    let ws_id_str = workspace_id.to_string();

    // 3. Fetch candidate nodes and edges with row locks
    let node_rows = tx
        .query(
            "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                    governance_policy, created_by, job_id, doc_path, doc_hash, \
                    byte_start, byte_end, attributes \
             FROM graph_nodes \
             WHERE (attributes->>'workspace_id') = $1 AND lifecycle_state = 'DRAFT' \
             FOR UPDATE;",
            &[&ws_id_str],
        )
        .await?;
    let candidate_nodes: Vec<GraphNode> = node_rows.iter().map(GraphNode::from_row).collect();
    let candidate_node_ids: Vec<Uuid> = candidate_nodes.iter().map(|n| n.id).collect();

    let edge_rows = tx
        .query(
            "SELECT edge_id, from_node_id, to_node_id, edge_type, created_by, lifecycle_state, attributes \
             FROM graph_edges \
             WHERE (attributes->>'workspace_id') = $1 AND lifecycle_state = 'DRAFT' \
             FOR UPDATE;",
            &[&ws_id_str],
        )
        .await?;
    let candidate_edges: Vec<GraphEdge> = edge_rows.iter().map(GraphEdge::from_row).collect();

    let mut reparented_tasks = 0;
    let mut conflicts = Vec::new();

    // 4. Check key collisions
    let key_collision_rows = tx
        .query(
            "SELECT cn.node_key, gn.id AS conflicting_id \
             FROM graph_nodes cn \
             JOIN graph_nodes gn ON gn.node_key = cn.node_key AND gn.id != cn.id \
             WHERE (cn.attributes->>'workspace_id') = $1 \
               AND cn.lifecycle_state = 'DRAFT' \
               AND cn.node_key IS NOT NULL \
               AND gn.lifecycle_state = 'ACTIVE';",
            &[&ws_id_str],
        )
        .await?;

    for row in key_collision_rows {
        let node_key: String = row.get("node_key");
        let conflicting_node_id: Uuid = row.get("conflicting_id");
        conflicts.push(MergeConflict::KeyCollision {
            node_key,
            conflicting_node_id,
        });
    }

    // 5. Inspect external parent nodes and apply auto-reparenting
    for edge in &candidate_edges {
        if candidate_node_ids.contains(&edge.to_node_id) {
            continue;
        }

        let parent_row_opt = tx
            .query_opt(
                "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                        governance_policy, created_by, job_id, doc_path, doc_hash, \
                        byte_start, byte_end, attributes \
                 FROM graph_nodes WHERE id = $1 FOR UPDATE;",
                &[&edge.to_node_id],
            )
            .await?;

        match parent_row_opt {
            None => {
                conflicts.push(MergeConflict::ParentSuperseded {
                    task_id: edge.from_node_id,
                    old_parent_id: edge.to_node_id,
                    reason: format!("Parent node '{}' not found", edge.to_node_id),
                });
            }
            Some(p_row) => {
                let parent = GraphNode::from_row(&p_row);
                if parent.lifecycle_state != "ACTIVE" {
                    if auto_reparent {
                        let replacement_opt = find_active_parent_replacement(
                            &tx,
                            parent.id,
                            parent.node_key.as_deref(),
                        )
                        .await?;

                        if let Some(rep) = replacement_opt {
                            let rep_policy = rep.governance_policy.parse::<GovernancePolicy>().ok();
                            if rep_policy == Some(GovernancePolicy::AutonomousElaboration) {
                                tx.execute(
                                    "UPDATE graph_edges SET to_node_id = $1 WHERE edge_id = $2;",
                                    &[&rep.id, &edge.edge_id],
                                )
                                .await?;

                                let rep_id_str = rep.id.to_string();
                                tx.execute(
                                    "UPDATE graph_nodes \
                                     SET attributes = jsonb_set(attributes, '{parent_node_id}', to_jsonb($1::text)) \
                                     WHERE id = $2;",
                                    &[&rep_id_str, &edge.from_node_id],
                                )
                                .await?;

                                reparented_tasks += 1;
                            } else {
                                conflicts.push(MergeConflict::ParentSuperseded {
                                    task_id: edge.from_node_id,
                                    old_parent_id: parent.id,
                                    reason: format!(
                                        "Replacement '{}' has policy '{}', expected 'AUTONOMOUS_ELABORATION'",
                                        rep.id, rep.governance_policy
                                    ),
                                });
                            }
                        } else {
                            conflicts.push(MergeConflict::ParentSuperseded {
                                task_id: edge.from_node_id,
                                old_parent_id: parent.id,
                                reason: format!(
                                    "Parent node '{}' is in state '{}' with no active replacement in lineage",
                                    parent.id, parent.lifecycle_state
                                ),
                            });
                        }
                    } else {
                        conflicts.push(MergeConflict::ParentSuperseded {
                            task_id: edge.from_node_id,
                            old_parent_id: parent.id,
                            reason: format!(
                                "Parent node '{}' is in state '{}'",
                                parent.id, parent.lifecycle_state
                            ),
                        });
                    }
                }
            }
        }
    }

    // 6. Check cycle detection with reparented edges
    let cycle_conflicts = detect_cycle_conflicts(&tx, workspace_id).await?;
    conflicts.extend(cycle_conflicts);

    // 7. If unresolved conflicts remain, abort rebase
    if !conflicts.is_empty() {
        return Err(MutationError::MergeConflict(conflicts));
    }

    // 8. Capture current max event_seq from audit_ledger
    let row = tx
        .query_one("SELECT COALESCE(MAX(event_seq), 0) FROM audit_ledger;", &[])
        .await?;
    let new_base_event_seq: i64 = row.get(0);

    // 9. Update workspace base_event_seq
    tx.execute(
        "UPDATE workspaces SET base_event_seq = $1 WHERE id = $2;",
        &[&new_base_event_seq, &workspace_id],
    )
    .await?;

    // 10. Record audit event
    let batch_id = Uuid::new_v4();
    let delta = serde_json::json!({
        "old_base_event_seq": ws.base_event_seq,
        "new_base_event_seq": new_base_event_seq,
        "reparented_tasks": reparented_tasks,
        "auto_reparent": auto_reparent,
    });
    let snapshot = serde_json::json!({
        "workspace_id": workspace_id,
        "base_event_seq": new_base_event_seq,
    });

    insert_audit_event(
        &tx,
        batch_id,
        "WORKSPACE_REBASED",
        workspace_id,
        "WORKSPACE",
        &actor.agent_id,
        &actor.actor_type,
        &actor.token_fingerprint(),
        delta,
        snapshot,
    )
    .await?;

    tx.commit().await?;

    Ok(RebaseResult {
        status: "REBASED".to_string(),
        workspace_id,
        old_base_event_seq: ws.base_event_seq,
        new_base_event_seq,
        reparented_tasks,
    })
}

/// Alias for `promote_workspace` conforming to the workspace merge interface naming convention.
pub async fn merge_workspace(
    client: &mut deadpool_postgres::Client,
    workspace_id: Uuid,
    auto_reparent: bool,
    actor: &AuthenticatedAgent,
) -> Result<PromotionResult, MutationError> {
    promote_workspace(client, workspace_id, auto_reparent, actor).await
}

/// Variant of `merge_workspace` operating on a standard `tokio_postgres::Client`.
pub async fn merge_workspace_client(
    client: &mut Client,
    workspace_id: Uuid,
    auto_reparent: bool,
    actor: &AuthenticatedAgent,
) -> Result<PromotionResult, MutationError> {
    promote_workspace_client(client, workspace_id, auto_reparent, actor).await
}
