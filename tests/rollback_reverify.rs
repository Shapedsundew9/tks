//! Integration test suite for Unified Administrative Rollback Utility (`revert_mutations`)
//! and Explicit Reverification Interface (`reverify_node`) (WP-2.3).
//!
//! Verifies:
//! - Dry-run preview verification: Invoking rollback with `dry_run = true` returns accurate list
//!   of affected nodes and edges with zero mutations committed to database.
//! - Safety confirmation verification: Attempting to revert a batch with dependent tasks from a second
//!   agent without `force = true` returns `ERR_CONFIRMATION_REQUIRED`; resubmitting with `force = true` succeeds.
//! - Invalidation and reverification loop: Reverted parent transitions child task to `NEEDS_REVERIFICATION`;
//!   calling `reverify_node` restores child to `ACTIVE` and emits `REVERIFIED` event in `audit_ledger`.
//! - Dependency inactive enforcement: Reverifying a node with non-active upstream parents fails with `ERR_DEPENDENCY_INACTIVE`.
//! - Re-parenting support via updated attributes unblocks degraded tasks.
//! - Pooled `StorageRepo` and `deadpool_postgres::Client` methods parity.

use tokio_postgres::Client;
use uuid::Uuid;

use tks::db;
use tks::gateway::auth::AuthenticatedAgent;
use tks::storage::envelope::assemble_context_envelope;
use tks::storage::rollback::{
    RevertExecutionResult, RevertFilter, revert_mutations, revert_mutations_client,
};
use tks::storage::{
    MutationError, StorageRepo, TaskStatus, elaborate_task_client, reverify_node,
    reverify_node_client, update_task_status_client,
};

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Helper to serialize database access across tests, connect, run migrations, and clean up test data.
async fn setup_test_db() -> (tokio::sync::MutexGuard<'static, ()>, StorageRepo, Client) {
    let guard = TEST_MUTEX.lock().await;
    let database_url = db::ensure_test_database_ready()
        .await
        .expect("Failed to prepare test db");
    let (mut client, _handle) = db::connect(&database_url)
        .await
        .expect("Failed to connect to database");

    // Ensure migrations are up to date
    db::run_migrations(&mut client)
        .await
        .expect("Failed to run migrations");

    // Clean up previous test artifacts
    client
        .execute(
            "DELETE FROM audit_ledger WHERE actor_id LIKE 'test_wp23_%';",
            &[],
        )
        .await
        .expect("Cleanup audit_ledger failed");
    client
        .execute(
            "DELETE FROM graph_nodes WHERE created_by LIKE 'test_wp23_%';",
            &[],
        )
        .await
        .expect("Cleanup graph_nodes failed");

    let repo = StorageRepo::from_database_url(&database_url).expect("StorageRepo creation failed");
    (guard, repo, client)
}

/// Helper to insert an active requirement node for testing.
async fn create_test_requirement(
    client: &Client,
    node_key: &str,
    title: &str,
    created_by: &str,
) -> Uuid {
    let row = client
        .query_one(
            "INSERT INTO graph_nodes (node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'REQUIREMENT', $2, 'Test requirement intent', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $3) \
             RETURNING id;",
            &[&node_key, &title, &created_by],
        )
        .await
        .expect("Failed to insert test requirement");

    row.get("id")
}

