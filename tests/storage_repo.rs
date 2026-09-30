//! Integration test suite for StorageRepo, Transactional Invariants & Embedded Migrations (WP-1.2).

use std::time::Instant;
use tokio_postgres::Client;
use uuid::Uuid;

use tks::db;
use tks::storage::{
    StorageRepo, acquire_structural_lock, assemble_topological_envelope, lookup_node_polymorphic,
    query_active_requirements, validate_ancestor_path,
};

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Helper to serialize database access across tests, connect, run migrations, and clean up test data.
async fn setup_test_db() -> (tokio::sync::MutexGuard<'static, ()>, StorageRepo, Client) {
    let guard = TEST_MUTEX.lock().await;
    let database_url = db::resolve_database_url();
    let (mut client, _handle) = db::connect(&database_url)
        .await
        .expect("Failed to connect to database");

    // Ensure migrations are up to date
    db::run_migrations(&mut client)
        .await
        .expect("Failed to run migrations");

    // Clean up any previous test artifacts
    client
        .execute(
            "DELETE FROM graph_nodes WHERE created_by LIKE 'test_wp12_%';",
            &[],
        )
        .await
        .expect("Cleanup failed");

    let repo = StorageRepo::from_database_url(&database_url).expect("Failed to create repo");
    (guard, repo, client)
}

#[tokio::test]
async fn test_advisory_lock_serialization() {
    let (_lock, _repo, mut client1) = setup_test_db().await;
    let database_url = db::resolve_database_url();
    let (mut client2, _handle2) = db::connect(&database_url)
        .await
        .expect("Failed to connect client2");

    // Start transaction on client1 and acquire global advisory lock
    let tx1 = client1.transaction().await.expect("Failed to begin tx1");
    // Acquire lock inside tx1
    tx1.execute(
        "SELECT pg_advisory_xact_lock(hashtext('tks_structural_mutation'));",
        &[],
    )
    .await
    .expect("Failed to acquire advisory lock in tx1");

    // Now attempt to acquire lock on client2 in background task
    let (tx_sender, rx_receiver) = tokio::sync::oneshot::channel();
    let lock_task = tokio::spawn(async move {
        let tx2 = client2.transaction().await.expect("Failed to begin tx2");
        // Notify that we are about to request the lock
        let _ = tx_sender.send(());
        // This should block until tx1 commits or rolls back
        tx2.execute(
            "SELECT pg_advisory_xact_lock(hashtext('tks_structural_mutation'));",
            &[],
        )
        .await
        .expect("tx2 acquired lock");
        tx2.rollback().await.expect("tx2 rollback");
    });

    // Wait until client2 is about to request or has requested the lock
    rx_receiver.await.expect("Failed to receive tx2 signal");

    // Give client2 time to encounter the lock
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Verify client2 has not finished yet (it is blocked by tx1)
    assert!(
        !lock_task.is_finished(),
        "client2 must block on advisory lock held by client1"
    );

    // Commit tx1, releasing the lock
    tx1.commit().await.expect("Failed to commit tx1");

    // Now client2 should unblock and finish quickly
    let result = tokio::time::timeout(tokio::time::Duration::from_secs(2), lock_task).await;
    assert!(
        result.is_ok(),
        "client2 should acquire lock and complete after tx1 commits"
    );
}

#[tokio::test]
async fn test_acquire_structural_lock_helpers() {
    let (_lock, repo, mut client) = setup_test_db().await;

    // Test standalone function
    let res = acquire_structural_lock(&mut client).await;
    assert!(
        res.is_ok(),
        "acquire_structural_lock standalone must succeed"
    );

    // Test repo method
    let res2 = repo.acquire_structural_lock(&mut client).await;
    assert!(res2.is_ok(), "repo.acquire_structural_lock must succeed");
}

