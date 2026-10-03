//! Integration test suite for Mutation Engine, Strict Lock Acquisition Hierarchy,
//! DAG Cycle Prevention, and Audit Event Sequencing (WP-2.1).

use std::sync::Arc;
use tokio_postgres::Client;
use uuid::Uuid;

use tks::db;
use tks::storage::{
    EdgeMutationPayload, MutationError, NodeMutationPayload, StorageRepo, assert_ancestor_path,
    assert_no_dag_cycle, begin_structural_mutation, check_dag_cycle, insert_audit_event,
    insert_structural_edge, resolve_node_polymorphic, update_leaf_attributes_locked,
    validate_ancestor_path,
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
            "DELETE FROM audit_ledger WHERE actor_id LIKE 'test_wp21_%';",
            &[],
        )
        .await
        .expect("Cleanup audit_ledger failed");
    client
        .execute(
            "DELETE FROM graph_nodes WHERE created_by LIKE 'test_wp21_%';",
            &[],
        )
        .await
        .expect("Cleanup graph_nodes failed");

    let repo = StorageRepo::from_database_url(&database_url).expect("Failed to create repo");
    (guard, repo, client)
}

#[tokio::test]
async fn test_dag_cycle_rejection_transitive_and_self_loops() {
    let (_guard, _repo, client) = setup_test_db().await;
    let actor = "test_wp21_cycle_actor";

    // 1. Create three active nodes: A, B, C
    let id_a = Uuid::new_v4();
    let id_b = Uuid::new_v4();
    let id_c = Uuid::new_v4();

    for (id, key, title, ntype) in [
        (id_a, "REQ-WP21-CYC-A", "Cycle Node A", "REQUIREMENT"),
        (id_b, "SPEC-WP21-CYC-B", "Cycle Node B", "SPECIFICATION"),
        (id_c, "TASK-WP21-CYC-C", "Cycle Node C", "TASK"),
    ] {
        client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
                 VALUES ($1, $2, $3, $4, 'Testing cycle detection', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $5);",
                &[&id, &key, &ntype, &title, &actor],
            )
            .await
            .expect("Failed to insert node");
    }

    // 2. Insert edge A -> B (FULFILLS) and B -> C (FULFILLS)
    assert_no_dag_cycle(&client, id_a, id_b)
        .await
        .expect("assert_no_dag_cycle A -> B should pass");

    let edge1 = insert_structural_edge(&client, id_a, id_b, "FULFILLS", actor, "ACTIVE")
        .await
        .expect("Failed to insert A -> B edge");
    assert_eq!(edge1.from_node_id, id_a);
    assert_eq!(edge1.to_node_id, id_b);

    assert_no_dag_cycle(&client, id_b, id_c)
        .await
        .expect("assert_no_dag_cycle B -> C should pass");

    let edge2 = insert_structural_edge(&client, id_b, id_c, "FULFILLS", actor, "ACTIVE")
        .await
        .expect("Failed to insert B -> C edge");
    assert_eq!(edge2.from_node_id, id_b);
    assert_eq!(edge2.to_node_id, id_c);

    // 3. Test check_dag_cycle: path from C reaching A exists if we add C -> A
    let is_cycle = check_dag_cycle(&client, id_c, id_a)
        .await
        .expect("check_dag_cycle failed");
    assert!(
        is_cycle,
        "check_dag_cycle should detect cycle for proposed C -> A edge"
    );

    let assert_cycle_err = assert_no_dag_cycle(&client, id_c, id_a)
        .await
        .expect_err("assert_no_dag_cycle should fail for C -> A");
    assert!(matches!(
        assert_cycle_err,
        MutationError::CycleDetected { from_id, to_id } if from_id == id_c && to_id == id_a
    ));

    // 4. Test insert_structural_edge rejects C -> A with CycleDetected
    let cycle_err = insert_structural_edge(&client, id_c, id_a, "FULFILLS", actor, "ACTIVE")
        .await
        .expect_err("insert_structural_edge should have aborted on cycle");

    match cycle_err {
        MutationError::CycleDetected { from_id, to_id } => {
            assert_eq!(from_id, id_c);
            assert_eq!(to_id, id_a);
        }
        other => panic!("Expected MutationError::CycleDetected, got {other:?}"),
    }

    let err_str = cycle_err.to_string();
    assert!(
        err_str.contains("ERR_GRAPH_CYCLE_DETECTED"),
        "Error string must contain ERR_GRAPH_CYCLE_DETECTED: {err_str}"
    );

    // 5. Verify graph topology remains unaltered: C -> A must NOT exist
    let edge_exists = client
        .query_opt(
            "SELECT edge_id FROM graph_edges WHERE from_node_id = $1 AND to_node_id = $2;",
            &[&id_c, &id_a],
        )
        .await
        .expect("Query edges failed");
    assert!(
        edge_exists.is_none(),
        "Edge C -> A must not exist in graph_edges after cycle abort"
    );

    // 6. Test self-loop rejection: A -> A
    let self_loop_err = insert_structural_edge(&client, id_a, id_a, "FULFILLS", actor, "ACTIVE")
        .await
        .expect_err("Self loop A -> A must fail with CycleDetected");
    assert!(matches!(
        self_loop_err,
        MutationError::CycleDetected {
            from_id,
            to_id
        } if from_id == id_a && to_id == id_a
    ));
}

