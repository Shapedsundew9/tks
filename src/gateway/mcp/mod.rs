//! Model Context Protocol (MCP) router over HTTP/SSE and JSON-RPC (WP-1.5, WP-2.4, D-19, TB-4).

pub mod tools;

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::gateway::AppState;
use crate::gateway::auth::MaybeAuthenticatedAgent;
use crate::gateway::mcp::tools::{
    handle_create_subtask, handle_get_context_envelope, handle_get_document_span,
    handle_propose_node_mutation, handle_query_requirements, handle_reverify_node,
    handle_revert_mutations, handle_update_node_status, list_tools,
};
use crate::storage::mutation::MutationError;
use crate::storage::rollback::RevertExecutionResult;

/// Incoming JSON-RPC 2.0 request payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(default = "default_jsonrpc_version")]
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

fn default_jsonrpc_version() -> String {
    "2.0".to_string()
}

/// Outgoing JSON-RPC 2.0 response payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    #[must_use]
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    #[must_use]
    pub fn error(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// Query parameters for `/mcp/message`.
#[derive(Debug, Deserialize)]
pub struct McpMessageQuery {
    pub session_id: Option<String>,
}

/// Handles `GET /mcp/sse`.
///
/// Establishes a Server-Sent Events stream for external agent harnesses.
/// Sends initial `endpoint` event pointing to `/mcp/message?sessionId=<uuid>`
/// and maintains connection with keep-alive pings.
pub async fn handle_mcp_sse() -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let session_id = Uuid::new_v4().to_string();
    let endpoint_url = format!("/mcp/message?sessionId={session_id}");

    let initial_event = Event::default().event("endpoint").data(endpoint_url);

    let stream = stream::once(async move { Ok(initial_event) });

    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// Handles `POST /mcp/message` (HTTP/SSE transport command endpoint).
pub async fn handle_mcp_message(
    State(state): State<AppState>,
    Query(_query): Query<McpMessageQuery>,
    caller: MaybeAuthenticatedAgent,
    Json(request): Json<JsonRpcRequest>,
) -> Response {
    let resp = process_jsonrpc_request(&state, request, caller.0.as_ref()).await;
    match resp {
        Some(response) => (StatusCode::OK, Json(response)).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

/// Handles `POST /mcp` (direct HTTP JSON-RPC endpoint).
pub async fn handle_mcp_rpc(
    State(state): State<AppState>,
    caller: MaybeAuthenticatedAgent,
    Json(request): Json<JsonRpcRequest>,
) -> Response {
    let resp = process_jsonrpc_request(&state, request, caller.0.as_ref()).await;
    match resp {
        Some(response) => (StatusCode::OK, Json(response)).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

/// Central JSON-RPC request dispatcher handling MCP lifecycle and tool invocations.
pub async fn process_jsonrpc_request(
    state: &AppState,
    request: JsonRpcRequest,
    caller: Option<&crate::gateway::auth::AuthenticatedAgent>,
) -> Option<JsonRpcResponse> {
    let id = request.id.clone().unwrap_or(Value::Null);

    match request.method.as_str() {
        "initialize" => {
            let res = serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "tks",
                    "version": "0.1.0"
                }
            });
            Some(JsonRpcResponse::success(id, res))
        }

        "notifications/initialized" | "initialized" => {
            // Notifications do not emit responses in JSON-RPC 2.0
            None
        }

        "tools/list" => {
            let tools = list_tools();
            let res = serde_json::json!({
                "tools": tools
            });
            Some(JsonRpcResponse::success(id, res))
        }

        "tools/call" => {
            let tool_name = request.params.get("name").and_then(|v| v.as_str());
            let mut arguments = request
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);

            // Forward auth_token from params to arguments if present
            if let (Some(token), Some(arg_obj)) = (
                request
                    .params
                    .get("auth_token")
                    .or_else(|| request.params.get("token")),
                arguments.as_object_mut(),
            ) {
                arg_obj.entry("auth_token").or_insert_with(|| token.clone());
            }

            let tool_name = match tool_name {
                Some(name) => name,
                None => {
                    return Some(JsonRpcResponse::error(
                        id,
                        -32602,
                        "Missing required 'name' in tools/call params",
                    ));
                }
            };

            dispatch_tool_call(state, id, tool_name, arguments, caller).await
        }

        // Direct method invocations for developer and test ergonomics
        "get_context_envelope"
        | "query_requirements"
        | "get_document_span"
        | "propose_node_mutation"
        | "create_subtask"
        | "update_node_status"
        | "revert_mutations"
        | "reverify_node" => {
            dispatch_tool_call(state, id, &request.method, request.params, caller).await
        }

        _ => Some(JsonRpcResponse::error(
            id,
            -32601,
            format!("Method not found: {}", request.method),
        )),
    }
}

/// Dispatches execution to the appropriate tool handler.
async fn dispatch_tool_call(
    state: &AppState,
    id: Value,
    tool_name: &str,
    arguments: Value,
    caller: Option<&crate::gateway::auth::AuthenticatedAgent>,
) -> Option<JsonRpcResponse> {
    let is_mutation_tool = matches!(
        tool_name,
        "propose_node_mutation"
            | "create_subtask"
            | "update_node_status"
            | "revert_mutations"
            | "reverify_node"
    );

    let effective_caller = match caller {
        Some(c) => Some(c.clone()),
        None => {
            let token_opt = arguments
                .get("auth_token")
                .or_else(|| arguments.get("token"))
                .or_else(|| arguments.get("auth").and_then(|a| a.get("token")))
                .and_then(|v| v.as_str());

            if let Some(token) = token_opt {
                state.auth.authenticate(token).await.ok()
            } else {
                None
            }
        }
    };

    if is_mutation_tool && effective_caller.is_none() {
        return Some(JsonRpcResponse::error(
            id,
            -32000,
            "ERR_AUTH_FAILED: Authentication required for mutation tools",
        ));
    }

    match tool_name {
        "get_context_envelope" => {
            match handle_get_context_envelope(state, arguments, effective_caller.as_ref()).await {
                Ok(envelope) => {
                    let formatted_text = envelope.format_markdown();
                    let result = serde_json::json!({
                        "content": [
                            {
                                "type": "text",
                                "text": formatted_text
                            }
                        ],
                        "envelope": envelope
                    });
                    Some(JsonRpcResponse::success(id, result))
                }
                Err(err) => Some(JsonRpcResponse::error(id, -32000, err)),
            }
        }

        "query_requirements" => match handle_query_requirements(state, arguments).await {
            Ok(results) => {
                let serialized = serde_json::to_string_pretty(&results).unwrap_or_default();
                let result = serde_json::json!({
                    "content": [
                        {
                            "type": "text",
                            "text": serialized
                        }
                    ],
                    "results": results
                });
                Some(JsonRpcResponse::success(id, result))
            }
            Err(err) => Some(JsonRpcResponse::error(id, -32000, err)),
        },

        "get_document_span" => match handle_get_document_span(state, arguments).await {
            Ok(span) => {
                let result = serde_json::json!({
                    "content": [
                        {
                            "type": "text",
                            "text": span
                        }
                    ],
                    "span": span
                });
                Some(JsonRpcResponse::success(id, result))
            }
            Err(err) => Some(JsonRpcResponse::error(id, -32000, err)),
        },

        "propose_node_mutation" => {
            let agent = match effective_caller.as_ref() {
                Some(a) => a,
                None => {
                    return Some(JsonRpcResponse::error(
                        id,
                        -32000,
                        "ERR_AUTH_FAILED: Authentication required for mutation tools",
                    ));
                }
            };

            match handle_propose_node_mutation(state, arguments, agent).await {
                Ok(val) => {
                    let node_id = val
                        .get("node_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let status = val
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("COMMITTED");
                    let result = serde_json::json!({
                        "content": [
                            {
                                "type": "text",
                                "text": format!("Mutation proposed: status={status}, node_id={node_id}")
                            }
                        ],
                        "status": status,
                        "node_id": val.get("node_id"),
                        "batch_id": val.get("batch_id"),
                        "mutation": val
                    });
                    Some(JsonRpcResponse::success(id, result))
                }
                Err(err) => Some(JsonRpcResponse::error(
                    id,
                    -32000,
                    format!("{}: {err}", err.code()),
                )),
            }
        }

        "create_subtask" => {
            let agent = match effective_caller.as_ref() {
                Some(a) => a,
                None => {
                    return Some(JsonRpcResponse::error(
                        id,
                        -32000,
                        "ERR_AUTH_FAILED: Authentication required for mutation tools",
                    ));
                }
            };

            match handle_create_subtask(state, arguments, agent).await {
                Ok(res) => {
                    let result = serde_json::json!({
                        "content": [
                            {
                                "type": "text",
                                "text": format!("Subtask created: task_id={}, status=ACTIVE", res.task_id)
                            }
                        ],
                        "task_id": res.task_id,
                        "node_key": res.node_key,
                        "status": res.status,
                        "batch_id": res.batch_id,
                        "subtask": res
                    });
                    Some(JsonRpcResponse::success(id, result))
                }
                Err(err) => Some(JsonRpcResponse::error(
                    id,
                    -32000,
                    format!("{}: {err}", err.code()),
                )),
            }
        }

        "update_node_status" => {
            let agent = match effective_caller.as_ref() {
                Some(a) => a,
                None => {
                    return Some(JsonRpcResponse::error(
                        id,
                        -32000,
                        "ERR_AUTH_FAILED: Authentication required for mutation tools",
                    ));
                }
            };

            match handle_update_node_status(state, arguments, agent).await {
                Ok(res) => {
                    let result = serde_json::json!({
                        "content": [
                            {
                                "type": "text",
                                "text": format!(
                                    "Task status updated: task_id={}, status={}, execution_status={}",
                                    res.task_id, res.status, res.execution_status
                                )
                            }
                        ],
                        "status": res.status,
                        "task_id": res.task_id,
                        "execution_status": res.execution_status,
                        "batch_id": res.batch_id,
                        "update": res
                    });
                    Some(JsonRpcResponse::success(id, result))
                }
                Err(err) => Some(JsonRpcResponse::error(
                    id,
                    -32000,
                    format!("{}: {err}", err.code()),
                )),
            }
        }

        "revert_mutations" => {
            let agent = match effective_caller.as_ref() {
                Some(a) => a,
                None => {
                    return Some(JsonRpcResponse::error(
                        id,
                        -32000,
                        "ERR_AUTH_FAILED: Authentication required for mutation tools",
                    ));
                }
            };

            match handle_revert_mutations(state, arguments, agent).await {
                Ok(res) => {
                    let result = match &res {
                        RevertExecutionResult::DryRun(preview) => serde_json::json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": format!(
                                        "Rollback dry-run preview: {} affected nodes, {} affected edges, {} cross-agent dependencies",
                                        preview.affected_nodes.len(),
                                        preview.affected_edges.len(),
                                        preview.cross_agent_dependencies.len()
                                    )
                                }
                            ],
                            "status": "PREVIEW",
                            "preview": preview,
                            "affected_nodes": preview.affected_nodes,
                            "affected_edges": preview.affected_edges,
                            "cross_agent_dependencies": preview.cross_agent_dependencies,
                            "total_events": preview.total_events
                        }),
                        RevertExecutionResult::Reverted(reverted) => serde_json::json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": format!(
                                        "Rollback executed: batch_id={}, reverted_events={}",
                                        reverted.batch_id, reverted.reverted_events
                                    )
                                }
                            ],
                            "status": "REVERTED",
                            "batch_id": reverted.batch_id,
                            "reverted_events": reverted.reverted_events,
                            "affected_nodes": reverted.affected_nodes,
                            "cascade_reverified_nodes": reverted.cascade_reverified_nodes
                        }),
                    };
                    Some(JsonRpcResponse::success(id, result))
                }
                Err(MutationError::ConfirmationRequired(preview)) => {
                    let mut err_resp = JsonRpcResponse::error(
                        id,
                        -32000,
                        format!(
                            "ERR_CONFIRMATION_REQUIRED: cross-agent dependent tasks detected: {:?}, total affected nodes: {}",
                            preview.cross_agent_dependencies,
                            preview.affected_nodes.len()
                        ),
                    );
                    if let Some(ref mut e) = err_resp.error {
                        e.data = Some(serde_json::json!({
                            "code": "ERR_CONFIRMATION_REQUIRED",
                            "preview": preview
                        }));
                    }
                    Some(err_resp)
                }
                Err(err) => Some(JsonRpcResponse::error(
                    id,
                    -32000,
                    format!("{}: {err}", err.code()),
                )),
            }
        }

        "reverify_node" => {
            let agent = match effective_caller.as_ref() {
                Some(a) => a,
                None => {
                    return Some(JsonRpcResponse::error(
                        id,
                        -32000,
                        "ERR_AUTH_FAILED: Authentication required for mutation tools",
                    ));
                }
            };

            match handle_reverify_node(state, arguments, agent).await {
                Ok(res) => {
                    let result = serde_json::json!({
                        "content": [
                            {
                                "type": "text",
                                "text": format!(
                                    "Node reverified: node_id={}, status=ACTIVE, staleness_score={}",
                                    res.node_id, res.staleness_score
                                )
                            }
                        ],
                        "status": res.status,
                        "node_id": res.node_id,
                        "staleness_score": res.staleness_score,
                        "batch_id": res.batch_id,
                        "event_seq": res.event_seq
                    });
                    Some(JsonRpcResponse::success(id, result))
                }
                Err(err) => Some(JsonRpcResponse::error(
                    id,
                    -32000,
                    format!("{}: {err}", err.code()),
                )),
            }
        }

        _ => Some(JsonRpcResponse::error(
            id,
            -32601,
            format!("Unknown tool: {tool_name}"),
        )),
    }
}
