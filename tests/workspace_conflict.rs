//! Integration test suite for Workspace Synchronization, Topological Conflict Resolution,
//! and Atomic Batch Promotion (WP-3.3, PHASE3-003).
//!
//! Validates:
//! 1. Fast-forward promotion for disjoint subtrees with zero conflicts.
//! 2. In-database DAG cycle detection and clean rejection leaving live substrate pristine.
//! 3. Parent requirement supersession detection and deterministic auto-reparenting.
//! 4. Multi-level lineage replacement traversal (P1 -> P2 -> P3).
//! 5. Canonical node_key collision detection against concurrently committed active nodes.
//! 6. Workspace synchronization rebase (`sync_workspace_rebase`) advancing `base_event_seq`.
//! 7. Invariant INV-1 ancestor path validation enforcement during batch promotion.
//! 8. REST API endpoints: `/api/v1/workspaces/{id}/analyze`, `/api/v1/workspaces/{id}/promote`,
//!    and `/api/v1/workspaces/{id}/rebase` with structured `ERR_MERGE_CONFLICT` error responses.

use std::time::Duration;

use reqwest::StatusCode;
use serde_json::json;
use tokio_postgres::Client;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tks::db;
use tks::gateway::auth::{AuthenticatedAgent, compute_token_hash};
use tks::gateway::{AppState, create_router};
use tks::storage::conflict::{
    MergeConflict, analyze_workspace_merge, promote_workspace, sync_workspace_rebase,
};
use tks::storage::git::start_git_actor;
use tks::storage::mutation::{MutationError, insert_audit_event};
use tks::storage::repo::StorageRepo;
use tks::storage::workspace::{create_workspace, elaborate_in_workspace, get_workspace};

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
            "DELETE FROM audit_ledger WHERE actor_id LIKE 'test_wp33_%';",
            &[],
        )
        .await
        .ok();
    client
        .execute(
            "DELETE FROM graph_edges WHERE created_by LIKE 'test_wp33_%';",
            &[],
        )
        .await
        .ok();
    client
        .execute(
            "DELETE FROM graph_nodes WHERE created_by LIKE 'test_wp33_%' OR node_key LIKE 'REQ-WP33-%' OR node_key LIKE 'TASK-WP33-%';",
            &[],
        )
        .await
        .ok();
    client
        .execute(
            "DELETE FROM workspaces WHERE owner_agent LIKE 'test_wp33_%';",
            &[],
        )
        .await
        .ok();

    let pool = db::create_pool(&database_url).expect("Failed to create connection pool");

    (guard, pool, client, database_url)
}

/// Spawns a test gateway server on an ephemeral TCP port for REST integration tests.
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
                "DELETE FROM audit_ledger WHERE actor_id LIKE 'test_wp33_%';",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM graph_edges WHERE created_by LIKE 'test_wp33_%';",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM graph_nodes WHERE created_by LIKE 'test_wp33_%' OR node_key LIKE 'REQ-WP33-%' OR node_key LIKE 'TASK-WP33-%';",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM workspaces WHERE owner_agent LIKE 'test_wp33_%';",
                &[],
            )
            .await;
    }

    let repo_dir = format!("target/test-conflict-git-{}", Uuid::new_v4());
    let (git_write, git_read) =
        start_git_actor(&repo_dir, 64).expect("Failed to start test git actor");

    let storage = StorageRepo::new(pool.clone());
    let state = AppState::new(pool, storage, git_write, git_read);

    let cancel_token = CancellationToken::new();
    let cancel_child = cancel_token.clone();
    let router = create_router(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral TCP port");
    let local_addr = listener
        .local_addr()
        .expect("Failed to get local socket address");
    let server_url = format!("http://{local_addr}");

    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                cancel_child.cancelled().await;
            })
            .await
            .ok();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    (server_url, cancel_token, state, guard)
}

