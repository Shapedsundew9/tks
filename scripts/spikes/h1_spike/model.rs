//! In-memory property graph and topological context envelope assembly (WP-0.4 / Spike 0).
//!
//! Provides lightweight in-memory representations of requirements, tasks, invariants,
//! and structural edges to empirically test Hypothesis H-1 (CAL-H1).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// In-memory representation of a substrate graph node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InMemoryNode {
    pub id: String,
    pub node_key: String,
    pub node_type: String,
    pub title: String,
    pub content: String,
    pub subsystem: String,
    pub lifecycle_state: String,
    pub governance_policy: String,
    #[serde(default)]
    pub attributes: serde_json::Value,
}

/// In-memory representation of a directed property graph edge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InMemoryEdge {
    pub edge_id: String,
    pub from_node_id: String,
    pub to_node_id: String,
    pub edge_type: String,
    pub lifecycle_state: String,
}

/// In-memory graph structure storing nodes and directional edges.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InMemoryGraph {
    pub nodes: HashMap<String, InMemoryNode>,
    pub edges: Vec<InMemoryEdge>,
}

/// Graph-bounded topological context envelope delivered to agents under Condition B.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextEnvelope {
    pub target_node: InMemoryNode,
    pub ancestor_requirements: Vec<InMemoryNode>,
    pub sibling_constraints: Vec<InMemoryNode>,
    pub total_nodes: usize,
}

impl InMemoryGraph {
    /// Creates an empty in-memory graph.
    #[allow(dead_code)]
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: Vec::new(),
        }
    }

    /// Deserializes a graph from raw JSON format containing `"nodes"` and `"edges"` arrays.
    ///
    /// # Errors
    ///
    /// Returns a deserialization error if JSON schema is invalid.
    pub fn from_json(json_str: &str) -> Result<Self, serde_json::Error> {
        #[derive(Deserialize)]
        struct RawGraph {
            nodes: Vec<InMemoryNode>,
            edges: Vec<InMemoryEdge>,
        }

        let raw: RawGraph = serde_json::from_str(json_str)?;
        let mut nodes = HashMap::with_capacity(raw.nodes.len());
        for node in raw.nodes {
            nodes.insert(node.id.clone(), node);
        }

        Ok(Self {
            nodes,
            edges: raw.edges,
        })
    }

    /// Adds a node to the graph.
    #[allow(dead_code)]
    pub fn add_node(&mut self, node: InMemoryNode) {
        self.nodes.insert(node.id.clone(), node);
    }

    /// Adds an edge to the graph.
    #[allow(dead_code)]
    pub fn add_edge(&mut self, edge: InMemoryEdge) {
        self.edges.push(edge);
    }

    /// Finds a node by exact ID or canonical `node_key`.
    #[must_use]
    pub fn get_node(&self, id_or_key: &str) -> Option<&InMemoryNode> {
        if let Some(node) = self.nodes.get(id_or_key) {
            return Some(node);
        }
        self.nodes.values().find(|n| n.node_key == id_or_key)
    }

    /// Returns all outgoing edges from a specific node ID.
    #[must_use]
    pub fn get_outgoing_edges(&self, node_id: &str) -> Vec<&InMemoryEdge> {
        self.edges
            .iter()
            .filter(|e| e.from_node_id == node_id)
            .collect()
    }

    /// Returns all incoming edges to a specific node ID.
    #[must_use]
    pub fn get_incoming_edges(&self, node_id: &str) -> Vec<&InMemoryEdge> {
        self.edges
            .iter()
            .filter(|e| e.to_node_id == node_id)
            .collect()
    }
}

impl ContextEnvelope {
    /// Formats the context envelope into a clean, structured Markdown block
    /// suitable for injection into an agent context window.
    #[must_use]
    pub fn format_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("=== TOPOLOGICAL CONTEXT ENVELOPE (TKS HYPOTHESIS H-1) ===\n\n");

        out.push_str(&format!(
            "#### Target Execution Node: [{}] {}\n",
            self.target_node.node_key, self.target_node.title
        ));
        out.push_str(&format!(
            "- Subsystem: {} | Policy: {} | State: {}\n",
            self.target_node.subsystem,
            self.target_node.governance_policy,
            self.target_node.lifecycle_state
        ));
        out.push_str(&format!("- Intent: {}\n\n", self.target_node.content));

        if !self.ancestor_requirements.is_empty() {
            out.push_str("#### Upstream Ancestor Requirements & Specifications:\n");
            for anc in &self.ancestor_requirements {
                out.push_str(&format!(
                    "- **[{}] {}** ({})\n  {}\n",
                    anc.node_key, anc.title, anc.subsystem, anc.content
                ));
            }
            out.push('\n');
        }

