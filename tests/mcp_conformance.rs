//! MCP Protocol Conformance, Edge Pre-Validation, and Gateway Hardening Test Suite (WP-4.4).
//!
//! Verifies:
//! 1. Official MCP 2024-11-05 wire protocol conformance:
//!    `tools/list` returns all 8 tools strictly using camelCase `inputSchema` with 0 snake_case occurrences.
//! 2. Invariant INV-9 relational edge target pre-validation:
//!    Attempting to create tasks or propose drafts referencing raw Git commit SHAs (e.g. `c0ffee...`)
//!    or non-existent IDs is rejected prior to transaction execution with `ERR_INVALID_EDGE_TARGET`
//!    (HTTP 422 in REST / JSON-RPC -32000 in MCP) and an Actionable Remediation Envelope (INV-8).
//! 3. Dual authentication extraction:
//!    Supports caller authentication from HTTP `Authorization: Bearer <token>` headers,
//!    top-level `auth_token` in JSON-RPC requests, params `auth_token`, and tool arguments `auth_token`.
//!    Unauthenticated calls fail with standardized `-32000` and `ERR_AUTH_FAILED` (TB-4, DEC-2.10).
//! 4. CLI Stdio adapter resilience:
//!    When the daemon is offline, returns valid JSON-RPC initialization error with code `-32000` and exits 0.

use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tks::cli::mcp_stdio::{McpStdioArgs, run_mcp_stdio_io};
use tks::db;
use tks::gateway::auth::compute_token_hash;
use tks::gateway::{AppState, create_router};
use tks::storage::StorageRepo;
use tks::storage::git::start_git_actor;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Spawns a test gateway server on an ephemeral TCP port.
async fn spawn_test_server() -> (
    String,
    CancellationToken,
    AppState,
    tokio::sync::MutexGuard<'static, ()>,
) {
    let guard = TEST_MUTEX.lock().await;
    let test_db_url = db::ensure_test_database_ready()
        .await
        .expect("Failed to prepare test database");
    let pool = db::create_pool(&test_db_url).expect("Failed to create PostgreSQL connection pool");

    // Clean test residue
    {
        let client = pool.get().await.expect("Failed to get DB client");
        let _ = client
            .execute(
                "DELETE FROM graph_edges WHERE from_node_id IN (SELECT id FROM graph_nodes WHERE node_key LIKE 'REQ-WP44-%' OR node_key LIKE 'TASK-WP44-%' OR created_by LIKE 'test_wp44_%') OR to_node_id IN (SELECT id FROM graph_nodes WHERE node_key LIKE 'REQ-WP44-%' OR node_key LIKE 'TASK-WP44-%' OR created_by LIKE 'test_wp44_%');",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM graph_nodes WHERE node_key LIKE 'REQ-WP44-%' OR node_key LIKE 'TASK-WP44-%' OR created_by LIKE 'test_wp44_%';",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM audit_ledger WHERE actor_id LIKE 'test_wp44_%';",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM agent_identities WHERE agent_id LIKE 'test_wp44_%';",
                &[],
            )
            .await;
    }

    let repo_dir = format!("target/test-mcp-git-{}", Uuid::new_v4());
    let (git_write, git_read) =
        start_git_actor(&repo_dir, 64).expect("Failed to start test git actor");

    let storage = StorageRepo::new(pool.clone());
    let state = AppState::new(pool, storage, git_write, git_read);

    let cancel_token = CancellationToken::new();
    let cancel_token_clone = cancel_token.clone();
    let state_clone = state.clone();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral TCP port");
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{port}");

    tokio::spawn(async move {
        let app = create_router(state_clone);
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                cancel_token_clone.cancelled().await;
            })
            .await
            .ok();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    (base_url, cancel_token, state, guard)
}

/// Helper inserting an active requirement node for testing.
async fn insert_test_requirement(state: &AppState, node_key: &str, title: &str) -> Uuid {
    let client = state.pool.get().await.expect("Failed to get DB client");
    let id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, $2, 'REQUIREMENT', $3, 'Active test requirement for edge prevalidation', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'test_wp44_seed');",
            &[&id, &node_key, &title],
        )
        .await
        .expect("Failed to insert active test requirement");
    id
}

