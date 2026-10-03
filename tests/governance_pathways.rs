//! Integration test suite for Governance Policy Engine & Disambiguated Mutation Pathways (WP-2.2).
//!
//! Verifies:
//! - Pathway 1 (Autonomous Task Elaboration): permits verified agents to create non-normative
//!   execution tasks (`TASK`) directly in `ACTIVE` state with immediate commit to `audit_ledger`,
//!   inheriting the parent's `governance_policy` (`AUTONOMOUS_ELABORATION`) by default (D-57, D-63, D-81, TB-7.7).
//! - Pathway 2 (Active Leaf Task Updates): executes status transitions (`OPEN`, `IN_PROGRESS`, `BLOCKED`, `COMPLETED`)
//!   directly in-place under native row lock (`FOR UPDATE`) with immediate append of discrete reversible audit events (D-30, D-57).
//! - Pathway 3 (Normative Requirement Proposals): strictly prohibits in-place edits to active normative specifications
//!   (`REQUIREMENT`, `SPECIFICATION`), intercepting mutations and creating candidate entities in `lifecycle_state = 'DRAFT'`
//!   with `HUMAN_REVIEW_REQUIRED` / `PENDING_REVIEW` policy (INV-2, INV-3, D-57).
//! - Pathway 4 (Candidate Draft Evolution & Compaction): intermediate edits on `DRAFT` nodes append to
//!   `attributes->'draft_revisions'` JSONB array without polluting `audit_ledger`; atomic staging approval squashes
//!   draft history into `draft_evolution_summary` JSONB upon approval (D-16, D-61).
//! - Governance policy enforcement: `LOCKED` nodes unconditionally reject all mutations with `ERR_GOVERNANCE_LOCKED`.
//! - Caller-scoped draft isolation (INV-7, C-21).

use tokio_postgres::Client;
use uuid::Uuid;

use tks::db;
use tks::gateway::auth::AuthenticatedAgent;
use tks::storage::{
    MutationError, StorageRepo, TaskStatus, elaborate_task, elaborate_task_client,
    mutate_draft_entity, mutate_draft_entity_client, propose_normative_draft,
    propose_normative_draft_client, update_task_status, update_task_status_client,
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
            "DELETE FROM audit_ledger WHERE actor_id LIKE 'test_wp22_%';",
            &[],
        )
        .await
        .expect("Cleanup audit_ledger failed");
    client
        .execute(
            "DELETE FROM graph_nodes WHERE created_by LIKE 'test_wp22_%';",
            &[],
        )
        .await
        .expect("Cleanup graph_nodes failed");

    let repo = StorageRepo::from_database_url(&database_url).expect("Failed to create repo");
    (guard, repo, client)
}