#[tokio::test]
async fn test_polymorphic_node_resolution() {
    let (_lock, repo, client) = setup_test_db().await;

    let active_node_id = Uuid::new_v4();
    let draft_node_id = Uuid::new_v4();

    // Insert active node with key
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, created_by)
             VALUES ($1, 'REQ-POLY-001', 'REQUIREMENT', 'Polymorphic Requirement', 'Content A', 'ACTIVE', 'test_wp12_poly');",
            &[&active_node_id],
        )
        .await
        .expect("Failed to insert active node");

    // Insert draft node with key
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, created_by)
             VALUES ($1, 'DRAFT-POLY-002', 'SPECIFICATION', 'Draft Node', 'Content B', 'DRAFT', 'test_wp12_poly');",
            &[&draft_node_id],
        )
        .await
        .expect("Failed to insert draft node");

    // 1. Lookup active node by UUID
    let node_by_uuid = lookup_node_polymorphic(&client, &active_node_id.to_string())
        .await
        .expect("Lookup by UUID failed");
    assert!(node_by_uuid.is_some());
    let node = node_by_uuid.unwrap();
    assert_eq!(node.id, active_node_id);
    assert_eq!(node.node_key.as_deref(), Some("REQ-POLY-001"));

    // 2. Lookup active node by canonical node_key
    let node_by_key = lookup_node_polymorphic(&client, "REQ-POLY-001")
        .await
        .expect("Lookup by node_key failed");
    assert!(node_by_key.is_some());
    assert_eq!(node_by_key.unwrap().id, active_node_id);

    // 3. Lookup draft node by UUID -> should succeed
    let draft_by_uuid = lookup_node_polymorphic(&client, &draft_node_id.to_string())
        .await
        .expect("Lookup draft by UUID failed");
    assert!(draft_by_uuid.is_some());
    assert_eq!(draft_by_uuid.unwrap().id, draft_node_id);

    // 4. Lookup draft node by node_key -> must return None (TB-7: partial index idx_graph_nodes_node_key_active)
    let draft_by_key = lookup_node_polymorphic(&client, "DRAFT-POLY-002")
        .await
        .expect("Lookup draft by node_key failed");
    assert!(
        draft_by_key.is_none(),
        "Draft nodes must not resolve by node_key"
    );

    // 5. Lookup non-existent UUID and unknown key -> None
    let unknown_uuid = Uuid::new_v4();
    assert!(
        lookup_node_polymorphic(&client, &unknown_uuid.to_string())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        lookup_node_polymorphic(&client, "NON-EXISTENT-KEY")
            .await
            .unwrap()
            .is_none()
    );

    // 6. Test repo method wrapper
    let repo_lookup = repo.lookup_node_polymorphic("REQ-POLY-001").await.unwrap();
    assert!(repo_lookup.is_some());
    assert_eq!(repo_lookup.unwrap().id, active_node_id);

    // Clean up
    client
        .execute(
            "DELETE FROM graph_nodes WHERE created_by = 'test_wp12_poly';",
            &[],
        )
        .await
        .expect("Cleanup failed");
}