/// Helper registering a test agent identity.
async fn register_test_agent(state: &AppState, agent_id: &str, raw_token: &str) {
    let client = state.pool.get().await.expect("Failed to get DB client");
    let token_hash = compute_token_hash(raw_token);
    client
        .execute(
            "INSERT INTO agent_identities (agent_id, actor_type, token_hash, is_active) \
             VALUES ($1, 'AGENT', $2, true) \
             ON CONFLICT (agent_id) DO UPDATE SET token_hash = $2, is_active = true;",
            &[&agent_id, &token_hash],
        )
        .await
        .expect("Failed to register test agent");
}

#[tokio::test]
async fn test_mcp_wire_protocol_camel_case_input_schema() {
    let (base_url, cancel_token, _state, _guard) = spawn_test_server().await;
    let http = reqwest::Client::new();
    let mcp_url = format!("{base_url}/mcp");

    // 1. Send tools/list request over MCP JSON-RPC
    let req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });

    let resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&req)
        .send()
        .await
        .expect("Failed to call tools/list");

    assert_eq!(resp.status(), StatusCode::OK);
    let resp_text = resp.text().await.expect("Failed to get response body");

    // Proof Criterion 1: Zero occurrences of snake_case 'input_schema' in wire serialization
    assert!(
        !resp_text.contains("\"input_schema\""),
        "MCP tools/list wire protocol MUST NOT contain snake_case 'input_schema'"
    );

    let resp_val: Value = serde_json::from_str(&resp_text).expect("Valid JSON response");
    let result = resp_val.get("result").expect("Must contain result object");
    let tools = result.get("tools").and_then(Value::as_array).expect("Tools must be an array");

    // Proof Criterion 2: Exactly 8 tools returned, each using camelCase inputSchema
    assert_eq!(tools.len(), 8, "Must return exactly 8 registered MCP tools");

    let expected_tools = [
        "get_context_envelope",
        "query_requirements",
        "get_document_span",
        "propose_node_mutation",
        "create_subtask",
        "update_node_status",
        "revert_mutations",
        "reverify_node",
    ];

    for expected_name in &expected_tools {
        let tool = tools
            .iter()
            .find(|t| t.get("name").and_then(Value::as_str) == Some(*expected_name))
            .unwrap_or_else(|| panic!("Tool '{expected_name}' not found in tools/list"));

        let schema = tool.get("inputSchema").unwrap_or_else(|| {
            panic!("Tool '{expected_name}' missing camelCase 'inputSchema' property")
        });

        assert_eq!(
            schema.get("type").and_then(Value::as_str),
            Some("object"),
            "Tool '{expected_name}' inputSchema must be type object"
        );
        assert!(
            schema.get("properties").is_some(),
            "Tool '{expected_name}' must define properties"
        );
        assert!(
            schema.get("required").is_some(),
            "Tool '{expected_name}' must define required array"
        );
    }

    cancel_token.cancel();
}