#[tokio::test]
async fn test_dag_cycle_rejection_candidate_draft_nodes() {
    let (_guard, _repo, client) = setup_test_db().await;
    let actor = "test_wp21_draft_cycle_actor";

    // Create three draft nodes: D, E, F
    let id_d = Uuid::new_v4();
    let id_e = Uuid::new_v4();
    let id_f = Uuid::new_v4();

    for id in [id_d, id_e, id_f] {
        client
            .execute(
                "INSERT INTO graph_nodes (id, node_type, title, content, lifecycle_state, governance_policy, created_by) \
                 VALUES ($1, 'TASK', 'Draft Candidate Node', 'Testing draft cycle detection', 'DRAFT', 'AUTONOMOUS_ELABORATION', $2);",
                &[&id, &actor],
            )
            .await
            .expect("Failed to insert draft node");
    }

    // Insert draft edge D -> E and E -> F
    insert_structural_edge(&client, id_d, id_e, "DERIVED_FROM", actor, "DRAFT")
        .await
        .expect("Failed to insert D -> E draft edge");
    insert_structural_edge(&client, id_e, id_f, "DERIVED_FROM", actor, "DRAFT")
        .await
        .expect("Failed to insert E -> F draft edge");

    // Attempt draft edge F -> D: must abort with CycleDetected
    let cycle_err = insert_structural_edge(&client, id_f, id_d, "DERIVED_FROM", actor, "DRAFT")
        .await
        .expect_err("Candidate draft cycle F -> D must abort with CycleDetected");

    assert!(matches!(
        cycle_err,
        MutationError::CycleDetected { from_id, to_id } if from_id == id_f && to_id == id_d
    ));
}

