use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::gateway::AppState;
use crate::gateway::auth::AuthenticatedAgent;
use crate::ingest::elicitation::{
    EpisodeCandidate, ExtractionSessionState, GoalCandidate, VerificationVerdict,
    check_technology_lint, detect_gaps, validate_episodes, verify_grounding_quote,
};
use crate::ingest::transcript::{ParsedTranscript, parse_vision_doc_excerpts};
use crate::storage::envelope::assemble_context_envelope_workspace;
use crate::storage::governance::TaskStatus;
use crate::storage::insert_structural_edge;
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
    #[serde(rename = "inputSchema", alias = "input_schema")]
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
        McpToolDefinition {
            name: "open_extraction_session".to_string(),
            description: "Initializes a structured Intent Elicitation extraction session for a conversational transcript or vision document, creating an in-flight job record in ingestion_jobs.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "doc_path": {
                        "type": "string",
                        "description": "Path to the conversational transcript (tks-transcript/v1) or vision document."
                    },
                    "content": {
                        "type": "string",
                        "description": "Optional inline raw document content (if not reading from disk)."
                    }
                },
                "required": ["doc_path"]
            }),
        },
        McpToolDefinition {
            name: "submit_episodes".to_string(),
            description: "Submits Pass 1 topic episode boundaries with contiguity verification and full turn coverage validation (t1..tN).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Active elicitation extraction session UUID."
                    },
                    "episodes": {
                        "type": "array",
                        "description": "Array of contiguous topic episodes covering all turns.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string", "description": "Episode identifier (e.g. 'E1')." },
                                "turns": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "description": "Two-element array of inclusive start and end turn IDs: ['t1', 't5']."
                                },
                                "topic": { "type": "string", "description": "Concise summary of episode topic." }
                            },
                            "required": ["id", "turns", "topic"]
                        }
                    }
                },
                "required": ["session_id", "episodes"]
            }),
        },
        McpToolDefinition {
            name: "submit_goal_candidates".to_string(),
            description: "Submits Pass 2 candidate goals with Pass 3 mechanical verbatim grounding verification and technology-term linting.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Active elicitation extraction session UUID."
                    },
                    "goals": {
                        "type": "array",
                        "description": "Array of candidate goal objects with verbatim grounding quotes.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string", "description": "Candidate goal identifier (e.g. 'G1')." },
                                "kind": { "type": "string", "enum": ["ACHIEVE", "MAINTAIN", "AVOID", "SOFTGOAL"], "description": "Goal taxonomy kind." },
                                "statement": { "type": "string", "description": "Technology-agnostic goal statement." },
                                "stakeholder": { "type": "string", "description": "Stating or owning stakeholder name." },
                                "status": { "type": "string", "enum": ["CONFIRMED", "TENTATIVE", "RETRACTED"], "description": "Goal status." },
                                "rationale": { "type": "string", "description": "Optional underlying stakeholder rationale." },
                                "episode": { "type": "string", "description": "Optional parent episode ID." },
                                "grounding": {
                                    "type": "array",
                                    "description": "Verbatim quote citations from transcript turns.",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "turn": { "type": "string" },
                                            "section": { "type": "string" },
                                            "quote": { "type": "string" }
                                        },
                                        "required": ["quote"]
                                    }
                                },
                                "acceptable_paraphrases": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                },
                                "forbidden_concretizations": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                },
                                "links": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "type": { "type": "string", "enum": ["REFINES", "CONFLICTS_WITH", "CONTRIBUTES_TO"] },
                                            "target": { "type": "string" }
                                        },
                                        "required": ["type", "target"]
                                    }
                                }
                            },
                            "required": ["id", "kind", "statement", "stakeholder", "status", "grounding"]
                        }
                    },
                    "lexicon": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional custom technology lexicon for premature concretization linting."
                    }
                },
                "required": ["session_id", "goals"]
            }),
        },
        McpToolDefinition {
            name: "submit_verification_verdicts".to_string(),
            description: "Submits Pass 4 natural language entailment verification verdicts for candidate goals.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Active elicitation extraction session UUID."
                    },
                    "verdicts": {
                        "type": "array",
                        "description": "Array of entailment verification verdicts.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "goal_id": { "type": "string", "description": "Target candidate goal ID." },
                                "forward": { "type": "string", "enum": ["ENTAILED", "NEUTRAL", "CONTRADICTED"], "description": "Forward entailment verdict." },
                                "reverse_overconstrained": { "type": "boolean", "description": "Whether goal narrows premise." },
                                "notes": { "type": "string", "description": "Optional judge rationale notes." }
                            },
                            "required": ["goal_id", "forward"]
                        }
                    }
                },
                "required": ["session_id", "verdicts"]
            }),
        },
        McpToolDefinition {
            name: "get_gap_report".to_string(),
            description: "Evaluates mechanical gap detectors against the current session state and returns open gaps with suggested questions.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Active elicitation extraction session UUID."
                    }
                },
                "required": ["session_id"]
            }),
        },
        McpToolDefinition {
            name: "stage_goals".to_string(),
            description: "Stages confirmed goals and verbatim source excerpts into graph_nodes (DRAFT state) with GROUNDED_IN and structural edges for staging review.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Active elicitation extraction session UUID."
                    },
                    "target_workspace_id": {
                        "type": "string",
                        "description": "Optional workspace UUID for isolated scratchpad staging."
                    }
                },
                "required": ["session_id"]
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
        .or_else(|| args.get("to_node_id"))
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

    if let Some(from_id) = args.get("from_node_id").and_then(|v| v.as_str()) {
        crate::storage::mutation::pre_validate_edge_target_endpoint(
            &**client,
            "from_node_id",
            from_id,
        )
        .await?;
    }

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

        state
            .event_bus
            .publish(crate::storage::event_bus::GraphChangeEvent {
                event_seq: res.event_seq,
                batch_id: res.batch_id,
                event_type: "TASK_ELABORATED".to_string(),
                entity_id: res.task_id,
                entity_type: "TASK".to_string(),
                actor_id: caller.agent_id.clone(),
                timestamp: chrono::Utc::now(),
            });

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

        state
            .event_bus
            .publish(crate::storage::event_bus::GraphChangeEvent {
                event_seq: res.event_seq,
                batch_id: res.batch_id,
                event_type: "TASK_STATUS_UPDATED".to_string(),
                entity_id: res.task_id,
                entity_type: "TASK".to_string(),
                actor_id: caller.agent_id.clone(),
                timestamp: chrono::Utc::now(),
            });

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

    let res = if let Some(ws_id) = workspace_id {
        elaborate_in_workspace(
            &mut client,
            ws_id,
            parent_node_id,
            title,
            content,
            attributes,
            caller,
        )
        .await?
    } else {
        elaborate_task(
            &mut client,
            parent_node_id,
            title,
            content,
            attributes,
            caller,
        )
        .await?
    };

    state
        .event_bus
        .publish(crate::storage::event_bus::GraphChangeEvent {
            event_seq: res.event_seq,
            batch_id: res.batch_id,
            event_type: "TASK_ELABORATED".to_string(),
            entity_id: res.task_id,
            entity_type: "TASK".to_string(),
            actor_id: caller.agent_id.clone(),
            timestamp: chrono::Utc::now(),
        });

    Ok(res)
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
    let res = update_task_status(&mut client, node_id, task_status, notes, caller).await?;

    state
        .event_bus
        .publish(crate::storage::event_bus::GraphChangeEvent {
            event_seq: res.event_seq,
            batch_id: res.batch_id,
            event_type: "TASK_STATUS_UPDATED".to_string(),
            entity_id: res.task_id,
            entity_type: "TASK".to_string(),
            actor_id: caller.agent_id.clone(),
            timestamp: chrono::Utc::now(),
        });

    Ok(res)
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

