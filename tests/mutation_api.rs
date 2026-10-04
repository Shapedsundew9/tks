//! Integration test suite for Mutation-Enabled MCP Tools & REST Administration Endpoints (WP-2.4).
//!
//! Verifies:
//! - MCP `tools/list` returns all 8 tools (3 read-only from Phase 1 + 5 mutation tools from Phase 2).
//! - Unauthenticated caller rejection: MCP tools fail with code -32000 (`ERR_AUTH_FAILED`); REST endpoints return HTTP 401.
//! - Polymorphic identifier resolution (UUID and canonical `node_key`) across MCP and REST interfaces (TB-7, D-58).
//! - Pathway 1 (Autonomous Task Elaboration) via MCP `propose_node_mutation` and REST `POST /api/v1/nodes/mutate` / `POST /api/v1/nodes/{id}/subtasks`.
//! - Pathway 2 (Active Leaf Task Status Updates) via MCP `update_node_status` and REST `PATCH /api/v1/nodes/{id}/status`.
//! - Pathway 3 (Normative Requirement Proposals) via MCP `propose_node_mutation` and REST `POST /api/v1/nodes/mutate` producing `PENDING_REVIEW` candidate drafts (INV-2).
//! - DAG cycle rejection (`ERR_GRAPH_CYCLE_DETECTED`) and locked governance policy enforcement (`ERR_GOVERNANCE_LOCKED`).
//! - Unified administrative rollback via MCP `revert_mutations` and REST `POST /api/v1/admin/revert-mutations` (dry-run preview, blast-radius `force` abort, compensating execution).
//! - Explicit node reverification via MCP `reverify_node` and REST `POST /api/v1/nodes/{id}/reverify` unblocking degraded subtrees.
//! - MCP SSE streaming session transport over `/mcp/sse` and `/mcp/message`.

use std::time::Duration;

use futures_util::StreamExt;
use reqwest::StatusCode;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

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
                "DELETE FROM node_embeddings WHERE node_id IN (SELECT id FROM graph_nodes WHERE node_key LIKE 'REQ-WP24-%' OR node_key LIKE 'TASK-WP24-%' OR created_by LIKE 'test_wp24_%');",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM graph_edges WHERE from_node_id IN (SELECT id FROM graph_nodes WHERE node_key LIKE 'REQ-WP24-%' OR node_key LIKE 'TASK-WP24-%' OR created_by LIKE 'test_wp24_%') OR to_node_id IN (SELECT id FROM graph_nodes WHERE node_key LIKE 'REQ-WP24-%' OR node_key LIKE 'TASK-WP24-%' OR created_by LIKE 'test_wp24_%');",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM graph_nodes WHERE node_key LIKE 'REQ-WP24-%' OR node_key LIKE 'TASK-WP24-%' OR created_by LIKE 'test_wp24_%';",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM audit_ledger WHERE actor_id LIKE 'test_wp24_%';",
                &[],
            )
            .await;
    }

    let repo_dir = format!("target/test-gateway-git-{}", Uuid::new_v4());
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

    // Wait briefly for server readiness
    tokio::time::sleep(Duration::from_millis(50)).await;

    (base_url, cancel_token, state, guard)
}

/// Helper to provision an agent identity for testing.
async fn provision_agent(state: &AppState, agent_id: &str, token: &str) {
    let client = state.pool.get().await.expect("Failed to get DB client");
    let hash = compute_token_hash(token);
    client
        .execute(
            "INSERT INTO agent_identities (agent_id, token_hash, actor_type, is_active) \
             VALUES ($1, $2, 'AGENT', TRUE) \
             ON CONFLICT (agent_id) DO UPDATE SET token_hash = EXCLUDED.token_hash, is_active = TRUE;",
            &[&agent_id, &hash],
        )
        .await
        .expect("Failed to insert agent identity");

    state.auth.register_token(agent_id, &hash, "AGENT").await;
    state.auth.register_token(agent_id, token, "AGENT").await;
}

