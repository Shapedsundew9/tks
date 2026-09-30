//! Topological context envelope assembly and Invariant INV-1 ancestor validation CTEs.

use serde::{Deserialize, Serialize};
use tokio_postgres::Client;
use uuid::Uuid;

use crate::storage::{GraphNode, StorageError};

/// Graph-bounded topological context envelope delivered to external agents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TopologicalEnvelope {
    pub target_node: GraphNode,
    pub ancestor_requirements: Vec<GraphNode>,
    pub sibling_constraints: Vec<GraphNode>,
    pub total_nodes: usize,
    pub staleness_warning: bool,
}

impl TopologicalEnvelope {
    /// Formats the topological context envelope into structured Markdown suitable
    /// for injection into an external agent prompt.
    #[must_use]
    pub fn format_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("=== TOPOLOGICAL CONTEXT ENVELOPE ===\n\n");

        if self.staleness_warning {
            out.push_str("> [!WARNING]\n");
            out.push_str("> This node is in NEEDS_REVERIFICATION state. Upstream requirements have been modified or reverted.\n\n");
        }

        let key = self.target_node.node_key.as_deref().unwrap_or("UNTITLED");
        let title = self.target_node.title.as_deref().unwrap_or("");
        out.push_str(&format!("#### Target Execution Node: [{key}] {title}\n"));
        out.push_str(&format!(
            "- Type: {} | Policy: {} | State: {}\n",
            self.target_node.node_type,
            self.target_node.governance_policy,
            self.target_node.lifecycle_state
        ));
        if let Some(content) = &self.target_node.content {
            out.push_str(&format!("- Intent: {content}\n\n"));
        } else {
            out.push('\n');
        }

        if !self.ancestor_requirements.is_empty() {
            out.push_str("#### Upstream Ancestor Requirements & Specifications:\n");
            for anc in &self.ancestor_requirements {
                let akey = anc.node_key.as_deref().unwrap_or("UNTITLED");
                let atitle = anc.title.as_deref().unwrap_or("");
                let acontent = anc.content.as_deref().unwrap_or("");
                out.push_str(&format!(
                    "- **[{akey}] {atitle}** ({})\n  {acontent}\n",
                    anc.node_type
                ));
            }
            out.push('\n');
        }

        if !self.sibling_constraints.is_empty() {
            out.push_str(
                "#### Bound Architectural Invariants & Non-Local Constraints (CONSTRAINED_BY):\n",
            );
            for con in &self.sibling_constraints {
                let ckey = con.node_key.as_deref().unwrap_or("UNTITLED");
                let ctitle = con.title.as_deref().unwrap_or("");
                let ccontent = con.content.as_deref().unwrap_or("");
                out.push_str(&format!(
                    "- **[{ckey}] {ctitle}** [Severity: BLOCKING]\n  {ccontent}\n"
                ));
            }
            out.push('\n');
        }

        out.push_str(&format!(
            "=== END CONTEXT ENVELOPE (Total Bound Nodes: {}) ===\n",
            self.total_nodes
        ));
        out
    }
}

