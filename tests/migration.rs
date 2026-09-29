//! Integration test verifying database migration execution and schema integrity.

use tks::db;

#[tokio::test]
async fn test_migration_and_schema_verification() {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost:5432/postgres".to_string());

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

    // 5. Clean up test records
    let _ = client
        .execute(
            "DELETE FROM graph_nodes WHERE node_key IN ('TEST-DRAFT-001', 'TEST-ACTIVE-001');",
            &[],
        )
        .await;
}