#[tokio::test]
async fn test_dag_acyclic_diamond_edges_permitted() {
    let (_guard, _repo, client) = setup_test_db().await;
    let actor = "test_wp21_diamond_actor";

    // Create diamond graph:
    //      D
    //     / \
    //    B   C
    //     \ /
    //      A
    let id_a = Uuid::new_v4();
    let id_b = Uuid::new_v4();
    let id_c = Uuid::new_v4();
    let id_d = Uuid::new_v4();

    for (id, key, ntype) in [
        (id_a, "TASK-WP21-DIA-A", "TASK"),
        (id_b, "SPEC-WP21-DIA-B", "SPECIFICATION"),
        (id_c, "SPEC-WP21-DIA-C", "SPECIFICATION"),
        (id_d, "REQ-WP21-DIA-D", "REQUIREMENT"),
    ] {
        client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
                 VALUES ($1, $2, $3, 'Diamond Node', 'Testing diamond topology', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $4);",
                &[&id, &key, &ntype, &actor],
            )
            .await
            .expect("Failed to insert node");
    }

    // A -> B, A -> C
    insert_structural_edge(&client, id_a, id_b, "FULFILLS", actor, "ACTIVE")
        .await
        .expect("A -> B should succeed");
    insert_structural_edge(&client, id_a, id_c, "FULFILLS", actor, "ACTIVE")
        .await
        .expect("A -> C should succeed");

    // B -> D, C -> D
    insert_structural_edge(&client, id_b, id_d, "FULFILLS", actor, "ACTIVE")
        .await
        .expect("B -> D should succeed");
    insert_structural_edge(&client, id_c, id_d, "FULFILLS", actor, "ACTIVE")
        .await
        .expect("C -> D should succeed");

    // Adding shortcut edge A -> D is acyclic and must succeed
    let shortcut_edge = insert_structural_edge(&client, id_a, id_d, "FULFILLS", actor, "ACTIVE")
        .await
        .expect("Shortcut A -> D in diamond should succeed");
    assert_eq!(shortcut_edge.from_node_id, id_a);
    assert_eq!(shortcut_edge.to_node_id, id_d);
}

#[tokio::test]
async fn test_strict_lock_hierarchy_concurrent_structural_and_leaf_updates() {
    let (_guard, repo, client) = setup_test_db().await;
    let actor = "test_wp21_concurrency_actor";

    // 1. Create a root requirement and 20 distinct task nodes
    let root_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'REQ-WP21-ROOT', 'REQUIREMENT', 'Root Requirement', 'Testing concurrency', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2);",
            &[&root_id, &actor],
        )
        .await
        .expect("Failed to insert root node");

    let mut task_ids = Vec::with_capacity(20);
    for i in 0..20 {
        let task_id = Uuid::new_v4();
        let key = format!("TASK-WP21-CONC-{i:02}");
        client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
                 VALUES ($1, $2, 'TASK', 'Concurrent Task', 'Testing concurrency', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $3);",
                &[&task_id, &key, &actor],
            )
            .await
            .expect("Failed to insert task node");
        task_ids.push(task_id);
    }

    let repo_arc = Arc::new(repo);
    let mut handles = Vec::new();

    // 2. Spawn 20 concurrent structural mutations (acquiring global advisory lock first)
    for task_id in task_ids.iter().copied() {
        let repo_clone = Arc::clone(&repo_arc);
        let root = root_id;
        let act = actor.to_string();
        handles.push(tokio::spawn(async move {
            let mut conn = repo_clone.get_client().await.expect("Failed to get client");
            // Begin structural mutation holding global advisory lock
            let tx = begin_structural_mutation(&mut conn)
                .await
                .expect("Failed to begin structural mutation");

            // Perform structural edge insertion
            let edge_res =
                insert_structural_edge(tx.client(), task_id, root, "FULFILLS", &act, "ACTIVE")
                    .await;

            // Commit transaction, releasing advisory lock
            tx.commit()
                .await
                .expect("Commit structural mutation failed");
            edge_res.map(|_| ())
        }));
    }

    // 3. Spawn 20 concurrent leaf attribute updates (native row-level lock without advisory lock)
    for (i, task_id) in task_ids.iter().copied().enumerate() {
        let repo_clone = Arc::clone(&repo_arc);
        let act = actor.to_string();
        handles.push(tokio::spawn(async move {
            let payload = NodeMutationPayload {
                title: Some(format!("Updated Concurrent Task {i}")),
                content: Some(format!("Updated content payload {i}")),
                attributes: Some(serde_json::json!({
                    "concurrency_iteration": i,
                    "execution_status": "IN_PROGRESS"
                })),
            };

            let res = repo_clone
                .update_leaf_attributes_locked(
                    task_id,
                    &payload,
                    &act,
                    "agent",
                    "fingerprint_concurrent_test",
                    None,
                )
                .await;

            res.map(|_| ())
        }));
    }

    // 4. Await all 40 concurrent operations: all must complete successfully with zero deadlocks (40P01)
    let mut succeeded_count = 0;
    for handle in handles {
        let task_result = handle.await.expect("Tokio task panicked");
        match task_result {
            Ok(()) => succeeded_count += 1,
            Err(e) => panic!("Concurrent operation failed with error: {e:?}"),
        }
    }

    assert_eq!(
        succeeded_count, 40,
        "All 20 structural mutations and 20 leaf updates must succeed without deadlock"
    );

    // Verify all 20 edges exist
    let edge_count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM graph_edges WHERE to_node_id = $1 AND edge_type = 'FULFILLS';",
            &[&root_id],
        )
        .await
        .expect("Query count failed")
        .get(0);
    assert_eq!(edge_count, 20, "All 20 edges must have been inserted");

    // Verify leaf updates committed
    let updated_titles: Vec<String> = client
        .query(
            "SELECT title FROM graph_nodes WHERE id = ANY($1) ORDER BY title;",
            &[&task_ids],
        )
        .await
        .expect("Query titles failed")
        .iter()
        .map(|r| r.get("title"))
        .collect();

    assert_eq!(updated_titles.len(), 20);
    assert!(
        updated_titles[0].starts_with("Updated Concurrent Task"),
        "Leaf attributes should reflect updates"
    );
}

