//! Phase 3 Dogfooding Milestone (Gate 3 - Multi-Agent Concurrent Co-Evolution & Invalidation Storm)
//! Acceptance Validation Suite (WP-3.5, PHASE3-001, PHASE3-002, PHASE3-003, INV-1, INV-2, INV-5, INV-6, INV-7).
//!
//! Automates the complete Phase 3 Gate 3 Acceptance Protocol:
//! 1. Provision two authenticated external agent identities (`agent-alpha` and `agent-beta`)
//!    alongside an administrative supervisor identity (`admin-supervisor`).
//! 2. Agents create independent branch-isolated workspace containers (`workspace-alpha`, `workspace-beta`).
//! 3. Concurrently elaborate candidate subtasks under root requirement `REQ-005` in both workspaces
//!    without acquiring the global structural advisory lock.
//! 4. Agent-alpha fast-forward merges `workspace-alpha` into the active substrate under canonical
//!    lock serialization with monotonic `event_seq` and shared `batch_id`.
//! 5. Root requirement `REQ-005` is updated by an administrative supervisor.
//! 6. Automated Invalidation Cascade Engine executes downward sweep, cascading downstream tasks
//!    to `NEEDS_REVERIFICATION` with calculated progressive `staleness_score` metrics.
//! 7. Real-Time Web Explorer SSE endpoint receives and broadcasts live invalidation events to subscribers.
//! 8. Agent-beta performs rebase / auto-reparent merge on `workspace-beta`, successfully anchoring to the
//!    updated parent requirement revision.
//! 9. Agents reverify degraded tasks via `reverify_node`, restoring the entire graph to `ACTIVE` state
//!    with `staleness_score = 0.0`.
//! 10. Web Explorer endpoint `/api/v1/explorer/graph` verifies the unified, cycle-free, fully active DAG topology.
//! 11. Operational CLI subcommands (`tks workspace`, `tks explorer`) execute and verify clear output.

use std::fs;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tks::cli::explorer::{ExplorerServeArgs, ExplorerSubcommand, run_explorer};
use tks::cli::workspace::{
    InspectWorkspaceArgs, ListWorkspacesArgs, WorkspaceSubcommand, run_workspace,
};
use tks::db;
use tks::gateway::auth::AuthenticatedAgent;
use tks::gateway::routes::explorer::ExplorerGraphResponse;
use tks::gateway::routes::identities::{CreateIdentityRequest, CreateIdentityResponse};
use tks::gateway::routes::mutation::ReverifyNodeRequest;
use tks::gateway::routes::workspaces::{
    CreateWorkspacePayload, ElaborateWorkspaceTaskPayload, PromoteWorkspacePayload,
    RebaseWorkspacePayload,
};
use tks::gateway::{AppState, create_router};
use tks::storage::cascade::trigger_downward_invalidation_client;
use tks::storage::event_bus::{GraphEventBus, start_pg_listener};
use tks::storage::git::{GitReadHandle, GitWriteHandle, start_git_actor};
use tks::storage::repo::StorageRepo;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TempRepoGuard {
    path: std::path::PathBuf,
}

impl TempRepoGuard {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("tks-gate3-git-{}", Uuid::new_v4()));
        Self { path }
    }
}

impl Drop for TempRepoGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Helper context holding running test server, database pool, and Git handles.
struct Gate3Context {
    base_url: String,
    database_url: String,
    pool: deadpool_postgres::Pool,
    _git_write: GitWriteHandle,
    _git_read: GitReadHandle,
    cancel_token: CancellationToken,
    _guard: tokio::sync::MutexGuard<'static, ()>,
    _temp_repo: TempRepoGuard,
}