/// Executes the `open_extraction_session` MCP tool (EB-4).
pub async fn handle_open_extraction_session(
    state: &AppState,
    args: Value,
    caller: Option<&AuthenticatedAgent>,
) -> Result<Value, String> {
    let doc_path = args
        .get("doc_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument 'doc_path'".to_string())?;

    let content_str = match args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => std::fs::read_to_string(doc_path)
            .map_err(|e| format!("Failed to read '{doc_path}': {e}"))?,
    };

    let mut hasher = Sha256::new();
    hasher.update(content_str.as_bytes());
    let doc_hash = format!("{:x}", hasher.finalize());

    let (document_kind, turns, participants, cycle) =
        if content_str.contains("format: tks-transcript/v1") {
            let parsed = ParsedTranscript::parse(&content_str)
                .map_err(|e| format!("Failed to parse transcript '{doc_path}': {e}"))?;
            (
                "transcript".to_string(),
                parsed.turns,
                parsed.frontmatter.participants,
                parsed.frontmatter.cycle,
            )
        } else {
            let excerpts = parse_vision_doc_excerpts(&content_str, doc_path, &doc_hash)
                .map_err(|e| format!("Failed to parse vision doc '{doc_path}': {e:?}"))?;
            let turns: Vec<crate::ingest::transcript::ParsedTurn> = excerpts
                .into_iter()
                .enumerate()
                .map(|(i, exc)| crate::ingest::transcript::ParsedTurn {
                    id: exc.alias.clone(),
                    number: (i + 1) as u32,
                    name: exc
                        .heading
                        .clone()
                        .unwrap_or_else(|| "Document".to_string()),
                    role: "stakeholder".to_string(),
                    timestamp: None,
                    block_byte_start: exc.byte_start,
                    block_byte_end: exc.byte_end,
                    utterance_byte_start: exc.byte_start,
                    utterance_byte_end: exc.byte_end,
                    block_text: exc.content.clone(),
                    utterance_text: exc.content,
                })
                .collect();
            ("document".to_string(), turns, vec![], 1)
        };

    let session_id = Uuid::new_v4();
    let session_state = ExtractionSessionState {
        session_id,
        doc_path: doc_path.to_string(),
        document_hash: doc_hash.clone(),
        document_kind: document_kind.clone(),
        cycle,
        participants: participants.clone(),
        turns: turns.clone(),
        episodes: Vec::new(),
        goals: Vec::new(),
        verdicts: Vec::new(),
        gaps: Vec::new(),
        custom_lexicon: None,
    };

    let session_json =
        serde_json::to_value(&session_state).map_err(|e| format!("Serialization error: {e}"))?;

    let actor = caller.map(|c| c.agent_id.as_str()).unwrap_or("agent");

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| format!("Pool error: {e}"))?;
    client
        .execute(
            "INSERT INTO ingestion_jobs (job_id, doc_path, document_hash, status, created_by, attributes) \
             VALUES ($1, $2, $3, 'PROCESSING', $4, $5);",
            &[&session_id, &doc_path, &doc_hash, &actor, &session_json],
        )
        .await
        .map_err(|e| format!("Database error inserting session job: {e}"))?;

    let turns_summary: Vec<Value> = turns
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "speaker": t.name,
                "role": t.role,
                "text": t.utterance_text,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "session_id": session_id,
        "doc_path": doc_path,
        "document_hash": doc_hash,
        "document_kind": document_kind,
        "turns_count": turns.len(),
        "cycle": cycle,
        "participants": participants,
        "turns": turns_summary,
        "next_pass": "Pass 1: Episode Segmentation",
        "recommended_prompt": "elicitation/episode-segmentation/v1"
    }))
}