#[tokio::test]
async fn test_pathway1_autonomous_task_elaboration_active_and_policy_inheritance() {
    let (_guard, repo, mut client) = setup_test_db().await;

    let agent = AuthenticatedAgent {
        agent_id: "test_wp22_agent_1".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Create active parent requirement with AUTONOMOUS_ELABORATION policy
    let parent_id = Uuid::new_v4();
    let parent_key = "REQ-WP22-ELAB-001";
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, $2, 'REQUIREMENT', 'Autonomous Parent Requirement', 'Permits sub-task elaboration', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $3);",
            &[&parent_id, &parent_key, &agent.agent_id],
        )
        .await
        .expect("Failed to insert parent requirement");

    // 2. Elaborate task using UUID parent identifier
    let res1 = elaborate_task_client(
        &mut client,
        &parent_id.to_string(),
        "Elaborated Task 1",
        Some("First level execution task"),
        Some(serde_json::json!({
            "estimated_hours": 4,
            "node_key": "TASK-WP22-SUB-001"
        })),
        &agent,
    )
    .await
    .expect("elaborate_task should succeed under AUTONOMOUS_ELABORATION parent");

    // Verify task is directly in ACTIVE state and inherited AUTONOMOUS_ELABORATION policy (D-57, D-63, D-81)
    assert_eq!(res1.lifecycle_state, "ACTIVE");
    assert_eq!(res1.status, "ACTIVE");
    assert_eq!(res1.governance_policy, "AUTONOMOUS_ELABORATION");
    assert_eq!(res1.node.node_type, "TASK");
    assert_eq!(res1.node.created_by, agent.agent_id);
    assert_eq!(res1.node_key.as_deref(), Some("TASK-WP22-SUB-001"));
    assert_eq!(
        res1.node
            .attributes
            .get("execution_status")
            .and_then(|v| v.as_str()),
        Some("OPEN")
    );

    // Verify upward FULFILLS edge created to parent requirement
    let edge_row = client
        .query_one(
            "SELECT from_node_id, to_node_id, edge_type, lifecycle_state, created_by \
             FROM graph_edges WHERE edge_id = $1;",
            &[&res1.edge_id],
        )
        .await
        .expect("Edge must exist");
    assert_eq!(edge_row.get::<_, Uuid>("from_node_id"), res1.task_id);
    assert_eq!(edge_row.get::<_, Uuid>("to_node_id"), parent_id);
    assert_eq!(edge_row.get::<_, String>("edge_type"), "FULFILLS");
    assert_eq!(edge_row.get::<_, String>("lifecycle_state"), "ACTIVE");
    assert_eq!(edge_row.get::<_, String>("created_by"), agent.agent_id);

    // Verify discrete audit event written to audit_ledger
    let audit_row = client
        .query_one(
            "SELECT event_seq, batch_id, event_type, entity_id, actor_id, delta \
             FROM audit_ledger WHERE event_seq = $1;",
            &[&res1.event_seq],
        )
        .await
        .expect("Audit record must exist");
    assert_eq!(audit_row.get::<_, i64>("event_seq"), res1.event_seq);
    assert_eq!(audit_row.get::<_, Uuid>("batch_id"), res1.batch_id);
    assert_eq!(audit_row.get::<_, String>("event_type"), "TASK_ELABORATED");
    assert_eq!(audit_row.get::<_, Uuid>("entity_id"), res1.task_id);
    assert_eq!(audit_row.get::<_, String>("actor_id"), agent.agent_id);

    // Verify vector embedding queued in node_embeddings (D-82)
    let emb_row = client
        .query_opt(
            "SELECT status FROM node_embeddings WHERE node_id = $1;",
            &[&res1.task_id],
        )
        .await
        .expect("Query embeddings queue failed");
    assert!(emb_row.is_some());
    assert_eq!(emb_row.unwrap().get::<_, String>("status"), "PENDING");

    // 3. Elaborate subtask under the newly created task (polymorphic resolution by node_key)
    let res2 = elaborate_task_client(
        &mut client,
        "TASK-WP22-SUB-001",
        "Nested Subtask 2",
        Some("Nested second-level task"),
        None,
        &agent,
    )
    .await
    .expect("Elaborating nested subtask under task should succeed");

    assert_eq!(res2.lifecycle_state, "ACTIVE");
    assert_eq!(res2.governance_policy, "AUTONOMOUS_ELABORATION");

    // Verify upward DERIVED_FROM edge to parent task
    let edge2_row = client
        .query_one(
            "SELECT edge_type, to_node_id FROM graph_edges WHERE edge_id = $1;",
            &[&res2.edge_id],
        )
        .await
        .expect("Nested edge must exist");
    assert_eq!(edge2_row.get::<_, String>("edge_type"), "DERIVED_FROM");
    assert_eq!(edge2_row.get::<_, Uuid>("to_node_id"), res1.task_id);

    // 4. Verify newly elaborated task context envelope can be assembled immediately (TB-6, D-63)
    let env = repo
        .assemble_context_envelope(res1.task_id, 2, None)
        .await
        .expect("Context envelope retrieval for active task must succeed immediately");
    assert_eq!(env.target_node.id, res1.task_id);
    assert!(
        env.ancestor_requirements.iter().any(|a| a.id == parent_id),
        "Envelope must contain parent requirement ancestor"
    );
}

