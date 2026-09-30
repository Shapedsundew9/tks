//! Read-only MCP tool handlers (WP-1.5, TB-1, TB-6, TB-7).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::gateway::AppState;
use crate::gateway::auth::AuthenticatedAgent;
use crate::storage::envelope::assemble_context_envelope;
use crate::storage::repo::lookup_node_polymorphic;
use crate::storage::search::query_active_requirements;
use crate::storage::{SearchResultNode, TopologicalEnvelope};

/// Definition of an MCP tool for `tools/list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Returns definitions for all read-only MCP tools supported in Phase 1.
#[must_use]
pub fn list_tools() -> Vec<McpToolDefinition> {
    vec![
        McpToolDefinition {
            name: "get_context_envelope".to_string(),
            description: "Retrieves a graph-bounded topological context envelope for a target requirement or task node, capturing upward ancestor requirements, sibling constraints, and semantic vector neighbors within quota (depth <= 3, budget <= 40).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "target_node_id": {
                        "type": "string",
                        "description": "Polymorphic node identifier: either 128-bit UUID or canonical key (e.g. 'REQ-CORE-001', 'INV-1')."
                    },
                    "node_id": {
                        "type": "string",
                        "description": "Alias for target_node_id."
                    },
                    "depth": {
                        "type": "integer",
                        "description": "Upward traversal depth (1 to 3 hops, default 2)."
                    },
                    "include_drafts": {
                        "type": "boolean",
                        "description": "Whether to include speculative candidate drafts authored by the calling agent (default false)."
                    }
                },
                "required": []
            }),
        },
        McpToolDefinition {
            name: "query_requirements".to_string(),
            description: "Executes native PostgreSQL full-text search against active requirements with canonical key sanitization preventing hyphen negation errors, returning matching records in <5ms.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query text or canonical identifier prefix (e.g. 'REQ-CORE-001', 'authentication')."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of search results to return (default 10, max 50)."
                    }
                },
                "required": ["query"]
            }),
        },
        McpToolDefinition {
            name: "get_document_span".to_string(),
            description: "Retrieves verbatim source specification text from the bare Git repository ODB concurrently without locking using embedded node span coordinates.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "Polymorphic node identifier (UUID or canonical key)."
                    }
                },
                "required": ["node_id"]
            }),
        },
    ]
}

/// Executes the `get_context_envelope` tool.
pub async fn handle_get_context_envelope(
    state: &AppState,
    args: Value,
    caller: Option<&AuthenticatedAgent>,
) -> Result<TopologicalEnvelope, String> {
    let target_str = args
        .get("target_node_id")
        .or_else(|| args.get("node_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument 'target_node_id' or 'node_id'".to_string())?;

    let depth = args
        .get("depth")
        .and_then(|v| v.as_u64())
        .map(|d| d as u32)
        .unwrap_or(2)
        .clamp(1, 3);

    let include_drafts = args
        .get("include_drafts")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| format!("Database pool error: {e}"))?;

    // Polymorphic resolution (UUID or canonical node_key; TB-7)
    let target_node = lookup_node_polymorphic(&client, target_str)
        .await
        .map_err(|e| format!("Lookup error: {e}"))?
        .ok_or_else(|| format!("Target node '{target_str}' not found"))?;

    let caller_author = if include_drafts {
        caller.map(|c| c.agent_id.as_str())
    } else {
        None
    };

    // Draft isolation check
    if target_node.lifecycle_state == "DRAFT" {
        let is_visible = caller_author
            .map(|author| author == target_node.created_by)
            .unwrap_or(false);
        if !is_visible {
            return Err(format!(
                "Target node '{target_str}' is an unapproved draft not accessible to caller"
            ));
        }
    }

    // Assemble complete context envelope with vector neighbors (TB-6, §6.1)
    let envelope = assemble_context_envelope(&client, target_node.id, depth, caller_author)
        .await
        .map_err(|e| format!("Failed to assemble context envelope: {e}"))?;

    Ok(envelope)
}

/// Executes the `query_requirements` tool.
pub async fn handle_query_requirements(
    state: &AppState,
    args: Value,
) -> Result<Vec<SearchResultNode>, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument 'query'".to_string())?;

    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|l| (l as u32).clamp(1, 50))
        .unwrap_or(10);

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| format!("Database pool error: {e}"))?;

    let results = query_active_requirements(&client, query, limit)
        .await
        .map_err(|e| format!("Full-text search error: {e}"))?;

    Ok(results)
}

/// Executes the `get_document_span` tool.
pub async fn handle_get_document_span(state: &AppState, args: Value) -> Result<String, String> {
    let node_id_str = args
        .get("node_id")
        .or_else(|| args.get("target_node_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument 'node_id'".to_string())?;

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| format!("Database pool error: {e}"))?;

    let node = lookup_node_polymorphic(&client, node_id_str)
        .await
        .map_err(|e| format!("Lookup error: {e}"))?
        .ok_or_else(|| format!("Node '{node_id_str}' not found"))?;

    let doc_hash = node
        .doc_hash
        .ok_or_else(|| format!("Node '{node_id_str}' has no associated document hash span"))?;
    let byte_start = node
        .byte_start
        .ok_or_else(|| format!("Node '{node_id_str}' has no start byte offset"))?;
    let byte_end = node
        .byte_end
        .ok_or_else(|| format!("Node '{node_id_str}' has no end byte offset"))?;

    if byte_start < 0 || byte_end < byte_start {
        return Err(format!(
            "Invalid byte offsets [{byte_start}..{byte_end}] on node '{node_id_str}'"
        ));
    }

    // Concurrent lock-free ODB read via spawn_blocking (TB-1, D-72)
    let span_text = state
        .git_read
        .read_blob_span(&doc_hash, byte_start as usize, byte_end as usize)
        .await
        .map_err(|e| format!("Failed to read git blob span: {e}"))?;

    Ok(span_text)
}