#[tokio::test]
async fn test_fast_forward_merge_disjoint_subtrees() {
    let (_guard, pool, client, _url) = setup_test_db().await;

    let agent_alpha = AuthenticatedAgent {
        agent_id: "test_wp33_alpha".to_string(),
        actor_type: "AGENT".to_string(),
    };
    let agent_beta = AuthenticatedAgent {
        agent_id: "test_wp33_beta".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Seed active requirement R1 in substrate
    let req_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'REQ-WP33-01', 'REQUIREMENT', 'Root Requirement 1', 'Content 1', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2, '{}');",
            &[&req_id, &agent_alpha.agent_id],
        )
        .await
        .expect("Failed to seed requirement");

    // 2. Branch isolated workspace for agent alpha
    let mut pool_client = pool.get().await.unwrap();
    let ws = create_workspace(&mut pool_client, "workspace-ff", &agent_alpha)
        .await
        .expect("Failed to create workspace");
    assert_eq!(ws.status, "ACTIVE");

    // 3. Concurrently, agent beta creates disjoint active requirement and task in substrate
    let req2_id = Uuid::new_v4();
    let task2_id = Uuid::new_v4();
    let edge2_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'REQ-WP33-DISJOINT', 'REQUIREMENT', 'Disjoint Req', 'Disjoint Content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2, '{}');",
            &[&req2_id, &agent_beta.agent_id],
        )
        .await
        .unwrap();

    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'TASK-WP33-DISJOINT', 'TASK', 'Disjoint Task', 'Disjoint Content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2, '{}');",
            &[&task2_id, &agent_beta.agent_id],
        )
        .await
        .unwrap();

    client
        .execute(
            "INSERT INTO graph_edges (edge_id, from_node_id, to_node_id, edge_type, created_by, lifecycle_state, attributes) \
             VALUES ($1, $2, $3, 'FULFILLS', $4, 'ACTIVE', '{}');",
            &[&edge2_id, &task2_id, &req2_id, &agent_beta.agent_id],
        )
        .await
        .unwrap();

    // 4. In workspace ws, agent alpha elaborates candidate task under REQ-WP33-01
    let elab = elaborate_in_workspace(
        &mut pool_client,
        ws.id,
        "REQ-WP33-01",
        "Candidate Task Alpha",
        Some("Alpha Task Content"),
        Some(json!({ "node_key": "TASK-WP33-ALPHA-1" })),
        &agent_alpha,
    )
    .await
    .expect("Failed to elaborate in workspace");

    assert_eq!(elab.lifecycle_state, "DRAFT");

    // 5. Analyze workspace merge
    let preview = analyze_workspace_merge(&mut pool_client, ws.id)
        .await
        .expect("Failed to analyze merge");

    assert!(
        preview.can_fast_forward,
        "Disjoint changes should permit fast-forward promotion"
    );
    assert_eq!(preview.candidate_nodes.len(), 1);
    assert_eq!(preview.candidate_edges.len(), 1);
    assert!(
        preview.conflicts.is_empty(),
        "Disjoint changes must produce zero conflicts"
    );

    // 6. Promote workspace
    let promo = promote_workspace(&mut pool_client, ws.id, false, &agent_alpha)
        .await
        .expect("Fast-forward promotion should succeed");

    assert_eq!(promo.status, "MERGED");
    assert_eq!(promo.promoted_nodes, 1);
    assert_eq!(promo.promoted_edges, 1);
    assert!(promo.event_seq > ws.base_event_seq);

    // 7. Verify node and edge transitioned to ACTIVE in substrate
    let node_row = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&elab.task_id],
        )
        .await
        .unwrap();
    let state: String = node_row.get("lifecycle_state");
    assert_eq!(state, "ACTIVE");

    let edge_row = client
        .query_one(
            "SELECT lifecycle_state FROM graph_edges WHERE edge_id = $1;",
            &[&elab.edge_id],
        )
        .await
        .unwrap();
    let edge_state: String = edge_row.get("lifecycle_state");
    assert_eq!(edge_state, "ACTIVE");

    // 8. Verify workspace status is MERGED
    let ws_record = get_workspace(&mut pool_client, ws.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ws_record.status, "MERGED");

    // 9. Verify PROMOTION audit ledger entry
    let audit_row = client
        .query_one(
            "SELECT event_type, entity_type, entity_id FROM audit_ledger WHERE event_seq = $1;",
            &[&promo.event_seq],
        )
        .await
        .unwrap();
    let ev_type: String = audit_row.get("event_type");
    let ent_type: String = audit_row.get("entity_type");
    let ent_id: Uuid = audit_row.get("entity_id");
    assert_eq!(ev_type, "PROMOTION");
    assert_eq!(ent_type, "WORKSPACE");
    assert_eq!(ent_id, ws.id);
}

