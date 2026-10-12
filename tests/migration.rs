//! Integration test verifying database migration execution and schema integrity.

use tks::db;

#[tokio::test]
async fn test_migration_and_schema_verification() {
    let database_url = db::ensure_test_database_ready()
        .await
        .expect("Failed to prepare test database");

    let (mut client, _handle) = db::connect(&database_url)
        .await
        .expect("Failed to connect to database");

    // Execute migrations
    let report = db::run_migrations(&mut client)
        .await
        .expect("Failed to run migrations");

    tracing::info!(
        "Applied migrations count: {}",
        report.applied_migrations().len()
    );

    // Clean up any leftover test records from prior runs
    let _ = client
        .execute("DELETE FROM graph_nodes WHERE node_key LIKE 'TEST-%';", &[])
        .await;

    // 1. Verify required tables exist in information_schema.tables
    let expected_tables = vec![
        "graph_nodes",
        "graph_edges",
        "node_embeddings",
        "audit_ledger",
        "ingestion_jobs",
        "agent_identities",
    ];

    for table in &expected_tables {
        let row = client
            .query_one(
                "SELECT EXISTS (
                    SELECT 1 FROM information_schema.tables
                    WHERE table_schema = 'public' AND table_name = $1
                );",
                &[table],
            )
            .await
            .unwrap_or_else(|e| panic!("Error checking table existence for {table}: {e}"));

        let exists: bool = row.get(0);
        assert!(exists, "Expected table '{table}' does not exist");
    }

    // Verify attributes column on ingestion_jobs (TB-7.5)
    let attr_col = client
        .query_one(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_name = 'ingestion_jobs' AND column_name = 'attributes'
            );",
            &[],
        )
        .await
        .expect("Error checking attributes column on ingestion_jobs");
    let attr_exists: bool = attr_col.get(0);
    assert!(
        attr_exists,
        "Expected column 'attributes' on ingestion_jobs"
    );

    // 2. Verify expected partial indexes exist and are valid in pg_indexes
    let expected_indexes = vec![
        "idx_node_embeddings_vector",
        "idx_node_embeddings_pending",
        "idx_graph_edges_to_node_active",
        "idx_graph_nodes_draft_author",
        "idx_graph_nodes_node_key_active",
        "idx_graph_nodes_doc_path",
        "idx_graph_nodes_search_tsv",
        "idx_graph_edges_active_unique",
        "idx_graph_edges_draft_author",
        "idx_graph_edges_type_active",
        "idx_audit_ledger_entity",
        "idx_audit_ledger_batch",
        "idx_audit_ledger_created_at",
    ];

    for index in &expected_indexes {
        let row = client
            .query_one(
                "SELECT EXISTS (
                    SELECT 1 FROM pg_indexes
                    WHERE schemaname = 'public' AND indexname = $1
                );",
                &[index],
            )
            .await
            .unwrap_or_else(|e| panic!("Error checking index existence for {index}: {e}"));

        let exists: bool = row.get(0);
        assert!(exists, "Expected index '{index}' does not exist");
    }

    // 3. Verify development seed identity (TB-5)
    let identity_row = client
        .query_one(
            "SELECT agent_id, token_hash, actor_type, is_active FROM agent_identities WHERE agent_id = $1;",
            &[&"tks_dev_token"],
        )
        .await
        .expect("Dev token 'tks_dev_token' must exist in agent_identities");

    let agent_id: String = identity_row.get(0);
    let actor_type: String = identity_row.get(2);
    let is_active: bool = identity_row.get(3);

    assert_eq!(agent_id, "tks_dev_token");
    assert_eq!(actor_type, "HUMAN");
    assert!(is_active, "tks_dev_token must be active");

    // 4. Verify D-62 check constraint: UNCLASSIFIED allowed in DRAFT, rejected in ACTIVE
    let insert_draft = client
        .execute(
            "INSERT INTO graph_nodes (node_key, node_type, title, content, lifecycle_state, created_by)
             VALUES ('TEST-DRAFT-001', 'UNCLASSIFIED', 'Draft Node', 'Draft Content', 'DRAFT', 'test_user');",
            &[],
        )
        .await;
    assert!(
        insert_draft.is_ok(),
        "UNCLASSIFIED must be allowed when lifecycle_state is DRAFT"
    );

    let insert_active_unclassified = client
        .execute(
            "INSERT INTO graph_nodes (node_key, node_type, title, content, lifecycle_state, created_by)
             VALUES ('TEST-ACTIVE-001', 'UNCLASSIFIED', 'Active Node', 'Active Content', 'ACTIVE', 'test_user');",
            &[],
        )
        .await;
    assert!(
        insert_active_unclassified.is_err(),
        "UNCLASSIFIED must be rejected when lifecycle_state is ACTIVE"
    );

    // 5. Verify vector(384) column and HNSW index compatibility on node_embeddings (D-77)
    let node_row = client
        .query_one(
            "INSERT INTO graph_nodes (node_key, node_type, title, content, lifecycle_state, created_by)
             VALUES ('TEST-VEC-001', 'REQUIREMENT', 'Vector Node', 'Testing vector column', 'ACTIVE', 'test_user')
             RETURNING id;",
            &[],
        )
        .await
        .expect("Failed to insert test requirement node for vector test");
    let test_node_id: uuid::Uuid = node_row.get(0);

    let insert_embedding = client
        .execute(
            "INSERT INTO node_embeddings (node_id, content_hash, embedding, status)
             VALUES ($1, 'dummy_hash', array_fill(0.0::real, ARRAY[384])::vector, 'COMPLETED');",
            &[&test_node_id],
        )
        .await;
    if let Err(e) = &insert_embedding {
        panic!("vector(384) embedding insert failed: {e}");
    }

    // 6. Verify V7 Goal Model node types and chk_goal_kind constraint (EB-3)
    // 6a. GOAL requires attributes->>'goal_kind'
    let insert_goal_no_kind = client
        .execute(
            "INSERT INTO graph_nodes (node_key, node_type, title, content, lifecycle_state, created_by, attributes)
             VALUES ('TEST-GOAL-001', 'GOAL', 'Goal Node', 'Goal without kind', 'DRAFT', 'test_user', '{}');",
            &[],
        )
        .await;
    assert!(
        insert_goal_no_kind.is_err(),
        "GOAL node without goal_kind must be rejected by chk_goal_kind"
    );

    // 6b. GOAL rejects invalid goal_kind
    let insert_goal_invalid_kind = client
        .execute(
            "INSERT INTO graph_nodes (node_key, node_type, title, content, lifecycle_state, created_by, attributes)
             VALUES ('TEST-GOAL-002', 'GOAL', 'Goal Node', 'Invalid kind', 'DRAFT', 'test_user', '{\"goal_kind\": \"INVALID\"}');",
            &[],
        )
        .await;
    assert!(
        insert_goal_invalid_kind.is_err(),
        "GOAL node with invalid goal_kind must be rejected by chk_goal_kind"
    );

    // 6c. GOAL accepts valid goal_kinds in both DRAFT and ACTIVE
    for kind in &["ACHIEVE", "MAINTAIN", "AVOID", "SOFTGOAL"] {
        let key_draft = format!("TEST-GOAL-{kind}-DRAFT");
        let attrs = serde_json::json!({ "goal_kind": kind });
        let res = client
            .execute(
                "INSERT INTO graph_nodes (node_key, node_type, title, content, lifecycle_state, created_by, attributes)
                 VALUES ($1, 'GOAL', 'Valid Goal', 'Valid goal content', 'DRAFT', 'test_user', $2);",
                &[&key_draft, &attrs],
            )
            .await;
        assert!(
            res.is_ok(),
            "GOAL with kind {kind} in DRAFT must succeed: {res:?}"
        );

        let key_active = format!("TEST-GOAL-{kind}-ACTIVE");
        let res_active = client
            .execute(
                "INSERT INTO graph_nodes (node_key, node_type, title, content, lifecycle_state, created_by, attributes)
                 VALUES ($1, 'GOAL', 'Valid Goal', 'Valid goal content', 'ACTIVE', 'test_user', $2);",
                &[&key_active, &attrs],
            )
            .await;
        assert!(
            res_active.is_ok(),
            "GOAL with kind {kind} in ACTIVE must succeed: {res_active:?}"
        );
    }

    // 6d. SOURCE_EXCERPT is allowed in DRAFT and ACTIVE
    let insert_excerpt_draft = client
        .execute(
            "INSERT INTO graph_nodes (node_key, node_type, title, content, lifecycle_state, created_by)
             VALUES ('TEST-EXC-001', 'SOURCE_EXCERPT', 'Excerpt Draft', 'Transcript excerpt text', 'DRAFT', 'test_user');",
            &[],
        )
        .await;
    assert!(
        insert_excerpt_draft.is_ok(),
        "SOURCE_EXCERPT must be allowed in DRAFT state"
    );

    let insert_excerpt_active = client
        .execute(
            "INSERT INTO graph_nodes (node_key, node_type, title, content, lifecycle_state, created_by)
             VALUES ('TEST-EXC-002', 'SOURCE_EXCERPT', 'Excerpt Active', 'Transcript excerpt text', 'ACTIVE', 'test_user');",
            &[],
        )
        .await;
    assert!(
        insert_excerpt_active.is_ok(),
        "SOURCE_EXCERPT must be allowed in ACTIVE state"
    );

    // 7. Clean up test records
    let _ = client
        .execute("DELETE FROM graph_nodes WHERE node_key LIKE 'TEST-%';", &[])
        .await;
}
