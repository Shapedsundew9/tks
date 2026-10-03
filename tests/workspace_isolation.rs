//! Integration test suite for Multi-Agent Workspace Isolation & Ephemeral Branch Containers (WP-3.2).
//!
//! Validates:
//! 1. Workspace lifecycle management (`create_workspace`, `get_workspace`, `list_workspaces`, `discard_workspace`).
//! 2. Autonomous candidate task elaboration (`elaborate_in_workspace`) creating `DRAFT` nodes and edges
//!    tagged with `attributes->'workspace_id'` and audit ledger entries, completely bypassing global advisory locks.
//! 3. Multi-agent boundary isolation & draft confidentiality (INV-7): candidate tasks in Workspace A
//!    are invisible to external agents querying the substrate or Workspace B.
//! 4. Context envelope and full-text search candidate overlay: authoring agent sees in-progress workspace drafts
//!    overlaid onto the live substrate, while external agents see only clean active state.
//! 5. REST API verification across `/api/v1/workspaces`, `/api/v1/workspaces/{id}`, and `/api/v1/workspaces/{id}/subtasks`.
//! 6. MCP tool verification: `create_subtask`, `propose_node_mutation`, `get_context_envelope`, and `query_requirements` with `workspace_id`.
//! 7. Concurrency verification: 10 external agents simultaneously creating workspaces and elaborating 50 subtasks each
//!    (500 total candidate nodes/edges) complete with zero lock wait timeouts or deadlock errors (`40P01`).

use std::time::Duration;

use reqwest::StatusCode;
use serde_json::json;
use tokio_postgres::Client;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tks::db;
use tks::gateway::auth::{AuthenticatedAgent, compute_token_hash};
use tks::gateway::{AppState, create_router};
use tks::storage::envelope::assemble_context_envelope_workspace;
use tks::storage::git::start_git_actor;
use tks::storage::mutation::MutationError;
use tks::storage::repo::StorageRepo;
use tks::storage::search::query_requirements_with_workspace;
use tks::storage::workspace::{
    create_workspace, create_workspace_client, discard_workspace, discard_workspace_client,
    elaborate_in_workspace, elaborate_in_workspace_client, get_workspace, get_workspace_client,
    get_workspace_edges, get_workspace_nodes, list_workspaces, list_workspaces_client,
};

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Helper to serialize database access across tests, connect, run migrations, and clean up test data.
async fn setup_test_db() -> (
    tokio::sync::MutexGuard<'static, ()>,
    deadpool_postgres::Pool,
    Client,
    String,
) {
    let guard = TEST_MUTEX.lock().await;
    let database_url = db::ensure_test_database_ready()
        .await
        .expect("Failed to prepare test db");
    let (mut client, _handle) = db::connect(&database_url)
        .await
        .expect("Failed to connect to database");

    // Ensure migrations (including V4) are up to date
    db::run_migrations(&mut client)
        .await
        .expect("Failed to run migrations");

    // Clean up previous test artifacts
    client
        .execute(
            "DELETE FROM audit_ledger WHERE actor_id LIKE 'test_wp32_%';",
            &[],
        )
        .await
        .ok();
    client
        .execute(
            "DELETE FROM graph_edges WHERE created_by LIKE 'test_wp32_%';",
            &[],
        )
        .await
        .ok();
    client
        .execute(
            "DELETE FROM graph_nodes WHERE created_by LIKE 'test_wp32_%' OR node_key LIKE 'REQ-WP32-%' OR node_key LIKE 'TASK-WP32-%';",
            &[],
        )
        .await
        .ok();
    client
        .execute(
            "DELETE FROM workspaces WHERE owner_agent LIKE 'test_wp32_%';",
            &[],
        )
        .await
        .ok();

    let pool = db::create_pool(&database_url).expect("Failed to create connection pool");

    (guard, pool, client, database_url)
}