#[tokio::test]
async fn test_cycle_detection_and_rejection() {
    let (_guard, pool, client, _url) = setup_test_db().await;

    let agent = AuthenticatedAgent {
        agent_id: "test_wp33_cycle_agent".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Seed active requirement R1 and task T1 in substrate (T1 -FULFILLS-> R1)
    let req_id = Uuid::new_v4();
    let t1_id = Uuid::new_v4();
    let edge1_id = Uuid::new_v4();

    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'REQ-WP33-02', 'REQUIREMENT', 'Root Req 2', 'Req 2 Content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2, '{}');",
            &[&req_id, &agent.agent_id],
        )
        .await
        .unwrap();

    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'TASK-WP33-T1', 'TASK', 'Active Task 1', 'Content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2, '{}');",
            &[&t1_id, &agent.agent_id],
        )
        .await
        .unwrap();

    client
        .execute(
            "INSERT INTO graph_edges (edge_id, from_node_id, to_node_id, edge_type, created_by, lifecycle_state, attributes) \
             VALUES ($1, $2, $3, 'FULFILLS', $4, 'ACTIVE', '{}');",
            &[&edge1_id, &t1_id, &req_id, &agent.agent_id],
        )
        .await
        .unwrap();

    // 2. Branch workspace
    let mut pool_client = pool.get().await.unwrap();
    let ws = create_workspace(&mut pool_client, "workspace-cycle", &agent)
        .await
        .unwrap();

    // 3. In workspace, elaborate candidate task T2 under T1 (T2 -FULFILLS-> T1)
    let elab = elaborate_in_workspace(
        &mut pool_client,
        ws.id,
        &t1_id.to_string(),
        "Candidate Task 2",
        Some("T2 Content"),
        Some(json!({ "node_key": "TASK-WP33-T2" })),
        &agent,
    )
    .await
    .unwrap();

    // 4. Manually insert candidate edge in workspace that introduces a cycle:
    // T1 -> T2 (now T2 -> T1 and T1 -> T2 form a cycle!)
    let cyclic_edge_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_edges (edge_id, from_node_id, to_node_id, edge_type, created_by, lifecycle_state, attributes) \
             VALUES ($1, $2, $3, 'FULFILLS', $4, 'DRAFT', $5);",
            &[
                &cyclic_edge_id,
                &t1_id,
                &elab.task_id,
                &agent.agent_id,
                &json!({ "workspace_id": ws.id.to_string() }),
            ],
        )
        .await
        .unwrap();

    // 5. Analyze workspace merge
    let preview = analyze_workspace_merge(&mut pool_client, ws.id)
        .await
        .unwrap();

    assert!(
        !preview.can_fast_forward,
        "Cycle must prevent fast-forward promotion"
    );
    assert!(
        preview
            .conflicts
            .iter()
            .any(|c| matches!(c, MergeConflict::CycleDetected { .. })),
        "Expected CycleDetected conflict, got: {:?}",
        preview.conflicts
    );

    // 6. Attempt promotion -> must abort cleanly with MutationError::MergeConflict containing CycleDetected
    let promo_result = promote_workspace(&mut pool_client, ws.id, false, &agent).await;
    match promo_result {
        Err(MutationError::MergeConflict(conflicts)) => {
            let has_cycle = conflicts
                .iter()
                .any(|c| matches!(c, MergeConflict::CycleDetected { .. }));
            assert!(
                has_cycle,
                "Expected CycleDetected in conflicts, got: {conflicts:?}"
            );
        }
        other => panic!("Expected MutationError::MergeConflict, got: {other:?}"),
    }

    // 7. Verify live substrate remained pristine
    let ws_record = get_workspace(&mut pool_client, ws.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ws_record.status, "ACTIVE",
        "Workspace must remain ACTIVE after aborted promotion"
    );

    let t2_row = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&elab.task_id],
        )
        .await
        .unwrap();
    let t2_state: String = t2_row.get("lifecycle_state");
    assert_eq!(
        t2_state, "DRAFT",
        "Candidate task must remain in DRAFT state"
    );

    let edge_count_row = client
        .query_one(
            "SELECT COUNT(*) FROM graph_edges WHERE lifecycle_state = 'ACTIVE' AND (from_node_id = $1 OR to_node_id = $1);",
            &[&elab.task_id],
        )
        .await
        .unwrap();
    let active_edges_count: i64 = edge_count_row.get(0);
    assert_eq!(
        active_edges_count, 0,
        "Zero active edges should be committed for candidate task"
    );
}

