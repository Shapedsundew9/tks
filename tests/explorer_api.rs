//! Integration test suite for Real-Time Substrate Web Explorer (WP-3.4, PHASE3-001).
//!
//! Verifies:
//! - Static assets delivery without external CDN dependencies (`/explorer`, `/explorer/app.js`, etc.)
//! - Topological graph generation and serialization for Cytoscape.js (`GET /api/v1/explorer/graph`)
//! - Response latency target verification ($<20\text{ ms}$)
//! - Multi-depth tree traversal and polymorphic root filtering (`?root=<id>&depth=<1..5>`)
//! - Candidate draft inclusion and isolation filtering (`?include_drafts=<bool>`)
//! - Live Server-Sent Events (SSE) notification streaming (`GET /api/v1/explorer/events`)
//! - Integration compatibility with slide-out node inspection (`GET /api/v1/nodes/{id}`)

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::StatusCode;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tks::db;
use tks::gateway::routes::explorer::ExplorerGraphResponse;
use tks::gateway::{AppState, create_router};
use tks::storage::event_bus::GraphChangeEvent;
use tks::storage::git::start_git_actor;
use tks::storage::{GraphEventBus, StorageRepo, start_pg_listener};

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Spawns a test gateway server on an ephemeral port with an active event bus and PostgreSQL listener.
async fn spawn_test_explorer_server() -> (
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

    // Clean previous explorer test residue
    {
        let client = pool.get().await.expect("Failed to get DB client");
        let _ = client
            .execute(
                "DELETE FROM node_embeddings WHERE node_id IN (SELECT id FROM graph_nodes WHERE node_key LIKE 'EXPL-%' OR doc_path LIKE 'specs/explorer%');",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM graph_edges WHERE from_node_id IN (SELECT id FROM graph_nodes WHERE node_key LIKE 'EXPL-%' OR doc_path LIKE 'specs/explorer%') OR to_node_id IN (SELECT id FROM graph_nodes WHERE node_key LIKE 'EXPL-%' OR doc_path LIKE 'specs/explorer%');",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM graph_nodes WHERE node_key LIKE 'EXPL-%' OR doc_path LIKE 'specs/explorer%';",
                &[],
            )
            .await;
    }

    let repo_dir = format!("target/test-explorer-git-{}", Uuid::new_v4());
    let (git_write, git_read) =
        start_git_actor(&repo_dir, 64).expect("Failed to start test git actor");

    let storage = StorageRepo::new(pool.clone());
    let cancel_token = CancellationToken::new();
    let event_bus = GraphEventBus::default();

    // Start background PostgreSQL notification tailer
    let _pg_listener_handle =
        start_pg_listener(&test_db_url, event_bus.clone(), cancel_token.clone());

    let state = AppState::new(pool, storage, git_write, git_read).with_event_bus(event_bus);

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

    // Wait briefly for server startup
    tokio::time::sleep(Duration::from_millis(60)).await;

    (base_url, cancel_token, state, guard)
}

