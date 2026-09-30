//! Model Context Protocol (MCP) router over HTTP/SSE and JSON-RPC (WP-1.5, D-19, TB-4).

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
    handle_get_context_envelope, handle_get_document_span, handle_query_requirements, list_tools,
};

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
            let arguments = request
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);

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
        "get_context_envelope" | "query_requirements" | "get_document_span" => {
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
    match tool_name {
        "get_context_envelope" => {
            match handle_get_context_envelope(state, arguments, caller).await {
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

        _ => Some(JsonRpcResponse::error(
            id,
            -32601,
            format!("Unknown tool: {tool_name}"),
        )),
    }
}