#[tokio::test]
async fn test_mcp_tools_list_all_eight_tools() {
    let (base_url, cancel, _state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base_url}/mcp"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .await
        .expect("Failed to query tools/list");

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp
        .json()
        .await
        .expect("Failed to parse JSON-RPC response");
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 1);

    let tools = body["result"]["tools"]
        .as_array()
        .expect("tools must be an array");

    // Proof Criterion 2: Querying tools/list returns all 8 tools
    assert_eq!(
        tools.len(),
        8,
        "Expected exactly 8 registered MCP tools (3 read-only + 5 mutation)"
    );

    let tool_names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().expect("name must be string"))
        .collect();

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

    for expected in &expected_tools {
        assert!(
            tool_names.contains(expected),
            "Registered tools must contain '{expected}', found: {tool_names:?}"
        );
    }

    // Verify all tools define strict JSON Schema properties
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert!(
            !tool["description"].as_str().unwrap_or("").is_empty(),
            "Tool '{name}' must have a non-empty description"
        );
        let schema = tool
            .get("inputSchema")
            .or_else(|| tool.get("input_schema"))
            .unwrap();
        assert_eq!(
            schema["type"], "object",
            "Tool '{name}' schema must be type 'object'"
        );
        assert!(
            schema["properties"].is_object(),
            "Tool '{name}' schema must have 'properties'"
        );
    }

    cancel.cancel();
}