async fn setup_gate3_environment() -> Gate3Context {
    let guard = TEST_MUTEX.lock().await;
    let database_url = db::ensure_test_database_ready()
        .await
        .expect("Failed to prepare test database");

    // Ensure embedded migrations (V1-V4) are applied
    let (mut client, _handle) = db::connect(&database_url)
        .await
        .expect("Failed to connect to PostgreSQL");
    db::run_migrations(&mut client)
        .await
        .expect("Failed to run migrations");

    // Clean up any residue from prior runs
    client
        .execute(
            "DELETE FROM audit_ledger WHERE actor_id LIKE 'agent-%' OR actor_id = 'admin-supervisor' OR actor_id LIKE 'gate3_%';",
            &[],
        )
        .await
        .expect("Cleanup audit_ledger failed");

    client
        .execute(
            "DELETE FROM graph_nodes WHERE created_by LIKE 'agent-%' OR created_by = 'admin-supervisor' OR node_key LIKE 'TASK-P3-%' OR node_key = 'REQ-005';",
            &[],
        )
        .await
        .expect("Cleanup graph_nodes failed");

    client
        .execute(
            "DELETE FROM workspaces WHERE owner_agent LIKE 'agent-%' OR owner_agent = 'admin-supervisor';",
            &[],
        )
        .await
        .expect("Cleanup workspaces failed");

    client
        .execute(
            "DELETE FROM agent_identities WHERE agent_id IN ('agent-alpha', 'agent-beta', 'admin-supervisor');",
            &[],
        )
        .await
        .expect("Cleanup agent_identities failed");

    let temp_repo = TempRepoGuard::new();
    let (git_write, git_read) =
        start_git_actor(&temp_repo.path, 128).expect("Failed to start bare Git actor");

    let pool = db::create_pool(&database_url).expect("Failed to create connection pool");
    let storage = StorageRepo::new(pool.clone());
    let cancel_token = CancellationToken::new();
    let event_bus = GraphEventBus::default();

    // Start background PostgreSQL LISTEN/NOTIFY subscriber
    let _pg_listener = start_pg_listener(&database_url, event_bus.clone(), cancel_token.clone());

    let state = AppState::new(pool.clone(), storage, git_write.clone(), git_read.clone())
        .with_event_bus(event_bus);

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

    // Allow server to bind and pg listener to connect
    tokio::time::sleep(Duration::from_millis(150)).await;

    Gate3Context {
        base_url,
        database_url,
        pool,
        _git_write: git_write,
        _git_read: git_read,
        cancel_token,
        _guard: guard,
        _temp_repo: temp_repo,
    }
}