/// Assembles a graph-bounded topological context envelope for `target_id`.
///
/// Bounded upward breadth-first traversal across `DERIVED_FROM` and `FULFILLS` edges
/// up to `depth` hops (clamped $\le 3$), targeted `CONSTRAINED_BY` constraint harvesting,
/// sibling invariant discovery, and quota enforcement (DEC-0.8, §6.1).
///
/// # Errors
///
/// Returns `StorageError::NotFound` if `target_id` is missing or invisible to caller.
/// Returns `StorageError::Database` on query execution error.
pub async fn assemble_topological_envelope(
    client: &Client,
    target_id: Uuid,
    depth: u32,
    include_drafts_for: Option<&str>,
) -> Result<TopologicalEnvelope, StorageError> {
    // 1. Fetch target node
    let target_row = client
        .query_opt(
            "SELECT id, node_key, node_type, title, content, lifecycle_state, \
             governance_policy, created_by, job_id, doc_path, doc_hash, \
             byte_start, byte_end, attributes \
             FROM graph_nodes \
             WHERE id = $1;",
            &[&target_id],
        )
        .await?;

    let target_row = target_row
        .ok_or_else(|| StorageError::NotFound(format!("Target node '{target_id}' not found")))?;

    let target_node = GraphNode::from_row(&target_row);

    // Verify draft visibility
    if target_node.lifecycle_state == "DRAFT" {
        let is_visible = include_drafts_for
            .map(|author| author == target_node.created_by)
            .unwrap_or(false);
        if !is_visible {
            return Err(StorageError::NotFound(format!(
                "Target node '{target_id}' is a draft not accessible to caller"
            )));
        }
    }

    let staleness_warning = target_node.lifecycle_state == "NEEDS_REVERIFICATION";
    let effective_depth = depth.clamp(1, 3) as i32;

    // 2. Execute CTE query for ancestors and constraints
    let author_param = include_drafts_for.map(|s| s.to_string());

    let cte_query = "
        WITH RECURSIVE
        ancestors AS (
            SELECT
                e.to_node_id AS node_id,
                1 AS depth,
                ARRAY[e.from_node_id, e.to_node_id] AS visited
            FROM graph_edges e
            JOIN graph_nodes parent ON parent.id = e.to_node_id
            WHERE e.from_node_id = $1
              AND e.edge_type IN ('DERIVED_FROM', 'FULFILLS')
              AND (
                  e.lifecycle_state = 'ACTIVE'
                  OR (e.lifecycle_state = 'DRAFT' AND ($2::text IS NOT NULL AND e.created_by = $2))
              )
              AND (
                  parent.lifecycle_state = 'ACTIVE'
                  OR parent.lifecycle_state = 'NEEDS_REVERIFICATION'
                  OR (parent.lifecycle_state = 'DRAFT' AND ($2::text IS NOT NULL AND parent.created_by = $2))
              )
            UNION ALL
            SELECT
                e.to_node_id AS node_id,
                a.depth + 1,
                a.visited || e.to_node_id
            FROM ancestors a
            JOIN graph_edges e ON e.from_node_id = a.node_id
            JOIN graph_nodes parent ON parent.id = e.to_node_id
            WHERE a.depth < $3
              AND NOT (e.to_node_id = ANY(a.visited))
              AND e.edge_type IN ('DERIVED_FROM', 'FULFILLS')
              AND (
                  e.lifecycle_state = 'ACTIVE'
                  OR (e.lifecycle_state = 'DRAFT' AND ($2::text IS NOT NULL AND e.created_by = $2))
              )
              AND (
                  parent.lifecycle_state = 'ACTIVE'
                  OR parent.lifecycle_state = 'NEEDS_REVERIFICATION'
                  OR (parent.lifecycle_state = 'DRAFT' AND ($2::text IS NOT NULL AND parent.created_by = $2))
              )
        ),
        distinct_ancestors AS (
            SELECT node_id, MIN(depth) AS depth
            FROM ancestors
            WHERE node_id != $1
            GROUP BY node_id
            ORDER BY depth
            LIMIT 25
        ),
        context_ids AS (
            SELECT $1::uuid AS id
            UNION
            SELECT node_id FROM distinct_ancestors
        ),
        direct_constraints AS (
            SELECT e.to_node_id AS node_id
            FROM context_ids c
            JOIN graph_edges e ON e.from_node_id = c.id
            WHERE e.edge_type = 'CONSTRAINED_BY'
              AND (
                  e.lifecycle_state = 'ACTIVE'
                  OR (e.lifecycle_state = 'DRAFT' AND ($2::text IS NOT NULL AND e.created_by = $2))
              )
            UNION
            SELECT e.from_node_id AS node_id
            FROM context_ids c
            JOIN graph_edges e ON e.to_node_id = c.id
            WHERE e.edge_type = 'CONSTRAINED_BY'
              AND (
                  e.lifecycle_state = 'ACTIVE'
                  OR (e.lifecycle_state = 'DRAFT' AND ($2::text IS NOT NULL AND e.created_by = $2))
              )
        ),
        immediate_parents AS (
            SELECT e.to_node_id AS parent_id
            FROM graph_edges e
            WHERE e.from_node_id = $1
              AND e.edge_type IN ('DERIVED_FROM', 'FULFILLS')
              AND (
                  e.lifecycle_state = 'ACTIVE'
                  OR (e.lifecycle_state = 'DRAFT' AND ($2::text IS NOT NULL AND e.created_by = $2))
              )
        ),
        sibling_invariants AS (
            SELECT e.from_node_id AS node_id
            FROM immediate_parents ip
            JOIN graph_edges e ON e.to_node_id = ip.parent_id
            JOIN graph_nodes n ON n.id = e.from_node_id
            WHERE e.from_node_id != $1
              AND e.edge_type IN ('DERIVED_FROM', 'CONSTRAINED_BY')
              AND (n.node_key LIKE 'INV-%' OR n.node_key LIKE 'C-%' OR n.node_type = 'REQUIREMENT')
              AND (
                  e.lifecycle_state = 'ACTIVE'
                  OR (e.lifecycle_state = 'DRAFT' AND ($2::text IS NOT NULL AND e.created_by = $2))
              )
        ),
        all_constraints AS (
            SELECT node_id FROM direct_constraints
            UNION
            SELECT node_id FROM sibling_invariants
        ),
        filtered_constraints AS (
            SELECT ac.node_id
            FROM all_constraints ac
            JOIN graph_nodes n ON n.id = ac.node_id
            WHERE ac.node_id NOT IN (SELECT id FROM context_ids)
              AND (
                  n.lifecycle_state = 'ACTIVE'
                  OR n.lifecycle_state = 'NEEDS_REVERIFICATION'
                  OR (n.lifecycle_state = 'DRAFT' AND ($2::text IS NOT NULL AND n.created_by = $2))
              )
            LIMIT 15
        )
        SELECT 'ANCESTOR' AS category, da.depth, n.*
        FROM distinct_ancestors da
        JOIN graph_nodes n ON n.id = da.node_id
        UNION ALL
        SELECT 'CONSTRAINT' AS category, 99 AS depth, n.*
        FROM filtered_constraints fc
        JOIN graph_nodes n ON n.id = fc.node_id
        ORDER BY depth, node_key;
    ";

    let rows = client
        .query(cte_query, &[&target_id, &author_param, &effective_depth])
        .await?;

    let mut ancestor_requirements = Vec::new();
    let mut sibling_constraints = Vec::new();

    for row in rows {
        let category: String = row.get("category");
        let node = GraphNode::from_row(&row);
        if category == "ANCESTOR" {
            ancestor_requirements.push(node);
        } else {
            sibling_constraints.push(node);
        }
    }

    let total_nodes = 1 + ancestor_requirements.len() + sibling_constraints.len();

    Ok(TopologicalEnvelope {
        target_node,
        ancestor_requirements,
        sibling_constraints,
        total_nodes,
        staleness_warning,
    })
}