#[tokio::test]
async fn test_explorer_static_assets_delivery() {
    let (base_url, cancel, _state, _guard) = spawn_test_explorer_server().await;
    let client = reqwest::Client::new();

    // 1. Verify GET /explorer delivers HTML interface
    let resp = client
        .get(format!("{base_url}/explorer"))
        .send()
        .await
        .expect("Failed to fetch /explorer");
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/html; charset=utf-8"
    );
    let html = resp.text().await.expect("Failed to read HTML body");
    assert!(
        html.contains("TKS Explorer"),
        "HTML must contain TKS Explorer title"
    );
    assert!(
        html.contains("Substrate DAG Canvas"),
        "HTML must contain Substrate DAG Canvas subtitle"
    );

    // Verify UI accessibility: NO external CDN dependencies
    assert!(
        !html.contains("cdnjs.cloudflare.com"),
        "Explorer must not reference external cdnjs CDN"
    );
    assert!(
        !html.contains("cdn.jsdelivr.net"),
        "Explorer must not reference external jsdelivr CDN"
    );
    assert!(
        !html.contains("unpkg.com"),
        "Explorer must not reference external unpkg CDN"
    );

    // 2. Verify GET /explorer/ trailing slash alias
    let resp_slash = client
        .get(format!("{base_url}/explorer/"))
        .send()
        .await
        .expect("Failed to fetch /explorer/");
    assert_eq!(resp_slash.status(), StatusCode::OK);

    // 3. Verify GET /explorer/app.js
    let resp_js = client
        .get(format!("{base_url}/explorer/app.js"))
        .send()
        .await
        .expect("Failed to fetch /explorer/app.js");
    assert_eq!(resp_js.status(), StatusCode::OK);
    assert_eq!(
        resp_js.headers().get("content-type").unwrap(),
        "application/javascript; charset=utf-8"
    );
    let js = resp_js.text().await.expect("Failed to read JS body");
    assert!(
        js.contains("cytoscape"),
        "app.js must contain cytoscape initialization logic"
    );

    // 4. Verify GET /explorer/style.css
    let resp_css = client
        .get(format!("{base_url}/explorer/style.css"))
        .send()
        .await
        .expect("Failed to fetch /explorer/style.css");
    assert_eq!(resp_css.status(), StatusCode::OK);
    assert_eq!(
        resp_css.headers().get("content-type").unwrap(),
        "text/css; charset=utf-8"
    );

    // 5. Verify GET /explorer/cytoscape.min.js
    let resp_cy = client
        .get(format!("{base_url}/explorer/cytoscape.min.js"))
        .send()
        .await
        .expect("Failed to fetch /explorer/cytoscape.min.js");
    assert_eq!(resp_cy.status(), StatusCode::OK);
    assert!(
        resp_cy.text().await.unwrap().contains("Cytoscape"),
        "cytoscape.min.js must contain bundled Cytoscape library"
    );

    // 6. Verify GET /explorer/dagre.min.js
    let resp_dagre = client
        .get(format!("{base_url}/explorer/dagre.min.js"))
        .send()
        .await
        .expect("Failed to fetch /explorer/dagre.min.js");
    assert_eq!(resp_dagre.status(), StatusCode::OK);

    // 7. Verify GET /explorer/cytoscape-dagre.min.js
    let resp_cydagre = client
        .get(format!("{base_url}/explorer/cytoscape-dagre.min.js"))
        .send()
        .await
        .expect("Failed to fetch /explorer/cytoscape-dagre.min.js");
    assert_eq!(resp_cydagre.status(), StatusCode::OK);

    cancel.cancel();
}