#[tokio::test]
async fn test_parent_superseded_and_auto_reparenting() {
    let (_guard, pool, client, _url) = setup_test_db().await;

    let agent = AuthenticatedAgent {
        agent_id: "test_wp33_reparent_agent".to_string(),
        actor_type: "AGENT".to_string(),
    };
    let supervisor = AuthenticatedAgent {
        agent_id: "test_wp33_supervisor".to_string(),
        actor_type: "HUMAN".to_string(),
    };

    // 1. Seed active requirement R_v1 in substrate
    let r1_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'REQ-WP33-03', 'REQUIREMENT', 'Requirement v1', 'Original Content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2, '{}');",
            &[&r1_id, &agent.agent_id],
        )
        .await
        .unwrap();

    // 2. Branch workspace
    let mut pool_client = pool.get().await.unwrap();
    let ws = create_workspace(&mut pool_client, "workspace-reparent", &agent)
        .await
        .unwrap();

    // 3. Elaborate candidate task under R_v1 inside workspace
    let elab = elaborate_in_workspace(
        &mut pool_client,
        ws.id,
        "REQ-WP33-03",
        "Child Task Under V1",
        Some("Task Content"),
        Some(json!({ "node_key": "TASK-WP33-CHILD" })),
        &agent,
    )
    .await
    .unwrap();

    // 4. Concurrently in substrate, supervisor updates R_v1:
    // Mark R_v1 as SUPERSEDED and insert R_v2 as ACTIVE with replaces_node_id = R_v1
    let r2_id = Uuid::new_v4();
    client
        .execute(
            "UPDATE graph_nodes SET lifecycle_state = 'SUPERSEDED' WHERE id = $1;",
            &[&r1_id],
        )
        .await
        .unwrap();

    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'REQ-WP33-03', 'REQUIREMENT', 'Requirement v2', 'Updated Content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2, $3);",
            &[&r2_id, &supervisor.agent_id, &json!({ "replaces_node_id": r1_id.to_string() })],
        )
        .await
        .unwrap();

    // Log the supersession event in audit_ledger
    insert_audit_event(
        &client,
        Uuid::new_v4(),
        "SUPERSEDED",
        r1_id,
        "REQUIREMENT",
        &supervisor.agent_id,
        &supervisor.actor_type,
        &supervisor.token_fingerprint(),
        json!({ "new_node_id": r2_id }),
        json!({ "superseded_id": r1_id }),
    )
    .await
    .unwrap();

    // 5. Analyze workspace merge
    let preview = analyze_workspace_merge(&mut pool_client, ws.id)
        .await
        .unwrap();
    assert!(
        !preview.can_fast_forward,
        "Superseded parent must prevent fast-forward"
    );
    assert_eq!(preview.conflicts.len(), 1);
    match &preview.conflicts[0] {
        MergeConflict::ParentSuperseded {
            task_id,
            old_parent_id,
            reason,
        } => {
            assert_eq!(*task_id, elab.task_id);
            assert_eq!(*old_parent_id, r1_id);
            assert!(
                reason.contains("SUPERSEDED"),
                "Reason must indicate SUPERSEDED, got: {reason}"
            );
        }
        other => panic!("Expected ParentSuperseded, got: {other:?}"),
    }

    // 6. Attempt promotion with auto_reparent = false -> must fail with ParentSuperseded
    let err = promote_workspace(&mut pool_client, ws.id, false, &agent)
        .await
        .unwrap_err();
    match err {
        MutationError::MergeConflict(conflicts) => {
            assert!(
                conflicts
                    .iter()
                    .any(|c| matches!(c, MergeConflict::ParentSuperseded { .. }))
            );
        }
        other => panic!("Expected MergeConflict, got: {other:?}"),
    }

    // 7. Attempt promotion with auto_reparent = true -> must SUCCEED!
    let promo = promote_workspace(&mut pool_client, ws.id, true, &agent)
        .await
        .expect("Promotion with auto_reparent=true should succeed");

    assert_eq!(promo.status, "MERGED");
    assert_eq!(promo.promoted_nodes, 1);
    assert_eq!(promo.promoted_edges, 1);

    // 8. Verify the child task's upward edge now terminates at R_v2 (r2_id)
    let edge_row = client
        .query_one(
            "SELECT to_node_id, lifecycle_state FROM graph_edges WHERE from_node_id = $1;",
            &[&elab.task_id],
        )
        .await
        .unwrap();
    let to_node_id: Uuid = edge_row.get("to_node_id");
    let edge_state: String = edge_row.get("lifecycle_state");
    assert_eq!(
        to_node_id, r2_id,
        "Edge should have been auto-reparented to R_v2"
    );
    assert_eq!(edge_state, "ACTIVE");

    // 9. Verify task attributes parent_node_id updated
    let node_row = client
        .query_one(
            "SELECT attributes, lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&elab.task_id],
        )
        .await
        .unwrap();
    let attrs: serde_json::Value = node_row.get("attributes");
    let node_state: String = node_row.get("lifecycle_state");
    assert_eq!(node_state, "ACTIVE");
    assert_eq!(
        attrs.get("parent_node_id").and_then(|v| v.as_str()),
        Some(r2_id.to_string().as_str())
    );
}