#[tokio::test]
async fn test_dry_run_preview_verification() {
    let (_guard, _repo, mut client) = setup_test_db().await;

    let actor = AuthenticatedAgent {
        agent_id: "test_wp23_agent_dry".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Create active requirement
    let req_id = create_test_requirement(
        &client,
        "REQ-WP23-DRY-01",
        "Dry Run Verification Requirement",
        &actor.agent_id,
    )
    .await;

    // 2. Elaborate task under requirement
    let task_res = elaborate_task_client(
        &mut client,
        &req_id.to_string(),
        "Dry Run Task",
        Some("Task for dry-run rollback preview"),
        None,
        &actor,
    )
    .await
    .expect("Task elaboration failed");

    let task_batch_id = task_res.batch_id;
    let task_id = task_res.task_id;

    // 3. Invoke rollback with dry_run = true
    let filter = RevertFilter {
        batch_id: Some(task_batch_id),
        agent_id: None,
        since: None,
        event_seq_range: None,
        dry_run: Some(true),
        force: None,
    };

    let result = revert_mutations_client(&mut client, filter, &actor)
        .await
        .expect("Dry run revert failed");

    // 4. Verify preview structure
    match result {
        RevertExecutionResult::DryRun(preview) => {
            assert!(
                preview.affected_nodes.contains(&task_id),
                "Preview must include targeted task ID"
            );
            assert_eq!(
                preview.total_events, 1,
                "Expected exactly 1 targeted audit event"
            );
            assert!(
                preview.cross_agent_dependencies.is_empty(),
                "No cross-agent dependencies expected"
            );
        }
        RevertExecutionResult::Reverted(_) => {
            panic!("Expected DryRun result, got Reverted");
        }
    }

    // 5. Verify database state is untouched (zero mutations committed)
    let task_row = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&task_id],
        )
        .await
        .expect("Task query failed");
    let state: String = task_row.get("lifecycle_state");
    assert_eq!(
        state, "ACTIVE",
        "Task must remain ACTIVE after dry-run rollback"
    );

    let revert_events = client
        .query(
            "SELECT event_seq FROM audit_ledger WHERE event_type = 'REVERT' AND entity_id = $1;",
            &[&task_id],
        )
        .await
        .expect("Audit query failed");
    assert!(
        revert_events.is_empty(),
        "No REVERT audit events should be emitted during dry-run"
    );
}

#[tokio::test]
async fn test_cross_agent_safety_confirmation_and_force() {
    let (_guard, _repo, mut client) = setup_test_db().await;

    let agent_alpha = AuthenticatedAgent {
        agent_id: "test_wp23_agent_alpha".to_string(),
        actor_type: "AGENT".to_string(),
    };

    let agent_beta = AuthenticatedAgent {
        agent_id: "test_wp23_agent_beta".to_string(),
        actor_type: "AGENT".to_string(),
    };

    let admin = AuthenticatedAgent {
        agent_id: "test_wp23_admin".to_string(),
        actor_type: "HUMAN".to_string(),
    };

    // 1. Create requirement by Alpha
    let req_id = create_test_requirement(
        &client,
        "REQ-WP23-CONF-01",
        "Cross-Agent Confirmation Requirement",
        &agent_alpha.agent_id,
    )
    .await;

    // 2. Elaborate parent task by Agent Alpha
    let parent_res = elaborate_task_client(
        &mut client,
        &req_id.to_string(),
        "Parent Task by Alpha",
        Some("Alpha's task"),
        None,
        &agent_alpha,
    )
    .await
    .expect("Parent task elaboration failed");

    let alpha_batch_id = parent_res.batch_id;
    let parent_id = parent_res.task_id;

    // 3. Elaborate child task under Alpha's task by Agent Beta
    let child_res = elaborate_task_client(
        &mut client,
        &parent_id.to_string(),
        "Child Task by Beta",
        Some("Beta's dependent task"),
        None,
        &agent_beta,
    )
    .await
    .expect("Child task elaboration failed");

    let child_id = child_res.task_id;

    // 4. Attempt revert of Alpha's batch without force (force: None or Some(false))
    let filter_unforced = RevertFilter {
        batch_id: Some(alpha_batch_id),
        agent_id: None,
        since: None,
        event_seq_range: None,
        dry_run: Some(false),
        force: Some(false),
    };

    let err = revert_mutations_client(&mut client, filter_unforced, &admin)
        .await
        .expect_err("Expected rollback to fail without force flag");

    assert_eq!(
        err.code(),
        "ERR_CONFIRMATION_REQUIRED",
        "Expected ERR_CONFIRMATION_REQUIRED error code"
    );

    match err {
        MutationError::ConfirmationRequired(preview) => {
            assert!(
                preview
                    .cross_agent_dependencies
                    .contains(&agent_beta.agent_id),
                "Preview must identify Agent Beta as cross-agent dependent"
            );
            assert!(preview.affected_nodes.contains(&parent_id));
            assert!(preview.affected_nodes.contains(&child_id));
        }
        other => panic!("Unexpected error variant: {other:?}"),
    }

    // 5. Verify database state is untouched after confirmation abort
    let parent_row = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&parent_id],
        )
        .await
        .expect("Parent query failed");
    assert_eq!(parent_row.get::<_, String>("lifecycle_state"), "ACTIVE");

    let child_row = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&child_id],
        )
        .await
        .expect("Child query failed");
    assert_eq!(child_row.get::<_, String>("lifecycle_state"), "ACTIVE");

    // 6. Resubmit rollback with force = true
    let filter_forced = RevertFilter {
        batch_id: Some(alpha_batch_id),
        agent_id: None,
        since: None,
        event_seq_range: None,
        dry_run: Some(false),
        force: Some(true),
    };

    let result = revert_mutations_client(&mut client, filter_forced, &admin)
        .await
        .expect("Forced rollback must succeed");

    match result {
        RevertExecutionResult::Reverted(res) => {
            assert_eq!(res.reverted_events, 1);
            assert!(res.affected_nodes.contains(&parent_id));
            assert!(res.cascade_reverified_nodes.contains(&child_id));
        }
        RevertExecutionResult::DryRun(_) => panic!("Expected Reverted result, got DryRun"),
    }

    // 7. Verify database states:
    // Parent transitioned to SUPERSEDED
    let parent_row = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&parent_id],
        )
        .await
        .expect("Parent query failed");
    assert_eq!(parent_row.get::<_, String>("lifecycle_state"), "SUPERSEDED");

    // Child task cascaded to NEEDS_REVERIFICATION
    let child_row = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&child_id],
        )
        .await
        .expect("Child query failed");
    assert_eq!(
        child_row.get::<_, String>("lifecycle_state"),
        "NEEDS_REVERIFICATION"
    );

    // Associated edge to parent is marked REVERTED
    let edge_row = client
        .query_one(
            "SELECT lifecycle_state FROM graph_edges WHERE from_node_id = $1;",
            &[&parent_id],
        )
        .await
        .expect("Edge query failed");
    assert_eq!(edge_row.get::<_, String>("lifecycle_state"), "REVERTED");
}