/// Validates Invariant INV-1 for candidate nodes: every non-requirement node
/// must maintain a directed upward edge path terminating at an authorized requirement node.
///
/// Evaluates child paths against the union of currently `ACTIVE` requirement nodes and
/// candidate requirement nodes included in the current promotion batch (`node_ids`),
/// traversing `FULFILLS`, `CONSTRAINED_BY`, and `DERIVED_FROM` edges upward.
///
/// # Errors
///
/// Returns `StorageError::Database` if query execution fails.
pub async fn validate_ancestor_path(
    client: &mut Client,
    node_ids: &[Uuid],
    allow_draft_parents: bool,
) -> Result<bool, StorageError> {
    if node_ids.is_empty() {
        return Ok(true);
    }

    let cte_query = "
        WITH RECURSIVE candidate_nodes AS (
            SELECT id, node_type, lifecycle_state
            FROM graph_nodes
            WHERE id = ANY($1)
        ),
        non_requirements AS (
            SELECT id FROM candidate_nodes
            WHERE node_type NOT IN ('REQUIREMENT')
        ),
        path_walk AS (
            SELECT
                nr.id AS origin_id,
                e.to_node_id AS current_id,
                ARRAY[nr.id, e.to_node_id] AS visited,
                1 AS depth
            FROM non_requirements nr
            JOIN graph_edges e ON e.from_node_id = nr.id
            WHERE e.edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM')
              AND (
                  e.lifecycle_state = 'ACTIVE'
                  OR $2 = true
                  OR (e.from_node_id = ANY($1) AND e.to_node_id = ANY($1))
              )
            UNION ALL
            SELECT
                pw.origin_id,
                e.to_node_id AS current_id,
                pw.visited || e.to_node_id,
                pw.depth + 1
            FROM path_walk pw
            JOIN graph_nodes curr ON curr.id = pw.current_id
            JOIN graph_edges e ON e.from_node_id = pw.current_id
            WHERE curr.node_type NOT IN ('REQUIREMENT')
              AND NOT (e.to_node_id = ANY(pw.visited))
              AND pw.depth < 20
              AND e.edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM')
              AND (
                  e.lifecycle_state = 'ACTIVE'
                  OR $2 = true
                  OR (e.from_node_id = ANY($1) AND e.to_node_id = ANY($1))
              )
              AND (
                  curr.lifecycle_state = 'ACTIVE'
                  OR curr.id = ANY($1)
                  OR ($2 = true AND curr.lifecycle_state = 'DRAFT')
              )
        ),
        valid_origins AS (
            SELECT DISTINCT pw.origin_id
            FROM path_walk pw
            JOIN graph_nodes term ON term.id = pw.current_id
            WHERE (
                $2 = true
                OR (
                    term.node_type = 'REQUIREMENT'
                    AND (term.lifecycle_state = 'ACTIVE' OR term.id = ANY($1))
                )
            )
        )
        SELECT 
            (SELECT COUNT(*) FROM candidate_nodes) AS found_count,
            (SELECT COUNT(*) FROM non_requirements WHERE id NOT IN (SELECT origin_id FROM valid_origins)) AS invalid_count;
    ";

    let row = client
        .query_one(cte_query, &[&node_ids, &allow_draft_parents])
        .await?;
    let found_count: i64 = row.get("found_count");
    let invalid_count: i64 = row.get("invalid_count");

    // All node_ids must exist in the database and all non-requirements must reach an authorized requirement root
    Ok(found_count as usize == node_ids.len() && invalid_count == 0)
}