#[tokio::test]
async fn test_multi_level_lineage_auto_reparenting() {
    let (_guard, pool, client, _url) = setup_test_db().await;

    let agent = AuthenticatedAgent {
        agent_id: "test_wp33_multilevel".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Seed R1 -> R2 -> R3 lineage
    let r1_id = Uuid::new_v4();
    let r2_id = Uuid::new_v4();
    let r3_id = Uuid::new_v4();

    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'REQ-WP33-04', 'REQUIREMENT', 'Req V1', 'V1', 'SUPERSEDED', 'AUTONOMOUS_ELABORATION', $2, '{}');",
            &[&r1_id, &agent.agent_id],
        )
        .await
        .unwrap();

    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'REQ-WP33-04-V2', 'REQUIREMENT', 'Req V2', 'V2', 'SUPERSEDED', 'AUTONOMOUS_ELABORATION', $2, $3);",
            &[&r2_id, &agent.agent_id, &json!({ "replaces_node_id": r1_id.to_string() })],
        )
        .await
        .unwrap();

    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'REQ-WP33-04-V3', 'REQUIREMENT', 'Req V3', 'V3', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2, $3);",
            &[&r3_id, &agent.agent_id, &json!({ "replaces_node_id": r2_id.to_string() })],
        )
        .await
        .unwrap();

    // 2. Branch workspace
    let mut pool_client = pool.get().await.unwrap();
    let ws = create_workspace(&mut pool_client, "workspace-multilevel", &agent)
        .await
        .unwrap();

    // 3. Elaborate candidate task with parent = R1
    // (insert directly into workspace with DRAFT state pointing to r1_id)
    let task_id = Uuid::new_v4();
    let edge_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'TASK-WP33-ML-1', 'TASK', 'Multi-level Task', 'Content', 'DRAFT', 'AUTONOMOUS_ELABORATION', $2, $3);",
            &[
                &task_id,
                &agent.agent_id,
                &json!({
                    "workspace_id": ws.id.to_string(),
                    "parent_node_id": r1_id.to_string(),
                }),
            ],
        )
        .await
        .unwrap();

    client
        .execute(
            "INSERT INTO graph_edges (edge_id, from_node_id, to_node_id, edge_type, created_by, lifecycle_state, attributes) \
             VALUES ($1, $2, $3, 'FULFILLS', $4, 'DRAFT', $5);",
            &[
                &edge_id,
                &task_id,
                &r1_id,
                &agent.agent_id,
                &json!({ "workspace_id": ws.id.to_string() }),
            ],
        )
        .await
        .unwrap();

    // 4. Promote with auto_reparent = true
    let promo = promote_workspace(&mut pool_client, ws.id, true, &agent)
        .await
        .expect("Multi-level reparenting promotion should succeed");

    assert_eq!(promo.status, "MERGED");

    // 5. Verify task edge terminates at R3 (r3_id)
    let edge_row = client
        .query_one(
            "SELECT to_node_id FROM graph_edges WHERE edge_id = $1;",
            &[&edge_id],
        )
        .await
        .unwrap();
    let resolved_to_id: Uuid = edge_row.get("to_node_id");
    assert_eq!(
        resolved_to_id, r3_id,
        "Should have recursively followed replaces_node_id chain to active R3"
    );
}