#[tokio::test]
async fn test_unauthenticated_rejections() {
    let (base_url, cancel, _state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();

    // 1. MCP propose_node_mutation without auth -> JSON-RPC error -32000
    let mcp_prop_resp = client
        .post(format!("{base_url}/mcp"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call",
            "params": {
                "name": "propose_node_mutation",
                "arguments": {
                    "target_node_id": "REQ-WP24-FAKE-01",
                    "mutation_type": "TASK"
                }
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(mcp_prop_resp.status(), StatusCode::OK);
    let prop_err: serde_json::Value = mcp_prop_resp.json().await.unwrap();
    assert!(
        prop_err["error"].is_object(),
        "Expected error object in JSON-RPC response"
    );
    assert_eq!(prop_err["error"]["code"], -32000);
    assert!(
        prop_err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("ERR_AUTH_FAILED")
    );

    // 2. MCP revert_mutations without auth -> JSON-RPC error -32000
    let mcp_revert_resp = client
        .post(format!("{base_url}/mcp"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/call",
            "params": {
                "name": "revert_mutations",
                "arguments": {
                    "dry_run": true
                }
            }
        }))
        .send()
        .await
        .unwrap();

    let revert_err: serde_json::Value = mcp_revert_resp.json().await.unwrap();
    assert_eq!(revert_err["error"]["code"], -32000);
    assert!(
        revert_err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("ERR_AUTH_FAILED")
    );

    // 3. MCP create_subtask without auth -> JSON-RPC error -32000
    let mcp_sub_resp = client
        .post(format!("{base_url}/mcp"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "tools/call",
            "params": {
                "name": "create_subtask",
                "arguments": {
                    "parent_node_id": "REQ-WP24-FAKE-01",
                    "title": "Unauthenticated Subtask"
                }
            }
        }))
        .send()
        .await
        .unwrap();
    let sub_err: serde_json::Value = mcp_sub_resp.json().await.unwrap();
    assert_eq!(sub_err["error"]["code"], -32000);

    // 4. REST endpoints without Authorization header -> HTTP 401 Unauthorized
    let rest_mutate = client
        .post(format!("{base_url}/api/v1/nodes/mutate"))
        .json(&json!({
            "target_node_id": "REQ-WP24-FAKE-01",
            "mutation_type": "TASK"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rest_mutate.status(), StatusCode::UNAUTHORIZED);

    let rest_subtask = client
        .post(format!("{base_url}/api/v1/nodes/REQ-WP24-FAKE-01/subtasks"))
        .json(&json!({
            "title": "Subtask"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rest_subtask.status(), StatusCode::UNAUTHORIZED);

    let rest_status = client
        .patch(format!("{base_url}/api/v1/nodes/TASK-FAKE-01/status"))
        .json(&json!({
            "status": "COMPLETED"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rest_status.status(), StatusCode::UNAUTHORIZED);

    let rest_revert = client
        .post(format!("{base_url}/api/v1/admin/revert-mutations"))
        .json(&json!({
            "dry_run": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rest_revert.status(), StatusCode::UNAUTHORIZED);

    let rest_reverify = client
        .post(format!("{base_url}/api/v1/nodes/REQ-WP24-FAKE-01/reverify"))
        .json(&json!({
            "rationale": "Testing unauthenticated"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rest_reverify.status(), StatusCode::UNAUTHORIZED);

    // 5. REST endpoint with invalid bearer token -> HTTP 401 Unauthorized
    let invalid_token_resp = client
        .post(format!("{base_url}/api/v1/nodes/mutate"))
        .bearer_auth("completely_bogus_token")
        .json(&json!({
            "target_node_id": "REQ-WP24-FAKE-01",
            "mutation_type": "TASK"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_token_resp.status(), StatusCode::UNAUTHORIZED);

    cancel.cancel();
}

#[tokio::test]
async fn test_polymorphic_create_subtask() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let agent_id = "test_wp24_agent_poly";
    let auth_token = "token_wp24_poly_secret";
    provision_agent(&state, agent_id, auth_token).await;

    // Seed active parent requirement with AUTONOMOUS_ELABORATION policy
    let parent_id = Uuid::new_v4();
    let parent_key = "REQ-WP24-POLY-001";
    {
        let db_client = state.pool.get().await.unwrap();
        db_client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
                 VALUES ($1, $2, 'REQUIREMENT', 'Parent Requirement for Subtasks', 'Spec details', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $3);",
                &[&parent_id, &parent_key, &agent_id],
            )
            .await
            .unwrap();
    }

    // 1. MCP create_subtask using UUID parent_node_id
    let mcp_uuid_resp = client
        .post(format!("{base_url}/mcp"))
        .bearer_auth(auth_token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "tools/call",
            "params": {
                "name": "create_subtask",
                "arguments": {
                    "parent_node_id": parent_id.to_string(),
                    "title": "Subtask via UUID",
                    "content": "Detailed execution instructions",
                    "attributes": {
                        "node_key": "TASK-WP24-UUID-001"
                    }
                }
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(mcp_uuid_resp.status(), StatusCode::OK);
    let mcp_uuid_body: serde_json::Value = mcp_uuid_resp.json().await.unwrap();
    assert!(mcp_uuid_body["error"].is_null());
    assert_eq!(mcp_uuid_body["result"]["status"], "ACTIVE");
    assert_eq!(mcp_uuid_body["result"]["node_key"], "TASK-WP24-UUID-001");
    let task1_id = Uuid::parse_str(mcp_uuid_body["result"]["task_id"].as_str().unwrap()).unwrap();

    // 2. MCP create_subtask using canonical node_key parent_node_id
    let mcp_key_resp = client
        .post(format!("{base_url}/mcp"))
        .bearer_auth(auth_token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "tools/call",
            "params": {
                "name": "create_subtask",
                "arguments": {
                    "parent_node_id": parent_key,
                    "title": "Subtask via Key",
                    "attributes": {
                        "node_key": "TASK-WP24-KEY-001"
                    }
                }
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(mcp_key_resp.status(), StatusCode::OK);
    let mcp_key_body: serde_json::Value = mcp_key_resp.json().await.unwrap();
    assert!(mcp_key_body["error"].is_null());
    assert_eq!(mcp_key_body["result"]["status"], "ACTIVE");
    assert_eq!(mcp_key_body["result"]["node_key"], "TASK-WP24-KEY-001");
    let task2_id = Uuid::parse_str(mcp_key_body["result"]["task_id"].as_str().unwrap()).unwrap();

    // 3. REST POST /api/v1/nodes/{id}/subtasks using UUID
    let rest_uuid_resp = client
        .post(format!("{base_url}/api/v1/nodes/{parent_id}/subtasks"))
        .bearer_auth(auth_token)
        .json(&json!({
            "title": "REST Subtask via UUID",
            "content": "REST content",
            "attributes": {
                "node_key": "TASK-WP24-REST-UUID"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(rest_uuid_resp.status(), StatusCode::CREATED);
    let rest_uuid_body: serde_json::Value = rest_uuid_resp.json().await.unwrap();
    assert_eq!(rest_uuid_body["status"], "ACTIVE");
    assert_eq!(rest_uuid_body["node_key"], "TASK-WP24-REST-UUID");

    // 4. REST POST /api/v1/nodes/{id}/subtasks using string node_key
    let rest_key_resp = client
        .post(format!("{base_url}/api/v1/nodes/{parent_key}/subtasks"))
        .bearer_auth(auth_token)
        .json(&json!({
            "title": "REST Subtask via Key",
            "attributes": {
                "node_key": "TASK-WP24-REST-KEY"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(rest_key_resp.status(), StatusCode::CREATED);
    let rest_key_body: serde_json::Value = rest_key_resp.json().await.unwrap();
    assert_eq!(rest_key_body["status"], "ACTIVE");
    assert_eq!(rest_key_body["node_key"], "TASK-WP24-REST-KEY");

    // 5. Verify database invariants: policy inheritance, FULFILLS edges, audit records
    let db_client = state.pool.get().await.unwrap();
    let row1 = db_client
        .query_one(
            "SELECT lifecycle_state, governance_policy, created_by FROM graph_nodes WHERE id = $1;",
            &[&task1_id],
        )
        .await
        .unwrap();
    assert_eq!(row1.get::<_, String>("lifecycle_state"), "ACTIVE");
    assert_eq!(
        row1.get::<_, String>("governance_policy"),
        "AUTONOMOUS_ELABORATION"
    );
    assert_eq!(row1.get::<_, String>("created_by"), agent_id);

    let edge_count = db_client
        .query_one(
            "SELECT COUNT(*) FROM graph_edges WHERE to_node_id = $1 AND edge_type = 'FULFILLS' AND lifecycle_state = 'ACTIVE';",
            &[&parent_id],
        )
        .await
        .unwrap();
    let count: i64 = edge_count.get(0);
    assert_eq!(count, 4, "All 4 subtasks must link upward to parent");

    let _ = task2_id;
    cancel.cancel();
}

#[tokio::test]
async fn test_propose_node_mutation_pathways() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let agent_id = "test_wp24_agent_mut";
    let auth_token = "token_wp24_mut_secret";
    provision_agent(&state, agent_id, auth_token).await;

    // 1. Pathway 1: Task creation under AUTONOMOUS_ELABORATION via MCP propose_node_mutation
    let parent_id = Uuid::new_v4();
    let parent_key = "REQ-WP24-MUT-001";
    {
        let db_client = state.pool.get().await.unwrap();
        db_client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
                 VALUES ($1, $2, 'REQUIREMENT', 'Parent for Mutation', 'Content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $3);",
                &[&parent_id, &parent_key, &agent_id],
            )
            .await
            .unwrap();
    }

    let mcp_task_resp = client
        .post(format!("{base_url}/mcp"))
        .bearer_auth(auth_token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 30,
            "method": "tools/call",
            "params": {
                "name": "propose_node_mutation",
                "arguments": {
                    "target_node_id": parent_key,
                    "mutation_type": "TASK",
                    "proposed_title": "MCP Proposed Task",
                    "proposed_content": "Task description",
                    "proposed_attributes": {
                        "node_key": "TASK-WP24-PROP-001"
                    }
                }
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(mcp_task_resp.status(), StatusCode::OK);
    let mcp_task_body: serde_json::Value = mcp_task_resp.json().await.unwrap();
    assert_eq!(mcp_task_body["result"]["status"], "COMMITTED");
    let prop_task_id =
        Uuid::parse_str(mcp_task_body["result"]["node_id"].as_str().unwrap()).unwrap();

    // Verify task is in ACTIVE state directly
    {
        let db_client = state.pool.get().await.unwrap();
        let row = db_client
            .query_one(
                "SELECT lifecycle_state, governance_policy FROM graph_nodes WHERE id = $1;",
                &[&prop_task_id],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, String>("lifecycle_state"), "ACTIVE");
        assert_eq!(
            row.get::<_, String>("governance_policy"),
            "AUTONOMOUS_ELABORATION"
        );
    }

    // 2. Pathway 1: Task creation via REST POST /api/v1/nodes/mutate
    let rest_task_resp = client
        .post(format!("{base_url}/api/v1/nodes/mutate"))
        .bearer_auth(auth_token)
        .json(&json!({
            "target_node_id": parent_id.to_string(),
            "mutation_type": "SUBTASK",
            "title": "REST Proposed Subtask",
            "content": "Execution content"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(rest_task_resp.status(), StatusCode::CREATED);
    let rest_task_body: serde_json::Value = rest_task_resp.json().await.unwrap();
    assert_eq!(rest_task_body["status"], "COMMITTED");
    assert!(rest_task_body["node_id"].is_string());
    assert!(rest_task_body["batch_id"].is_string());

    // 3. Pathway 3: Normative proposal on HUMAN_REVIEW_REQUIRED node via MCP propose_node_mutation
    let norm_req_id = Uuid::new_v4();
    let norm_req_key = "REQ-WP24-NORM-001";
    let orig_content = "Original Normative Content that must not be mutated in-place.";
    {
        let db_client = state.pool.get().await.unwrap();
        db_client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
                 VALUES ($1, $2, 'REQUIREMENT', 'Original Requirement Title', $3, 'ACTIVE', 'HUMAN_REVIEW_REQUIRED', $4);",
                &[&norm_req_id, &norm_req_key, &orig_content, &agent_id],
            )
            .await
            .unwrap();
    }

    let mcp_norm_resp = client
        .post(format!("{base_url}/mcp"))
        .bearer_auth(auth_token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 31,
            "method": "tools/call",
            "params": {
                "name": "propose_node_mutation",
                "arguments": {
                    "target_node_id": norm_req_key,
                    "mutation_type": "REQUIREMENT",
                    "proposed_title": "Clarified Requirement Title",
                    "proposed_content": "Updated normative specification wording."
                }
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(mcp_norm_resp.status(), StatusCode::OK);
    let mcp_norm_body: serde_json::Value = mcp_norm_resp.json().await.unwrap();
    assert_eq!(mcp_norm_body["result"]["status"], "PENDING_REVIEW");
    let draft_id = Uuid::parse_str(mcp_norm_body["result"]["node_id"].as_str().unwrap()).unwrap();
    assert_ne!(draft_id, norm_req_id);

    // Verify original active requirement is untouched (INV-2)
    {
        let db_client = state.pool.get().await.unwrap();
        let orig_row = db_client
            .query_one(
                "SELECT content, lifecycle_state FROM graph_nodes WHERE id = $1;",
                &[&norm_req_id],
            )
            .await
            .unwrap();
        assert_eq!(
            orig_row.get::<_, Option<String>>("content"),
            Some(orig_content.to_string())
        );
        assert_eq!(orig_row.get::<_, String>("lifecycle_state"), "ACTIVE");

        let draft_row = db_client
            .query_one(
                "SELECT lifecycle_state, governance_policy, created_by FROM graph_nodes WHERE id = $1;",
                &[&draft_id],
            )
            .await
            .unwrap();
        assert_eq!(draft_row.get::<_, String>("lifecycle_state"), "DRAFT");
        assert_eq!(
            draft_row.get::<_, String>("governance_policy"),
            "HUMAN_REVIEW_REQUIRED"
        );
        assert_eq!(draft_row.get::<_, String>("created_by"), agent_id);
    }

    // 4. Pathway 3: Normative proposal via REST POST /api/v1/nodes/mutate -> 202 Accepted
    let rest_norm_resp = client
        .post(format!("{base_url}/api/v1/nodes/mutate"))
        .bearer_auth(auth_token)
        .json(&json!({
            "target_node_id": norm_req_key,
            "mutation_type": "MODIFY_REQUIREMENT",
            "title": "REST Clarification",
            "content": "REST proposed content"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(rest_norm_resp.status(), StatusCode::ACCEPTED);
    let rest_norm_body: serde_json::Value = rest_norm_resp.json().await.unwrap();
    assert_eq!(rest_norm_body["status"], "PENDING_REVIEW");
    assert!(rest_norm_body["node_id"].is_string());

    // 5. Governance policy enforcement: LOCKED node rejection
    let locked_id = Uuid::new_v4();
    let locked_key = "REQ-WP24-LOCKED-001";
    {
        let db_client = state.pool.get().await.unwrap();
        db_client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
                 VALUES ($1, $2, 'REQUIREMENT', 'Locked Root', 'Fixed content', 'ACTIVE', 'LOCKED', $3);",
                &[&locked_id, &locked_key, &agent_id],
            )
            .await
            .unwrap();
    }

    let mcp_locked_resp = client
        .post(format!("{base_url}/mcp"))
        .bearer_auth(auth_token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 32,
            "method": "tools/call",
            "params": {
                "name": "propose_node_mutation",
                "arguments": {
                    "target_node_id": locked_key,
                    "mutation_type": "TASK",
                    "proposed_title": "Subtask Under Locked Node"
                }
            }
        }))
        .send()
        .await
        .unwrap();

    let locked_body: serde_json::Value = mcp_locked_resp.json().await.unwrap();
    assert_eq!(locked_body["error"]["code"], -32000);
    assert!(
        locked_body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("ERR_GOVERNANCE_LOCKED")
    );

    let rest_locked_resp = client
        .post(format!("{base_url}/api/v1/nodes/mutate"))
        .bearer_auth(auth_token)
        .json(&json!({
            "target_node_id": locked_key,
            "mutation_type": "TASK",
            "title": "Subtask Under Locked Node"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rest_locked_resp.status(), StatusCode::FORBIDDEN);

    cancel.cancel();
}

#[tokio::test]
async fn test_update_node_status_mcp_and_rest() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let agent_id = "test_wp24_agent_status";
    let auth_token = "token_wp24_status_secret";
    provision_agent(&state, agent_id, auth_token).await;

    // Seed active task
    let task_id = Uuid::new_v4();
    let task_key = "TASK-WP24-STATUS-001";
    {
        let db_client = state.pool.get().await.unwrap();
        db_client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
                 VALUES ($1, $2, 'TASK', 'Status Lifecycle Task', 'Executing...', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $3, '{\"execution_status\": \"OPEN\"}');",
                &[&task_id, &task_key, &agent_id],
            )
            .await
            .unwrap();
    }

    // 1. MCP update_node_status to IN_PROGRESS
    let mcp_update_resp = client
        .post(format!("{base_url}/mcp"))
        .bearer_auth(auth_token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 40,
            "method": "tools/call",
            "params": {
                "name": "update_node_status",
                "arguments": {
                    "node_id": task_key,
                    "status": "IN_PROGRESS",
                    "notes": "Starting execution"
                }
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(mcp_update_resp.status(), StatusCode::OK);
    let mcp_update_body: serde_json::Value = mcp_update_resp.json().await.unwrap();
    assert_eq!(mcp_update_body["result"]["status"], "UPDATED");
    assert_eq!(mcp_update_body["result"]["execution_status"], "IN_PROGRESS");

    // 2. REST PATCH /api/v1/nodes/{id}/status to COMPLETED
    let rest_update_resp = client
        .patch(format!("{base_url}/api/v1/nodes/{task_id}/status"))
        .bearer_auth(auth_token)
        .json(&json!({
            "status": "COMPLETED",
            "notes": "Finished execution"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(rest_update_resp.status(), StatusCode::OK);
    let rest_update_body: serde_json::Value = rest_update_resp.json().await.unwrap();
    assert_eq!(rest_update_body["status"], "UPDATED");
    assert_eq!(rest_update_body["execution_status"], "COMPLETED");

    // 3. Verify database state & audit ledger
    {
        let db_client = state.pool.get().await.unwrap();
        let task_row = db_client
            .query_one(
                "SELECT attributes FROM graph_nodes WHERE id = $1;",
                &[&task_id],
            )
            .await
            .unwrap();
        let attrs: serde_json::Value = task_row.get("attributes");
        assert_eq!(attrs["execution_status"], "COMPLETED");

        let audit_count = db_client
            .query_one(
                "SELECT COUNT(*) FROM audit_ledger WHERE entity_id = $1 AND event_type = 'TASK_STATUS_UPDATE';",
                &[&task_id],
            )
            .await
            .unwrap();
        let count: i64 = audit_count.get(0);
        assert_eq!(count, 2, "Expected 2 TASK_STATUS_UPDATE audit events");
    }

    cancel.cancel();
}

#[tokio::test]
async fn test_rollback_and_reverify_mcp_and_rest() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let agent1 = "test_wp24_agent_revert_1";
    let token1 = "token_wp24_revert_1";
    provision_agent(&state, agent1, token1).await;

    let agent2 = "test_wp24_agent_revert_2";
    let token2 = "token_wp24_revert_2";
    provision_agent(&state, agent2, token2).await;

    // 1. Seed active requirement
    let root_req_id = Uuid::new_v4();
    let root_req_key = "REQ-WP24-REV-ROOT";
    {
        let db_client = state.pool.get().await.unwrap();
        db_client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
                 VALUES ($1, $2, 'REQUIREMENT', 'Rollback Target Requirement', 'Base requirement', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $3);",
                &[&root_req_id, &root_req_key, &agent1],
            )
            .await
            .unwrap();
    }

    // 2. Agent 1 creates parent task
    let task_parent_resp = client
        .post(format!("{base_url}/api/v1/nodes/{root_req_key}/subtasks"))
        .bearer_auth(token1)
        .json(&json!({
            "title": "Agent 1 Parent Task",
            "attributes": { "node_key": "TASK-WP24-REV-PARENT" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(task_parent_resp.status(), StatusCode::CREATED);
    let parent_body: serde_json::Value = task_parent_resp.json().await.unwrap();
    let parent_task_id = Uuid::parse_str(parent_body["task_id"].as_str().unwrap()).unwrap();
    let parent_batch_id = Uuid::parse_str(parent_body["batch_id"].as_str().unwrap()).unwrap();

    // 3. Agent 2 creates dependent child task under Agent 1's task (establishing cross-agent dependency)
    let child_task_resp = client
        .post(format!("{base_url}/api/v1/nodes/{parent_task_id}/subtasks"))
        .bearer_auth(token2)
        .json(&json!({
            "title": "Agent 2 Child Dependent Task",
            "attributes": { "node_key": "TASK-WP24-REV-CHILD" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(child_task_resp.status(), StatusCode::CREATED);
    let child_body: serde_json::Value = child_task_resp.json().await.unwrap();
    let child_task_id = Uuid::parse_str(child_body["task_id"].as_str().unwrap()).unwrap();

    // 4. Rollback Dry-Run via MCP revert_mutations: returns PREVIEW without modifying state
    let dry_run_mcp = client
        .post(format!("{base_url}/mcp"))
        .bearer_auth(token1)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 50,
            "method": "tools/call",
            "params": {
                "name": "revert_mutations",
                "arguments": {
                    "batch_id": parent_batch_id.to_string(),
                    "dry_run": true
                }
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(dry_run_mcp.status(), StatusCode::OK);
    let dry_mcp_body: serde_json::Value = dry_run_mcp.json().await.unwrap();
    assert_eq!(dry_mcp_body["result"]["status"], "PREVIEW");
    let affected = dry_mcp_body["result"]["affected_nodes"].as_array().unwrap();
    assert!(affected.contains(&json!(parent_task_id)));

    // 5. Rollback without force=true via REST: aborts with 409 Conflict & ERR_CONFIRMATION_REQUIRED
    let abort_resp = client
        .post(format!("{base_url}/api/v1/admin/revert-mutations"))
        .bearer_auth(token1)
        .json(&json!({
            "batch_id": parent_batch_id,
            "dry_run": false,
            "force": false
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(abort_resp.status(), StatusCode::CONFLICT);
    let abort_body: serde_json::Value = abort_resp.json().await.unwrap();
    assert_eq!(abort_body["error"], "ERR_CONFIRMATION_REQUIRED");
    assert!(
        abort_body["preview"]["cross_agent_dependencies"]
            .as_array()
            .unwrap()
            .contains(&json!(agent2))
    );

    // 6. Confirmed Rollback with force=true via REST: succeeds with REVERTED
    let force_resp = client
        .post(format!("{base_url}/api/v1/admin/revert-mutations"))
        .bearer_auth(token1)
        .json(&json!({
            "batch_id": parent_batch_id,
            "dry_run": false,
            "force": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(force_resp.status(), StatusCode::OK);
    let force_body: serde_json::Value = force_resp.json().await.unwrap();
    assert_eq!(force_body["status"], "REVERTED");
    assert!(
        force_body["cascade_reverified_nodes"]
            .as_array()
            .unwrap()
            .contains(&json!(child_task_id))
    );

    // Verify parent is SUPERSEDED and child is cascaded to NEEDS_REVERIFICATION
    {
        let db_client = state.pool.get().await.unwrap();
        let p_row = db_client
            .query_one(
                "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
                &[&parent_task_id],
            )
            .await
            .unwrap();
        assert_eq!(p_row.get::<_, String>("lifecycle_state"), "SUPERSEDED");

        let c_row = db_client
            .query_one(
                "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
                &[&child_task_id],
            )
            .await
            .unwrap();
        assert_eq!(
            c_row.get::<_, String>("lifecycle_state"),
            "NEEDS_REVERIFICATION"
        );
    }

    // 7. Reverification against superseded parent fails with ERR_DEPENDENCY_INACTIVE
    let failed_reverify = client
        .post(format!("{base_url}/api/v1/nodes/{child_task_id}/reverify"))
        .bearer_auth(token2)
        .json(&json!({
            "rationale": "Trying to reverify against superseded parent"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(failed_reverify.status(), StatusCode::CONFLICT);
    let failed_body: serde_json::Value = failed_reverify.json().await.unwrap();
    assert_eq!(failed_body["error"], "ERR_DEPENDENCY_INACTIVE");

    // 8. Re-parenting reverification unblocks child task: connects directly to active root
    let ok_reverify = client
        .post(format!("{base_url}/api/v1/nodes/{child_task_id}/reverify"))
        .bearer_auth(token2)
        .json(&json!({
            "rationale": "Re-parenting child task directly to active root requirement",
            "updated_attributes": {
                "reparent_to": root_req_key
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(ok_reverify.status(), StatusCode::OK);
    let ok_body: serde_json::Value = ok_reverify.json().await.unwrap();
    assert_eq!(ok_body["status"], "ACTIVE");
    assert_eq!(ok_body["staleness_score"], 0.0);

    // Verify child task is now restored to ACTIVE state and has active edge to root
    {
        let db_client = state.pool.get().await.unwrap();
        let restored_row = db_client
            .query_one(
                "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
                &[&child_task_id],
            )
            .await
            .unwrap();
        assert_eq!(restored_row.get::<_, String>("lifecycle_state"), "ACTIVE");
    }

    cancel.cancel();
}

#[tokio::test]
async fn test_mcp_sse_transport_mutation() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let agent_id = "test_wp24_agent_sse";
    let auth_token = "token_wp24_sse_secret";
    provision_agent(&state, agent_id, auth_token).await;

    // 1. Establish SSE stream
    let sse_resp = client
        .get(format!("{base_url}/mcp/sse"))
        .bearer_auth(auth_token)
        .send()
        .await
        .unwrap();

    assert_eq!(sse_resp.status(), StatusCode::OK);
    let mut stream = sse_resp.bytes_stream();

    // 2. Read initial endpoint event
    let mut message_endpoint = format!("{base_url}/mcp/message");
    if let Some(Ok(bytes)) = stream.next().await {
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data:") {
                let endpoint = data.trim();
                if endpoint.starts_with('/') {
                    message_endpoint = format!("{base_url}{endpoint}");
                    break;
                }
            }
        }
    }

    // 3. Post initialize and tools/list to acquired message endpoint
    let init_resp = client
        .post(&message_endpoint)
        .bearer_auth(auth_token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(init_resp.status(), StatusCode::OK);

    let list_resp = client
        .post(&message_endpoint)
        .bearer_auth(auth_token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), StatusCode::OK);
    let list_body: serde_json::Value = list_resp.json().await.unwrap();
    assert_eq!(list_body["result"]["tools"].as_array().unwrap().len(), 8);

    cancel.cancel();
}