#[tokio::test]
async fn test_invalidation_and_reverification_loop() {
    let (_guard, _repo, mut client) = setup_test_db().await;

    let actor = AuthenticatedAgent {
        agent_id: "test_wp23_agent_loop".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Create requirement
    let req_id = create_test_requirement(
        &client,
        "REQ-WP23-LOOP-01",
        "Loop Verification Requirement",
        &actor.agent_id,
    )
    .await;

    // 2. Elaborate parent task
    let parent_res = elaborate_task_client(
        &mut client,
        &req_id.to_string(),
        "Parent Execution Task",
        Some("Parent task for status updates"),
        None,
        &actor,
    )
    .await
    .expect("Parent elaboration failed");

    let parent_id = parent_res.task_id;

    // 3. Elaborate child task under parent
    let child_res = elaborate_task_client(
        &mut client,
        &parent_id.to_string(),
        "Child Task Under Parent",
        Some("Child task awaiting parent completion"),
        None,
        &actor,
    )
    .await
    .expect("Child elaboration failed");

    let child_id = child_res.task_id;

    // 4. Update status of parent task from OPEN to IN_PROGRESS
    let status_update_res = update_task_status_client(
        &mut client,
        &parent_id.to_string(),
        TaskStatus::InProgress,
        Some("Starting implementation"),
        &actor,
    )
    .await
    .expect("Status update failed");

    let status_batch_id = status_update_res.batch_id;

    // 5. Revert the status update batch
    let filter = RevertFilter {
        batch_id: Some(status_batch_id),
        agent_id: None,
        since: None,
        event_seq_range: None,
        dry_run: Some(false),
        force: Some(true),
    };

    let revert_res = revert_mutations_client(&mut client, filter, &actor)
        .await
        .expect("Status revert failed");

    match revert_res {
        RevertExecutionResult::Reverted(res) => {
            assert_eq!(res.reverted_events, 1);
            // Child cascaded to NEEDS_REVERIFICATION
            assert!(res.cascade_reverified_nodes.contains(&child_id));
        }
        _ => panic!("Expected Reverted result"),
    }

    // 6. Verify parent task status was restored to OPEN and remains ACTIVE
    let parent_row = client
        .query_one(
            "SELECT lifecycle_state, attributes FROM graph_nodes WHERE id = $1;",
            &[&parent_id],
        )
        .await
        .expect("Parent query failed");
    assert_eq!(parent_row.get::<_, String>("lifecycle_state"), "ACTIVE");
    let parent_attrs: serde_json::Value = parent_row.get("attributes");
    assert_eq!(
        parent_attrs
            .get("execution_status")
            .and_then(|v| v.as_str()),
        Some("OPEN")
    );

    // 7. Verify child task is in NEEDS_REVERIFICATION
    let child_row = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&child_id],
        )
        .await
        .expect("Child query failed");
    assert_eq!(
        child_row.get::<_, String>("lifecycle_state"),
        "NEEDS_REVERIFICATION"
    );

    // 8. Verify get_context_envelope returns staleness_warning: true for degraded node
    let env = assemble_context_envelope(&client, child_id, 2, None)
        .await
        .expect("Context envelope failed");
    assert!(
        env.staleness_warning,
        "Context envelope must have staleness_warning: true"
    );

    // 9. Call reverify_node on child task: since parent is ACTIVE, reverification succeeds!
    let reverify_res = reverify_node_client(
        &mut client,
        &child_id.to_string(),
        "Verified child against restored parent task",
        Some(serde_json::json!({ "reverified_by": "test_wp23_agent_loop" })),
        &actor,
    )
    .await
    .expect("Reverification must succeed when upstream parent is ACTIVE");

    assert_eq!(reverify_res.status, "ACTIVE");
    assert_eq!(reverify_res.staleness_score, 0.0);
    assert_eq!(reverify_res.node_id, child_id);

    // 10. Verify child task in database is restored to ACTIVE with staleness_score = 0.0
    let child_after = client
        .query_one(
            "SELECT lifecycle_state, attributes FROM graph_nodes WHERE id = $1;",
            &[&child_id],
        )
        .await
        .expect("Child query failed");
    assert_eq!(child_after.get::<_, String>("lifecycle_state"), "ACTIVE");
    let child_attrs: serde_json::Value = child_after.get("attributes");
    assert_eq!(
        child_attrs.get("staleness_score").and_then(|v| v.as_f64()),
        Some(0.0)
    );

    // 11. Verify discrete REVERIFIED event in audit_ledger
    let audit_row = client
        .query_one(
            "SELECT event_type, actor_id, delta FROM audit_ledger \
             WHERE entity_id = $1 AND event_type = 'REVERIFIED';",
            &[&child_id],
        )
        .await
        .expect("Audit query failed");
    assert_eq!(audit_row.get::<_, String>("event_type"), "REVERIFIED");
    assert_eq!(audit_row.get::<_, String>("actor_id"), actor.agent_id);
}