/// Spawns a test gateway server on an ephemeral TCP port for REST and MCP integration tests.
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
    let (mut migration_client, _handle) = db::connect(&test_db_url)
        .await
        .expect("Failed to connect for migrations");
    db::run_migrations(&mut migration_client)
        .await
        .expect("Failed to run migrations");

    let pool = db::create_pool(&test_db_url).expect("Failed to create PostgreSQL connection pool");

    // Clean test residue
    {
        let client = pool.get().await.expect("Failed to get DB client");
        let _ = client
            .execute(
                "DELETE FROM audit_ledger WHERE actor_id LIKE 'test_wp32_%';",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM graph_edges WHERE created_by LIKE 'test_wp32_%';",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM graph_nodes WHERE created_by LIKE 'test_wp32_%' OR node_key LIKE 'REQ-WP32-%' OR node_key LIKE 'TASK-WP32-%';",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM workspaces WHERE owner_agent LIKE 'test_wp32_%';",
                &[],
            )
            .await;
    }

    let repo_dir = format!("target/test-workspace-git-{}", Uuid::new_v4());
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

    state.auth.register_token(agent_id, token, "AGENT").await;
}

#[tokio::test]
async fn test_workspace_crud_lifecycle() {
    let (_guard, pool, mut client, _db_url) = setup_test_db().await;

    let agent_a = AuthenticatedAgent {
        agent_id: "test_wp32_agent_crud_a".to_string(),
        actor_type: "AGENT".to_string(),
    };
    let agent_b = AuthenticatedAgent {
        agent_id: "test_wp32_agent_crud_b".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Create workspace via create_workspace_client
    let ws = create_workspace_client(&mut client, "branch-feature-a", None, &agent_a)
        .await
        .expect("Failed to create workspace");

    assert_eq!(ws.workspace_name, "branch-feature-a");
    assert_eq!(ws.owner_agent, "test_wp32_agent_crud_a");
    assert_eq!(ws.status, "ACTIVE");
    assert!(ws.base_event_seq >= 0);

    // 2. Query workspace with get_workspace_client
    let fetched = get_workspace_client(&client, ws.id)
        .await
        .expect("Failed to fetch workspace")
        .expect("Workspace should exist");
    assert_eq!(fetched.id, ws.id);
    assert_eq!(fetched.owner_agent, ws.owner_agent);

    // 3. List workspaces for agent_a vs agent_b
    let list_a = list_workspaces_client(&client, Some(&agent_a.agent_id))
        .await
        .expect("Failed to list workspaces for agent A");
    assert_eq!(list_a.len(), 1);
    assert_eq!(list_a[0].id, ws.id);

    let list_b = list_workspaces_client(&client, Some(&agent_b.agent_id))
        .await
        .expect("Failed to list workspaces for agent B");
    assert_eq!(list_b.len(), 0);

    // 4. Test deadpool client parity for create, list, and get
    let mut pool_client = pool.get().await.expect("Failed to get pool client");
    let ws_pooled = create_workspace(&mut pool_client, "branch-feature-pooled", &agent_a)
        .await
        .expect("Failed to create workspace with pooled client");
    assert_eq!(ws_pooled.workspace_name, "branch-feature-pooled");

    let fetched_pooled = get_workspace(&mut pool_client, ws_pooled.id)
        .await
        .expect("Failed to get workspace with pooled client")
        .expect("Workspace must exist");
    assert_eq!(fetched_pooled.id, ws_pooled.id);

    let list_a_pooled = list_workspaces(&mut pool_client, Some(&agent_a.agent_id))
        .await
        .expect("Failed to list workspaces with pooled client");
    assert_eq!(list_a_pooled.len(), 2);

    // 5. Discard workspace
    let discarded = discard_workspace(&mut pool_client, ws.id, &agent_a)
        .await
        .expect("Failed to discard workspace");
    assert_eq!(discarded.status, "DISCARDED");

    // 6. Verify non-owner cannot discard agent_a's remaining workspace
    let discard_err = discard_workspace(&mut pool_client, ws_pooled.id, &agent_b)
        .await
        .expect_err("Agent B must not be allowed to discard Agent A's workspace");
    match discard_err {
        MutationError::GovernanceRejected(msg) => {
            assert!(msg.contains("not authorized"));
        }
        other => panic!("Expected GovernanceRejected, got: {other:?}"),
    }

    // 7. Discard remaining workspace with client variant
    let discarded_client = discard_workspace_client(&mut client, ws_pooled.id, &agent_a)
        .await
        .expect("Failed to discard workspace with client");
    assert_eq!(discarded_client.status, "DISCARDED");
}

#[tokio::test]
async fn test_workspace_candidate_elaboration_and_audit() {
    let (_guard, pool, mut client, _db_url) = setup_test_db().await;

    let agent = AuthenticatedAgent {
        agent_id: "test_wp32_agent_elab".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // Seed active parent requirement in substrate
    let parent_id = Uuid::new_v4();
    let parent_key = "REQ-WP32-ELAB-001";
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, $2, 'REQUIREMENT', 'Parent Root Requirement', 'Requirement Content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $3);",
            &[&parent_id, &parent_key, &agent.agent_id],
        )
        .await
        .expect("Failed to seed parent requirement");

    // Create workspace
    let ws = create_workspace_client(&mut client, "ws-elab-test", None, &agent)
        .await
        .expect("Failed to create workspace");

    // Elaborate subtask inside workspace
    let res = elaborate_in_workspace_client(
        &mut client,
        ws.id,
        parent_key,
        "Candidate Task 1",
        Some("Detailed candidate task execution plan"),
        Some(json!({ "estimated_hours": 4 })),
        &agent,
    )
    .await
    .expect("Failed to elaborate in workspace");

    assert_eq!(res.lifecycle_state, "DRAFT");
    assert_eq!(res.status, "OPEN");

    // Verify candidate task node in database
    let node_row = client
        .query_one(
            "SELECT lifecycle_state, (attributes->>'workspace_id') as ws_id, created_by FROM graph_nodes WHERE id = $1;",
            &[&res.task_id],
        )
        .await
        .expect("Failed to query created task node");
    let lifecycle: String = node_row.get("lifecycle_state");
    let ws_id_attr: Option<String> = node_row.get("ws_id");
    let created_by: String = node_row.get("created_by");

    assert_eq!(lifecycle, "DRAFT");
    assert_eq!(ws_id_attr, Some(ws.id.to_string()));
    assert_eq!(created_by, agent.agent_id);

    // Verify candidate edge in database
    let edge_row = client
        .query_one(
            "SELECT lifecycle_state, (attributes->>'workspace_id') as ws_id FROM graph_edges WHERE edge_id = $1;",
            &[&res.edge_id],
        )
        .await
        .expect("Failed to query created edge");
    let edge_lifecycle: String = edge_row.get("lifecycle_state");
    let edge_ws_id: Option<String> = edge_row.get("ws_id");

    assert_eq!(edge_lifecycle, "DRAFT");
    assert_eq!(edge_ws_id, Some(ws.id.to_string()));

    // Verify audit ledger entry
    let audit_row = client
        .query_one(
            "SELECT event_type, entity_type, actor_id FROM audit_ledger WHERE entity_id = $1 ORDER BY event_seq DESC LIMIT 1;",
            &[&res.task_id],
        )
        .await
        .expect("Failed to query audit ledger");
    let op: String = audit_row.get("event_type");
    let ent_type: String = audit_row.get("entity_type");
    let actor_id: String = audit_row.get("actor_id");

    assert_eq!(op, "WORKSPACE_TASK_ELABORATED");
    assert_eq!(ent_type, "TASK");
    assert_eq!(actor_id, agent.agent_id);

    // Verify iterative elaboration: subtask 2 can take candidate task 1 as its parent
    let mut pool_client = pool.get().await.expect("Failed to get pool client");
    let res_child = elaborate_in_workspace(
        &mut pool_client,
        ws.id,
        &res.task_id.to_string(),
        "Candidate Task 1.1 (Nested Child)",
        Some("Nested subtask"),
        None,
        &agent,
    )
    .await
    .expect("Should allow elaborating under draft task within same workspace");

    assert_eq!(res_child.lifecycle_state, "DRAFT");

    // Verify get_workspace_nodes and get_workspace_edges
    let ws_nodes = get_workspace_nodes(&client, ws.id)
        .await
        .expect("Failed to get workspace nodes");
    assert_eq!(ws_nodes.len(), 2);

    let ws_edges = get_workspace_edges(&client, ws.id)
        .await
        .expect("Failed to get workspace edges");
    assert_eq!(ws_edges.len(), 2);
}

#[tokio::test]
async fn test_multi_agent_boundary_isolation_and_draft_confidentiality() {
    let (_guard, pool, mut client, _db_url) = setup_test_db().await;

    let agent_alpha = AuthenticatedAgent {
        agent_id: "test_wp32_agent_alpha".to_string(),
        actor_type: "AGENT".to_string(),
    };
    let agent_beta = AuthenticatedAgent {
        agent_id: "test_wp32_agent_beta".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // Seed parent requirement
    let parent_id = Uuid::new_v4();
    let parent_key = "REQ-WP32-ISOLATION-001";
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, $2, 'REQUIREMENT', 'Parent Requirement for Isolation Test', 'Isolation spec', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'system');",
            &[&parent_id, &parent_key],
        )
        .await
        .expect("Failed to seed parent requirement");

    // Agent Alpha creates Workspace Alpha
    let ws_alpha = create_workspace_client(&mut client, "ws-alpha", None, &agent_alpha)
        .await
        .expect("Failed to create Workspace Alpha");

    // Agent Beta creates Workspace Beta
    let ws_beta = create_workspace_client(&mut client, "ws-beta", None, &agent_beta)
        .await
        .expect("Failed to create Workspace Beta");

    // Agent Alpha elaborates Task Alpha in Workspace Alpha
    let task_alpha = elaborate_in_workspace_client(
        &mut client,
        ws_alpha.id,
        parent_key,
        "Alpha Candidate Task",
        Some("Alpha internal logic"),
        None,
        &agent_alpha,
    )
    .await
    .expect("Failed to elaborate in Workspace Alpha");

    // Agent Beta elaborates Task Beta in Workspace Beta
    let task_beta = elaborate_in_workspace_client(
        &mut client,
        ws_beta.id,
        parent_key,
        "Beta Candidate Task",
        Some("Beta internal logic"),
        None,
        &agent_beta,
    )
    .await
    .expect("Failed to elaborate in Workspace Beta");

    // 1. Boundary Isolation: get_workspace_nodes
    let alpha_nodes = get_workspace_nodes(&client, ws_alpha.id)
        .await
        .expect("Failed to get alpha nodes");
    assert_eq!(alpha_nodes.len(), 1);
    assert_eq!(alpha_nodes[0].id, task_alpha.task_id);

    let beta_nodes = get_workspace_nodes(&client, ws_beta.id)
        .await
        .expect("Failed to get beta nodes");
    assert_eq!(beta_nodes.len(), 1);
    assert_eq!(beta_nodes[0].id, task_beta.task_id);

    // 2. Cross-agent elaboration rejected: Agent Beta cannot elaborate in Workspace Alpha
    let mut pool_client = pool.get().await.expect("Failed to get pool client");
    let cross_elab_err = elaborate_in_workspace(
        &mut pool_client,
        ws_alpha.id,
        parent_key,
        "Intruder Subtask",
        None,
        None,
        &agent_beta,
    )
    .await
    .expect_err("Agent Beta must not be allowed to elaborate in Workspace Alpha");

    match cross_elab_err {
        MutationError::GovernanceRejected(msg) => {
            assert!(msg.contains("not authorized"));
        }
        other => panic!("Expected GovernanceRejected, got: {other:?}"),
    }

    // 3. Draft Confidentiality (INV-7): Agent Beta querying Workspace Alpha candidate draft task fails
    let envelope_unauth = assemble_context_envelope_workspace(
        &client,
        task_alpha.task_id,
        5,
        Some(&agent_beta.agent_id),
        Some(ws_beta.id),
    )
    .await;
    assert!(
        envelope_unauth.is_err(),
        "Agent Beta accessing Workspace Alpha candidate task must be rejected with NotFound"
    );

    // 4. Authorized envelope overlay: Agent Alpha sees candidate_tasks in Workspace Alpha
    let envelope_alpha = assemble_context_envelope_workspace(
        &client,
        parent_id,
        5,
        Some(&agent_alpha.agent_id),
        Some(ws_alpha.id),
    )
    .await
    .expect("Agent Alpha must successfully assemble envelope with their workspace");

    assert_eq!(envelope_alpha.candidate_tasks.len(), 1);
    assert_eq!(envelope_alpha.candidate_tasks[0].id, task_alpha.task_id);

    // 5. Clean live substrate view: omitting workspace_id yields 0 candidate tasks
    let envelope_clean = assemble_context_envelope_workspace(
        &client,
        parent_id,
        5,
        Some(&agent_alpha.agent_id),
        None,
    )
    .await
    .expect("Clean envelope lookup failed");
    assert_eq!(envelope_clean.candidate_tasks.len(), 0);

    // 6. Full-text search candidate overlay:
    let results_alpha =
        query_requirements_with_workspace(&client, "Alpha Candidate", 5, Some(ws_alpha.id))
            .await
            .expect("Search with workspace failed");
    assert!(
        results_alpha.iter().any(|r| r.id == task_alpha.task_id),
        "Alpha search with workspace must include candidate task"
    );

    // Search without workspace must NOT include candidate draft
    let results_clean = query_requirements_with_workspace(&client, "Alpha Candidate", 5, None)
        .await
        .expect("Clean search failed");
    assert!(
        !results_clean.iter().any(|r| r.id == task_alpha.task_id),
        "Clean search must NOT leak candidate draft"
    );
}

