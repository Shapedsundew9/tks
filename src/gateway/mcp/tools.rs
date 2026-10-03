//! Model Context Protocol (MCP) tool handlers (WP-1.5, WP-2.4, TB-1, TB-6, TB-7).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::gateway::AppState;
use crate::gateway::auth::AuthenticatedAgent;
use crate::storage::envelope::assemble_context_envelope_workspace;
use crate::storage::governance::TaskStatus;
use crate::storage::mutation::{
    ElaboratedTaskResult, MutationError, TaskUpdateResult, elaborate_task, propose_normative_draft,
    update_task_status,
};
use crate::storage::repo::lookup_node_polymorphic;
use crate::storage::reverify::{ReverifyResult, reverify_node};
use crate::storage::rollback::{RevertExecutionResult, RevertFilter, revert_mutations};
use crate::storage::search::query_requirements_with_workspace;
use crate::storage::workspace::elaborate_in_workspace;
use crate::storage::{SearchResultNode, TopologicalEnvelope};

/// Definition of an MCP tool for `tools/list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Returns definitions for all MCP tools supported in Phase 1 and Phase 2 (8 tools total).
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
                    },
                    "workspace_id": {
                        "type": "string",
                        "description": "Optional workspace UUID scoping candidate draft visibility to an isolated scratchpad branch (INV-7)."
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
                    },
                    "workspace_id": {
                        "type": "string",
                        "description": "Optional workspace UUID scoping candidate search to an isolated scratchpad branch (INV-7)."
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
        McpToolDefinition {
            name: "propose_node_mutation".to_string(),
            description: "Submits a candidate mutation (subtask elaboration, normative requirement proposal, or status transition) under governance policy enforcement, polymorphic identifier resolution, and cycle prevention.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "target_node_id": {
                        "type": "string",
                        "description": "Polymorphic target node identifier (UUID or canonical key such as 'REQ-CORE-001')."
                    },
                    "node_id": {
                        "type": "string",
                        "description": "Alias for target_node_id."
                    },
                    "mutation_type": {
                        "type": "string",
                        "description": "Mutation type: 'TASK', 'SUBTASK', 'MODIFY_REQUIREMENT', 'NORM_SPEC', 'STATUS', etc."
                    },
                    "proposed_title": {
                        "type": "string",
                        "description": "Optional title for elaborated subtask or candidate draft."
                    },
                    "title": {
                        "type": "string",
                        "description": "Alias for proposed_title."
                    },
                    "proposed_content": {
                        "type": "string",
                        "description": "Optional content or description for elaborated subtask or candidate draft."
                    },
                    "content": {
                        "type": "string",
                        "description": "Alias for proposed_content."
                    },
                    "proposed_attributes": {
                        "type": "object",
                        "description": "Optional progressive JSONB attributes."
                    },
                    "attributes": {
                        "type": "object",
                        "description": "Alias for proposed_attributes."
                    },
                    "edge_type": {
                        "type": "string",
                        "description": "Optional upward structural edge type ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM')."
                    },
                    "workspace_id": {
                        "type": "string",
                        "description": "Optional workspace UUID scoping candidate mutations to an isolated scratchpad branch without global locking."
                    }
                },
                "required": ["target_node_id", "mutation_type"]
            }),
        },
        McpToolDefinition {
            name: "create_subtask".to_string(),
            description: "Autonomously elaborates a non-normative execution task (TASK) directly in ACTIVE state under an AUTONOMOUS_ELABORATION parent node, inheriting governance policy and establishing upward structural edges (INV-1, INV-5).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "parent_node_id": {
                        "type": "string",
                        "description": "Polymorphic parent node identifier (UUID or canonical key)."
                    },
                    "parent_id": {
                        "type": "string",
                        "description": "Alias for parent_node_id."
                    },
                    "title": {
                        "type": "string",
                        "description": "Title of the execution subtask."
                    },
                    "content": {
                        "type": "string",
                        "description": "Optional specification or description of the subtask."
                    },
                    "attributes": {
                        "type": "object",
                        "description": "Optional progressive JSONB attributes (e.g. node_key, edge_type)."
                    },
                    "workspace_id": {
                        "type": "string",
                        "description": "Optional workspace UUID to elaborate the task in an isolated branch scratchpad without taking the global advisory lock."
                    }
                },
                "required": ["parent_node_id", "title"]
            }),
        },
        McpToolDefinition {
            name: "update_node_status".to_string(),
            description: "Updates the execution status ('OPEN', 'IN_PROGRESS', 'BLOCKED', 'COMPLETED') of an active task in-place under native row-level lock, appending a discrete reversible audit ledger event (INV-2, D-30).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "Polymorphic node identifier of the task (UUID or canonical key)."
                    },
                    "task_id": {
                        "type": "string",
                        "description": "Alias for node_id."
                    },
                    "status": {
                        "type": "string",
                        "description": "Target execution status ('OPEN', 'IN_PROGRESS', 'BLOCKED', 'COMPLETED')."
                    },
                    "notes": {
                        "type": "string",
                        "description": "Optional operational transition notes."
                    }
                },
                "required": ["node_id", "status"]
            }),
        },
        McpToolDefinition {
            name: "revert_mutations".to_string(),
            description: "Administrative rollback utility supporting dry-run previews under READ COMMITTED, cross-agent confirmation safety aborts, and compensating inverse REVERT events under global advisory lock, cascading active child tasks to NEEDS_REVERIFICATION (INV-1, INV-2, D-73, D-80).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "batch_id": {
                        "type": "string",
                        "description": "Optional batch UUID correlating mutations to revert."
                    },
                    "agent_id": {
                        "type": "string",
                        "description": "Optional authoring agent identifier whose mutations should be reverted."
                    },
                    "since": {
                        "type": "string",
                        "description": "Optional ISO 8601 timestamp threshold reverting mutations committed since this time."
                    },
                    "event_seq_range": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "Optional sequence range [min_event_seq, max_event_seq] to revert."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "If true, simulates rollback under READ COMMITTED and returns preview without modifying state."
                    },
                    "force": {
                        "type": "boolean",
                        "description": "If true, overrides cross-agent safety confirmation abort."
                    }
                },
                "required": []
            }),
        },
        McpToolDefinition {
            name: "reverify_node".to_string(),
            description: "Explicit reverification interface validating that all direct upstream dependencies are in ACTIVE state (INV-1), restoring degraded nodes from NEEDS_REVERIFICATION back to ACTIVE, resetting staleness_score to 0.0, and appending a REVERIFIED audit event (D-76).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "node_id": {
                        "type": "string",
                        "description": "Polymorphic node identifier of the degraded node (UUID or canonical key)."
                    },
                    "target_node_id": {
                        "type": "string",
                        "description": "Alias for node_id."
                    },
                    "rationale": {
                        "type": "string",
                        "description": "Operational rationale explaining why the node has been reverified."
                    },
                    "updated_attributes": {
                        "type": "object",
                        "description": "Optional updated attributes (e.g. reparent_to / parent_id for re-anchoring)."
                    }
                },
                "required": ["node_id", "rationale"]
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

    let workspace_id = args
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|e| format!("Invalid workspace_id: {e}"))?;

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| format!("Database pool error: {e}"))?;

    // Validate workspace access if workspace_id provided (INV-7)
    if let Some(ws_id) = workspace_id {
        let ws_row = client
            .query_opt(
                "SELECT id, owner_agent, status FROM workspaces WHERE id = $1;",
                &[&ws_id],
            )
            .await
            .map_err(|e| format!("Database error: {e}"))?;

        let (ws_owner, ws_status): (String, String) = ws_row
            .map(|r| (r.get(1), r.get(2)))
            .ok_or_else(|| format!("Workspace '{ws_id}' not found"))?;

        if ws_status != "ACTIVE" {
            return Err(format!("Workspace '{ws_id}' is not ACTIVE"));
        }

        if let Some(c) = caller {
            if ws_owner != c.agent_id && c.actor_type != "HUMAN" && c.agent_id != "system" {
                return Err(format!(
                    "Caller '{}' is not authorized to access workspace owned by '{ws_owner}'",
                    c.agent_id
                ));
            }
        } else {
            return Err("Authentication required to access workspace context".to_string());
        }
    }

    // Polymorphic resolution (UUID or canonical node_key; TB-7)
    let target_node = lookup_node_polymorphic(&client, target_str)
        .await
        .map_err(|e| format!("Lookup error: {e}"))?
        .ok_or_else(|| format!("Target node '{target_str}' not found"))?;

    let caller_author = if include_drafts || workspace_id.is_some() {
        caller.map(|c| c.agent_id.as_str())
    } else {
        None
    };

    // Draft isolation check
    if target_node.lifecycle_state == "DRAFT" {
        let node_ws = target_node
            .attributes
            .get("workspace_id")
            .and_then(|v| v.as_str());

        if let Some(nws) = node_ws {
            let ws_matches = workspace_id.map(|w| w.to_string() == nws).unwrap_or(false);
            let author_matches = caller_author
                .map(|a| a == target_node.created_by)
                .unwrap_or(true);

            if !ws_matches || !author_matches {
                return Err(format!(
                    "Target node '{target_str}' is an isolated workspace draft not accessible to caller"
                ));
            }
        } else {
            let is_visible = caller_author
                .map(|author| author == target_node.created_by)
                .unwrap_or(false);
            if !is_visible {
                return Err(format!(
                    "Target node '{target_str}' is an unapproved draft not accessible to caller"
                ));
            }
        }
    }

    // Assemble complete context envelope with vector neighbors and optional workspace candidate overlay (TB-6, §6.1, WP-3.2)
    let envelope = assemble_context_envelope_workspace(
        &client,
        target_node.id,
        depth,
        caller_author,
        workspace_id,
    )
    .await
    .map_err(|e| format!("Failed to assemble context envelope: {e}"))?;

    Ok(envelope)
}