#[tokio::test]
async fn test_polymorphic_node_resolution() {
    let (_guard, _repo, client) = setup_test_db().await;
    let actor = "test_wp21_poly_actor";
    let node_id = Uuid::new_v4();
    let node_key = "REQ-WP21-POLY-001";

    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, $2, 'REQUIREMENT', 'Polymorphic Test Node', 'Polymorphic content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $3);",
            &[&node_id, &node_key, &actor],
        )
        .await
        .expect("Failed to insert node");

    // 1. Resolve by UUID string
    let by_uuid = resolve_node_polymorphic(&client, &node_id.to_string())
        .await
        .expect("Failed to resolve by UUID");
    assert_eq!(by_uuid.id, node_id);
    assert_eq!(by_uuid.node_key.as_deref(), Some(node_key));

    // 2. Resolve by canonical node_key
    let by_key = resolve_node_polymorphic(&client, node_key)
        .await
        .expect("Failed to resolve by canonical key");
    assert_eq!(by_key.id, node_id);
    assert_eq!(by_key.node_key.as_deref(), Some(node_key));

    // 3. Resolve nonexistent UUID -> NotFound
    let missing_uuid = Uuid::new_v4();
    let missing_uuid_err = resolve_node_polymorphic(&client, &missing_uuid.to_string())
        .await
        .expect_err("Missing UUID must return NotFound");
    assert!(matches!(missing_uuid_err, MutationError::NotFound(_)));

    // 4. Resolve nonexistent key -> NotFound
    let missing_key_err = resolve_node_polymorphic(&client, "NONEXISTENT-KEY")
        .await
        .expect_err("Missing key must return NotFound");
    assert!(matches!(missing_key_err, MutationError::NotFound(_)));

    // 5. Resolve empty string -> NotFound
    let empty_err = resolve_node_polymorphic(&client, "   ")
        .await
        .expect_err("Empty string must return NotFound");
    assert!(matches!(empty_err, MutationError::NotFound(_)));
}