#[tokio::test]
async fn test_reverify_inactive_dependency_rejection_and_reparenting() {
    let (_guard, _repo, mut client) = setup_test_db().await;

    let actor = AuthenticatedAgent {
        agent_id: "test_wp23_agent_inact".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Create requirement 1 and requirement 2
    let req1_id = create_test_requirement(
        &client,
        "REQ-WP23-INACT-01",
        "Inactive Dependency Requirement 1",
        &actor.agent_id,
    )
    .await;

    let req2_id = create_test_requirement(
        &client,
        "REQ-WP23-INACT-02",
        "Active Replacement Requirement 2",
        &actor.agent_id,
    )
    .await;

    // 2. Elaborate parent task under req1
    let parent_res = elaborate_task_client(
        &mut client,
        &req1_id.to_string(),
        "Parent To Be Superseded",
        None,
        None,
        &actor,
    )
    .await
    .expect("Parent task failed");

    let parent_id = parent_res.task_id;
    let parent_batch = parent_res.batch_id;

    // 3. Elaborate child task under parent
    let child_res = elaborate_task_client(
        &mut client,
        &parent_id.to_string(),
        "Child Needing Reverification",
        None,
        None,
        &actor,
    )
    .await
    .expect("Child task failed");

    let child_id = child_res.task_id;

    // 4. Revert parent task creation (so parent becomes SUPERSEDED)
    let filter = RevertFilter {
        batch_id: Some(parent_batch),
        agent_id: None,
        since: None,
        event_seq_range: None,
        dry_run: Some(false),
        force: Some(true),
    };

    revert_mutations_client(&mut client, filter, &actor)
        .await
        .expect("Parent revert failed");

    // Parent is SUPERSEDED, child is NEEDS_REVERIFICATION
    let parent_state: String = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&parent_id],
        )
        .await
        .unwrap()
        .get("lifecycle_state");
    assert_eq!(parent_state, "SUPERSEDED");

    // 5. Attempt reverify on child without fixing parent: should fail with ERR_DEPENDENCY_INACTIVE
    let err = reverify_node_client(
        &mut client,
        &child_id.to_string(),
        "Attempting reverify without active parent",
        None,
        &actor,
    )
    .await
    .expect_err("Reverify must fail when parent is not ACTIVE");

    assert_eq!(
        err.code(),
        "ERR_DEPENDENCY_INACTIVE",
        "Expected ERR_DEPENDENCY_INACTIVE error code"
    );

    // 6. Reverifying child while supplying reparent_to = req2_id unblocks and restores child to ACTIVE
    let reparent_attrs = serde_json::json!({
        "reparent_to": req2_id.to_string(),
        "notes": "Re-anchored to active requirement 2"
    });

    let reverify_res = reverify_node_client(
        &mut client,
        &child_id.to_string(),
        "Re-parented to active requirement 2",
        Some(reparent_attrs),
        &actor,
    )
    .await
    .expect("Reverification with active re-parenting must succeed");

    assert_eq!(reverify_res.status, "ACTIVE");

    // Verify child is ACTIVE and its active upward edge now points to req2_id
    let child_state: String = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&child_id],
        )
        .await
        .unwrap()
        .get("lifecycle_state");
    assert_eq!(child_state, "ACTIVE");

    let active_edges = client
        .query(
            "SELECT to_node_id FROM graph_edges WHERE from_node_id = $1 AND lifecycle_state = 'ACTIVE';",
            &[&child_id],
        )
        .await
        .unwrap();
    assert_eq!(active_edges.len(), 1);
    assert_eq!(active_edges[0].get::<_, Uuid>("to_node_id"), req2_id);
}