#[tokio::test]
async fn test_rest_workspace_endpoints() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let agent_alpha = "test_wp32_agent_rest_alpha";
    let token_alpha = "token_wp32_rest_alpha_secret";
    provision_agent(&state, agent_alpha, token_alpha).await;

    let agent_beta = "test_wp32_agent_rest_beta";
    let token_beta = "token_wp32_rest_beta_secret";
    provision_agent(&state, agent_beta, token_beta).await;

    // Seed parent requirement
    let parent_id = Uuid::new_v4();
    let parent_key = "REQ-WP32-REST-001";
    {
        let db_client = state.pool.get().await.unwrap();
        db_client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
                 VALUES ($1, $2, 'REQUIREMENT', 'Parent Requirement for REST Test', 'REST details', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $3);",
                &[&parent_id, &parent_key, &agent_alpha],
            )
            .await
            .unwrap();
    }

    // 1. POST /api/v1/workspaces - Create workspace
    let create_resp = client
        .post(format!("{base_url}/api/v1/workspaces"))
        .bearer_auth(token_alpha)
        .json(&json!({
            "name": "rest-feature-branch",
            "attributes": { "purpose": "integration testing" }
        }))
        .send()
        .await
        .expect("Failed to call POST /api/v1/workspaces");

    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let create_body: serde_json::Value = create_resp.json().await.unwrap();
    let ws_id_str = create_body["workspace_id"]
        .as_str()
        .expect("workspace_id must be string");
    let ws_id = Uuid::parse_str(ws_id_str).expect("Valid UUID");
    assert_eq!(create_body["status"], "ACTIVE");
    assert_eq!(create_body["owner_agent"], agent_alpha);

    // 2. GET /api/v1/workspaces - List workspaces
    let list_resp_alpha = client
        .get(format!("{base_url}/api/v1/workspaces"))
        .bearer_auth(token_alpha)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp_alpha.status(), StatusCode::OK);
    let list_body_alpha: Vec<serde_json::Value> = list_resp_alpha.json().await.unwrap();
    assert_eq!(list_body_alpha.len(), 1);
    assert_eq!(list_body_alpha[0]["id"], ws_id_str);

    let list_resp_beta = client
        .get(format!("{base_url}/api/v1/workspaces"))
        .bearer_auth(token_beta)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp_beta.status(), StatusCode::OK);
    let list_body_beta: Vec<serde_json::Value> = list_resp_beta.json().await.unwrap();
    assert_eq!(list_body_beta.len(), 0);

    // 3. GET /api/v1/workspaces/{id} - Inspect workspace
    let inspect_resp = client
        .get(format!("{base_url}/api/v1/workspaces/{ws_id}"))
        .bearer_auth(token_alpha)
        .send()
        .await
        .unwrap();
    assert_eq!(inspect_resp.status(), StatusCode::OK);
    let inspect_body: serde_json::Value = inspect_resp.json().await.unwrap();
    assert_eq!(inspect_body["workspace_name"], "rest-feature-branch");

    // Agent Beta inspecting Agent Alpha's workspace gets 404 (confidentiality, INV-7)
    let inspect_unauth = client
        .get(format!("{base_url}/api/v1/workspaces/{ws_id}"))
        .bearer_auth(token_beta)
        .send()
        .await
        .unwrap();
    assert_eq!(inspect_unauth.status(), StatusCode::NOT_FOUND);

    // 4. POST /api/v1/workspaces/{id}/subtasks - Elaborate task in workspace
    let elab_resp = client
        .post(format!("{base_url}/api/v1/workspaces/{ws_id}/subtasks"))
        .bearer_auth(token_alpha)
        .json(&json!({
            "parent_id": parent_key,
            "title": "REST Candidate Subtask 1",
            "content": "Detailed execution steps"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(elab_resp.status(), StatusCode::CREATED);
    let elab_body: serde_json::Value = elab_resp.json().await.unwrap();
    assert_eq!(elab_body["lifecycle_state"], "DRAFT");
    assert_eq!(elab_body["workspace_id"], ws_id_str);

    // 5. POST /api/v1/nodes/mutate with workspace_id
    let mutate_resp = client
        .post(format!("{base_url}/api/v1/nodes/mutate"))
        .bearer_auth(token_alpha)
        .json(&json!({
            "target_node_id": parent_key,
            "mutation_type": "TASK",
            "title": "REST Candidate Subtask 2 via mutate",
            "workspace_id": ws_id_str
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(mutate_resp.status(), StatusCode::CREATED);

    // 6. DELETE /api/v1/workspaces/{id} - Discard workspace
    let delete_resp = client
        .delete(format!("{base_url}/api/v1/workspaces/{ws_id}"))
        .bearer_auth(token_alpha)
        .send()
        .await
        .unwrap();
    assert_eq!(delete_resp.status(), StatusCode::OK);
    let delete_body: serde_json::Value = delete_resp.json().await.unwrap();
    assert_eq!(delete_body["status"], "DISCARDED");

    cancel.cancel();
}

#[tokio::test]
async fn test_mcp_workspace_integration() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let agent_id = "test_wp32_mcp_agent";
    let token = "token_wp32_mcp_secret";
    provision_agent(&state, agent_id, token).await;

    // Seed parent requirement
    let parent_id = Uuid::new_v4();
    let parent_key = "REQ-WP32-MCP-001";
    {
        let db_client = state.pool.get().await.unwrap();
        db_client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
                 VALUES ($1, $2, 'REQUIREMENT', 'Parent Requirement for MCP', 'MCP details', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $3);",
                &[&parent_id, &parent_key, &agent_id],
            )
            .await
            .unwrap();
    }

    // Create workspace via REST
    let ws_resp = client
        .post(format!("{base_url}/api/v1/workspaces"))
        .bearer_auth(token)
        .json(&json!({ "name": "mcp-test-ws" }))
        .send()
        .await
        .unwrap();
    let ws_body: serde_json::Value = ws_resp.json().await.unwrap();
    let ws_id_str = ws_body["workspace_id"].as_str().unwrap();

    // 1. MCP create_subtask with workspace_id
    let mcp_sub_resp = client
        .post(format!("{base_url}/mcp"))
        .bearer_auth(token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "tools/call",
            "params": {
                "name": "create_subtask",
                "arguments": {
                    "parent_node_id": parent_key,
                    "title": "MCP Workspace Candidate Task",
                    "content": "MCP elaborated plan",
                    "workspace_id": ws_id_str
                }
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(mcp_sub_resp.status(), StatusCode::OK);
    let mcp_sub_body: serde_json::Value = mcp_sub_resp.json().await.unwrap();
    let sub_content = mcp_sub_body["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(sub_content.contains("Subtask created") && sub_content.contains("DRAFT"));
    assert_eq!(mcp_sub_body["result"]["lifecycle_state"], "DRAFT");

    // 2. MCP get_context_envelope with workspace_id returns candidate tasks
    let mcp_env_resp = client
        .post(format!("{base_url}/mcp"))
        .bearer_auth(token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 101,
            "method": "tools/call",
            "params": {
                "name": "get_context_envelope",
                "arguments": {
                    "node_id": parent_key,
                    "workspace_id": ws_id_str
                }
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(mcp_env_resp.status(), StatusCode::OK);
    let env_body: serde_json::Value = mcp_env_resp.json().await.unwrap();
    assert!(
        env_body.get("error").is_none(),
        "MCP envelope error: {:?}",
        env_body
    );
    let env_text = env_body["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        env_text.contains("Candidate Tasks (Workspace Drafts)")
            || env_text.contains("MCP Workspace Candidate Task"),
        "Context envelope must render candidate draft tasks"
    );

    // 3. MCP query_requirements with workspace_id returns candidate drafts
    let mcp_query_resp = client
        .post(format!("{base_url}/mcp"))
        .bearer_auth(token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 102,
            "method": "tools/call",
            "params": {
                "name": "query_requirements",
                "arguments": {
                    "query": "MCP Workspace Candidate",
                    "workspace_id": ws_id_str
                }
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(mcp_query_resp.status(), StatusCode::OK);
    let query_body: serde_json::Value = mcp_query_resp.json().await.unwrap();
    assert!(
        query_body.get("error").is_none(),
        "MCP query error: {:?}",
        query_body
    );

    cancel.cancel();
}

#[tokio::test]
async fn test_high_concurrency_lock_free_elaboration() {
    let (_guard, pool, client, _db_url) = setup_test_db().await;

    // Seed parent requirement with AUTONOMOUS_ELABORATION policy
    let parent_id = Uuid::new_v4();
    let parent_key = "REQ-WP32-CONCURRENCY-001";
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, $2, 'REQUIREMENT', 'Parent Requirement for High Concurrency', 'Concurrency benchmark', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'system');",
            &[&parent_id, &parent_key],
        )
        .await
        .expect("Failed to seed parent requirement");

    // Concurrency verification: 10 external agents simultaneously creating workspaces
    // and elaborating 50 subtasks each (500 total candidate nodes/edges) complete with
    // zero lock wait timeouts or deadlock errors (40P01).
    const AGENT_COUNT: usize = 10;
    const SUBTASKS_PER_AGENT: usize = 50;

    let mut handles = Vec::with_capacity(AGENT_COUNT);

    for agent_idx in 0..AGENT_COUNT {
        let pool_clone = pool.clone();
        let parent_key_str = parent_key.to_string();

        let handle = tokio::spawn(async move {
            let agent = AuthenticatedAgent {
                agent_id: format!("test_wp32_concurrent_agent_{agent_idx}"),
                actor_type: "AGENT".to_string(),
            };

            let mut client = pool_clone
                .get()
                .await
                .expect("Agent failed to get pooled client");

            // 1. Create workspace
            let ws_name = format!("workspace-concurrent-{agent_idx}");
            let ws = create_workspace(&mut client, &ws_name, &agent)
                .await
                .expect("Agent failed to create workspace");

            // 2. Elaborate 50 subtasks in workspace
            let mut created_tasks = Vec::with_capacity(SUBTASKS_PER_AGENT);
            for task_idx in 0..SUBTASKS_PER_AGENT {
                let task_title = format!("Agent {agent_idx} Subtask {task_idx}");
                let res = elaborate_in_workspace(
                    &mut client,
                    ws.id,
                    &parent_key_str,
                    &task_title,
                    Some("Candidate subtask execution step"),
                    Some(json!({ "agent_index": agent_idx, "task_index": task_idx })),
                    &agent,
                )
                .await
                .unwrap_or_else(|e| {
                    panic!("Agent {agent_idx} failed on task {task_idx} with error: {e:?}")
                });

                created_tasks.push(res.task_id);
            }

            (agent.agent_id, ws.id, created_tasks)
        });

        handles.push(handle);
    }

    // Await all agent tasks
    let mut results = Vec::with_capacity(AGENT_COUNT);
    for h in handles {
        let res = h.await.expect("Agent thread panicked");
        results.push(res);
    }

    assert_eq!(results.len(), AGENT_COUNT);

    // Verify all 500 candidate tasks were created with proper attributes
    let (total_draft_nodes,): (i64,) = {
        let row = client
            .query_one(
                "SELECT COUNT(*) FROM graph_nodes WHERE node_type = 'TASK' AND lifecycle_state = 'DRAFT' AND created_by LIKE 'test_wp32_concurrent_agent_%';",
                &[],
            )
            .await
            .expect("Failed to count draft nodes");
        (row.get(0),)
    };
    assert_eq!(
        total_draft_nodes,
        (AGENT_COUNT * SUBTASKS_PER_AGENT) as i64,
        "Expected exactly 500 draft task nodes created across all workspaces"
    );

    let (total_draft_edges,): (i64,) = {
        let row = client
            .query_one(
                "SELECT COUNT(*) FROM graph_edges WHERE lifecycle_state = 'DRAFT' AND created_by LIKE 'test_wp32_concurrent_agent_%';",
                &[],
            )
            .await
            .expect("Failed to count draft edges");
        (row.get(0),)
    };
    assert_eq!(
        total_draft_edges,
        (AGENT_COUNT * SUBTASKS_PER_AGENT) as i64,
        "Expected exactly 500 draft edges created across all workspaces"
    );

    // Verify each workspace contains exactly 50 nodes and edges
    for (_agent_id, ws_id, tasks) in results {
        assert_eq!(tasks.len(), SUBTASKS_PER_AGENT);
        let ws_nodes = get_workspace_nodes(&client, ws_id)
            .await
            .expect("Failed to get workspace nodes");
        assert_eq!(ws_nodes.len(), SUBTASKS_PER_AGENT);

        let ws_edges = get_workspace_edges(&client, ws_id)
            .await
            .expect("Failed to get workspace edges");
        assert_eq!(ws_edges.len(), SUBTASKS_PER_AGENT);
    }
}