#[tokio::test]
async fn test_audit_event_sequencing_monotonicity_and_batch_correlation() {
    let (_guard, _repo, client) = setup_test_db().await;
    let actor_id = "test_wp21_audit_agent";
    let actor_type = "agent";
    let token_fingerprint = "fp_audit_sequence_test_12345";
    let batch_id = Uuid::new_v4();
    let entity_id = Uuid::new_v4();

    // Insert 5 audit events correlated by batch_id
    let mut event_seqs = Vec::with_capacity(5);
    for i in 0..5 {
        let delta = serde_json::json!({ "step": i, "action": "audit_event_test" });
        let snapshot = serde_json::json!({ "step": i, "state": "COMPLETED" });

        let seq = insert_audit_event(
            &client,
            batch_id,
            "TEST_MUTATION_STEP",
            entity_id,
            "TASK",
            actor_id,
            actor_type,
            token_fingerprint,
            delta,
            snapshot,
        )
        .await
        .expect("Failed to insert audit event");

        event_seqs.push(seq);
    }

    // Verify monotonic sequencing: seq[i] < seq[i+1]
    for i in 0..4 {
        assert!(
            event_seqs[i] < event_seqs[i + 1],
            "event_seq must be strictly monotonically increasing: {} not < {}",
            event_seqs[i],
            event_seqs[i + 1]
        );
    }

    // Query audit_ledger by batch_id
    let rows = client
        .query(
            "SELECT event_seq, batch_id, event_type, entity_id, entity_type, \
                    actor_id, actor_type, token_fingerprint, delta, snapshot \
             FROM audit_ledger \
             WHERE batch_id = $1 \
             ORDER BY event_seq ASC;",
            &[&batch_id],
        )
        .await
        .expect("Query audit_ledger failed");

    assert_eq!(rows.len(), 5, "Must find exactly 5 events for batch_id");
    for (i, row) in rows.iter().enumerate() {
        let seq: i64 = row.get("event_seq");
        let bid: Uuid = row.get("batch_id");
        let aid: String = row.get("actor_id");
        let atype: String = row.get("actor_type");
        let tfp: String = row.get("token_fingerprint");
        let delta: serde_json::Value = row.get("delta");

        assert_eq!(seq, event_seqs[i]);
        assert_eq!(bid, batch_id);
        assert_eq!(aid, actor_id);
        assert_eq!(atype, actor_type);
        assert_eq!(tfp, token_fingerprint);
        assert_eq!(delta["step"], i);
    }
}

#[tokio::test]
async fn test_inv1_ancestor_path_validation_cte() {
    let (_guard, _repo, client) = setup_test_db().await;
    let actor = "test_wp21_inv1_actor";

    // 1. Active requirement root
    let root_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'REQ-WP21-INV1-ROOT', 'REQUIREMENT', 'INV1 Root', 'Requirement content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2);",
            &[&root_id, &actor],
        )
        .await
        .expect("Failed to insert requirement root");

    // 2. Task 1 with unbroken path to requirement root: T1 -FULFILLS-> Root
    let t1_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'TASK-WP21-INV1-T1', 'TASK', 'Valid Task 1', 'Task content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2);",
            &[&t1_id, &actor],
        )
        .await
        .expect("Failed to insert task 1");

    insert_structural_edge(&client, t1_id, root_id, "FULFILLS", actor, "ACTIVE")
        .await
        .expect("Failed to insert T1 -> Root edge");

    // Validating T1 must return true
    let t1_valid = validate_ancestor_path(&client, &[t1_id], false)
        .await
        .expect("validate_ancestor_path failed");
    assert!(t1_valid, "Task 1 with path to active root must validate");
    assert_ancestor_path(&client, &[t1_id], false)
        .await
        .expect("assert_ancestor_path should succeed for T1");

    // 3. Orphan Task 2 with no upward edges
    let t2_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'TASK-WP21-INV1-T2', 'TASK', 'Orphan Task 2', 'Task content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2);",
            &[&t2_id, &actor],
        )
        .await
        .expect("Failed to insert task 2");

    let t2_valid = validate_ancestor_path(&client, &[t2_id], false)
        .await
        .expect("validate_ancestor_path failed for orphan");
    assert!(
        !t2_valid,
        "Orphan task without requirement root must fail validation"
    );

    let t2_assert_err = assert_ancestor_path(&client, &[t2_id], false)
        .await
        .expect_err("assert_ancestor_path must fail for orphan task");
    assert!(matches!(
        t2_assert_err,
        MutationError::InvalidAncestorPath(_)
    ));

    // 4. Task 3 chaining through Task 1: T3 -DERIVED_FROM-> T1 -FULFILLS-> Root
    let t3_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'TASK-WP21-INV1-T3', 'TASK', 'Chained Task 3', 'Task content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2);",
            &[&t3_id, &actor],
        )
        .await
        .expect("Failed to insert task 3");

    insert_structural_edge(&client, t3_id, t1_id, "DERIVED_FROM", actor, "ACTIVE")
        .await
        .expect("Failed to insert T3 -> T1 edge");

    let t3_valid = validate_ancestor_path(&client, &[t3_id], false)
        .await
        .expect("validate_ancestor_path failed for chained task");
    assert!(
        t3_valid,
        "Chained task T3 reaching requirement root via T1 must validate"
    );
    assert_ancestor_path(&client, &[t3_id], false)
        .await
        .expect("assert_ancestor_path should succeed for T3");
}