/// Executes the `query_requirements` tool with optional workspace candidate overlay.
pub async fn handle_query_requirements(
    state: &AppState,
    args: Value,
    caller: Option<&AuthenticatedAgent>,
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

    let workspace_id = args
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|e| format!("Invalid workspace_id: {e}"))?;

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| format!("Database pool error: {e}"))?;

    if let Some(ws_id) = workspace_id {
        // Verify workspace exists, is ACTIVE, and caller is authorized (INV-7)
        let ws_row = client
            .query_opt(
                "SELECT id, owner_agent, status FROM workspaces WHERE id = $1;",
                &[&ws_id],
            )
            .await
            .map_err(|e| format!("Database error: {e}"))?;

        let (ws_owner, ws_status): (String, String) = ws_row
            .map(|r| (r.get(1), r.get(2)))
            .ok_or_else(|| format!("Workspace '{ws_id}' not found"))?;

        if ws_status != "ACTIVE" {
            return Err(format!("Workspace '{ws_id}' is not ACTIVE"));
        }

        if let Some(c) = caller {
            if ws_owner != c.agent_id && c.actor_type != "HUMAN" && c.agent_id != "system" {
                return Err(format!(
                    "Caller '{}' is not authorized to access workspace owned by '{ws_owner}'",
                    c.agent_id
                ));
            }
        } else {
            return Err("Authentication required to access workspace requirements".to_string());
        }
    }

    let results = query_requirements_with_workspace(&client, query, limit, workspace_id)
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