#[tokio::test]
async fn test_deadpool_client_and_repo_methods_parity() {
    let (_guard, repo, client) = setup_test_db().await;

    let actor = AuthenticatedAgent {
        agent_id: "test_wp23_agent_repo".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Create requirement
    let req_id = create_test_requirement(
        &client,
        "REQ-WP23-REPO-01",
        "Repo Methods Parity Requirement",
        &actor.agent_id,
    )
    .await;

    // 2. Elaborate task using repo
    let mut deadpool_client = repo.get_client().await.unwrap();
    let task_res = tks::storage::elaborate_task(
        &mut deadpool_client,
        &req_id.to_string(),
        "Repo Task",
        Some("Repo test task"),
        None,
        &actor,
    )
    .await
    .expect("Elaborate task via deadpool client failed");

    // 3. Dry-run revert using deadpool client
    let filter = RevertFilter {
        batch_id: Some(task_res.batch_id),
        agent_id: None,
        since: None,
        event_seq_range: None,
        dry_run: Some(true),
        force: None,
    };

    let dry_res = revert_mutations(&mut deadpool_client, filter.clone(), &actor)
        .await
        .expect("revert_mutations via deadpool client failed");
    assert!(dry_res.is_dry_run());

    // 4. Committed revert using repo method
    let revert_res = repo
        .revert_mutations(
            RevertFilter {
                dry_run: Some(false),
                force: Some(true),
                ..filter
            },
            &actor,
        )
        .await
        .expect("repo.revert_mutations failed");

    assert!(!revert_res.is_dry_run());

    // 5. Test reverify_node on deadpool client and repo
    let degraded_task_id = task_res.task_id;
    // Attempt reverify via deadpool client: since task was reverted to SUPERSEDED, reverify expects NEEDS_REVERIFICATION or ACTIVE
    let err_client = reverify_node(
        &mut deadpool_client,
        &degraded_task_id.to_string(),
        "Testing deadpool client reverify",
        None,
        &actor,
    )
    .await
    .expect_err("SUPERSEDED task cannot be directly reverified");
    assert!(err_client.to_string().contains("cannot be reverified"));

    // Attempt reverify via repo method
    let err_repo = repo
        .reverify_node(
            &degraded_task_id.to_string(),
            "Testing repo method",
            None,
            &actor,
        )
        .await
        .expect_err("SUPERSEDED task cannot be directly reverified");
    assert!(err_repo.to_string().contains("cannot be reverified"));
}

#[tokio::test]
async fn test_revert_filter_by_agent_and_seq_range() {
    let (_guard, _repo, mut client) = setup_test_db().await;

    let agent_x = AuthenticatedAgent {
        agent_id: "test_wp23_agent_x".to_string(),
        actor_type: "AGENT".to_string(),
    };
    let agent_y = AuthenticatedAgent {
        agent_id: "test_wp23_agent_y".to_string(),
        actor_type: "AGENT".to_string(),
    };

    let req_id = create_test_requirement(
        &client,
        "REQ-WP23-FILTER-01",
        "Filter Combinations Requirement",
        &agent_x.agent_id,
    )
    .await;

    // Elaborate tasks with agent X and agent Y
    let task_x1 = elaborate_task_client(
        &mut client,
        &req_id.to_string(),
        "Task X1",
        None,
        None,
        &agent_x,
    )
    .await
    .unwrap();

    let task_y1 = elaborate_task_client(
        &mut client,
        &req_id.to_string(),
        "Task Y1",
        None,
        None,
        &agent_y,
    )
    .await
    .unwrap();

    let task_x2 = elaborate_task_client(
        &mut client,
        &req_id.to_string(),
        "Task X2",
        None,
        None,
        &agent_x,
    )
    .await
    .unwrap();

    // Revert only agent X's tasks by agent_id filter
    let filter_agent = RevertFilter {
        batch_id: None,
        agent_id: Some(agent_x.agent_id.clone()),
        since: None,
        event_seq_range: None,
        dry_run: Some(false),
        force: Some(true),
    };

    let revert_res = revert_mutations_client(&mut client, filter_agent, &agent_x)
        .await
        .unwrap();

    match revert_res {
        RevertExecutionResult::Reverted(res) => {
            assert_eq!(res.reverted_events, 2);
            assert!(res.affected_nodes.contains(&task_x1.task_id));
            assert!(res.affected_nodes.contains(&task_x2.task_id));
        }
        _ => panic!("Expected Reverted result"),
    }

    // Verify task X1 and X2 are SUPERSEDED, but task Y1 is still ACTIVE
    let x1_state: String = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&task_x1.task_id],
        )
        .await
        .unwrap()
        .get("lifecycle_state");
    assert_eq!(x1_state, "SUPERSEDED");

    let y1_state: String = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&task_y1.task_id],
        )
        .await
        .unwrap()
        .get("lifecycle_state");
    assert_eq!(y1_state, "ACTIVE");

    // Test event_seq_range filter on task Y1
    let filter_range = RevertFilter {
        batch_id: None,
        agent_id: None,
        since: None,
        event_seq_range: Some((task_y1.event_seq, task_y1.event_seq)),
        dry_run: Some(false),
        force: Some(true),
    };

    let revert_range_res = revert_mutations_client(&mut client, filter_range, &agent_y)
        .await
        .unwrap();
    match revert_range_res {
        RevertExecutionResult::Reverted(res) => {
            assert_eq!(res.reverted_events, 1);
            assert!(res.affected_nodes.contains(&task_y1.task_id));
        }
        _ => panic!("Expected Reverted result"),
    }

    let y1_after: String = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&task_y1.task_id],
        )
        .await
        .unwrap()
        .get("lifecycle_state");
    assert_eq!(y1_after, "SUPERSEDED");
}

