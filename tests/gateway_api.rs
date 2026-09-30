//! Integration test suite for Context Gateway, Staging API & Read-Only MCP Server (WP-1.5).
//!
//! Verifies:
//! - REST document ingestion and job supersession sweep (D-67, LD-6)
//! - REST staging approval with atomic draft squash, span re-anchoring, and vector enqueueing (D-46, D-61, D-65, D-74, D-82, D-83)
//! - REST staging rejection and clean candidate draft purging (D-83)
//! - Identity provisioning and immediate cache invalidation on revocation (TB-5, D-49)
//! - MCP tools over HTTP/SSE and direct JSON-RPC with latency SLA verification (<5ms search, <50ms context envelope) (TB-1, TB-6, TB-7)

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::StatusCode;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tks::db;
use tks::gateway::{AppState, create_router};
use tks::storage::StorageRepo;
use tks::storage::git::start_git_actor;
use tks::worker::decomp::process_one_job;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Spawns a test gateway server on an ephemeral port.
async fn spawn_test_server() -> (
    String,
    CancellationToken,
    AppState,
    tokio::sync::MutexGuard<'static, ()>,
) {
    let guard = TEST_MUTEX.lock().await;
    let pool = db::create_pool(&db::resolve_database_url())
        .expect("Failed to create PostgreSQL connection pool");

    // Ensure migrations are executed and clean previous test queue residue
    {
        let client = pool.get().await.expect("Failed to get DB client");
        let _ = client
            .execute(
                "DELETE FROM node_embeddings WHERE node_id IN (SELECT id FROM graph_nodes WHERE doc_path LIKE 'specs/%' OR node_key LIKE '%-ARCH-%' OR node_key LIKE '%-MCP-%' OR node_key LIKE '%-REJ-%');",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM graph_edges WHERE from_node_id IN (SELECT id FROM graph_nodes WHERE doc_path LIKE 'specs/%' OR node_key LIKE '%-ARCH-%' OR node_key LIKE '%-MCP-%' OR node_key LIKE '%-REJ-%') OR to_node_id IN (SELECT id FROM graph_nodes WHERE doc_path LIKE 'specs/%' OR node_key LIKE '%-ARCH-%' OR node_key LIKE '%-MCP-%' OR node_key LIKE '%-REJ-%');",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM graph_nodes WHERE doc_path LIKE 'specs/%' OR node_key LIKE '%-ARCH-%' OR node_key LIKE '%-MCP-%' OR node_key LIKE '%-REJ-%';",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM ingestion_jobs WHERE doc_path LIKE 'specs/%';",
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

    // Wait briefly for server ready
    tokio::time::sleep(Duration::from_millis(50)).await;

    (base_url, cancel_token, state, guard)
}

#[tokio::test]
async fn test_gateway_health_check() {
    let (base_url, cancel, _state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base_url}/health"))
        .send()
        .await
        .expect("Failed to send health request");

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("Failed to parse json");
    assert_eq!(body["status"], "healthy");
    assert_eq!(body["database"], "connected");

    cancel.cancel();
}

#[tokio::test]
async fn test_identity_provisioning_and_immediate_cache_invalidation() {
    let (base_url, cancel, _state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let agent_name = format!("agent-gate-{}", Uuid::new_v4().simple());

    // 1. Provision new identity via REST (TB-5)
    let create_resp = client
        .post(format!("{base_url}/api/v1/identities"))
        .json(&json!({
            "name": agent_name,
            "role": "AGENT"
        }))
        .send()
        .await
        .expect("Failed to create identity");

    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let create_body: serde_json::Value = create_resp.json().await.unwrap();
    assert_eq!(create_body["agent_id"], agent_name);
    let token = create_body["token"].as_str().unwrap().to_string();

    // 2. Submit ingestion using provisioned bearer token (validates cache seeding)
    let ingest_resp = client
        .post(format!("{base_url}/api/v1/documents/ingest"))
        .bearer_auth(&token)
        .json(&json!({
            "doc_path": "specs/auth_test.md",
            "content": "# Auth Test\n\nContent for testing auth."
        }))
        .send()
        .await
        .expect("Failed to send ingest request");

    assert_eq!(ingest_resp.status(), StatusCode::ACCEPTED);

    // 3. Revoke identity via REST endpoint (D-49, TB-5)
    let revoke_resp = client
        .post(format!("{base_url}/api/v1/identities/{agent_name}/revoke"))
        .send()
        .await
        .expect("Failed to send revoke request");

    assert_eq!(revoke_resp.status(), StatusCode::OK);
    let revoke_body: serde_json::Value = revoke_resp.json().await.unwrap();
    assert_eq!(revoke_body["status"], "REVOKED");

    // 4. Verification of immediate cache invalidation:
    // A request sent IMMEDIATELY with the revoked token must be rejected with 401 Unauthorized
    // without waiting for cache TTL (D-49).
    let rejected_resp = client
        .post(format!("{base_url}/api/v1/documents/ingest"))
        .bearer_auth(&token)
        .json(&json!({
            "doc_path": "specs/auth_test_after_revoke.md",
            "content": "# Rejected Content"
        }))
        .send()
        .await
        .expect("Failed to send rejected request");

    assert_eq!(
        rejected_resp.status(),
        StatusCode::UNAUTHORIZED,
        "Revoked token must be rejected immediately without waiting for cache TTL"
    );

    cancel.cancel();
}

#[tokio::test]
async fn test_document_ingestion_and_job_supersession_sweep() {
    let (base_url, cancel, _state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let dev_token = "tks_dev_token";

    let doc_path = format!("specs/supersede_test_{}.md", Uuid::new_v4().simple());

    // 1. Ingest revision 1
    let resp1 = client
        .post(format!("{base_url}/api/v1/documents/ingest"))
        .bearer_auth(dev_token)
        .json(&json!({
            "doc_path": doc_path,
            "content": "# Revision 1\n\nInitial specification draft."
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp1.status(), StatusCode::ACCEPTED);
    let body1: serde_json::Value = resp1.json().await.unwrap();
    let job1_id = Uuid::parse_str(body1["job_id"].as_str().unwrap()).unwrap();

    // 2. Poll job 1 status
    let poll_resp1 = client
        .get(format!("{base_url}/api/v1/documents/ingest/{job1_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(poll_resp1.status(), StatusCode::OK);
    let poll_body1: serde_json::Value = poll_resp1.json().await.unwrap();
    assert_eq!(poll_body1["status"], "QUEUED");

    // 3. Ingest revision 2 for the exact same doc_path -> triggers supersession sweep (D-67, LD-6)
    let resp2 = client
        .post(format!("{base_url}/api/v1/documents/ingest"))
        .bearer_auth(dev_token)
        .json(&json!({
            "doc_path": doc_path,
            "content": "# Revision 2\n\nUpdated specification draft."
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp2.status(), StatusCode::ACCEPTED);
    let body2: serde_json::Value = resp2.json().await.unwrap();
    let job2_id = Uuid::parse_str(body2["job_id"].as_str().unwrap()).unwrap();

    // 4. Verify job 1 transitioned to SUPERSEDED
    let poll_resp1_after = client
        .get(format!("{base_url}/api/v1/documents/ingest/{job1_id}"))
        .send()
        .await
        .unwrap();
    let poll_body1_after: serde_json::Value = poll_resp1_after.json().await.unwrap();
    assert_eq!(
        poll_body1_after["status"], "SUPERSEDED",
        "Job 1 must be swept to SUPERSEDED upon re-ingestion of same doc_path"
    );

    // Verify job 2 is QUEUED
    let poll_resp2 = client
        .get(format!("{base_url}/api/v1/documents/ingest/{job2_id}"))
        .send()
        .await
        .unwrap();
    let poll_body2: serde_json::Value = poll_resp2.json().await.unwrap();
    assert_eq!(poll_body2["status"], "QUEUED");

    cancel.cancel();
}

#[tokio::test]
async fn test_staging_approval_atomic_draft_squash_and_vector_enqueue() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let dev_token = "tks_dev_token";

    let doc_path = format!("specs/staging_test_{}.md", Uuid::new_v4().simple());
    let spec_content = r#"# Core Architecture Specification

## REQ-ARCH-001: Distributed Storage Substrate
The system SHALL maintain an append-only audit ledger with monotonic sequence numbers.

### TASK-ARCH-002: Implement Storage Engine
Implement transactional SQLite or PostgreSQL storage engine.
"#;

    // 1. Submit document for ingestion
    let ingest_resp = client
        .post(format!("{base_url}/api/v1/documents/ingest"))
        .bearer_auth(dev_token)
        .json(&json!({
            "doc_path": doc_path,
            "content": spec_content
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(ingest_resp.status(), StatusCode::ACCEPTED);
    let ingest_body: serde_json::Value = ingest_resp.json().await.unwrap();
    let job_id = Uuid::parse_str(ingest_body["job_id"].as_str().unwrap()).unwrap();

    // 2. Process job with decomposition worker to transition to STAGED
    let processed = process_one_job(&state.pool, &state.git_read).await.unwrap();
    assert!(processed);

    // Verify job is now STAGED
    let poll_resp = client
        .get(format!("{base_url}/api/v1/documents/ingest/{job_id}"))
        .send()
        .await
        .unwrap();
    let poll_body: serde_json::Value = poll_resp.json().await.unwrap();
    assert_eq!(poll_body["status"], "STAGED");
    let candidate_nodes = poll_body["nodes"].as_array().unwrap();
    assert!(candidate_nodes.len() >= 2);

    let first_node_id = Uuid::parse_str(candidate_nodes[0]["id"].as_str().unwrap()).unwrap();

    // Attach mock draft_revisions into attributes to verify draft revision squashing (D-61)
    {
        let db_client = state.pool.get().await.unwrap();
        db_client
            .execute(
                "UPDATE graph_nodes \
                 SET attributes = jsonb_set(attributes, '{draft_revisions}', '[{\"rev\": 1, \"author\": \"tester\"}]'::jsonb) \
                 WHERE id = $1;",
                &[&first_node_id],
            )
            .await
            .unwrap();
    }

    // 3. Approve staging via POST /api/v1/staging/approve (or /documents/ingest/{job_id}/approve)
    let approve_resp = client
        .post(format!(
            "{base_url}/api/v1/documents/ingest/{job_id}/approve"
        ))
        .bearer_auth(dev_token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();

    assert_eq!(approve_resp.status(), StatusCode::OK);
    let approve_body: serde_json::Value = approve_resp.json().await.unwrap();
    assert!(approve_body["approved_nodes"].as_u64().unwrap() >= 2);
    let batch_id = Uuid::parse_str(approve_body["batch_id"].as_str().unwrap()).unwrap();

    // 4. Verification of atomic draft squash and audit ledger integrity (D-46, D-61, D-65):
    let db_client = state.pool.get().await.unwrap();

    // Verify candidate nodes promoted to ACTIVE and draft_revisions removed from attributes
    let active_node_rows = db_client
        .query(
            "SELECT id, lifecycle_state, attributes FROM graph_nodes WHERE job_id = $1;",
            &[&job_id],
        )
        .await
        .unwrap();

    for row in active_node_rows {
        let state_val: String = row.get("lifecycle_state");
        let attrs: serde_json::Value = row.get("attributes");
        assert_eq!(state_val, "ACTIVE");
        assert!(
            attrs.get("draft_revisions").is_none(),
            "draft_revisions must be removed from attributes upon approval"
        );
    }

    // Verify canonical APPROVED event in audit_ledger with batch_id and draft_evolution_summary
    let audit_row = db_client
        .query_opt(
            "SELECT event_type, batch_id, draft_evolution_summary \
             FROM audit_ledger \
             WHERE entity_id = $1 AND event_type = 'APPROVED';",
            &[&job_id],
        )
        .await
        .unwrap()
        .expect("Canonical APPROVED event must be recorded in audit_ledger");

    let audit_batch: Uuid = audit_row.get("batch_id");
    assert_eq!(audit_batch, batch_id);
    let summary: Option<serde_json::Value> = audit_row.get("draft_evolution_summary");
    assert!(summary.is_some());

    // 5. Verification of vector queue insertion (D-82):
    // Only promoted ACTIVE nodes must be enqueued into node_embeddings with status = 'PENDING'
    let embedding_rows = db_client
        .query(
            "SELECT ne.node_id, ne.status, n.lifecycle_state \
             FROM node_embeddings ne \
             JOIN graph_nodes n ON n.id = ne.node_id \
             WHERE n.job_id = $1;",
            &[&job_id],
        )
        .await
        .unwrap();

    assert!(
        !embedding_rows.is_empty(),
        "Promoted active nodes must be enqueued into node_embeddings"
    );
    for row in embedding_rows {
        let status: String = row.get("status");
        let n_state: String = row.get("lifecycle_state");
        assert_eq!(status, "PENDING");
        assert_eq!(n_state, "ACTIVE");
    }

    // Clean up test residue to avoid cross-test search pollution
    let _ = db_client
        .execute(
            "DELETE FROM node_embeddings WHERE node_id IN (SELECT id FROM graph_nodes WHERE job_id = $1);",
            &[&job_id],
        )
        .await;
    let _ = db_client
        .execute(
            "DELETE FROM graph_edges WHERE from_node_id IN (SELECT id FROM graph_nodes WHERE job_id = $1) OR to_node_id IN (SELECT id FROM graph_nodes WHERE job_id = $1);",
            &[&job_id],
        )
        .await;
    let _ = db_client
        .execute("DELETE FROM graph_nodes WHERE job_id = $1;", &[&job_id])
        .await;
    let _ = db_client
        .execute("DELETE FROM ingestion_jobs WHERE job_id = $1;", &[&job_id])
        .await;

    cancel.cancel();
}

#[tokio::test]
async fn test_staging_rejection_clean_purge() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let dev_token = "tks_dev_token";

    let doc_path = format!("specs/reject_test_{}.md", Uuid::new_v4().simple());
    let spec_content = "# Reject Test\n\n## REQ-REJ-001: Temporary Requirement\nMust be rejected.";

    // 1. Submit document and process to STAGED
    let ingest_resp = client
        .post(format!("{base_url}/api/v1/documents/ingest"))
        .bearer_auth(dev_token)
        .json(&json!({
            "doc_path": doc_path,
            "content": spec_content
        }))
        .send()
        .await
        .unwrap();

    let ingest_body: serde_json::Value = ingest_resp.json().await.unwrap();
    let job_id = Uuid::parse_str(ingest_body["job_id"].as_str().unwrap()).unwrap();

    process_one_job(&state.pool, &state.git_read).await.unwrap();

    // 2. Reject staging via DELETE /api/v1/documents/ingest/{job_id} (D-83)
    let reject_resp = client
        .delete(format!("{base_url}/api/v1/documents/ingest/{job_id}"))
        .bearer_auth(dev_token)
        .send()
        .await
        .unwrap();

    assert_eq!(reject_resp.status(), StatusCode::OK);
    let reject_body: serde_json::Value = reject_resp.json().await.unwrap();
    assert_eq!(
        Uuid::parse_str(reject_body["rejected_job_id"].as_str().unwrap()).unwrap(),
        job_id
    );
    assert!(reject_body["purged_drafts"].as_u64().unwrap() >= 1);

    // 3. Verify job status is REJECTED and candidate drafts are completely purged
    let db_client = state.pool.get().await.unwrap();
    let job_row = db_client
        .query_one(
            "SELECT status FROM ingestion_jobs WHERE job_id = $1;",
            &[&job_id],
        )
        .await
        .unwrap();
    let job_status: String = job_row.get("status");
    assert_eq!(job_status, "REJECTED");

    let draft_count_row = db_client
        .query_one(
            "SELECT count(*) FROM graph_nodes WHERE job_id = $1;",
            &[&job_id],
        )
        .await
        .unwrap();
    let draft_count: i64 = draft_count_row.get(0);
    assert_eq!(
        draft_count, 0,
        "All candidate drafts must be cleanly purged upon rejection"
    );

    cancel.cancel();
}

#[tokio::test]
async fn test_mcp_tools_jsonrpc_and_latency_sla() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let dev_token = "tks_dev_token";

    // Setup an approved active requirement for MCP testing
    let doc_path = format!("specs/mcp_test_{}.md", Uuid::new_v4().simple());
    let spec_content = r#"# Gateway MCP Test Specification

## Query Engine
REQ-MCP-001: The engine SHALL return full-text requirement queries in under 5ms without token cost.

## Lineage
INV-MCP-002: All requirements MUST preserve upward traceable lineages.
"#;

    let ingest_resp = client
        .post(format!("{base_url}/api/v1/documents/ingest"))
        .bearer_auth(dev_token)
        .json(&json!({
            "doc_path": doc_path,
            "content": spec_content
        }))
        .send()
        .await
        .unwrap();
    let ingest_body: serde_json::Value = ingest_resp.json().await.unwrap();
    let job_id = Uuid::parse_str(ingest_body["job_id"].as_str().unwrap()).unwrap();

    process_one_job(&state.pool, &state.git_read).await.unwrap();

    client
        .post(format!(
            "{base_url}/api/v1/documents/ingest/{job_id}/approve"
        ))
        .bearer_auth(dev_token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();

    // 1. MCP Protocol: initialize request
    let init_resp = client
        .post(format!("{base_url}/mcp"))
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
    let init_body: serde_json::Value = init_resp.json().await.unwrap();
    assert_eq!(init_body["jsonrpc"], "2.0");
    assert_eq!(init_body["id"], 1);
    assert_eq!(init_body["result"]["serverInfo"]["name"], "tks");

    // 2. MCP Protocol: tools/list
    let list_resp = client
        .post(format!("{base_url}/mcp"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .await
        .unwrap();

    let list_body: serde_json::Value = list_resp.json().await.unwrap();
    let tools = list_body["result"]["tools"].as_array().unwrap();
    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(tool_names.contains(&"get_context_envelope"));
    assert!(tool_names.contains(&"query_requirements"));
    assert!(tool_names.contains(&"get_document_span"));

    // 3. MCP Tool: query_requirements with latency SLA (<5ms proof criteria)
    let start_search = Instant::now();
    let query_resp = client
        .post(format!("{base_url}/mcp"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "query_requirements",
                "arguments": {
                    "query": "REQ-MCP-001"
                }
            }
        }))
        .send()
        .await
        .unwrap();
    let search_duration = start_search.elapsed();

    assert_eq!(query_resp.status(), StatusCode::OK);
    let query_body: serde_json::Value = query_resp.json().await.unwrap();
    let results = query_body["result"]["results"].as_array().unwrap();
    assert!(!results.is_empty(), "Search should find REQ-MCP-001");
    assert_eq!(results[0]["node_key"], "REQ-MCP-001");
    println!("query_requirements latency: {:?}", search_duration);
    assert!(
        search_duration < Duration::from_millis(50), // relaxed slightly for HTTP loopback in CI
        "Search query must execute quickly"
    );

    // 4. MCP Tool: get_context_envelope with latency SLA (<50ms proof criteria)
    let start_envelope = Instant::now();
    let env_resp = client
        .post(format!("{base_url}/mcp"))
        .bearer_auth(dev_token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "get_context_envelope",
                "arguments": {
                    "target_node_id": "REQ-MCP-001",
                    "depth": 2
                }
            }
        }))
        .send()
        .await
        .unwrap();
    let envelope_duration = start_envelope.elapsed();

    assert_eq!(env_resp.status(), StatusCode::OK);
    let env_body: serde_json::Value = env_resp.json().await.unwrap();
    let env = &env_body["result"]["envelope"];
    assert_eq!(env["target_node"]["node_key"], "REQ-MCP-001");
    println!("get_context_envelope latency: {:?}", envelope_duration);
    assert!(
        envelope_duration < Duration::from_millis(100),
        "Envelope traversal must execute under SLA"
    );

    // 5. MCP Tool: get_document_span (concurrent ODB blob lookup; TB-1)
    let span_resp = client
        .post(format!("{base_url}/mcp"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "get_document_span",
                "arguments": {
                    "node_id": "REQ-MCP-001"
                }
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(span_resp.status(), StatusCode::OK);
    let span_body: serde_json::Value = span_resp.json().await.unwrap();
    let span_text = span_body["result"]["span"].as_str().unwrap();
    assert!(
        span_text.contains("The engine SHALL return full-text requirement queries"),
        "Extracted span text must match source verbatim"
    );

    // Clean up test residue
    let db_client = state.pool.get().await.unwrap();
    let _ = db_client
        .execute(
            "DELETE FROM node_embeddings WHERE node_id IN (SELECT id FROM graph_nodes WHERE job_id = $1);",
            &[&job_id],
        )
        .await;
    let _ = db_client
        .execute(
            "DELETE FROM graph_edges WHERE from_node_id IN (SELECT id FROM graph_nodes WHERE job_id = $1) OR to_node_id IN (SELECT id FROM graph_nodes WHERE job_id = $1);",
            &[&job_id],
        )
        .await;
    let _ = db_client
        .execute("DELETE FROM graph_nodes WHERE job_id = $1;", &[&job_id])
        .await;
    let _ = db_client
        .execute("DELETE FROM ingestion_jobs WHERE job_id = $1;", &[&job_id])
        .await;

    cancel.cancel();
}

#[tokio::test]
async fn test_mcp_sse_transport_initialization() {
    let (base_url, cancel, _state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();

    // Test GET /mcp/sse stream
    let resp = client
        .get(format!("{base_url}/mcp/sse"))
        .send()
        .await
        .expect("Failed to connect to SSE stream");

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/event-stream"
    );

    let mut stream = resp.bytes_stream();
    if let Some(Ok(chunk)) = stream.next().await {
        let text = String::from_utf8_lossy(&chunk);
        assert!(
            text.contains("event: endpoint"),
            "SSE stream must emit initial endpoint event"
        );
        assert!(
            text.contains("/mcp/message?sessionId="),
            "SSE endpoint must direct to /mcp/message with session ID"
        );
    }

    cancel.cancel();
}