/// Executes the `propose_node_mutation` MCP tool.
pub async fn handle_propose_node_mutation(
    state: &AppState,
    args: Value,
    caller: &AuthenticatedAgent,
) -> Result<Value, MutationError> {
    let target_node_id = args
        .get("target_node_id")
        .or_else(|| args.get("node_id"))
        .or_else(|| args.get("target_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            MutationError::Other("Missing required argument 'target_node_id'".to_string())
        })?;

    let mutation_type = args
        .get("mutation_type")
        .and_then(|v| v.as_str())
        .unwrap_or("TASK");

    let proposed_title = args
        .get("proposed_title")
        .or_else(|| args.get("title"))
        .and_then(|v| v.as_str());

    let proposed_content = args
        .get("proposed_content")
        .or_else(|| args.get("content"))
        .and_then(|v| v.as_str());

    let proposed_attributes = args
        .get("proposed_attributes")
        .or_else(|| args.get("attributes"))
        .cloned();

    let edge_type = args.get("edge_type").and_then(|v| v.as_str());

    let workspace_id = args
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|e| MutationError::Other(format!("Invalid workspace_id: {e}")))?;

    let mut client = state.pool.get().await.map_err(MutationError::Pool)?;

    let mut_type_upper = mutation_type.to_uppercase();

    if mut_type_upper.contains("TASK")
        || mut_type_upper.contains("SUBTASK")
        || mut_type_upper.contains("ELABORAT")
    {
        let title = proposed_title
            .map(|s| s.to_string())
            .or_else(|| {
                proposed_attributes
                    .as_ref()
                    .and_then(|a| a.get("title"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "Subtask".to_string());

        let mut attrs = proposed_attributes.unwrap_or_else(|| serde_json::json!({}));
        if !attrs.is_object() {
            attrs = serde_json::json!({});
        }
        if let Some(et) = edge_type {
            attrs["edge_type"] = serde_json::json!(et);
        }

        let res = if let Some(ws_id) = workspace_id {
            elaborate_in_workspace(
                &mut client,
                ws_id,
                target_node_id,
                &title,
                proposed_content,
                Some(attrs),
                caller,
            )
            .await?
        } else {
            elaborate_task(
                &mut client,
                target_node_id,
                &title,
                proposed_content,
                Some(attrs),
                caller,
            )
            .await?
        };

        Ok(serde_json::json!({
            "status": "COMMITTED",
            "node_id": res.task_id,
            "task_id": res.task_id,
            "batch_id": res.batch_id,
            "event_seq": res.event_seq,
            "lifecycle_state": res.lifecycle_state,
            "governance_policy": res.governance_policy,
            "node_key": res.node_key,
            "node": res.node
        }))
    } else if mut_type_upper.contains("STATUS") {
        let status_str = proposed_attributes
            .as_ref()
            .and_then(|a| a.get("execution_status").or_else(|| a.get("status")))
            .and_then(|v| v.as_str())
            .or(proposed_content)
            .unwrap_or("IN_PROGRESS");

        let task_status = status_str
            .parse::<TaskStatus>()
            .map_err(MutationError::GovernanceRejected)?;

        let notes = proposed_attributes
            .as_ref()
            .and_then(|a| a.get("notes"))
            .and_then(|v| v.as_str())
            .or(proposed_content);

        let res =
            update_task_status(&mut client, target_node_id, task_status, notes, caller).await?;

        Ok(serde_json::json!({
            "status": "COMMITTED",
            "node_id": res.task_id,
            "task_id": res.task_id,
            "batch_id": res.batch_id,
            "event_seq": res.event_seq,
            "execution_status": res.execution_status,
            "node": res.node
        }))
    } else {
        // Normative candidate draft proposal (Pathway 3)
        let content = proposed_content.unwrap_or_default();
        let res = propose_normative_draft(
            &mut client,
            target_node_id,
            proposed_title,
            content,
            proposed_attributes,
            caller,
        )
        .await?;

        let batch_id = res.batch_id.unwrap_or(res.draft_id);

        Ok(serde_json::json!({
            "status": "PENDING_REVIEW",
            "node_id": res.draft_id,
            "draft_id": res.draft_id,
            "target_id": res.target_id,
            "batch_id": batch_id,
            "lifecycle_state": res.lifecycle_state,
            "governance_policy": res.governance_policy,
            "node": res.node
        }))
    }
}

/// Executes the `create_subtask` MCP tool.
pub async fn handle_create_subtask(
    state: &AppState,
    args: Value,
    caller: &AuthenticatedAgent,
) -> Result<ElaboratedTaskResult, MutationError> {
    let parent_node_id = args
        .get("parent_node_id")
        .or_else(|| args.get("parent_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            MutationError::Other("Missing required argument 'parent_node_id'".to_string())
        })?;

    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MutationError::Other("Missing required argument 'title'".to_string()))?;

    let content = args.get("content").and_then(|v| v.as_str());
    let attributes = args.get("attributes").cloned();

    let workspace_id = args
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|e| MutationError::Other(format!("Invalid workspace_id: {e}")))?;

    let mut client = state.pool.get().await.map_err(MutationError::Pool)?;

    if let Some(ws_id) = workspace_id {
        elaborate_in_workspace(
            &mut client,
            ws_id,
            parent_node_id,
            title,
            content,
            attributes,
            caller,
        )
        .await
    } else {
        elaborate_task(
            &mut client,
            parent_node_id,
            title,
            content,
            attributes,
            caller,
        )
        .await
    }
}