/// Executes the `submit_episodes` MCP tool (EB-4).
pub async fn handle_submit_episodes(
    state: &AppState,
    args: Value,
    _caller: Option<&AuthenticatedAgent>,
) -> Result<Value, String> {
    let session_id_str = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument 'session_id'".to_string())?;
    let session_id =
        Uuid::parse_str(session_id_str).map_err(|e| format!("Invalid session_id: {e}"))?;

    let episodes_val = args
        .get("episodes")
        .ok_or_else(|| "Missing required argument 'episodes'".to_string())?;
    let episodes: Vec<EpisodeCandidate> = serde_json::from_value(episodes_val.clone())
        .map_err(|e| format!("Invalid episodes payload: {e}"))?;

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| format!("Pool error: {e}"))?;
    let row = client
        .query_opt(
            "SELECT attributes FROM ingestion_jobs WHERE job_id = $1;",
            &[&session_id],
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or_else(|| format!("Extraction session '{session_id}' not found"))?;

    let attrs: Value = row.get("attributes");
    let mut session: ExtractionSessionState = serde_json::from_value(attrs)
        .map_err(|e| format!("Failed to deserialize session state: {e}"))?;

    validate_episodes(&episodes, session.turns.len())?;

    session.episodes = episodes;
    let updated_attrs =
        serde_json::to_value(&session).map_err(|e| format!("Serialization error: {e}"))?;

    client
        .execute(
            "UPDATE ingestion_jobs SET attributes = $2, updated_at = NOW() WHERE job_id = $1;",
            &[&session_id, &updated_attrs],
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?;

    Ok(serde_json::json!({
        "session_id": session_id,
        "episodes_accepted": session.episodes.len(),
        "next_pass": "Pass 2: Schema-Constrained Extraction",
        "recommended_prompt": "elicitation/goal-extraction/v1"
    }))
}

/// Executes the `submit_goal_candidates` MCP tool (EB-4).
pub async fn handle_submit_goal_candidates(
    state: &AppState,
    args: Value,
    _caller: Option<&AuthenticatedAgent>,
) -> Result<Value, String> {
    let session_id_str = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument 'session_id'".to_string())?;
    let session_id =
        Uuid::parse_str(session_id_str).map_err(|e| format!("Invalid session_id: {e}"))?;

    let goals_val = args
        .get("goals")
        .ok_or_else(|| "Missing required argument 'goals'".to_string())?;
    let mut goals: Vec<GoalCandidate> = serde_json::from_value(goals_val.clone())
        .map_err(|e| format!("Invalid goals payload: {e}"))?;

    let custom_lexicon: Option<Vec<String>> = args
        .get("lexicon")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| format!("Pool error: {e}"))?;
    let row = client
        .query_opt(
            "SELECT attributes FROM ingestion_jobs WHERE job_id = $1;",
            &[&session_id],
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or_else(|| format!("Extraction session '{session_id}' not found"))?;

    let attrs: Value = row.get("attributes");
    let mut session: ExtractionSessionState = serde_json::from_value(attrs)
        .map_err(|e| format!("Failed to deserialize session state: {e}"))?;

    let turn_map: HashMap<&str, (&str, &str)> = session
        .turns
        .iter()
        .map(|t| (t.id.as_str(), (t.utterance_text.as_str(), t.role.as_str())))
        .collect();

    let lexicon_ref: Option<Vec<&str>> = custom_lexicon
        .as_ref()
        .map(|v| v.iter().map(|s| s.as_str()).collect());

    let mut all_warnings = Vec::new();

    for goal in &mut goals {
        if !matches!(
            goal.kind.as_str(),
            "ACHIEVE" | "MAINTAIN" | "AVOID" | "SOFTGOAL"
        ) {
            return Err(format!(
                "Goal {} has invalid kind '{}': expected ACHIEVE, MAINTAIN, AVOID, or SOFTGOAL",
                goal.id, goal.kind
            ));
        }
        if !matches!(
            goal.status.as_str(),
            "CONFIRMED" | "TENTATIVE" | "RETRACTED"
        ) {
            return Err(format!(
                "Goal {} has invalid status '{}': expected CONFIRMED, TENTATIVE, or RETRACTED",
                goal.id, goal.status
            ));
        }
        if goal.grounding.is_empty() {
            return Err(format!(
                "Goal {} must specify at least 1 grounding citation",
                goal.id
            ));
        }

        let mut quote_strings = Vec::new();
        let mut has_stakeholder_grounding = false;

        for citation in &goal.grounding {
            if let Some(ref turn_id) = citation.turn {
                let (utterance, role) = turn_map.get(turn_id.as_str()).ok_or_else(|| {
                    format!("Goal {} cites non-existent turn '{}'", goal.id, turn_id)
                })?;

                if !verify_grounding_quote(&citation.quote, utterance) {
                    return Err(format!(
                        "Grounding verification failed for goal {}: quote {:?} not found in turn '{}'",
                        goal.id, citation.quote, turn_id
                    ));
                }

                if *role == "stakeholder" {
                    has_stakeholder_grounding = true;
                }
            } else if let Some(ref _sec) = citation.section {
                has_stakeholder_grounding = true;
            } else {
                return Err(format!("Goal {} citation missing turn or section", goal.id));
            }

            quote_strings.push(citation.quote.as_str());
        }

        if !has_stakeholder_grounding && goal.status != "RETRACTED" {
            goal.warnings
                .push("Facilitator turn cited without stakeholder confirmation".to_string());
        }

        let lint_warnings =
            check_technology_lint(&goal.statement, &quote_strings, lexicon_ref.as_deref());
        for w in lint_warnings {
            goal.warnings.push(w.clone());
            all_warnings.push(format!("Goal {}: {w}", goal.id));
        }
    }

    session.goals = goals;
    if let Some(lex) = custom_lexicon {
        session.custom_lexicon = Some(lex);
    }

    let updated_attrs =
        serde_json::to_value(&session).map_err(|e| format!("Serialization error: {e}"))?;

    client
        .execute(
            "UPDATE ingestion_jobs SET attributes = $2, updated_at = NOW() WHERE job_id = $1;",
            &[&session_id, &updated_attrs],
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?;

    Ok(serde_json::json!({
        "session_id": session_id,
        "goals_accepted": session.goals.len(),
        "warnings": all_warnings,
        "next_pass": "Pass 4: Entailment Verification",
        "recommended_prompt": "elicitation/entailment-judge/v1"
    }))
}

/// Executes the `submit_verification_verdicts` MCP tool (EB-4).
pub async fn handle_submit_verification_verdicts(
    state: &AppState,
    args: Value,
    _caller: Option<&AuthenticatedAgent>,
) -> Result<Value, String> {
    let session_id_str = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument 'session_id'".to_string())?;
    let session_id =
        Uuid::parse_str(session_id_str).map_err(|e| format!("Invalid session_id: {e}"))?;

    let verdicts_val = args
        .get("verdicts")
        .ok_or_else(|| "Missing required argument 'verdicts'".to_string())?;
    let verdicts: Vec<VerificationVerdict> = serde_json::from_value(verdicts_val.clone())
        .map_err(|e| format!("Invalid verdicts payload: {e}"))?;

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| format!("Pool error: {e}"))?;
    let row = client
        .query_opt(
            "SELECT attributes FROM ingestion_jobs WHERE job_id = $1;",
            &[&session_id],
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or_else(|| format!("Extraction session '{session_id}' not found"))?;

    let attrs: Value = row.get("attributes");
    let mut session: ExtractionSessionState = serde_json::from_value(attrs)
        .map_err(|e| format!("Failed to deserialize session state: {e}"))?;

    let mut entailed = 0usize;
    let mut neutral = 0usize;
    let mut contradicted = 0usize;

    let verdict_map: HashMap<&str, &VerificationVerdict> =
        verdicts.iter().map(|v| (v.goal_id.as_str(), v)).collect();

    for goal in &mut session.goals {
        if let Some(&v) = verdict_map.get(goal.id.as_str()) {
            match v.forward.as_str() {
                "ENTAILED" => entailed += 1,
                "NEUTRAL" => neutral += 1,
                "CONTRADICTED" => contradicted += 1,
                other => {
                    return Err(format!(
                        "Invalid forward verdict '{}' for goal {}: expected ENTAILED, NEUTRAL, or CONTRADICTED",
                        other, goal.id
                    ));
                }
            }
            goal.verification = Some(v.clone());
        }
    }

    session.verdicts = verdicts;
    let updated_attrs =
        serde_json::to_value(&session).map_err(|e| format!("Serialization error: {e}"))?;

    client
        .execute(
            "UPDATE ingestion_jobs SET attributes = $2, updated_at = NOW() WHERE job_id = $1;",
            &[&session_id, &updated_attrs],
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?;

    Ok(serde_json::json!({
        "session_id": session_id,
        "verdicts_recorded": session.verdicts.len(),
        "summary": {
            "entailed": entailed,
            "neutral": neutral,
            "contradicted": contradicted
        },
        "next_pass": "Pass 5: Gap Analysis",
        "recommended_tool": "get_gap_report"
    }))
}

/// Executes the `get_gap_report` MCP tool (EB-4).
pub async fn handle_get_gap_report(
    state: &AppState,
    args: Value,
    _caller: Option<&AuthenticatedAgent>,
) -> Result<Value, String> {
    let session_id_str = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument 'session_id'".to_string())?;
    let session_id =
        Uuid::parse_str(session_id_str).map_err(|e| format!("Invalid session_id: {e}"))?;

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| format!("Pool error: {e}"))?;
    let row = client
        .query_opt(
            "SELECT attributes FROM ingestion_jobs WHERE job_id = $1;",
            &[&session_id],
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or_else(|| format!("Extraction session '{session_id}' not found"))?;

    let attrs: Value = row.get("attributes");
    let mut session: ExtractionSessionState = serde_json::from_value(attrs)
        .map_err(|e| format!("Failed to deserialize session state: {e}"))?;

    let gaps = detect_gaps(&session);
    session.gaps = gaps.clone();

    let updated_attrs =
        serde_json::to_value(&session).map_err(|e| format!("Serialization error: {e}"))?;

    client
        .execute(
            "UPDATE ingestion_jobs SET attributes = $2, updated_at = NOW() WHERE job_id = $1;",
            &[&session_id, &updated_attrs],
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?;

    Ok(serde_json::json!({
        "session_id": session_id,
        "gap_count": gaps.len(),
        "gaps": gaps,
        "next_step": "stage_goals"
    }))
}

/// Executes the `stage_goals` MCP tool (EB-4).
pub async fn handle_stage_goals(
    state: &AppState,
    args: Value,
    caller: Option<&AuthenticatedAgent>,
) -> Result<Value, String> {
    let session_id_str = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument 'session_id'".to_string())?;
    let session_id =
        Uuid::parse_str(session_id_str).map_err(|e| format!("Invalid session_id: {e}"))?;

    let workspace_id = args
        .get("target_workspace_id")
        .and_then(|v| v.as_str())
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|e| format!("Invalid target_workspace_id: {e}"))?;

    let actor = caller.map(|c| c.agent_id.as_str()).unwrap_or("agent");

    let mut pool_client = state
        .pool
        .get()
        .await
        .map_err(|e| format!("Pool error: {e}"))?;
    let tx = pool_client
        .build_transaction()
        .start()
        .await
        .map_err(|e| format!("Transaction error: {e}"))?;

    let row = tx
        .query_opt(
            "SELECT doc_path, attributes FROM ingestion_jobs WHERE job_id = $1 FOR UPDATE;",
            &[&session_id],
        )
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or_else(|| format!("Extraction session '{session_id}' not found"))?;

    let doc_path: String = row.get("doc_path");
    let attrs: Value = row.get("attributes");
    let session: ExtractionSessionState = serde_json::from_value(attrs)
        .map_err(|e| format!("Failed to deserialize session state: {e}"))?;

    let confirmed_goals: Vec<&GoalCandidate> = session
        .goals
        .iter()
        .filter(|g| g.status == "CONFIRMED")
        .collect();

    let session_prefix = &session_id.to_string()[..8];
    let mut candidate_to_uuid: HashMap<String, Uuid> = HashMap::new();
    let mut staged_goals_list = Vec::new();
    let mut total_excerpts = 0usize;

    // 1. Insert GOAL and SOURCE_EXCERPT nodes and GROUNDED_IN edges
    for (idx, goal) in confirmed_goals.iter().enumerate() {
        let goal_uuid = Uuid::new_v4();
        let goal_key = format!("GOAL-{session_prefix}-{:02}", idx + 1);
        candidate_to_uuid.insert(goal.id.clone(), goal_uuid);

        let goal_attrs = serde_json::json!({
            "goal_kind": goal.kind,
            "stakeholder": goal.stakeholder,
            "status": goal.status,
            "grounding": goal.grounding,
            "verification": goal.verification,
            "warnings": goal.warnings,
            "workspace_id": workspace_id.map(|w| w.to_string()),
        });

        tx.execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, created_by, job_id, doc_path, attributes) \
             VALUES ($1, $2, 'GOAL', $3, $4, 'DRAFT', $5, $6, $7, $8);",
            &[
                &goal_uuid,
                &goal_key,
                &goal.statement,
                &goal.rationale.as_deref().unwrap_or(&goal.statement),
                &actor,
                &session_id,
                &doc_path,
                &goal_attrs,
            ],
        )
        .await
        .map_err(|e| format!("Failed to insert GOAL node: {e}"))?;

        let mut excerpt_uuids = Vec::new();
        for (c_idx, citation) in goal.grounding.iter().enumerate() {
            let exc_uuid = Uuid::new_v4();
            let exc_key = format!("EXC-{session_prefix}-{:02}-{:02}", idx + 1, c_idx + 1);

            let exc_attrs = serde_json::json!({
                "turn": citation.turn,
                "section": citation.section,
                "workspace_id": workspace_id.map(|w| w.to_string()),
            });

            tx.execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, created_by, job_id, doc_path, attributes) \
                 VALUES ($1, $2, 'SOURCE_EXCERPT', $3, $4, 'DRAFT', $5, $6, $7, $8);",
                &[
                    &exc_uuid,
                    &exc_key,
                    &format!("Excerpt for {}", goal.id),
                    &citation.quote,
                    &actor,
                    &session_id,
                    &doc_path,
                    &exc_attrs,
                ],
            )
            .await
            .map_err(|e| format!("Failed to insert excerpt node: {e}"))?;

            tx.execute(
                "INSERT INTO graph_edges (from_node_id, to_node_id, edge_type, created_by, lifecycle_state) \
                 VALUES ($1, $2, 'GROUNDED_IN', $3, 'DRAFT');",
                &[&goal_uuid, &exc_uuid, &actor],
            )
            .await
            .map_err(|e| format!("Failed to insert GROUNDED_IN edge: {e}"))?;

            excerpt_uuids.push(exc_uuid);
            total_excerpts += 1;
        }

        staged_goals_list.push(serde_json::json!({
            "id": goal_uuid,
            "node_key": goal_key,
            "kind": goal.kind,
            "statement": goal.statement,
            "excerpts": excerpt_uuids,
        }));
    }

    // 2. Insert candidate links between staged goals
    let mut total_edges = total_excerpts;
    for goal in &confirmed_goals {
        let from_uuid = candidate_to_uuid[&goal.id];
        if let Some(links) = &goal.links {
            for link in links {
                if let Some(&to_uuid) = candidate_to_uuid.get(&link.target) {
                    let (e_from, e_to, e_type) = match link.link_type.as_str() {
                        "REFINES" => (from_uuid, to_uuid, "REFINES"),
                        "CONTRIBUTES_TO" => (from_uuid, to_uuid, "CONTRIBUTES_TO"),
                        "CONFLICTS_WITH" => {
                            if from_uuid < to_uuid {
                                (from_uuid, to_uuid, "CONFLICTS_WITH")
                            } else {
                                (to_uuid, from_uuid, "CONFLICTS_WITH")
                            }
                        }
                        _ => continue,
                    };

                    let res =
                        insert_structural_edge(&*tx, e_from, e_to, e_type, actor, "DRAFT").await;
                    if let Ok(_edge) = res {
                        total_edges += 1;
                    }
                }
            }
        }
    }

    // 3. Mark ingestion_jobs as STAGED
    tx.execute(
        "UPDATE ingestion_jobs SET status = 'STAGED', updated_at = NOW() WHERE job_id = $1;",
        &[&session_id],
    )
    .await
    .map_err(|e| format!("Database error updating job status: {e}"))?;

    tx.commit()
        .await
        .map_err(|e| format!("Commit error: {e}"))?;

    Ok(serde_json::json!({
        "session_id": session_id,
        "status": "STAGED",
        "staged_goals_count": staged_goals_list.len(),
        "staged_goals": staged_goals_list,
        "staged_excerpts_count": total_excerpts,
        "staged_edges_count": total_edges
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_tools_list_serializes_camel_case_input_schema() {
        let tools = list_tools();
        assert_eq!(
            tools.len(),
            14,
            "Expected exactly 14 MCP tools (8 core + 6 elicitation)"
        );

        let serialized_value =
            serde_json::to_value(&tools).expect("Failed to serialize tools list");
        let serialized_str =
            serde_json::to_string(&tools).expect("Failed to serialize tools string");

        // Verify zero occurrences of snake_case input_schema in serialized JSON
        assert!(
            !serialized_str.contains("\"input_schema\""),
            "Serialized tools JSON must not contain snake_case 'input_schema'"
        );

        // Verify 100% of tools serialize with camelCase inputSchema
        let tools_array = serialized_value
            .as_array()
            .expect("Tools must be a JSON array");
        assert_eq!(tools_array.len(), 14);

        for tool in tools_array {
            let tool_name = tool
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("<unknown>");
            assert!(
                tool.get("inputSchema").is_some(),
                "Tool '{tool_name}' must serialize schema under camelCase 'inputSchema'"
            );
            assert!(
                tool.get("input_schema").is_none(),
                "Tool '{tool_name}' must not have snake_case 'input_schema'"
            );

            let schema = tool.get("inputSchema").unwrap();
            assert_eq!(
                schema.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "Tool '{tool_name}' inputSchema must be type: object"
            );
            assert!(
                schema.get("properties").is_some(),
                "Tool '{tool_name}' inputSchema must define properties"
            );
            assert!(
                schema.get("required").is_some(),
                "Tool '{tool_name}' inputSchema must define required array"
            );
        }
    }
}