#[tokio::test]
async fn test_explorer_graph_generation_and_latency_sla() {
    let (base_url, cancel, state, _guard) = spawn_test_explorer_server().await;
    let client = reqwest::Client::new();

    // Insert test requirements and tasks in PostgreSQL
    let req_id = Uuid::new_v4();
    let task1_id = Uuid::new_v4();
    let task2_id = Uuid::new_v4();
    let edge1_id = Uuid::new_v4();
    let edge2_id = Uuid::new_v4();

    {
        let db_client = state.pool.get().await.expect("Failed to get DB client");
        db_client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
                 VALUES \
                 ($1, 'EXPL-REQ-001', 'REQUIREMENT', 'Root Explorer Requirement', 'Spec body for explorer test', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'tester', '{\"version\": 1}'::jsonb), \
                 ($2, 'EXPL-TSK-001', 'TASK', 'First Child Task', 'Task body 1', 'ACTIVE', 'HUMAN_REVIEW_REQUIRED', 'tester', '{\"priority\": \"HIGH\"}'::jsonb), \
                 ($3, 'EXPL-TSK-002', 'TASK', 'Second Child Task', 'Task body 2', 'NEEDS_REVERIFICATION', 'AUTONOMOUS_ELABORATION', 'tester', '{\"staleness_score\": 0.75, \"invalidated_by\": \"EXPL-REQ-001\"}'::jsonb);",
                &[&req_id, &task1_id, &task2_id],
            )
            .await
            .expect("Failed to insert test nodes");

        db_client
            .execute(
                "INSERT INTO graph_edges (edge_id, from_node_id, to_node_id, edge_type, lifecycle_state, created_by, attributes) \
                 VALUES \
                 ($1, $2, $3, 'FULFILLS', 'ACTIVE', 'tester', '{}'::jsonb), \
                 ($4, $5, $3, 'CONSTRAINED_BY', 'ACTIVE', 'tester', '{}'::jsonb);",
                &[&edge1_id, &task1_id, &req_id, &edge2_id, &task2_id],
            )
            .await
            .expect("Failed to insert test edges");
    }

    // Warm-up query
    let _ = client
        .get(format!("{base_url}/api/v1/explorer/graph"))
        .send()
        .await
        .expect("Warmup query failed");

    // Performance assertion: graph generation executes in < 20ms SLA
    let start = Instant::now();
    let resp = client
        .get(format!("{base_url}/api/v1/explorer/graph"))
        .send()
        .await
        .expect("Failed to send explorer graph request");
    let elapsed = start.elapsed();

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    if status != StatusCode::OK {
        eprintln!("Error response: status={status}, body={body_text}");
    }
    assert_eq!(status, StatusCode::OK);
    assert!(
        elapsed < Duration::from_millis(20),
        "Explorer graph generation must complete within 20ms SLA (took {:?})",
        elapsed
    );

    let graph: ExplorerGraphResponse =
        serde_json::from_str(&body_text).expect("Failed to parse graph JSON");
    assert!(
        graph.nodes.len() >= 3,
        "Graph must contain at least 3 test nodes"
    );
    assert!(
        graph.edges.len() >= 2,
        "Graph must contain at least 2 test edges"
    );

    // Verify node attributes and Cytoscape format
    let root_node = graph
        .nodes
        .iter()
        .find(|n| n.data.node_key == "EXPL-REQ-001")
        .expect("EXPL-REQ-001 must be present in nodes");
    assert_eq!(root_node.data.node_type, "REQUIREMENT");
    assert_eq!(root_node.data.lifecycle_state, "ACTIVE");
    assert_eq!(root_node.data.governance_policy, "AUTONOMOUS_ELABORATION");
    assert_eq!(root_node.data.staleness_score, 0.0);
    assert!(root_node.classes.as_ref().unwrap().contains("active"));

    // Verify degraded node staleness score extraction
    let stale_node = graph
        .nodes
        .iter()
        .find(|n| n.data.node_key == "EXPL-TSK-002")
        .expect("EXPL-TSK-002 must be present in nodes");
    assert_eq!(stale_node.data.lifecycle_state, "NEEDS_REVERIFICATION");
    assert_eq!(stale_node.data.staleness_score, 0.75);
    assert!(
        stale_node.classes.as_ref().unwrap().contains("pulsing"),
        "Degraded node must have pulsing class"
    );

    // Verify edge attributes and directed link
    let fulfills_edge = graph
        .edges
        .iter()
        .find(|e| e.data.id == edge1_id.to_string())
        .expect("edge1_id must be present in edges");
    assert_eq!(fulfills_edge.data.source, task1_id.to_string());
    assert_eq!(fulfills_edge.data.target, req_id.to_string());
    assert_eq!(fulfills_edge.data.edge_type, "FULFILLS");
    assert!(fulfills_edge.classes.as_ref().unwrap().contains("fulfills"));

    cancel.cancel();
}