#[tokio::test]
async fn test_multi_level_dependency_cascade_and_reverification() {
    let (_guard, _repo, mut client) = setup_test_db().await;

    let actor = AuthenticatedAgent {
        agent_id: "test_wp23_multi_level".to_string(),
        actor_type: "AGENT".to_string(),
    };

    let req_id = create_test_requirement(
        &client,
        "REQ-WP23-MULTI-01",
        "Multi-Level Requirement",
        &actor.agent_id,
    )
    .await;

    // Elaborate Level 1 (parent)
    let l1_res = elaborate_task_client(
        &mut client,
        &req_id.to_string(),
        "Level 1 Task",
        None,
        None,
        &actor,
    )
    .await
    .unwrap();

    // Elaborate Level 2 (child)
    let l2_res = elaborate_task_client(
        &mut client,
        &l1_res.task_id.to_string(),
        "Level 2 Task",
        None,
        None,
        &actor,
    )
    .await
    .unwrap();

    // Elaborate Level 3 (grandchild)
    let l3_res = elaborate_task_client(
        &mut client,
        &l2_res.task_id.to_string(),
        "Level 3 Task",
        None,
        None,
        &actor,
    )
    .await
    .unwrap();

    // Revert Level 1 task creation: both Level 2 and Level 3 must cascade to NEEDS_REVERIFICATION
    let filter = RevertFilter {
        batch_id: Some(l1_res.batch_id),
        agent_id: None,
        since: None,
        event_seq_range: None,
        dry_run: Some(false),
        force: Some(true),
    };

    let revert_res = revert_mutations_client(&mut client, filter, &actor)
        .await
        .unwrap();

    match revert_res {
        RevertExecutionResult::Reverted(res) => {
            assert!(res.affected_nodes.contains(&l1_res.task_id));
            assert!(res.cascade_reverified_nodes.contains(&l2_res.task_id));
            assert!(res.cascade_reverified_nodes.contains(&l3_res.task_id));
        }
        _ => panic!("Expected Reverted result"),
    }

    let l2_state: String = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&l2_res.task_id],
        )
        .await
        .unwrap()
        .get("lifecycle_state");
    assert_eq!(l2_state, "NEEDS_REVERIFICATION");

    let l3_state: String = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&l3_res.task_id],
        )
        .await
        .unwrap()
        .get("lifecycle_state");
    assert_eq!(l3_state, "NEEDS_REVERIFICATION");

    // Reverify Level 2 by re-parenting to req_id
    reverify_node_client(
        &mut client,
        &l2_res.task_id.to_string(),
        "Re-parent L2 directly to requirement",
        Some(serde_json::json!({ "reparent_to": req_id.to_string() })),
        &actor,
    )
    .await
    .unwrap();

    let l2_after: String = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&l2_res.task_id],
        )
        .await
        .unwrap()
        .get("lifecycle_state");
    assert_eq!(l2_after, "ACTIVE");

    // Now reverify Level 3 without re-parenting: since its parent L2 is now ACTIVE, reverification succeeds!
    let l3_rev = reverify_node_client(
        &mut client,
        &l3_res.task_id.to_string(),
        "Reverify L3 against restored L2",
        None,
        &actor,
    )
    .await
    .unwrap();

    assert_eq!(l3_rev.status, "ACTIVE");
    let l3_after: String = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&l3_res.task_id],
        )
        .await
        .unwrap()
        .get("lifecycle_state");
    assert_eq!(l3_after, "ACTIVE");
}