#[tokio::test]
async fn test_edge_mutation_payload_and_direct_leaf_update() {
    let (_guard, repo, mut client) = setup_test_db().await;
    let actor = "test_wp21_payload_actor";

    // 1. Exercise EdgeMutationPayload
    let payload = EdgeMutationPayload {
        from_id: Uuid::new_v4(),
        to_id: Uuid::new_v4(),
        edge_type: "FULFILLS".to_string(),
        created_by: actor.to_string(),
        lifecycle_state: "ACTIVE".to_string(),
    };
    let json_bytes = serde_json::to_string(&payload).expect("Serialize EdgeMutationPayload");
    assert!(json_bytes.contains("FULFILLS"));

    // 2. Direct update_leaf_attributes_locked with &mut client
    let node_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'TASK-WP21-DIRECT', 'TASK', 'Initial Title', 'Initial Content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2);",
            &[&node_id, &actor],
        )
        .await
        .expect("Failed to insert test node");

    let leaf_payload = NodeMutationPayload {
        title: Some("Direct Updated Title".to_string()),
        content: None,
        attributes: Some(serde_json::json!({ "direct_test": true })),
    };

    let (updated_node, event_seq) = update_leaf_attributes_locked(
        &mut client,
        node_id,
        &leaf_payload,
        actor,
        "human",
        "fingerprint_direct_test",
        None,
    )
    .await
    .expect("update_leaf_attributes_locked direct call failed");

    assert_eq!(updated_node.title.as_deref(), Some("Direct Updated Title"));
    assert_eq!(updated_node.content.as_deref(), Some("Initial Content"));
    assert_eq!(
        updated_node.attributes.get("direct_test"),
        Some(&serde_json::json!(true))
    );
    assert!(event_seq > 0);

    // 3. Test repo.execute_structural_mutation helper
    let subtask_id = Uuid::new_v4();
    let edge = repo
        .execute_structural_mutation(|tx| {
            Box::pin(async move {
                // Insert subtask node inside transaction
                tx.execute(
                    "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
                     VALUES ($1, 'TASK-WP21-SUBTASK', 'TASK', 'Subtask', 'Subtask Content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2);",
                    &[&subtask_id, &actor],
                )
                .await?;

                // Insert edge inside transaction
                let edge = insert_structural_edge(
                    tx.client(),
                    subtask_id,
                    node_id,
                    "DERIVED_FROM",
                    actor,
                    "ACTIVE",
                )
                .await?;

                Ok(edge)
            })
        })
        .await
        .expect("execute_structural_mutation should succeed");

    assert_eq!(edge.from_node_id, subtask_id);
    assert_eq!(edge.to_node_id, node_id);
}