#[tokio::test]
async fn test_pathway1_autonomous_elaboration_rejections_and_invariants() {
    let (_guard, _repo, mut client) = setup_test_db().await;

    let agent = AuthenticatedAgent {
        agent_id: "test_wp22_agent_rejection".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Create a LOCKED requirement
    let locked_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'REQ-WP22-LOCKED', 'REQUIREMENT', 'Locked Requirement', 'No mutations', 'ACTIVE', 'LOCKED', $2);",
            &[&locked_id, &agent.agent_id],
        )
        .await
        .expect("Failed to insert locked node");

    let locked_err = elaborate_task_client(
        &mut client,
        &locked_id.to_string(),
        "Illegal Task Under Locked",
        None,
        None,
        &agent,
    )
    .await
    .expect_err("Elaboration under LOCKED node must fail");

    assert_eq!(locked_err.code(), "ERR_GOVERNANCE_LOCKED");
    assert!(matches!(locked_err, MutationError::GovernanceLocked(_)));

    // 2. Create a HUMAN_REVIEW_REQUIRED requirement
    let review_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'REQ-WP22-REVIEW', 'REQUIREMENT', 'Human Review Requirement', 'Needs review', 'ACTIVE', 'HUMAN_REVIEW_REQUIRED', $2);",
            &[&review_id, &agent.agent_id],
        )
        .await
        .expect("Failed to insert review node");

    let review_err = elaborate_task_client(
        &mut client,
        &review_id.to_string(),
        "Illegal Task Under Human Review",
        None,
        None,
        &agent,
    )
    .await
    .expect_err("Elaboration under HUMAN_REVIEW_REQUIRED node must fail");

    assert_eq!(review_err.code(), "ERR_GOVERNANCE_REJECTED");
    assert!(matches!(review_err, MutationError::GovernanceRejected(_)));

    // 3. Unknown parent node
    let not_found_err = elaborate_task_client(
        &mut client,
        "NON-EXISTENT-PARENT",
        "Task Under Ghost",
        None,
        None,
        &agent,
    )
    .await
    .expect_err("Elaboration under non-existent parent must fail");

    assert_eq!(not_found_err.code(), "ERR_NOT_FOUND");
    assert!(matches!(not_found_err, MutationError::NotFound(_)));
}

#[tokio::test]
async fn test_pathway2_active_leaf_task_status_updates() {
    let (_guard, _repo, mut client) = setup_test_db().await;

    let agent = AuthenticatedAgent {
        agent_id: "test_wp22_agent_task_updater".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Create active task
    let task_id = Uuid::new_v4();
    let task_key = "TASK-WP22-STATUS-001";
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, $2, 'TASK', 'Task for Status Update', 'Working through execution statuses', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $3, '{\"execution_status\": \"OPEN\"}');",
            &[&task_id, &task_key, &agent.agent_id],
        )
        .await
        .expect("Failed to insert task");

    // 2. Transition from OPEN to IN_PROGRESS
    let update1 = update_task_status_client(
        &mut client,
        &task_id.to_string(),
        TaskStatus::InProgress,
        Some("Started implementation work"),
        &agent,
    )
    .await
    .expect("update_task_status to IN_PROGRESS should succeed");

    assert_eq!(update1.status, "UPDATED");
    assert_eq!(update1.execution_status, "IN_PROGRESS");
    assert_eq!(
        update1
            .node
            .attributes
            .get("execution_status")
            .and_then(|v| v.as_str()),
        Some("IN_PROGRESS")
    );
    assert_eq!(
        update1
            .node
            .attributes
            .get("status_notes")
            .and_then(|v| v.as_str()),
        Some("Started implementation work")
    );

    // Verify NO draft_revisions array was created (D-57)
    assert!(
        update1.node.attributes.get("draft_revisions").is_none(),
        "Task status update must not generate draft_revisions"
    );

    // Verify discrete audit event written to audit_ledger
    let audit1 = client
        .query_one(
            "SELECT event_type, delta, entity_id FROM audit_ledger WHERE event_seq = $1;",
            &[&update1.event_seq],
        )
        .await
        .expect("Audit record 1 must exist");
    assert_eq!(audit1.get::<_, String>("event_type"), "TASK_STATUS_UPDATE");
    assert_eq!(audit1.get::<_, Uuid>("entity_id"), task_id);
    let delta1: serde_json::Value = audit1.get("delta");
    assert_eq!(
        delta1.get("previous_status").and_then(|v| v.as_str()),
        Some("OPEN")
    );
    assert_eq!(
        delta1.get("execution_status").and_then(|v| v.as_str()),
        Some("IN_PROGRESS")
    );

    // 3. Transition from IN_PROGRESS to COMPLETED using canonical node_key
    let update2 = update_task_status_client(
        &mut client,
        task_key,
        TaskStatus::Completed,
        Some("Implementation complete and all tests pass"),
        &agent,
    )
    .await
    .expect("update_task_status to COMPLETED should succeed");

    assert_eq!(update2.execution_status, "COMPLETED");
    assert_ne!(update2.event_seq, update1.event_seq);
    assert_ne!(update2.batch_id, update1.batch_id);

    // 4. Verify invalid update attempts
    // (a) Attempt to update status of a REQUIREMENT node
    let req_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'REQ-WP22-NOT-TASK', 'REQUIREMENT', 'Non Task', 'Cannot have task status', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2);",
            &[&req_id, &agent.agent_id],
        )
        .await
        .expect("Failed to insert requirement");

    let req_err = update_task_status_client(
        &mut client,
        &req_id.to_string(),
        TaskStatus::InProgress,
        None,
        &agent,
    )
    .await
    .expect_err("update_task_status on REQUIREMENT must fail");

    assert_eq!(req_err.code(), "ERR_GOVERNANCE_REJECTED");

    // (b) Attempt to update status of a LOCKED task
    let locked_task_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'TASK-WP22-LOCKED', 'TASK', 'Locked Task', 'Locked from updates', 'ACTIVE', 'LOCKED', $2);",
            &[&locked_task_id, &agent.agent_id],
        )
        .await
        .expect("Failed to insert locked task");

    let locked_err = update_task_status_client(
        &mut client,
        &locked_task_id.to_string(),
        TaskStatus::Completed,
        None,
        &agent,
    )
    .await
    .expect_err("update_task_status on LOCKED task must fail");

    assert_eq!(locked_err.code(), "ERR_GOVERNANCE_LOCKED");
}