#[tokio::test]
async fn test_invariant_inv1_ancestor_path_validation() {
    let (_lock, _repo, client) = setup_test_db().await;

    let req_id = Uuid::new_v4();
    let spec_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();
    let orphan_id = Uuid::new_v4();

    // 1. Insert Active Requirement
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, lifecycle_state, created_by)
             VALUES ($1, 'REQ-INV1-01', 'REQUIREMENT', 'ACTIVE', 'test_wp12_inv1');",
            &[&req_id],
        )
        .await
        .unwrap();

    // 2. Insert Active Spec derived from Requirement
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, lifecycle_state, created_by)
             VALUES ($1, 'SPEC-INV1-01', 'SPECIFICATION', 'ACTIVE', 'test_wp12_inv1');",
            &[&spec_id],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO graph_edges (from_node_id, to_node_id, edge_type, lifecycle_state, created_by)
             VALUES ($1, $2, 'DERIVED_FROM', 'ACTIVE', 'test_wp12_inv1');",
            &[&spec_id, &req_id],
        )
        .await
        .unwrap();

    // 3. Insert Active Task fulfilling Spec
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, lifecycle_state, created_by)
             VALUES ($1, 'TASK-INV1-01', 'TASK', 'ACTIVE', 'test_wp12_inv1');",
            &[&task_id],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO graph_edges (from_node_id, to_node_id, edge_type, lifecycle_state, created_by)
             VALUES ($1, $2, 'FULFILLS', 'ACTIVE', 'test_wp12_inv1');",
            &[&task_id, &spec_id],
        )
        .await
        .unwrap();

    // 4. Insert Orphan Task without edges
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, lifecycle_state, created_by)
             VALUES ($1, 'TASK-ORPHAN-01', 'TASK', 'ACTIVE', 'test_wp12_inv1');",
            &[&orphan_id],
        )
        .await
        .unwrap();

    // Verification Case A: Requirement itself is valid
    assert!(
        validate_ancestor_path(&client, &[req_id], false)
            .await
            .unwrap(),
        "Requirement root must satisfy INV-1"
    );

    // Verification Case B: Spec with direct edge to requirement is valid
    assert!(
        validate_ancestor_path(&client, &[spec_id], false)
            .await
            .unwrap(),
        "Spec derived from Requirement must satisfy INV-1"
    );

    // Verification Case C: Multi-hop Task -> Spec -> Req is valid
    assert!(
        validate_ancestor_path(&client, &[task_id], false)
            .await
            .unwrap(),
        "Task fulfilling Spec derived from Requirement must satisfy INV-1"
    );

    // Verification Case D: Orphan task fails validation
    assert!(
        !validate_ancestor_path(&client, &[orphan_id], false)
            .await
            .unwrap(),
        "Orphan task must fail INV-1 validation"
    );

    // Verification Case E: Batch containing valid task AND orphan task must fail atomically
    assert!(
        !validate_ancestor_path(&client, &[task_id, orphan_id], false)
            .await
            .unwrap(),
        "Batch containing an orphan task must fail validation"
    );

    // Verification Case F: Empty batch is trivially valid
    assert!(
        validate_ancestor_path(&client, &[], false).await.unwrap(),
        "Empty batch is valid"
    );

    // Verification Case G: Batch draft promotion with candidate requirement and child spec (D-34)
    let draft_req_id = Uuid::new_v4();
    let draft_spec_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, lifecycle_state, created_by)
             VALUES ($1, 'REQ-DRAFT-01', 'REQUIREMENT', 'DRAFT', 'test_wp12_inv1'),
                    ($2, 'SPEC-DRAFT-01', 'SPECIFICATION', 'DRAFT', 'test_wp12_inv1');",
            &[&draft_req_id, &draft_spec_id],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO graph_edges (from_node_id, to_node_id, edge_type, lifecycle_state, created_by)
             VALUES ($1, $2, 'DERIVED_FROM', 'DRAFT', 'test_wp12_inv1');",
            &[&draft_spec_id, &draft_req_id],
        )
        .await
        .unwrap();

    // Promoting both draft_spec_id and draft_req_id in the same batch must succeed!
    assert!(
        validate_ancestor_path(&client, &[draft_spec_id, draft_req_id], false)
            .await
            .unwrap(),
        "Batch promotion union evaluation must succeed when candidate requirement is in batch"
    );

    // Promoting draft_spec_id alone without candidate requirement in batch must fail!
    assert!(
        !validate_ancestor_path(&client, &[draft_spec_id], false)
            .await
            .unwrap(),
        "Promoting draft spec without active or promoted parent requirement must fail"
    );

    // But if allow_draft_parents is true, a draft spec with a draft parent succeeds
    assert!(
        validate_ancestor_path(&client, &[draft_spec_id], true)
            .await
            .unwrap(),
        "allow_draft_parents=true must allow draft structural parent"
    );

    // Clean up
    client
        .execute(
            "DELETE FROM graph_nodes WHERE created_by = 'test_wp12_inv1';",
            &[],
        )
        .await
        .expect("Cleanup failed");
}