#[tokio::test]
async fn test_mcp_edge_prevalidation_rejects_raw_git_sha() {
    let (base_url, cancel_token, state, _guard) = spawn_test_server().await;
    let http = reqwest::Client::new();
    let mcp_url = format!("{base_url}/mcp");

    let agent_id = "test_wp44_agent_preval";
    let token = "test_wp44_secret_token_12345";
    register_test_agent(&state, agent_id, token).await;

    // Seed an active requirement in graph_nodes
    let valid_req_id = insert_test_requirement(&state, "REQ-WP44-001", "Core Gateway Security").await;

    let raw_git_sha = "c0ffee1234567890abcdef1234567890abcdef12";

    // 1. Attempt create_subtask via MCP referencing raw Git SHA as parent_node_id
    let create_subtask_req = json!({
        "jsonrpc": "2.0",
        "id": 101,
        "method": "tools/call",
        "params": {
            "name": "create_subtask",
            "arguments": {
                "parent_node_id": raw_git_sha,
                "title": "Malicious Subtask Targeting Raw Commit",
                "auth_token": token
            }
        }
    });

    let resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&create_subtask_req)
        .send()
        .await
        .expect("Failed to send create_subtask");

    assert_eq!(resp.status(), StatusCode::OK);
    let resp_val: Value = resp.json().await.expect("Valid JSON response");

    let err = resp_val.get("error").expect("Must return JSON-RPC error");
    assert_eq!(err.get("code").and_then(Value::as_i64), Some(-32000));

    let err_msg = err.get("message").and_then(Value::as_str).unwrap_or_default();
    assert!(
        err_msg.contains("ERR_INVALID_EDGE_TARGET"),
        "Error message must specify ERR_INVALID_EDGE_TARGET: {err_msg}"
    );

    // Verify Actionable Remediation Envelope in data (INV-8, INV-9, TB-13.3)
    let err_data = err.get("data").expect("Error data must contain remediation envelope");
    assert_eq!(
        err_data.get("code").and_then(Value::as_str),
        Some("ERR_INVALID_EDGE_TARGET")
    );
    assert_eq!(
        err_data.get("invalid_field").and_then(Value::as_str),
        Some("to_node_id")
    );
    assert_eq!(
        err_data.get("invalid_value").and_then(Value::as_str),
        Some(raw_git_sha)
    );
    assert!(
        err_data.get("suggested_repairs").is_some(),
        "Must contain suggested_repairs"
    );
    assert!(
        err_data.get("valid_target_candidates").is_some(),
        "Must contain valid_target_candidates"
    );

    // 2. Attempt propose_node_mutation via MCP referencing raw Git SHA as target_node_id
    let propose_mutation_req = json!({
        "jsonrpc": "2.0",
        "id": 102,
        "method": "tools/call",
        "params": {
            "name": "propose_node_mutation",
            "arguments": {
                "target_node_id": raw_git_sha,
                "mutation_type": "TASK",
                "title": "Task on raw SHA",
                "auth_token": token
            }
        }
    });

    let resp2 = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&propose_mutation_req)
        .send()
        .await
        .expect("Failed to send propose_node_mutation");

    let resp2_val: Value = resp2.json().await.expect("Valid JSON response");
    let err2 = resp2_val.get("error").expect("Must return JSON-RPC error");
    assert_eq!(err2.get("code").and_then(Value::as_i64), Some(-32000));
    assert!(
        err2.get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("ERR_INVALID_EDGE_TARGET")
    );

    // 3. Attempt REST POST /api/v1/nodes/mutate with raw Git SHA -> must return HTTP 422
    let rest_mutate_req = json!({
        "target_node_id": raw_git_sha,
        "mutation_type": "TASK",
        "title": "REST Task on raw SHA"
    });

    let rest_resp = http
        .post(format!("{base_url}/api/v1/nodes/mutate"))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&rest_mutate_req)
        .send()
        .await
        .expect("Failed to post REST mutate");

    assert_eq!(
        rest_resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "REST mutation on raw Git SHA must return HTTP 422"
    );

    let rest_val: Value = rest_resp.json().await.expect("Valid JSON response");
    assert_eq!(
        rest_val.get("code").and_then(Value::as_str),
        Some("ERR_INVALID_EDGE_TARGET")
    );
    assert_eq!(
        rest_val.get("invalid_field").and_then(Value::as_str),
        Some("to_node_id")
    );
    assert_eq!(
        rest_val.get("invalid_value").and_then(Value::as_str),
        Some(raw_git_sha)
    );

    // 4. Attempt REST POST /api/v1/nodes/{id}/subtasks with non-existent UUID -> HTTP 422
    let missing_uuid = Uuid::new_v4();
    let rest_subtask_resp = http
        .post(format!("{base_url}/api/v1/nodes/{missing_uuid}/subtasks"))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "title": "Subtask targeting missing UUID"
        }))
        .send()
        .await
        .expect("Failed to post REST subtask");

    assert_eq!(
        rest_subtask_resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "Subtask targeting non-existent UUID must return HTTP 422"
    );

    // 5. Valid call with registered active requirement UUID succeeds without error
    let valid_subtask_req = json!({
        "jsonrpc": "2.0",
        "id": 103,
        "method": "tools/call",
        "params": {
            "name": "create_subtask",
            "arguments": {
                "parent_node_id": valid_req_id.to_string(),
                "title": "Legitimate Subtask Under Active Requirement",
                "auth_token": token
            }
        }
    });

    let valid_resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&valid_subtask_req)
        .send()
        .await
        .expect("Failed to send valid create_subtask");

    let valid_val: Value = valid_resp.json().await.expect("Valid JSON response");
    assert!(
        valid_val.get("result").is_some(),
        "Valid parent UUID must succeed: {:?}",
        valid_val
    );

    cancel_token.cancel();
}