#[tokio::test]
async fn test_pathway3_normative_requirement_proposals_safety_and_draft_staging() {
    let (_guard, _repo, mut client) = setup_test_db().await;

    let agent = AuthenticatedAgent {
        agent_id: "test_wp22_agent_normative".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Create active normative requirement
    let req_id = Uuid::new_v4();
    let req_key = "REQ-WP22-NORMATIVE-001";
    let original_content = "Active normative text that must never be mutated in-place.";
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, $2, 'REQUIREMENT', 'Original Requirement Title', $3, 'ACTIVE', 'HUMAN_REVIEW_REQUIRED', $4);",
            &[&req_id, &req_key, &original_content, &agent.agent_id],
        )
        .await
        .expect("Failed to insert requirement");

    // 2. Propose normative draft replacing the requirement
    let proposal = propose_normative_draft_client(
        &mut client,
        req_key,
        Some("Refined Requirement Title"),
        "Updated proposed normative content with clarified specifications.",
        Some(serde_json::json!({ "priority": "HIGH" })),
        &agent,
    )
    .await
    .expect("propose_normative_draft should succeed");

    assert_eq!(proposal.status, "PENDING_REVIEW");
    assert_eq!(proposal.lifecycle_state, "DRAFT");
    assert_eq!(proposal.governance_policy, "HUMAN_REVIEW_REQUIRED");
    assert_eq!(proposal.target_id, req_id);
    assert_ne!(proposal.draft_id, req_id);
    assert_eq!(proposal.node.created_by, agent.agent_id);
    assert_eq!(
        proposal
            .node
            .attributes
            .get("replaces_node_id")
            .and_then(|v| v.as_str()),
        Some(req_id.to_string()).as_deref()
    );

    // 3. INVARIANT SAFETY CHECK (INV-2, INV-3): Original active requirement MUST remain 100% UNTOUCHED
    let orig_row = client
        .query_one(
            "SELECT title, content, lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&req_id],
        )
        .await
        .expect("Original row must exist");
    assert_eq!(
        orig_row.get::<_, Option<String>>("title"),
        Some("Original Requirement Title".to_string())
    );
    assert_eq!(
        orig_row.get::<_, Option<String>>("content"),
        Some(original_content.to_string())
    );
    assert_eq!(orig_row.get::<_, String>("lifecycle_state"), "ACTIVE");

    // 4. Verify candidate draft edge connects to target with DERIVED_FROM
    let draft_edge = client
        .query_opt(
            "SELECT edge_type, lifecycle_state FROM graph_edges WHERE from_node_id = $1 AND to_node_id = $2;",
            &[&proposal.draft_id, &req_id],
        )
        .await
        .expect("Query draft edge failed");
    assert!(draft_edge.is_some());
    let edge_r = draft_edge.unwrap();
    assert_eq!(edge_r.get::<_, String>("edge_type"), "DERIVED_FROM");
    assert_eq!(edge_r.get::<_, String>("lifecycle_state"), "DRAFT");

    // 5. Verify LOCKED requirement rejects normative draft proposal
    let locked_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'REQ-WP22-LOCK-PROP', 'REQUIREMENT', 'Locked Root', 'Locked text', 'ACTIVE', 'LOCKED', $2);",
            &[&locked_id, &agent.agent_id],
        )
        .await
        .expect("Failed to insert locked node");

    let lock_err = propose_normative_draft_client(
        &mut client,
        &locked_id.to_string(),
        Some("Illegal Draft"),
        "Illegal content",
        None,
        &agent,
    )
    .await
    .expect_err("Normative proposal on LOCKED node must fail");

    assert_eq!(lock_err.code(), "ERR_GOVERNANCE_LOCKED");
}