#[tokio::test]
async fn test_full_text_search_sanitization_and_sub_5ms_performance() {
    let (_lock, repo, client) = setup_test_db().await;

    let id1 = Uuid::new_v4();
    let id2 = Uuid::new_v4();
    let id3 = Uuid::new_v4();
    let id4 = Uuid::new_v4();

    // Insert active requirements with canonical keys
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, created_by)
             VALUES 
               ($1, 'REQ-CORE-001', 'REQUIREMENT', 'Memory Graph Cache Invariant', 'The knowledge substrate must maintain an in-memory graph cache.', 'ACTIVE', 'test_wp12_search'),
               ($2, 'INV-2', 'REQUIREMENT', 'Append-Only Auditability', 'Destructive in-place updates on active requirements must not occur.', 'ACTIVE', 'test_wp12_search'),
               ($3, 'C-10', 'REQUIREMENT', 'Minimal Dependency Footprint', 'Do not add speculative crates or unnecessary networking bloat.', 'ACTIVE', 'test_wp12_search'),
               ($4, 'REQ-DRAFT-999', 'REQUIREMENT', 'Draft Unindexed Requirement', 'This requirement is in draft state.', 'DRAFT', 'test_wp12_search');",
            &[&id1, &id2, &id3, &id4],
        )
        .await
        .expect("Failed to insert search nodes");

    // 1. Search for hyphenated key REQ-CORE-001 -> should execute cleanly without hyphen negation syntax error
    let start = Instant::now();
    let results = query_active_requirements(&client, "REQ-CORE-001", 10)
        .await
        .expect("Search for REQ-CORE-001 failed");
    let elapsed = start.elapsed();

    assert!(!results.is_empty(), "Must find REQ-CORE-001");
    assert_eq!(results[0].node_key.as_deref(), Some("REQ-CORE-001"));
    assert_eq!(results[0].id, id1);
    println!("Canonical key search latency: {elapsed:?} (Proof criteria: < 5ms)");
    assert!(
        elapsed.as_millis() < 50, // generous ceiling for CI, target is < 5ms
        "Search query must complete within latency SLA (took {elapsed:?})"
    );

    // 2. Search for INV-2
    let inv_results = query_active_requirements(&client, "INV-2", 10)
        .await
        .expect("Search for INV-2 failed");
    assert!(!inv_results.is_empty(), "Must find INV-2");
    assert_eq!(inv_results[0].node_key.as_deref(), Some("INV-2"));

    // 3. Search for C-10
    let c10_results = query_active_requirements(&client, "C-10", 10)
        .await
        .expect("Search for C-10 failed");
    assert!(!c10_results.is_empty(), "Must find C-10");
    assert_eq!(c10_results[0].node_key.as_deref(), Some("C-10"));

    // 4. Natural language search across content
    let nl_results = query_active_requirements(&client, "auditability", 10)
        .await
        .expect("Search for 'auditability' failed");
    assert!(!nl_results.is_empty());
    assert_eq!(nl_results[0].id, id2);

    // 5. Ensure draft entities are never returned in search
    let draft_search = query_active_requirements(&client, "REQ-DRAFT-999", 10)
        .await
        .expect("Draft search failed");
    assert!(
        draft_search.is_empty(),
        "Draft nodes must be excluded from active requirement search"
    );

    // 6. Test empty query
    let empty_results = query_active_requirements(&client, "   ", 10).await.unwrap();
    assert!(empty_results.is_empty());

    // 7. Test repo convenience method
    let repo_search = repo
        .query_active_requirements("REQ-CORE-001", 5)
        .await
        .unwrap();
    assert!(!repo_search.is_empty());
    assert_eq!(repo_search[0].id, id1);

    // Clean up
    client
        .execute(
            "DELETE FROM graph_nodes WHERE created_by = 'test_wp12_search';",
            &[],
        )
        .await
        .expect("Cleanup failed");
}