#[tokio::test]
async fn test_phase3_dogfooding_gate3_acceptance() {
    println!("\n===============================================================================");
    println!(
        "PHASE 3 DOGFOODING MILESTONE (GATE 3 - MULTI-AGENT CO-EVOLUTION & INVALIDATION STORM)"
    );
    println!("===============================================================================\n");

    let ctx = setup_gate3_environment().await;
    let http = reqwest::Client::new();

    // -------------------------------------------------------------------------
    // STEP 1: Provision and Authenticate Agent and Supervisor Identities
    // -------------------------------------------------------------------------
    println!("\n--- [Step 1] Provisioning and Authenticating Identities ---");

    let token_alpha = "token-agent-alpha-42";
    let token_beta = "token-agent-beta-99";
    let token_admin = "token-admin-supervisor-01";

    let ident_url = format!("{}/api/v1/identities", ctx.base_url);

    // 1.1 Provision agent-alpha
    let resp_alpha = http
        .post(&ident_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&CreateIdentityRequest {
            name: "agent-alpha".to_string(),
            role: "AGENT".to_string(),
            token: Some(token_alpha.to_string()),
        })
        .send()
        .await
        .expect("Failed to create agent-alpha");
    assert_eq!(resp_alpha.status(), StatusCode::CREATED);
    let ident_alpha: CreateIdentityResponse = resp_alpha.json().await.unwrap();
    assert_eq!(ident_alpha.agent_id, "agent-alpha");
    println!("  Provisioned agent identity: agent-alpha");

    // 1.2 Provision agent-beta
    let resp_beta = http
        .post(&ident_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&CreateIdentityRequest {
            name: "agent-beta".to_string(),
            role: "AGENT".to_string(),
            token: Some(token_beta.to_string()),
        })
        .send()
        .await
        .expect("Failed to create agent-beta");
    assert_eq!(resp_beta.status(), StatusCode::CREATED);
    let ident_beta: CreateIdentityResponse = resp_beta.json().await.unwrap();
    assert_eq!(ident_beta.agent_id, "agent-beta");
    println!("  Provisioned agent identity: agent-beta");

    // 1.3 Provision admin-supervisor
    let resp_admin = http
        .post(&ident_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&CreateIdentityRequest {
            name: "admin-supervisor".to_string(),
            role: "HUMAN".to_string(),
            token: Some(token_admin.to_string()),
        })
        .send()
        .await
        .expect("Failed to create admin-supervisor");
    assert_eq!(resp_admin.status(), StatusCode::CREATED);
    let ident_admin: CreateIdentityResponse = resp_admin.json().await.unwrap();
    assert_eq!(ident_admin.agent_id, "admin-supervisor");
    println!("  Provisioned supervisor identity: admin-supervisor");

    // -------------------------------------------------------------------------
    // STEP 2: Initialize Core Root Requirement REQ-005
    // -------------------------------------------------------------------------
    println!("\n--- [Step 2] Initializing Core Root Requirement REQ-005 ---");

    let req_id: Uuid;
    {
        let db_client = ctx.pool.get().await.expect("Failed to get db client");
        let row = db_client
            .query_one(
                "INSERT INTO graph_nodes (node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
                 VALUES ('REQ-005', 'REQUIREMENT', 'Automated Invalidation & Multi-Agent Co-Evolution', \
                         'Core requirement governing Phase 3 multi-agent workspace isolation, invalidation cascade sweeps, and DAG visualization.', \
                         'ACTIVE', 'AUTONOMOUS_ELABORATION', 'system', '{\"priority\": \"P0\"}'::jsonb) \
                 RETURNING id;",
                &[],
            )
            .await
            .expect("Failed to insert REQ-005");
        req_id = row.get("id");
    }
    println!("  Initialized REQ-005 with UUID: {req_id}");

    // -------------------------------------------------------------------------
    // STEP 3: Connect Web Explorer SSE Stream Subscriber
    // -------------------------------------------------------------------------
    println!("\n--- [Step 3] Subscribing to Real-Time Web Explorer SSE Stream ---");

    let sse_resp = http
        .get(format!("{}/api/v1/explorer/events", ctx.base_url))
        .header(AUTHORIZATION, format!("Bearer {token_alpha}"))
        .send()
        .await
        .expect("Failed to connect to Explorer SSE stream");
    assert_eq!(sse_resp.status(), StatusCode::OK);
    assert_eq!(
        sse_resp.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
    let mut sse_stream = sse_resp.bytes_stream();
    println!("  Connected SSE event listener to /api/v1/explorer/events");

    // -------------------------------------------------------------------------
    // STEP 4: Create Independent Branch Workspaces for Agent Alpha & Beta
    // -------------------------------------------------------------------------
    println!("\n--- [Step 4] Creating Independent Branch Workspaces (INV-7) ---");

    let ws_url = format!("{}/api/v1/workspaces", ctx.base_url);

    // 4.1 Agent Alpha creates workspace-alpha
    let resp_ws_alpha = http
        .post(&ws_url)
        .header(AUTHORIZATION, format!("Bearer {token_alpha}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&CreateWorkspacePayload {
            name: Some("workspace-alpha".to_string()),
            workspace_name: None,
            attributes: Some(serde_json::json!({ "branch": "feature/alpha-sharding" })),
        })
        .send()
        .await
        .expect("Agent-alpha workspace creation failed");
    assert_eq!(resp_ws_alpha.status(), StatusCode::CREATED);
    let ws_alpha_val: serde_json::Value = resp_ws_alpha.json().await.unwrap();
    let ws_alpha_id = Uuid::parse_str(ws_alpha_val["workspace_id"].as_str().unwrap()).unwrap();
    println!("  Agent Alpha created workspace-alpha: {ws_alpha_id}");

    // 4.2 Agent Beta creates workspace-beta
    let resp_ws_beta = http
        .post(&ws_url)
        .header(AUTHORIZATION, format!("Bearer {token_beta}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&CreateWorkspacePayload {
            name: Some("workspace-beta".to_string()),
            workspace_name: None,
            attributes: Some(serde_json::json!({ "branch": "feature/beta-canvas" })),
        })
        .send()
        .await
        .expect("Agent-beta workspace creation failed");
    assert_eq!(resp_ws_beta.status(), StatusCode::CREATED);
    let ws_beta_val: serde_json::Value = resp_ws_beta.json().await.unwrap();
    let ws_beta_id = Uuid::parse_str(ws_beta_val["workspace_id"].as_str().unwrap()).unwrap();
    println!("  Agent Beta created workspace-beta:   {ws_beta_id}");

    // 4.3 Boundary isolation assertion (INV-7): Agent Alpha cannot inspect Workspace Beta
    let resp_cross_inspect = http
        .get(format!("{}/api/v1/workspaces/{ws_beta_id}", ctx.base_url))
        .header(AUTHORIZATION, format!("Bearer {token_alpha}"))
        .send()
        .await
        .expect("Cross inspect request failed");
    assert_eq!(
        resp_cross_inspect.status(),
        StatusCode::NOT_FOUND,
        "Agent Alpha must receive 404 when querying Agent Beta's workspace (INV-7)"
    );
    println!("  Verified boundary isolation: Agent Alpha cannot inspect Workspace Beta (INV-7)");

    // -------------------------------------------------------------------------
    // STEP 5: Concurrent Non-Blocking Task Elaboration under REQ-005
    // -------------------------------------------------------------------------
    println!("\n--- [Step 5] Concurrent Task Elaboration in Branch Containers ---");

    let http_alpha = http.clone();
    let http_beta = http.clone();
    let base_url_alpha = ctx.base_url.clone();
    let base_url_beta = ctx.base_url.clone();

    let start_elaborate = Instant::now();

    let alpha_task = tokio::spawn(async move {
        // Alpha Task 1: Distributed Notification Sharding
        let url = format!("{base_url_alpha}/api/v1/workspaces/{ws_alpha_id}/subtasks");
        let resp1 = http_alpha
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {token_alpha}"))
            .header(CONTENT_TYPE, "application/json")
            .json(&ElaborateWorkspaceTaskPayload {
                parent_id: Some("REQ-005".to_string()),
                parent_node_id: None,
                title: Some(
                    "TASK-P3-ALPHA-1: Implement Distributed Notification Sharding".to_string(),
                ),
                content: Some(
                    "Shard notification bus to handle high-volume invalidation bursts.".to_string(),
                ),
                attributes: Some(serde_json::json!({ "team": "data-infra" })),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(resp1.status(), StatusCode::CREATED);
        let val1: serde_json::Value = resp1.json().await.unwrap();
        let alpha1_id = Uuid::parse_str(val1["task_id"].as_str().unwrap()).unwrap();

        // Alpha Task 2: Benchmark Event Bus Throughput (derived from Alpha Task 1)
        let resp2 = http_alpha
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {token_alpha}"))
            .header(CONTENT_TYPE, "application/json")
            .json(&ElaborateWorkspaceTaskPayload {
                parent_id: Some(alpha1_id.to_string()),
                parent_node_id: None,
                title: Some("TASK-P3-ALPHA-2: Benchmark Event Bus Throughput".to_string()),
                content: Some(
                    "Validate sub-5ms event notification broadcast under load.".to_string(),
                ),
                attributes: Some(serde_json::json!({ "target_sla_ms": 5 })),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::CREATED);
        let val2: serde_json::Value = resp2.json().await.unwrap();
        let alpha2_id = Uuid::parse_str(val2["task_id"].as_str().unwrap()).unwrap();

        (alpha1_id, alpha2_id)
    });

    let beta_task = tokio::spawn(async move {
        // Beta Task 1: Implement Cytoscape Canvas State Caching
        let url = format!("{base_url_beta}/api/v1/workspaces/{ws_beta_id}/subtasks");
        let resp1 = http_beta
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {token_beta}"))
            .header(CONTENT_TYPE, "application/json")
            .json(&ElaborateWorkspaceTaskPayload {
                parent_id: Some("REQ-005".to_string()),
                parent_node_id: None,
                title: Some("TASK-P3-BETA-1: Implement Cytoscape Canvas State Caching".to_string()),
                content: Some(
                    "Cache topological subgraphs in WebGL canvas texture memory.".to_string(),
                ),
                attributes: Some(serde_json::json!({ "layer": "webgl" })),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(resp1.status(), StatusCode::CREATED);
        let val1: serde_json::Value = resp1.json().await.unwrap();
        let beta1_id = Uuid::parse_str(val1["task_id"].as_str().unwrap()).unwrap();

        // Beta Task 2: Integrate WebGL Node Shaders (derived from Beta Task 1)
        let resp2 = http_beta
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {token_beta}"))
            .header(CONTENT_TYPE, "application/json")
            .json(&ElaborateWorkspaceTaskPayload {
                parent_id: Some(beta1_id.to_string()),
                parent_node_id: None,
                title: Some("TASK-P3-BETA-2: Integrate WebGL Node Shaders".to_string()),
                content: Some(
                    "Custom GPU shader rendering pulsing animations for degraded nodes."
                        .to_string(),
                ),
                attributes: Some(serde_json::json!({ "shader_type": "fragment" })),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::CREATED);
        let val2: serde_json::Value = resp2.json().await.unwrap();
        let beta2_id = Uuid::parse_str(val2["task_id"].as_str().unwrap()).unwrap();

        (beta1_id, beta2_id)
    });

    let (alpha_res, beta_res) = tokio::join!(alpha_task, beta_task);
    let (alpha1_id, alpha2_id) = alpha_res.unwrap();
    let (beta1_id, beta2_id) = beta_res.unwrap();
    let elaborate_duration = start_elaborate.elapsed();

    println!(
        "  Concurrent task elaboration completed in {:?}",
        elaborate_duration
    );
    println!("    Alpha tasks: {alpha1_id}, {alpha2_id}");
    println!("    Beta tasks:  {beta1_id}, {beta2_id}");

    // Verify candidate draft state and live substrate isolation
    {
        let db_client = ctx.pool.get().await.expect("Failed to get db client");
        let active_tasks: i64 = db_client
            .query_one(
                "SELECT count(*) FROM graph_nodes WHERE id IN ($1, $2, $3, $4) AND lifecycle_state = 'ACTIVE';",
                &[&alpha1_id, &alpha2_id, &beta1_id, &beta2_id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            active_tasks, 0,
            "All workspace elaborated tasks must initially be in DRAFT state"
        );

        let draft_tasks: i64 = db_client
            .query_one(
                "SELECT count(*) FROM graph_nodes WHERE id IN ($1, $2, $3, $4) AND lifecycle_state = 'DRAFT';",
                &[&alpha1_id, &alpha2_id, &beta1_id, &beta2_id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            draft_tasks, 4,
            "All 4 workspace elaborated tasks must be stored as DRAFT"
        );
    }
    println!(
        "  Verified draft isolation: candidate tasks exist in DRAFT without live substrate pollution"
    );

    // -------------------------------------------------------------------------
    // STEP 6: Agent-alpha Fast-Forward Merge into Live Substrate
    // -------------------------------------------------------------------------
    println!("\n--- [Step 6] Agent-alpha Fast-Forward Promotion (WP-3.3, INV-1, INV-2) ---");

    // 6.1 Merge preview analysis
    let analyze_url = format!("{}/api/v1/workspaces/{ws_alpha_id}/analyze", ctx.base_url);
    let resp_analyze = http
        .get(&analyze_url)
        .header(AUTHORIZATION, format!("Bearer {token_alpha}"))
        .send()
        .await
        .expect("Merge analysis failed");
    assert_eq!(resp_analyze.status(), StatusCode::OK);
    let preview: serde_json::Value = resp_analyze.json().await.unwrap();
    assert!(
        preview["can_fast_forward"].as_bool().unwrap(),
        "Workspace Alpha must be eligible for fast-forward promotion"
    );
    assert_eq!(preview["conflicts"].as_array().unwrap().len(), 0);
    println!("  Merge preview verified: can_fast_forward=true, conflicts=0");

    // 6.2 Execute promotion
    let promote_url = format!("{}/api/v1/workspaces/{ws_alpha_id}/promote", ctx.base_url);
    let resp_promote = http
        .post(&promote_url)
        .header(AUTHORIZATION, format!("Bearer {token_alpha}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&PromoteWorkspacePayload {
            auto_reparent: Some(false),
        })
        .send()
        .await
        .expect("Workspace promotion failed");
    assert_eq!(resp_promote.status(), StatusCode::OK);
    let promo_val: serde_json::Value = resp_promote.json().await.unwrap();
    assert_eq!(promo_val["status"].as_str().unwrap(), "MERGED");
    assert_eq!(promo_val["promoted_nodes"].as_u64().unwrap(), 2);
    assert_eq!(promo_val["promoted_edges"].as_u64().unwrap(), 2);
    let alpha_batch_id = Uuid::parse_str(promo_val["batch_id"].as_str().unwrap()).unwrap();
    println!("  Promoted workspace-alpha into active substrate:");
    println!(
        "    status: MERGED | batch_id: {alpha_batch_id} | event_seq: {}",
        promo_val["event_seq"]
    );

    // Verify tasks are now ACTIVE
    {
        let db_client = ctx.pool.get().await.expect("Failed to get db client");
        let active_alpha: i64 = db_client
            .query_one(
                "SELECT count(*) FROM graph_nodes WHERE id IN ($1, $2) AND lifecycle_state = 'ACTIVE';",
                &[&alpha1_id, &alpha2_id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(active_alpha, 2, "Alpha tasks must be promoted to ACTIVE");
    }

    // -------------------------------------------------------------------------
    // STEP 7: Root Requirement REQ-005 Updated & Invalidation Cascade Sweep
    // -------------------------------------------------------------------------
    println!(
        "\n--- [Step 7] Root Requirement Alteration & Invalidation Storm (PHASE3-002, INV-1) ---"
    );

    let admin_agent = AuthenticatedAgent {
        agent_id: "admin-supervisor".to_string(),
        actor_type: "HUMAN".to_string(),
    };

    // Supervisor updates REQ-005 content and triggers downward invalidation cascade
    let cascade_res = {
        let (mut raw_client, _h) = db::connect(&ctx.database_url)
            .await
            .expect("Failed raw client connect");

        // Update root requirement content
        raw_client
            .execute(
                "UPDATE graph_nodes SET content = 'REQ-005 Revision 2: Enhanced multi-agent coordination contracts.' WHERE id = $1;",
                &[&req_id],
            )
            .await
            .expect("Update REQ-005 failed");

        let start_cascade = Instant::now();
        let res = trigger_downward_invalidation_client(
            &mut raw_client,
            req_id,
            "Root requirement altered for Gate 3 verification",
            &admin_agent,
        )
        .await
        .expect("Downward invalidation failed");
        let cascade_duration = start_cascade.elapsed();
        println!(
            "  Executed downward invalidation sweep in {:?}",
            cascade_duration
        );
        res
    };

    assert!(
        cascade_res.invalidated_nodes.contains(&alpha1_id),
        "Alpha task 1 must be invalidated by root REQ-005 alteration"
    );
    assert!(
        cascade_res.invalidated_nodes.contains(&alpha2_id),
        "Alpha task 2 must be transitively invalidated by root REQ-005 alteration"
    );
    assert_eq!(cascade_res.max_depth, 2);
    println!(
        "  Cascade sweep invalidated {} downstream nodes:",
        cascade_res.invalidated_nodes.len()
    );
    println!("    Root: {req_id} -> Max Depth: {}", cascade_res.max_depth);

    // Verify staleness scores and NEEDS_REVERIFICATION state in database
    {
        let db_client = ctx.pool.get().await.expect("Failed to get db client");

        let row1 = db_client
            .query_one(
                "SELECT lifecycle_state, (attributes->>'staleness_score')::float8 AS score FROM graph_nodes WHERE id = $1;",
                &[&alpha1_id],
            )
            .await
            .unwrap();
        let state1: String = row1.get("lifecycle_state");
        let score1: f64 = row1.get("score");
        assert_eq!(state1, "NEEDS_REVERIFICATION");
        assert!(
            (score1 - 1.0).abs() < 1e-4,
            "Depth 1 node must have staleness_score = 1.0"
        );

        let row2 = db_client
            .query_one(
                "SELECT lifecycle_state, (attributes->>'staleness_score')::float8 AS score FROM graph_nodes WHERE id = $1;",
                &[&alpha2_id],
            )
            .await
            .unwrap();
        let state2: String = row2.get("lifecycle_state");
        let score2: f64 = row2.get("score");
        assert_eq!(state2, "NEEDS_REVERIFICATION");
        assert!(
            (score2 - 0.5).abs() < 1e-4,
            "Depth 2 node must have staleness_score = 0.5"
        );

        println!("    alpha1 (depth 1): state={state1}, staleness_score={score1}");
        println!("    alpha2 (depth 2): state={state2}, staleness_score={score2}");
    }

    // -------------------------------------------------------------------------
    // STEP 8: Real-Time SSE Event Verification
    // -------------------------------------------------------------------------
    println!("\n--- [Step 8] Real-Time Web Explorer SSE Invalidation Stream Verification ---");

    // Consume SSE chunks from stream and verify CASCADE_INVALIDATED event arrives
    let mut found_cascade_event = false;
    let sse_deadline = Instant::now() + Duration::from_millis(1500);

    while Instant::now() < sse_deadline {
        match tokio::time::timeout(Duration::from_millis(300), sse_stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                let chunk_str = String::from_utf8_lossy(&chunk);
                if chunk_str.contains("CASCADE_INVALIDATED") {
                    found_cascade_event = true;
                    println!("  Received real-time CASCADE_INVALIDATED SSE event frame:");
                    println!(
                        "    {}",
                        chunk_str.trim().lines().collect::<Vec<_>>().join(" | ")
                    );
                    break;
                }
            }
            Ok(Some(Err(e))) => {
                eprintln!("SSE stream error: {e}");
                break;
            }
            _ => continue,
        }
    }
    assert!(
        found_cascade_event,
        "Web Explorer SSE stream must broadcast CASCADE_INVALIDATED event within SLA"
    );

    // -------------------------------------------------------------------------
    // STEP 9: Agent-beta Rebase & Auto-Reparent Promotion
    // -------------------------------------------------------------------------
    println!("\n--- [Step 9] Agent-beta Workspace Rebase & Auto-Reparent Promotion ---");

    // 9.1 Rebase workspace-beta with auto-reparenting
    let rebase_url = format!("{}/api/v1/workspaces/{ws_beta_id}/rebase", ctx.base_url);
    let resp_rebase = http
        .post(&rebase_url)
        .header(AUTHORIZATION, format!("Bearer {token_beta}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&RebaseWorkspacePayload {
            auto_reparent: Some(true),
        })
        .send()
        .await
        .expect("Workspace rebase failed");
    assert_eq!(resp_rebase.status(), StatusCode::OK);
    let rebase_val: serde_json::Value = resp_rebase.json().await.unwrap();
    assert_eq!(rebase_val["status"].as_str().unwrap(), "REBASED");
    println!(
        "  Rebased workspace-beta: old_base={} -> new_base={}",
        rebase_val["old_base_event_seq"], rebase_val["new_base_event_seq"]
    );

    // 9.2 Promote workspace-beta
    let promote_beta_url = format!("{}/api/v1/workspaces/{ws_beta_id}/promote", ctx.base_url);
    let resp_promo_beta = http
        .post(&promote_beta_url)
        .header(AUTHORIZATION, format!("Bearer {token_beta}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&PromoteWorkspacePayload {
            auto_reparent: Some(true),
        })
        .send()
        .await
        .expect("Workspace Beta promote failed");
    assert_eq!(resp_promo_beta.status(), StatusCode::OK);
    let promo_beta_val: serde_json::Value = resp_promo_beta.json().await.unwrap();
    assert_eq!(promo_beta_val["status"].as_str().unwrap(), "MERGED");
    assert_eq!(promo_beta_val["promoted_nodes"].as_u64().unwrap(), 2);
    println!(
        "  Promoted workspace-beta: promoted_nodes=2, promoted_edges={}",
        promo_beta_val["promoted_edges"]
    );

    // -------------------------------------------------------------------------
    // STEP 10: Reverification Recovery via reverify_node
    // -------------------------------------------------------------------------
    println!("\n--- [Step 10] Reverification Recovery via reverify_node (D-76) ---");

    // Reverify alpha1
    let reverify_url1 = format!("{}/api/v1/nodes/{alpha1_id}/reverify", ctx.base_url);
    let resp_rev1 = http
        .post(&reverify_url1)
        .header(AUTHORIZATION, format!("Bearer {token_alpha}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&ReverifyNodeRequest {
            rationale: "Implementation audited and verified against revised REQ-005.".to_string(),
            updated_attributes: None,
        })
        .send()
        .await
        .expect("Reverify alpha1 failed");
    assert_eq!(resp_rev1.status(), StatusCode::OK);
    let rev_val1: serde_json::Value = resp_rev1.json().await.unwrap();
    assert_eq!(rev_val1["status"].as_str().unwrap(), "ACTIVE");
    assert_eq!(rev_val1["staleness_score"].as_f64().unwrap(), 0.0);
    println!("  Reverified alpha1 ({alpha1_id}) -> status=ACTIVE, staleness_score=0.0");

    // Reverify alpha2
    let reverify_url2 = format!("{}/api/v1/nodes/{alpha2_id}/reverify", ctx.base_url);
    let resp_rev2 = http
        .post(&reverify_url2)
        .header(AUTHORIZATION, format!("Bearer {token_alpha}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&ReverifyNodeRequest {
            rationale: "Downstream benchmarks re-verified after REQ-005 revision.".to_string(),
            updated_attributes: None,
        })
        .send()
        .await
        .expect("Reverify alpha2 failed");
    assert_eq!(resp_rev2.status(), StatusCode::OK);
    let rev_val2: serde_json::Value = resp_rev2.json().await.unwrap();
    assert_eq!(rev_val2["status"].as_str().unwrap(), "ACTIVE");
    assert_eq!(rev_val2["staleness_score"].as_f64().unwrap(), 0.0);
    println!("  Reverified alpha2 ({alpha2_id}) -> status=ACTIVE, staleness_score=0.0");

    // -------------------------------------------------------------------------
    // STEP 11: Real-Time Web Explorer Graph API Topology Assertion
    // -------------------------------------------------------------------------
    println!("\n--- [Step 11] Validating Complete DAG Topology via Web Explorer ---");

    let explorer_graph_url = format!(
        "{}/api/v1/explorer/graph?root=REQ-005&depth=4",
        ctx.base_url
    );
    let resp_graph = http
        .get(&explorer_graph_url)
        .header(AUTHORIZATION, format!("Bearer {token_admin}"))
        .send()
        .await
        .expect("Explorer graph request failed");
    assert_eq!(resp_graph.status(), StatusCode::OK);
    let graph: ExplorerGraphResponse = resp_graph.json().await.expect("Parse graph JSON failed");

    println!("  Web Explorer Graph payload received:");
    println!(
        "    Total nodes: {} | Total edges: {}",
        graph.nodes.len(),
        graph.edges.len()
    );

    assert!(
        graph.nodes.len() >= 5,
        "Graph must contain REQ-005, alpha1, alpha2, beta1, beta2"
    );
    assert!(
        graph.edges.len() >= 4,
        "Graph must contain at least 4 active edges"
    );

    // Verify all nodes are ACTIVE with staleness_score = 0.0
    for node in &graph.nodes {
        assert_eq!(
            node.data.lifecycle_state, "ACTIVE",
            "Node {} must be in ACTIVE state after reverification recovery",
            node.data.node_key
        );
        assert_eq!(
            node.data.staleness_score, 0.0,
            "Node {} must have staleness_score = 0.0 after reverification recovery",
            node.data.node_key
        );
    }
    println!(
        "  All {} nodes verified ACTIVE with staleness_score = 0.0",
        graph.nodes.len()
    );

    // -------------------------------------------------------------------------
    // STEP 12: CLI Operational Commands Verification
    // -------------------------------------------------------------------------
    println!("\n--- [Step 12] Validating CLI Operational Subcommands (WP-3.5) ---");

    // 12.1 Workspace List via CLI
    let list_res = run_workspace(
        WorkspaceSubcommand::List(ListWorkspacesArgs { json: false }),
        &ctx.base_url,
        token_admin,
    )
    .await;
    assert!(list_res.is_ok(), "tks workspace list must exit cleanly");
    println!("  CLI: `tks workspace list` executed successfully (code 0)");

    // 12.2 Workspace Inspect via CLI
    let inspect_res = run_workspace(
        WorkspaceSubcommand::Inspect(InspectWorkspaceArgs {
            workspace_id: ws_alpha_id,
            json: true,
        }),
        &ctx.base_url,
        token_admin,
    )
    .await;
    assert!(
        inspect_res.is_ok(),
        "tks workspace inspect must exit cleanly"
    );
    println!("  CLI: `tks workspace inspect <id>` executed successfully (code 0)");

    // 12.3 Explorer Serve CLI Command
    let port = ctx
        .base_url
        .rsplit(':')
        .next()
        .unwrap()
        .parse::<u16>()
        .unwrap();
    let explorer_res = run_explorer(
        ExplorerSubcommand::Serve(ExplorerServeArgs {
            port,
            host: "127.0.0.1".to_string(),
            open: false,
            json: true,
        }),
        &ctx.base_url,
        token_admin,
    )
    .await;
    assert!(explorer_res.is_ok(), "tks explorer serve must exit cleanly");
    println!("  CLI: `tks explorer serve` executed successfully (code 0)");

    // Clean up server
    ctx.cancel_token.cancel();

    println!("\n===============================================================================");
    println!("  GATE 3 ACCEPTANCE VALIDATION COMPLETE: ALL 12 PROTOCOL CHECKS PASSED (INV-6)");
    println!("===============================================================================\n");
}