#[tokio::test]
async fn test_pathway4_candidate_draft_evolution_and_caller_isolation() {
    let (_guard, _repo, mut client) = setup_test_db().await;

    let agent_a = AuthenticatedAgent {
        agent_id: "test_wp22_agent_author_A".to_string(),
        actor_type: "AGENT".to_string(),
    };
    let agent_b = AuthenticatedAgent {
        agent_id: "test_wp22_agent_intruder_B".to_string(),
        actor_type: "AGENT".to_string(),
    };
    let human_admin = AuthenticatedAgent {
        agent_id: "test_wp22_human_admin".to_string(),
        actor_type: "HUMAN".to_string(),
    };

    // 1. Create candidate DRAFT node authored by agent A
    let draft_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes) \
             VALUES ($1, NULL, 'SPECIFICATION', 'Draft Spec V1', 'Initial draft spec text', 'DRAFT', 'HUMAN_REVIEW_REQUIRED', $2, '{\"draft_revisions\": []}');",
            &[&draft_id, &agent_a.agent_id],
        )
        .await
        .expect("Failed to insert draft node");

    // 2. Agent A evolves draft: revision 1
    let edit1 = mutate_draft_entity_client(
        &mut client,
        &draft_id.to_string(),
        serde_json::json!({
            "title": "Draft Spec V1.1",
            "content": "Updated content with technical parameters",
            "attributes": { "timeout_ms": 5000 }
        }),
        &agent_a,
    )
    .await
    .expect("Agent A mutate draft should succeed");

    assert_eq!(edit1.status, "UPDATED");
    assert_eq!(edit1.revisions_count, 1);
    assert_eq!(edit1.node.title.as_deref(), Some("Draft Spec V1.1"));
    assert_eq!(
        edit1.node.content.as_deref(),
        Some("Updated content with technical parameters")
    );

    // 3. Agent A evolves draft: revision 2
    let edit2 = mutate_draft_entity_client(
        &mut client,
        &draft_id.to_string(),
        serde_json::json!({
            "content": "Finalized candidate content for review",
            "attributes": { "ready_for_review": true }
        }),
        &agent_a,
    )
    .await
    .expect("Agent A second mutate draft should succeed");

    assert_eq!(edit2.revisions_count, 2);
    let revs = edit2
        .node
        .attributes
        .get("draft_revisions")
        .and_then(|v| v.as_array())
        .expect("draft_revisions array must exist");
    assert_eq!(revs.len(), 2);
    assert_eq!(
        revs[0].get("revision_seq").and_then(|v| v.as_i64()),
        Some(1)
    );
    assert_eq!(
        revs[1].get("revision_seq").and_then(|v| v.as_i64()),
        Some(2)
    );
    assert_eq!(
        revs[1].get("actor_id").and_then(|v| v.as_str()),
        Some(agent_a.agent_id.as_str())
    );

    // 4. Verify ZERO rows were emitted to audit_ledger during intermediate draft edits (D-16, D-61)
    let audit_count: i64 = client
        .query_one(
            "SELECT count(*) FROM audit_ledger WHERE entity_id = $1;",
            &[&draft_id],
        )
        .await
        .expect("Query audit count failed")
        .get(0);
    assert_eq!(
        audit_count, 0,
        "Intermediate draft revisions must not emit audit ledger rows"
    );

    // 5. Caller-scoped draft isolation: Agent B attempts to mutate Agent A's draft
    let isolation_err = mutate_draft_entity_client(
        &mut client,
        &draft_id.to_string(),
        serde_json::json!({ "title": "Unauthorized Hijack Title" }),
        &agent_b,
    )
    .await
    .expect_err("Agent B mutating Agent A draft must fail with isolation error");

    assert_eq!(isolation_err.code(), "ERR_GOVERNANCE_REJECTED");
    assert!(
        isolation_err
            .to_string()
            .contains("Caller-scoped draft isolation")
    );

    // 6. Human supervisor CAN mutate candidate draft
    let human_edit = mutate_draft_entity_client(
        &mut client,
        &draft_id.to_string(),
        serde_json::json!({ "title": "Supervisor Approved Title" }),
        &human_admin,
    )
    .await
    .expect("Human supervisor can edit draft");

    assert_eq!(human_edit.revisions_count, 3);
    assert_eq!(
        human_edit.node.title.as_deref(),
        Some("Supervisor Approved Title")
    );

    // 7. Attempting mutate_draft_entity on an ACTIVE node fails
    let active_id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'SPEC-WP22-ACTIVE-CAND', 'SPECIFICATION', 'Active Spec', 'Active content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $2);",
            &[&active_id, &agent_a.agent_id],
        )
        .await
        .expect("Failed to insert active spec");

    let active_err = mutate_draft_entity_client(
        &mut client,
        &active_id.to_string(),
        serde_json::json!({ "title": "Mutate active illegal" }),
        &agent_a,
    )
    .await
    .expect_err("mutate_draft_entity on ACTIVE node must fail");

    assert_eq!(active_err.code(), "ERR_GOVERNANCE_REJECTED");
}