#[tokio::test]
async fn test_mcp_dual_authentication_extraction() {
    let (base_url, cancel_token, state, _guard) = spawn_test_server().await;
    let http = reqwest::Client::new();
    let mcp_url = format!("{base_url}/mcp");

    let agent_id = "test_wp44_agent_dual_auth";
    let token = "test_wp44_dual_auth_secret_999";
    register_test_agent(&state, agent_id, token).await;

    let valid_req_id = insert_test_requirement(&state, "REQ-WP44-002", "Dual Auth Test Requirement").await;

    // 1. Unauthenticated attempt fails with code -32000 and ERR_AUTH_FAILED
    let unauth_req = json!({
        "jsonrpc": "2.0",
        "id": 201,
        "method": "tools/call",
        "params": {
            "name": "create_subtask",
            "arguments": {
                "parent_node_id": valid_req_id.to_string(),
                "title": "Unauthenticated Task Proposal"
            }
        }
    });

    let resp1 = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&unauth_req)
        .send()
        .await
        .expect("Failed to send unauthenticated request");

    let resp1_val: Value = resp1.json().await.expect("Valid JSON");
    let err1 = resp1_val.get("error").expect("Must return error");
    assert_eq!(err1.get("code").and_then(Value::as_i64), Some(-32000));
    assert!(
        err1.get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("ERR_AUTH_FAILED")
    );

    // 2. Authentication via HTTP Authorization: Bearer <token>
    let http_auth_req = json!({
        "jsonrpc": "2.0",
        "id": 202,
        "method": "tools/call",
        "params": {
            "name": "create_subtask",
            "arguments": {
                "parent_node_id": valid_req_id.to_string(),
                "title": "Authenticated via HTTP Bearer Header"
            }
        }
    });

    let resp2 = http
        .post(&mcp_url)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&http_auth_req)
        .send()
        .await
        .expect("Failed to send HTTP Bearer authenticated request");

    let resp2_val: Value = resp2.json().await.expect("Valid JSON");
    assert!(
        resp2_val.get("result").is_some(),
        "HTTP Bearer authenticated call must succeed: {:?}",
        resp2_val
    );

    // 3. Authentication via top-level JSON-RPC auth_token
    let top_level_auth_req = json!({
        "jsonrpc": "2.0",
        "id": 203,
        "auth_token": token,
        "method": "tools/call",
        "params": {
            "name": "create_subtask",
            "arguments": {
                "parent_node_id": valid_req_id.to_string(),
                "title": "Authenticated via top-level auth_token"
            }
        }
    });

    let resp3 = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&top_level_auth_req)
        .send()
        .await
        .expect("Failed to send top-level auth request");

    let resp3_val: Value = resp3.json().await.expect("Valid JSON");
    assert!(
        resp3_val.get("result").is_some(),
        "Top-level auth_token call must succeed: {:?}",
        resp3_val
    );

    // 4. Authentication via params.auth_token
    let params_auth_req = json!({
        "jsonrpc": "2.0",
        "id": 204,
        "method": "tools/call",
        "params": {
            "name": "create_subtask",
            "auth_token": token,
            "arguments": {
                "parent_node_id": valid_req_id.to_string(),
                "title": "Authenticated via params auth_token"
            }
        }
    });

    let resp4 = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&params_auth_req)
        .send()
        .await
        .expect("Failed to send params auth request");

    let resp4_val: Value = resp4.json().await.expect("Valid JSON");
    assert!(
        resp4_val.get("result").is_some(),
        "Params auth_token call must succeed: {:?}",
        resp4_val
    );

    cancel_token.cancel();
}

#[tokio::test]
async fn test_mcp_stdio_resilience_and_offline_initialization_error() {
    // Test that when TKS daemon is unreachable, mcp-stdio exits cleanly with code 0
    // and emits a valid JSON-RPC error response with code -32000 on stdout.
    let unreachable_port = 59999;
    let server_url = format!("http://127.0.0.1:{unreachable_port}");

    let args = McpStdioArgs {
        server_url: Some(server_url),
        auth_token: Some("dummy".to_string()),
    };

    let simulated_input = b"{\"jsonrpc\":\"2.0\",\"id\":99,\"method\":\"initialize\"}\n";
    let input_reader = tokio::io::BufReader::new(&simulated_input[..]);
    let mut output_writer = Vec::new();

    let res = run_mcp_stdio_io(args, input_reader, &mut output_writer).await;
    assert!(res.is_ok(), "run_mcp_stdio_io must exit cleanly with Ok(()) when daemon is offline");

    let out_str = String::from_utf8(output_writer).expect("Valid UTF-8 output");
    let err_json: Value = serde_json::from_str(out_str.trim()).expect("Valid JSON-RPC output");
    assert_eq!(err_json.get("id").and_then(Value::as_i64), Some(99));
    let err_obj = err_json.get("error").expect("Must contain error");
    assert_eq!(err_obj.get("code").and_then(Value::as_i64), Some(-32000));
}