        if !self.sibling_constraints.is_empty() {
            out.push_str(
                "#### Bound Architectural Invariants & Non-Local Constraints (CONSTRAINED_BY):\n",
            );
            for con in &self.sibling_constraints {
                out.push_str(&format!(
                    "- **[{}] {}** [Severity: BLOCKING]\n  {}\n",
                    con.node_key, con.title, con.content
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

/// Assembles a graph-bounded topological context envelope for a target node up to `depth`.
///
/// Traversal rule:
/// 1. Locates `target_node` by node ID or canonical `node_key`.
/// 2. Extracts ancestor requirements up to `depth` hops following upward structural edges
///    (`DERIVED_FROM`, `FULFILLS`).
/// 3. Extracts sibling and ancestor `CONSTRAINED_BY` rules modeling non-local invariant contracts.
/// 4. Deduplicates all returned nodes.
///
/// # Panics
///
/// Panics if `target_node_id` is not present in `graph`.
#[must_use]
pub fn assemble_in_memory_envelope(
    graph: &InMemoryGraph,
    target_node_id: &str,
    depth: usize,
) -> ContextEnvelope {
    let target_node = graph
        .get_node(target_node_id)
        .unwrap_or_else(|| panic!("Target node '{target_node_id}' not found in in-memory graph"))
        .clone();

    let mut visited_ancestor_ids = HashSet::new();
    let mut ancestor_requirements = Vec::new();

    let mut current_frontier = vec![target_node.id.clone()];

    for _d in 0..depth {
        let mut next_frontier = Vec::new();
        for node_id in &current_frontier {
            // Find upward edges: from_node_id -> to_node_id (e.g. child DERIVED_FROM parent)
            for edge in graph.get_outgoing_edges(node_id) {
                if (edge.edge_type == "DERIVED_FROM" || edge.edge_type == "FULFILLS")
                    && !visited_ancestor_ids.contains(&edge.to_node_id)
                    && edge.to_node_id != target_node.id
                    && let Some(parent_node) = graph.nodes.get(&edge.to_node_id)
                {
                    visited_ancestor_ids.insert(edge.to_node_id.clone());
                    ancestor_requirements.push(parent_node.clone());
                    next_frontier.push(edge.to_node_id.clone());
                }
            }
        }
        if next_frontier.is_empty() {
            break;
        }
        current_frontier = next_frontier;
    }

    // Extract sibling constraints:
    // Any node connected via CONSTRAINED_BY to target_node or any ancestor
    let mut bound_context_ids = HashSet::new();
    bound_context_ids.insert(target_node.id.clone());
    for anc in &ancestor_requirements {
        bound_context_ids.insert(anc.id.clone());
    }

    let mut visited_constraint_ids = HashSet::new();
    let mut sibling_constraints = Vec::new();

    for ctx_id in &bound_context_ids {
        // Outgoing CONSTRAINED_BY edges (ctx_node -CONSTRAINED_BY-> rule_node)
        for edge in graph.get_outgoing_edges(ctx_id) {
            if edge.edge_type == "CONSTRAINED_BY"
                && !bound_context_ids.contains(&edge.to_node_id)
                && !visited_constraint_ids.contains(&edge.to_node_id)
                && let Some(rule_node) = graph.nodes.get(&edge.to_node_id)
            {
                visited_constraint_ids.insert(edge.to_node_id.clone());
                sibling_constraints.push(rule_node.clone());
            }
        }

        // Incoming CONSTRAINED_BY edges (in case direction is reversed in some schemas)
        for edge in graph.get_incoming_edges(ctx_id) {
            if edge.edge_type == "CONSTRAINED_BY"
                && !bound_context_ids.contains(&edge.from_node_id)
                && !visited_constraint_ids.contains(&edge.from_node_id)
                && let Some(rule_node) = graph.nodes.get(&edge.from_node_id)
            {
                visited_constraint_ids.insert(edge.from_node_id.clone());
                sibling_constraints.push(rule_node.clone());
            }
        }
    }

    // Also collect sibling invariants under the same immediate parent
    for edge in graph.get_outgoing_edges(&target_node.id) {
        if edge.edge_type == "DERIVED_FROM" {
            let parent_id = &edge.to_node_id;
            for sibling_edge in graph.get_incoming_edges(parent_id) {
                if sibling_edge.from_node_id != target_node.id
                    && (sibling_edge.edge_type == "DERIVED_FROM"
                        || sibling_edge.edge_type == "CONSTRAINED_BY")
                    && !bound_context_ids.contains(&sibling_edge.from_node_id)
                    && !visited_constraint_ids.contains(&sibling_edge.from_node_id)
                    && let Some(sib_node) = graph.nodes.get(&sibling_edge.from_node_id)
                    && (sib_node.node_type == "INVARIANT" || sib_node.node_type == "POLICY")
                {
                    visited_constraint_ids.insert(sibling_edge.from_node_id.clone());
                    sibling_constraints.push(sib_node.clone());
                }
            }
        }
    }

    let total_nodes = 1 + ancestor_requirements.len() + sibling_constraints.len();

    ContextEnvelope {
        target_node,
        ancestor_requirements,
        sibling_constraints,
        total_nodes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_graph() -> InMemoryGraph {
        let mut graph = InMemoryGraph::new();

        let epic = InMemoryNode {
            id: "epic-1".into(),
            node_key: "EPIC-AUTH".into(),
            node_type: "EPIC".into(),
            title: "Auth Epic".into(),
            content: "Auth system".into(),
            subsystem: "auth".into(),
            lifecycle_state: "ACTIVE".into(),
            governance_policy: "HUMAN_REVIEW_REQUIRED".into(),
            attributes: serde_json::Value::Null,
        };

        let req = InMemoryNode {
            id: "req-1".into(),
            node_key: "REQ-AUTH-01".into(),
            node_type: "REQUIREMENT".into(),
            title: "Token Parsing".into(),
            content: "Must parse bearer".into(),
            subsystem: "auth".into(),
            lifecycle_state: "ACTIVE".into(),
            governance_policy: "HUMAN_REVIEW_REQUIRED".into(),
            attributes: serde_json::Value::Null,
        };

        let task = InMemoryNode {
            id: "task-1".into(),
            node_key: "TASK-AUTH-01".into(),
            node_type: "TASK".into(),
            title: "Implement Token Parser".into(),
            content: "Implement parser".into(),
            subsystem: "auth".into(),
            lifecycle_state: "ACTIVE".into(),
            governance_policy: "AUTONOMOUS_ELABORATION".into(),
            attributes: serde_json::Value::Null,
        };

        let inv = InMemoryNode {
            id: "inv-1".into(),
            node_key: "INV-SEC-08".into(),
            node_type: "INVARIANT".into(),
            title: "Zero Credential Leakage".into(),
            content: "Must redact tokens".into(),
            subsystem: "auth".into(),
            lifecycle_state: "ACTIVE".into(),
            governance_policy: "HUMAN_REVIEW_REQUIRED".into(),
            attributes: serde_json::Value::Null,
        };

        graph.add_node(epic);
        graph.add_node(req);
        graph.add_node(task);
        graph.add_node(inv);

        // task -> req (DERIVED_FROM)
        graph.add_edge(InMemoryEdge {
            edge_id: "e1".into(),
            from_node_id: "task-1".into(),
            to_node_id: "req-1".into(),
            edge_type: "DERIVED_FROM".into(),
            lifecycle_state: "ACTIVE".into(),
        });

        // req -> epic (DERIVED_FROM)
        graph.add_edge(InMemoryEdge {
            edge_id: "e2".into(),
            from_node_id: "req-1".into(),
            to_node_id: "epic-1".into(),
            edge_type: "DERIVED_FROM".into(),
            lifecycle_state: "ACTIVE".into(),
        });

        // task -> inv (CONSTRAINED_BY)
        graph.add_edge(InMemoryEdge {
            edge_id: "e3".into(),
            from_node_id: "task-1".into(),
            to_node_id: "inv-1".into(),
            edge_type: "CONSTRAINED_BY".into(),
            lifecycle_state: "ACTIVE".into(),
        });

        graph
    }

    #[test]
    fn test_assemble_envelope_depth_2() {
        let graph = create_test_graph();
        let envelope = assemble_in_memory_envelope(&graph, "task-1", 2);

        assert_eq!(envelope.target_node.node_key, "TASK-AUTH-01");
        assert_eq!(envelope.ancestor_requirements.len(), 2);
        assert_eq!(envelope.ancestor_requirements[0].node_key, "REQ-AUTH-01");
        assert_eq!(envelope.ancestor_requirements[1].node_key, "EPIC-AUTH");
        assert_eq!(envelope.sibling_constraints.len(), 1);
        assert_eq!(envelope.sibling_constraints[0].node_key, "INV-SEC-08");
        assert_eq!(envelope.total_nodes, 4);

        let md = envelope.format_markdown();
        assert!(md.contains("TASK-AUTH-01"));
        assert!(md.contains("REQ-AUTH-01"));
        assert!(md.contains("INV-SEC-08"));
    }

    #[test]
    fn test_lookup_by_node_key() {
        let graph = create_test_graph();
        let envelope = assemble_in_memory_envelope(&graph, "TASK-AUTH-01", 1);
        assert_eq!(envelope.target_node.id, "task-1");
        assert_eq!(envelope.ancestor_requirements.len(), 1);
        assert_eq!(envelope.ancestor_requirements[0].node_key, "REQ-AUTH-01");
    }

    #[test]
    #[should_panic(expected = "Target node 'non-existent' not found")]
    fn test_missing_node_panics() {
        let graph = create_test_graph();
        let _ = assemble_in_memory_envelope(&graph, "non-existent", 2);
    }
}