#[tokio::test]
async fn test_explorer_graph_polymorphic_root_and_depth_bounding() {
    let (base_url, cancel, state, _guard) = spawn_test_explorer_server().await;
    let client = reqwest::Client::new();

    // Create a 3-level tree: Root (L0) <- Task1 (L1) <- Task2 (L2) <- Task3 (L3)
    let n0 = Uuid::new_v4();
    let n1 = Uuid::new_v4();
    let n2 = Uuid::new_v4();
    let n3 = Uuid::new_v4();

    {
        let db_client = state.pool.get().await.expect("Failed to get DB client");
        db_client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, lifecycle_state, governance_policy, created_by) \
                 VALUES \
                 ($1, 'EXPL-TREE-ROOT', 'REQUIREMENT', 'Root Node', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'tester'), \
                 ($2, 'EXPL-TREE-L1', 'TASK', 'Level 1 Task', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'tester'), \
                 ($3, 'EXPL-TREE-L2', 'TASK', 'Level 2 Task', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'tester'), \
                 ($4, 'EXPL-TREE-L3', 'TASK', 'Level 3 Task', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'tester');",
                &[&n0, &n1, &n2, &n3],
            )
            .await
            .expect("Failed to insert hierarchy nodes");

        db_client
            .execute(
                "INSERT INTO graph_edges (edge_id, from_node_id, to_node_id, edge_type, lifecycle_state, created_by) \
                 VALUES \
                 (gen_random_uuid(), $1, $2, 'FULFILLS', 'ACTIVE', 'tester'), \
                 (gen_random_uuid(), $3, $1, 'FULFILLS', 'ACTIVE', 'tester'), \
                 (gen_random_uuid(), $4, $3, 'FULFILLS', 'ACTIVE', 'tester');",
                &[&n1, &n0, &n2, &n3],
            )
            .await
            .expect("Failed to insert hierarchy edges");
    }

    // 1. Query root by canonical key with depth = 1: should discover Root (L0) and L1 only
    let resp_d1 = client
        .get(format!(
            "{base_url}/api/v1/explorer/graph?root=EXPL-TREE-ROOT&depth=1"
        ))
        .send()
        .await
        .expect("Failed to query depth 1");
    let status_d1 = resp_d1.status();
    let body_d1 = resp_d1.text().await.unwrap_or_default();
    if status_d1 != StatusCode::OK {
        eprintln!(
            "test_explorer_graph_polymorphic_root_and_depth_bounding error: {status_d1}, body: {body_d1}"
        );
    }
    assert_eq!(status_d1, StatusCode::OK);
    let g1: ExplorerGraphResponse = serde_json::from_str(&body_d1).unwrap();
    let g1_keys: Vec<String> = g1.nodes.into_iter().map(|n| n.data.node_key).collect();
    assert!(g1_keys.contains(&"EXPL-TREE-ROOT".to_string()));
    assert!(g1_keys.contains(&"EXPL-TREE-L1".to_string()));
    assert!(
        !g1_keys.contains(&"EXPL-TREE-L2".to_string()),
        "Depth 1 must exclude L2"
    );
    assert!(
        !g1_keys.contains(&"EXPL-TREE-L3".to_string()),
        "Depth 1 must exclude L3"
    );

    // 2. Query root by canonical key with depth = 2: should discover Root, L1, and L2
    let resp_d2 = client
        .get(format!(
            "{base_url}/api/v1/explorer/graph?root=EXPL-TREE-ROOT&depth=2"
        ))
        .send()
        .await
        .expect("Failed to query depth 2");
    assert_eq!(resp_d2.status(), StatusCode::OK);
    let g2: ExplorerGraphResponse = resp_d2.json().await.unwrap();
    let g2_keys: Vec<String> = g2.nodes.into_iter().map(|n| n.data.node_key).collect();
    assert!(g2_keys.contains(&"EXPL-TREE-ROOT".to_string()));
    assert!(g2_keys.contains(&"EXPL-TREE-L1".to_string()));
    assert!(g2_keys.contains(&"EXPL-TREE-L2".to_string()));
    assert!(
        !g2_keys.contains(&"EXPL-TREE-L3".to_string()),
        "Depth 2 must exclude L3"
    );

    // 3. Query root by UUID with depth = 3: should discover all 4 levels
    let resp_d3 = client
        .get(format!(
            "{base_url}/api/v1/explorer/graph?root={n0}&depth=3"
        ))
        .send()
        .await
        .expect("Failed to query depth 3 by UUID");
    assert_eq!(resp_d3.status(), StatusCode::OK);
    let g3: ExplorerGraphResponse = resp_d3.json().await.unwrap();
    let g3_keys: Vec<String> = g3.nodes.into_iter().map(|n| n.data.node_key).collect();
    assert!(g3_keys.contains(&"EXPL-TREE-ROOT".to_string()));
    assert!(g3_keys.contains(&"EXPL-TREE-L1".to_string()));
    assert!(g3_keys.contains(&"EXPL-TREE-L2".to_string()));
    assert!(g3_keys.contains(&"EXPL-TREE-L3".to_string()));

    // 4. Query non-existent root -> 404 NOT_FOUND
    let resp_404 = client
        .get(format!(
            "{base_url}/api/v1/explorer/graph?root=EXPL-DOES-NOT-EXIST"
        ))
        .send()
        .await
        .expect("Failed to query non-existent root");
    assert_eq!(resp_404.status(), StatusCode::NOT_FOUND);

    cancel.cancel();
}