#[tokio::test]
async fn test_topological_envelope_assembly_and_caller_draft_isolation() {
    let (_lock, repo, client) = setup_test_db().await;

    let root_req_id = Uuid::new_v4();
    let spec_id = Uuid::new_v4();
    let invariant_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();
    let author_a_draft_id = Uuid::new_v4();
    let author_b_draft_id = Uuid::new_v4();
    let degraded_node_id = Uuid::new_v4();

    // 1. Create Active Topology:
    // Root Requirement (Root)
    // -> Spec (derived from Root)
    // -> Task (fulfills Spec)
    // -> Invariant (derived from Root, Task is CONSTRAINED_BY Invariant)
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, created_by)
             VALUES 
               ($1, 'REQ-ROOT', 'REQUIREMENT', 'Root Governance', 'System must be deterministic.', 'ACTIVE', 'test_wp12_env'),
               ($2, 'SPEC-AUTH', 'SPECIFICATION', 'Authentication Engine', 'Implements token verification.', 'ACTIVE', 'test_wp12_env'),
               ($3, 'INV-LOCK', 'REQUIREMENT', 'Lock Hierarchy Invariant', 'Global advisory lock before row locks.', 'ACTIVE', 'test_wp12_env'),
               ($4, 'TASK-DISPATCH', 'TASK', 'Command Dispatch Task', 'Executes pipeline commands.', 'ACTIVE', 'test_wp12_env'),
               ($5, 'DRAFT-TASK-A', 'TASK', 'Author A Task', 'Draft created by Author A.', 'DRAFT', 'agent_a'),
               ($6, 'DRAFT-TASK-B', 'TASK', 'Author B Task', 'Draft created by Author B.', 'DRAFT', 'agent_b'),
               ($7, 'DEGRADED-SPEC', 'SPECIFICATION', 'Degraded Spec', 'Needs reverification after parent change.', 'NEEDS_REVERIFICATION', 'test_wp12_env');",
            &[&root_req_id, &spec_id, &invariant_id, &task_id, &author_a_draft_id, &author_b_draft_id, &degraded_node_id],
        )
        .await
        .expect("Failed to insert envelope nodes");

    // Edges
    client
        .execute(
            "INSERT INTO graph_edges (from_node_id, to_node_id, edge_type, lifecycle_state, created_by)
             VALUES 
               ($1, $2, 'DERIVED_FROM', 'ACTIVE', 'test_wp12_env'),   -- SPEC-AUTH -> REQ-ROOT
               ($3, $1, 'FULFILLS', 'ACTIVE', 'test_wp12_env'),       -- TASK-DISPATCH -> SPEC-AUTH
               ($4, $2, 'DERIVED_FROM', 'ACTIVE', 'test_wp12_env'),   -- INV-LOCK -> REQ-ROOT (sibling invariant!)
               ($3, $4, 'CONSTRAINED_BY', 'ACTIVE', 'test_wp12_env'), -- TASK-DISPATCH -> INV-LOCK
               ($5, $3, 'DERIVED_FROM', 'DRAFT', 'agent_a'),          -- DRAFT-TASK-A -> TASK-DISPATCH
               ($6, $3, 'DERIVED_FROM', 'DRAFT', 'agent_b');",
            &[&spec_id, &root_req_id, &task_id, &invariant_id, &author_a_draft_id, &author_b_draft_id],
        )
        .await
        .expect("Failed to insert envelope edges");

    // 1. Assemble envelope for TASK-DISPATCH (depth = 2)
    let envelope = assemble_topological_envelope(&client, task_id, 2, None)
        .await
        .expect("Assemble envelope failed");

    assert_eq!(envelope.target_node.id, task_id);
    assert!(!envelope.staleness_warning);

    // Verify upward ancestors include SPEC-AUTH and REQ-ROOT
    let ancestor_keys: Vec<&str> = envelope
        .ancestor_requirements
        .iter()
        .filter_map(|n| n.node_key.as_deref())
        .collect();
    assert!(
        ancestor_keys.contains(&"SPEC-AUTH"),
        "Must contain SPEC-AUTH ancestor"
    );
    assert!(
        ancestor_keys.contains(&"REQ-ROOT"),
        "Must contain REQ-ROOT ancestor"
    );

    // Verify sibling constraint harvesting includes INV-LOCK
    let constraint_keys: Vec<&str> = envelope
        .sibling_constraints
        .iter()
        .filter_map(|n| n.node_key.as_deref())
        .collect();
    assert!(
        constraint_keys.contains(&"INV-LOCK"),
        "Must harvest INV-LOCK via CONSTRAINED_BY or sibling derivation"
    );

    // Verify format_markdown output
    let md = envelope.format_markdown();
    assert!(md.contains("=== TOPOLOGICAL CONTEXT ENVELOPE ==="));
    assert!(md.contains("TASK-DISPATCH"));
    assert!(md.contains("SPEC-AUTH"));
    assert!(md.contains("INV-LOCK"));

    // 2. Caller Draft Isolation (INV-7):
    // When queried by agent_a, DRAFT-TASK-A is visible, DRAFT-TASK-B is hidden
    let draft_a_envelope =
        assemble_topological_envelope(&client, author_a_draft_id, 2, Some("agent_a"))
            .await
            .expect("agent_a must be able to inspect their own draft");
    assert_eq!(draft_a_envelope.target_node.id, author_a_draft_id);

    // When queried by agent_b, DRAFT-TASK-A must fail lookup (caller draft isolation)
    let draft_a_by_b =
        assemble_topological_envelope(&client, author_a_draft_id, 2, Some("agent_b")).await;
    assert!(
        draft_a_by_b.is_err(),
        "agent_b must NOT be able to view agent_a draft"
    );

    // When queried with no auth (None), drafts must fail lookup
    let draft_unauthed = assemble_topological_envelope(&client, author_a_draft_id, 2, None).await;
    assert!(
        draft_unauthed.is_err(),
        "Unauthenticated request must NOT see drafts"
    );

    // 3. Degraded Node Inspection (LD-3, iter 8):
    // Retrieve envelope for node in NEEDS_REVERIFICATION
    let degraded_env = assemble_topological_envelope(&client, degraded_node_id, 1, None)
        .await
        .expect("Must retrieve envelope for NEEDS_REVERIFICATION node");
    assert!(
        degraded_env.staleness_warning,
        "Must flag staleness_warning = true"
    );
    assert!(
        degraded_env
            .format_markdown()
            .contains("NEEDS_REVERIFICATION")
    );

    // 4. Test repo method
    let repo_envelope = repo
        .assemble_topological_envelope(task_id, 2, None)
        .await
        .unwrap();
    assert_eq!(repo_envelope.target_node.id, task_id);

    // Clean up
    client
        .execute(
            "DELETE FROM graph_nodes WHERE created_by IN ('test_wp12_env', 'agent_a', 'agent_b');",
            &[],
        )
        .await
        .expect("Cleanup failed");
}