#[tokio::test]
async fn test_deadpool_client_and_repo_pathway_methods() {
    let (_guard, repo, client) = setup_test_db().await;

    let agent = AuthenticatedAgent {
        agent_id: "test_wp22_agent_deadpool".to_string(),
        actor_type: "AGENT".to_string(),
    };

    // 1. Create root requirement
    let root_id = Uuid::new_v4();
    let root_key = "REQ-WP22-DP-ROOT";
    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, $2, 'REQUIREMENT', 'Deadpool Root Req', 'Root content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', $3);",
            &[&root_id, &root_key, &agent.agent_id],
        )
        .await
        .expect("Failed to insert root req");

    // 2. Test elaborate_task with deadpool client
    let mut dp_client = repo
        .get_client()
        .await
        .expect("Failed to get deadpool client");
    let task_res = elaborate_task(
        &mut dp_client,
        root_key,
        "Task via deadpool client",
        Some("Elaborated using &mut deadpool_postgres::Client"),
        None,
        &agent,
    )
    .await
    .expect("elaborate_task via deadpool client should succeed");

    assert_eq!(task_res.status, "ACTIVE");
    assert_eq!(task_res.lifecycle_state, "ACTIVE");

    // 3. Test update_task_status with deadpool client
    let status_res = update_task_status(
        &mut dp_client,
        &task_res.task_id.to_string(),
        TaskStatus::InProgress,
        Some("Updated via deadpool client"),
        &agent,
    )
    .await
    .expect("update_task_status via deadpool client should succeed");

    assert_eq!(status_res.execution_status, "IN_PROGRESS");

    // 4. Test propose_normative_draft with deadpool client
    let draft_res = propose_normative_draft(
        &mut dp_client,
        root_key,
        Some("New Proposed Root Title"),
        "Updated proposed root content",
        None,
        &agent,
    )
    .await
    .expect("propose_normative_draft via deadpool client should succeed");

    assert_eq!(draft_res.status, "PENDING_REVIEW");

    // 5. Test mutate_draft_entity with deadpool client
    let mutate_res = mutate_draft_entity(
        &mut dp_client,
        &draft_res.draft_id.to_string(),
        serde_json::json!({
            "title": "New Proposed Root Title Rev 2",
            "content": "Updated content v2"
        }),
        &agent,
    )
    .await
    .expect("mutate_draft_entity via deadpool client should succeed");

    assert_eq!(mutate_res.status, "UPDATED");
    assert_eq!(mutate_res.revisions_count, 1);

    // 6. Test repo methods directly
    let repo_task_res = repo
        .elaborate_task(
            root_key,
            "Task via Repo Method",
            Some("Elaborated directly via repo.elaborate_task()"),
            None,
            &agent,
        )
        .await
        .expect("repo.elaborate_task should succeed");

    assert_eq!(repo_task_res.status, "ACTIVE");

    let repo_status_res = repo
        .update_task_status(
            &repo_task_res.task_id.to_string(),
            TaskStatus::Completed,
            Some("Completed via repo method"),
            &agent,
        )
        .await
        .expect("repo.update_task_status should succeed");

    assert_eq!(repo_status_res.execution_status, "COMPLETED");
}