#[tokio::test]
async fn test_explorer_graph_draft_filtering() {
    let (base_url, cancel, state, _guard) = spawn_test_explorer_server().await;
    let client = reqwest::Client::new();

    let active_id = Uuid::new_v4();
    let draft_id = Uuid::new_v4();
    let draft_edge_id = Uuid::new_v4();

    {
        let db_client = state.pool.get().await.expect("Failed to get DB client");
        db_client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, lifecycle_state, governance_policy, created_by) \
                 VALUES \
                 ($1, 'EXPL-ACTIVE-NODE', 'REQUIREMENT', 'Active Requirement', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'tester'), \
                 ($2, 'EXPL-DRAFT-NODE', 'TASK', 'Candidate Workspace Draft', 'DRAFT', 'AUTONOMOUS_ELABORATION', 'tester');",
                &[&active_id, &draft_id],
            )
            .await
            .expect("Failed to insert active and draft nodes");

        db_client
            .execute(
                "INSERT INTO graph_edges (edge_id, from_node_id, to_node_id, edge_type, lifecycle_state, created_by) \
                 VALUES ($1, $2, $3, 'FULFILLS', 'DRAFT', 'tester');",
                &[&draft_edge_id, &draft_id, &active_id],
            )
            .await
            .expect("Failed to insert draft edge");
    }

    // 1. Without include_drafts (default false): draft node and edge must be excluded
    let resp_no_drafts = client
        .get(format!(
            "{base_url}/api/v1/explorer/graph?root=EXPL-ACTIVE-NODE&include_drafts=false"
        ))
        .send()
        .await
        .expect("Failed to query without drafts");
    assert_eq!(resp_no_drafts.status(), StatusCode::OK);
    let g_no_drafts: ExplorerGraphResponse = resp_no_drafts.json().await.unwrap();
    let keys: Vec<String> = g_no_drafts
        .nodes
        .into_iter()
        .map(|n| n.data.node_key)
        .collect();
    assert!(keys.contains(&"EXPL-ACTIVE-NODE".to_string()));
    assert!(
        !keys.contains(&"EXPL-DRAFT-NODE".to_string()),
        "DRAFT node must be excluded when include_drafts=false"
    );
    assert!(
        g_no_drafts.edges.is_empty(),
        "DRAFT edge must be excluded when include_drafts=false"
    );

    // 2. With include_drafts=true: draft node and edge must be included
    let resp_with_drafts = client
        .get(format!(
            "{base_url}/api/v1/explorer/graph?root=EXPL-ACTIVE-NODE&include_drafts=true"
        ))
        .send()
        .await
        .expect("Failed to query with drafts");
    assert_eq!(resp_with_drafts.status(), StatusCode::OK);
    let g_with_drafts: ExplorerGraphResponse = resp_with_drafts.json().await.unwrap();
    let keys_with: Vec<String> = g_with_drafts
        .nodes
        .into_iter()
        .map(|n| n.data.node_key)
        .collect();
    assert!(keys_with.contains(&"EXPL-ACTIVE-NODE".to_string()));
    assert!(
        keys_with.contains(&"EXPL-DRAFT-NODE".to_string()),
        "DRAFT node must be included when include_drafts=true"
    );
    assert_eq!(
        g_with_drafts.edges.len(),
        1,
        "DRAFT edge must be included when include_drafts=true"
    );

    cancel.cancel();
}