/// Executes the `update_node_status` MCP tool.
pub async fn handle_update_node_status(
    state: &AppState,
    args: Value,
    caller: &AuthenticatedAgent,
) -> Result<TaskUpdateResult, MutationError> {
    let node_id = args
        .get("node_id")
        .or_else(|| args.get("task_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| MutationError::Other("Missing required argument 'node_id'".to_string()))?;

    let status_str = args
        .get("status")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MutationError::Other("Missing required argument 'status'".to_string()))?;

    let task_status = status_str
        .parse::<TaskStatus>()
        .map_err(MutationError::GovernanceRejected)?;

    let notes = args.get("notes").and_then(|v| v.as_str());

    let mut client = state.pool.get().await.map_err(MutationError::Pool)?;
    update_task_status(&mut client, node_id, task_status, notes, caller).await
}

/// Executes the `revert_mutations` MCP tool.
pub async fn handle_revert_mutations(
    state: &AppState,
    args: Value,
    caller: &AuthenticatedAgent,
) -> Result<RevertExecutionResult, MutationError> {
    let batch_id = args
        .get("batch_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());

    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let since = args
        .get("since")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    let event_seq_range = args
        .get("event_seq_range")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            if arr.len() == 2 {
                let start = arr[0].as_i64()?;
                let end = arr[1].as_i64()?;
                Some((start, end))
            } else {
                None
            }
        });

    let dry_run = args.get("dry_run").and_then(|v| v.as_bool());
    let force = args.get("force").and_then(|v| v.as_bool());

    let filter = RevertFilter {
        batch_id,
        agent_id,
        since,
        event_seq_range,
        dry_run,
        force,
    };

    let mut client = state.pool.get().await.map_err(MutationError::Pool)?;
    revert_mutations(&mut client, filter, caller).await
}

/// Executes the `reverify_node` MCP tool.
pub async fn handle_reverify_node(
    state: &AppState,
    args: Value,
    caller: &AuthenticatedAgent,
) -> Result<ReverifyResult, MutationError> {
    let node_id = args
        .get("node_id")
        .or_else(|| args.get("target_node_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| MutationError::Other("Missing required argument 'node_id'".to_string()))?;

    let rationale = args
        .get("rationale")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MutationError::Other("Missing required argument 'rationale'".to_string()))?;

    let updated_attributes = args.get("updated_attributes").cloned();

    let mut client = state.pool.get().await.map_err(MutationError::Pool)?;
    reverify_node(&mut client, node_id, rationale, updated_attributes, caller).await
}