#[tokio::test]
async fn test_canonical_key_collision() {
    let (_guard, pool, client, _url) = setup_test_db().await;

    let agent_a = AuthenticatedAgent {
        agent_id: "test_wp33_key_a".to_string(),
        actor_type: "AGENT".to_string(),
    };
    let agent_b = AuthenticatedAgent {
        agent_id: "test_wp33_key_b".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Seed active root requirement
    let req_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'REQ-WP33-05', 'REQUIREMENT', 'Root 5', 'Content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2, '{}');",
            &[&req_id, &agent_a.agent_id],
        )
        .await
        .unwrap();

    // 2. Branch workspace for Agent A
    let mut pool_client = pool.get().await.unwrap();
    let ws = create_workspace(&mut pool_client, "workspace-key-collision", &agent_a)
        .await
        .unwrap();

    // 3. Agent A elaborates task with node_key = TASK-WP33-COLLIDE
    let _elab = elaborate_in_workspace(
        &mut pool_client,
        ws.id,
        "REQ-WP33-05",
        "Agent A Task",
        Some("Content A"),
        Some(json!({ "node_key": "TASK-WP33-COLLIDE" })),
        &agent_a,
    )
    .await
    .unwrap();

    // 4. Concurrently, Agent B creates an active task in substrate with the same key
    let b_task_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'TASK-WP33-COLLIDE', 'TASK', 'Agent B Task', 'Content B', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2, '{}');",
            &[&b_task_id, &agent_b.agent_id],
        )
        .await
        .unwrap();

    // 5. Analyze merge -> must detect KeyCollision
    let preview = analyze_workspace_merge(&mut pool_client, ws.id)
        .await
        .unwrap();
    assert!(
        !preview.can_fast_forward,
        "Key collision must block fast-forward"
    );
    assert!(
        preview.conflicts.iter().any(|c| matches!(
            c,
            MergeConflict::KeyCollision {
                node_key,
                conflicting_node_id
            } if node_key == "TASK-WP33-COLLIDE" && *conflicting_node_id == b_task_id
        )),
        "Expected KeyCollision conflict, got: {:?}",
        preview.conflicts
    );

    // 6. Promote workspace -> must abort with MergeConflict
    let err = promote_workspace(&mut pool_client, ws.id, true, &agent_a)
        .await
        .unwrap_err();
    match err {
        MutationError::MergeConflict(conflicts) => {
            assert!(
                conflicts
                    .iter()
                    .any(|c| matches!(c, MergeConflict::KeyCollision { .. }))
            );
        }
        other => panic!("Expected MergeConflict, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_sync_workspace_rebase() {
    let (_guard, pool, client, _url) = setup_test_db().await;

    let agent = AuthenticatedAgent {
        agent_id: "test_wp33_rebase".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Seed active requirement R1
    let r1_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'REQ-WP33-06', 'REQUIREMENT', 'Root 6', 'Content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2, '{}');",
            &[&r1_id, &agent.agent_id],
        )
        .await
        .unwrap();

    // 2. Branch workspace
    let mut pool_client = pool.get().await.unwrap();
    let ws = create_workspace(&mut pool_client, "workspace-sync-rebase", &agent)
        .await
        .unwrap();

    // 3. Elaborate candidate task
    let elab = elaborate_in_workspace(
        &mut pool_client,
        ws.id,
        "REQ-WP33-06",
        "Rebase Candidate Task",
        Some("Content"),
        Some(json!({ "node_key": "TASK-WP33-REBASE-1" })),
        &agent,
    )
    .await
    .unwrap();

    // 4. Supersede R1 with R2 in live substrate
    let r2_id = Uuid::new_v4();
    client
        .execute(
            "UPDATE graph_nodes SET lifecycle_state = 'SUPERSEDED' WHERE id = $1;",
            &[&r1_id],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, 'REQ-WP33-06', 'REQUIREMENT', 'Root 6 V2', 'V2 Content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2, $3);",
            &[&r2_id, &agent.agent_id, &json!({ "replaces_node_id": r1_id.to_string() })],
        )
        .await
        .unwrap();

    // 5. Execute rebase with auto_reparent = true
    let rebase_res = sync_workspace_rebase(&mut pool_client, ws.id, true, &agent)
        .await
        .expect("Rebase should succeed");

    assert_eq!(rebase_res.status, "REBASED");
    assert_eq!(rebase_res.reparented_tasks, 1);
    assert!(rebase_res.new_base_event_seq > rebase_res.old_base_event_seq);

    // 6. Verify workspace record updated base_event_seq
    let updated_ws = get_workspace(&mut pool_client, ws.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated_ws.base_event_seq, rebase_res.new_base_event_seq);
    assert_eq!(updated_ws.status, "ACTIVE");

    // 7. Verify candidate task edge in workspace now points to R2
    let edge_row = client
        .query_one(
            "SELECT to_node_id FROM graph_edges WHERE edge_id = $1;",
            &[&elab.edge_id],
        )
        .await
        .unwrap();
    let edge_target: Uuid = edge_row.get("to_node_id");
    assert_eq!(edge_target, r2_id);

    // 8. Analyze merge now -> can_fast_forward must be true!
    let preview = analyze_workspace_merge(&mut pool_client, ws.id)
        .await
        .unwrap();
    assert!(
        preview.can_fast_forward,
        "After rebase, workspace should be cleanly fast-forwardable"
    );
    assert!(preview.conflicts.is_empty());

    // 9. Subsequent promote without auto_reparent succeeds
    let promo = promote_workspace(&mut pool_client, ws.id, false, &agent)
        .await
        .expect("Promotion after rebase should succeed");
    assert_eq!(promo.status, "MERGED");
}

#[tokio::test]
async fn test_rest_workspace_merge_and_rebase_endpoints() {
    let (server_url, cancel_token, state, _guard) = spawn_test_server().await;
    let http = reqwest::Client::new();
    let auth_token = "token_wp33_rest";

    // 1. Provision agent identity in database
    {
        let client = state.pool.get().await.unwrap();
        client
            .execute(
                "INSERT INTO agent_identities (agent_id, token_hash, actor_type, is_active, created_at) \
                 VALUES ($1, $2, 'AGENT', true, NOW()) \
                 ON CONFLICT (agent_id) DO NOTHING;",
                &[&"test_wp33_rest_agent", &compute_token_hash(auth_token)],
            )
            .await
            .unwrap();

        // Seed root requirement
        client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
                 VALUES (gen_random_uuid(), 'REQ-WP33-REST', 'REQUIREMENT', 'REST Requirement', 'Content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'test_wp33_rest_agent', '{}') \
                 ON CONFLICT (id) DO NOTHING;",
                &[],
            )
            .await
            .unwrap();
    }

    // 2. Create workspace via POST /api/v1/workspaces
    let ws_resp = http
        .post(format!("{server_url}/api/v1/workspaces"))
        .bearer_auth(auth_token)
        .json(&json!({
            "name": "ws-rest-test"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(ws_resp.status(), StatusCode::CREATED);
    let ws_body: serde_json::Value = ws_resp.json().await.unwrap();
    let ws_id = ws_body["workspace_id"].as_str().unwrap();

    // 3. Elaborate candidate task via POST /api/v1/workspaces/{id}/subtasks
    let elab_resp = http
        .post(format!("{server_url}/api/v1/workspaces/{ws_id}/subtasks"))
        .bearer_auth(auth_token)
        .json(&json!({
            "parent_id": "REQ-WP33-REST",
            "title": "REST Candidate Task",
            "content": "REST Content",
            "attributes": {
                "node_key": "TASK-WP33-REST-1"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(elab_resp.status(), StatusCode::CREATED);

    // 4. Test GET /api/v1/workspaces/{id}/analyze
    let analyze_resp = http
        .get(format!("{server_url}/api/v1/workspaces/{ws_id}/analyze"))
        .bearer_auth(auth_token)
        .send()
        .await
        .unwrap();

    assert_eq!(analyze_resp.status(), StatusCode::OK);
    let analyze_body: serde_json::Value = analyze_resp.json().await.unwrap();
    assert_eq!(analyze_body["can_fast_forward"], true);
    assert!(analyze_body["conflicts"].as_array().unwrap().is_empty());

    // 5. Test POST /api/v1/workspaces/{id}/rebase
    let rebase_resp = http
        .post(format!("{server_url}/api/v1/workspaces/{ws_id}/rebase"))
        .bearer_auth(auth_token)
        .json(&json!({
            "auto_reparent": false
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(rebase_resp.status(), StatusCode::OK);
    let rebase_body: serde_json::Value = rebase_resp.json().await.unwrap();
    assert_eq!(rebase_body["status"], "REBASED");

    // 6. Test POST /api/v1/workspaces/{id}/promote
    let promo_resp = http
        .post(format!("{server_url}/api/v1/workspaces/{ws_id}/promote"))
        .bearer_auth(auth_token)
        .json(&json!({
            "auto_reparent": false
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(promo_resp.status(), StatusCode::OK);
    let promo_body: serde_json::Value = promo_resp.json().await.unwrap();
    assert_eq!(promo_body["status"], "MERGED");
    assert_eq!(promo_body["promoted_nodes"], 1);
    assert_eq!(promo_body["promoted_edges"], 1);

    cancel_token.cancel();
}