#[tokio::test]
async fn test_explorer_real_time_sse_event_delivery() {
    let (base_url, cancel, state, _guard) = spawn_test_explorer_server().await;
    let client = reqwest::Client::new();

    // 1. Connect to live SSE event stream
    let resp = client
        .get(format!("{base_url}/api/v1/explorer/events"))
        .send()
        .await
        .expect("Failed to connect to SSE stream");

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/event-stream"
    );

    let mut stream = resp.bytes_stream();

    // 2. Publish a live graph change event onto the event bus
    let test_node_id = Uuid::new_v4();
    let batch_id = Uuid::new_v4();
    let event = GraphChangeEvent {
        event_seq: 142,
        batch_id,
        event_type: "CASCADE_INVALIDATED".to_string(),
        entity_id: test_node_id,
        entity_type: "REQUIREMENT".to_string(),
        actor_id: "agent-alpha".to_string(),
        timestamp: chrono::Utc::now(),
    };

    let start_publish = Instant::now();
    let receivers = state.event_bus.publish(event);
    assert!(
        receivers > 0,
        "At least one receiver (SSE stream) must be connected"
    );

    // 3. Receive emitted SSE event frame with latency SLA verification (<10ms)
    let sse_chunk = tokio::time::timeout(Duration::from_millis(500), stream.next())
        .await
        .expect("Timed out waiting for SSE event")
        .expect("Stream unexpectedly ended")
        .expect("Failed to read SSE chunk");

    let elapsed = start_publish.elapsed();
    assert!(
        elapsed < Duration::from_millis(15),
        "SSE broadcast delivery should complete within ~10ms (took {:?})",
        elapsed
    );

    let chunk_text = String::from_utf8_lossy(&sse_chunk);
    assert!(
        chunk_text.contains("event: graph_event"),
        "SSE chunk must have 'event: graph_event'"
    );
    assert!(
        chunk_text.contains("CASCADE_INVALIDATED"),
        "SSE chunk must contain event type"
    );
    assert!(
        chunk_text.contains(&test_node_id.to_string()),
        "SSE chunk must contain target entity ID"
    );
    assert!(
        chunk_text.contains("142"),
        "SSE chunk must contain monotonic event sequence number"
    );

    cancel.cancel();
}

#[tokio::test]
async fn test_explorer_node_inspector_endpoint_compatibility() {
    let (base_url, cancel, state, _guard) = spawn_test_explorer_server().await;
    let client = reqwest::Client::new();

    let node_id = Uuid::new_v4();

    {
        let db_client = state.pool.get().await.expect("Failed to get DB client");
        db_client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
                 VALUES ($1, 'EXPL-INSPECT-001', 'REQUIREMENT', 'Inspectable Node', 'Verbatim content for inspector test', 'ACTIVE', 'LOCKED', 'supervisor', '{\"tag\": \"core\"}'::jsonb);",
                &[&node_id],
            )
            .await
            .expect("Failed to insert inspectable node");
    }

    // Call GET /api/v1/nodes/{id} to verify compatibility with explorer slide-out inspector
    let resp = client
        .get(format!("{base_url}/api/v1/nodes/{node_id}"))
        .send()
        .await
        .expect("Failed to inspect node");

    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("Failed to parse JSON");
    assert_eq!(body["node"]["node_key"], "EXPL-INSPECT-001");
    assert_eq!(body["node"]["governance_policy"], "LOCKED");
    assert_eq!(
        body["node"]["content"],
        "Verbatim content for inspector test"
    );

    cancel.cancel();
}
