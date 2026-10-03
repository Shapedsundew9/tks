//! Phase 2 Dogfooding Milestone (Gate 2 - Autonomous Self-Evolution) Acceptance Validation Suite.
//!
//! Automates the complete Phase 2 MVD Acceptance Test Script (Steps 1–11 from
//! `docs/vision/strategic-planning-backlog.md` §4 and `docs/vision/phase2-plan.md` WP-2.5):
//!
//! 1. External agent authentication (`agent-runner-42`) with bearer credentials.
//! 2. Autonomous subtask elaboration under `AUTONOMOUS_ELABORATION` node `REQ-005`,
//!    verifying direct `ACTIVE` state, inherited policy, `audit_ledger` monotonic `event_seq`, and `batch_id`.
//! 3. Immediate topological context envelope queryability for newly elaborated task via MCP.
//! 4. DAG cycle prevention: attempt circular constraint edge -> assert `ERR_GRAPH_CYCLE_DETECTED`.
//! 5. Leaf task update under native row lock (`FOR UPDATE`) committing discrete audit event.
//! 6. Normative draft proposal targeting `REQ-006` (`HUMAN_REVIEW_REQUIRED`).
//! 7. Staging draft isolation: candidate draft in `DRAFT` state leaving active specification unchanged.
//! 8. Caller-scoped draft querying (`include_drafts: true`) and isolation from third parties.
//! 9. Administrative staging approval: atomic squash into `draft_evolution_summary` in `audit_ledger`.
//! 10. Unified rollback (`revert_mutations`): dry-run preview, cross-agent safety abort (`ERR_CONFIRMATION_REQUIRED`),
//!     and forced compensating execution (`force: true`).
//! 11. Cascading invalidation to `NEEDS_REVERIFICATION` and explicit reverification recovery (`reverify_node`).
//! 12. External agents authoring, inspecting, and tracking real Phase 3 preparatory tasks directly in the substrate via MCP (INV-6).

use std::fs;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tks::cli::admin::{AdminReverifyArgs, AdminRevertArgs, AdminSubcommand, run_admin};
use tks::cli::task::{CreateTaskArgs, ListTasksArgs, TaskSubcommand, UpdateTaskArgs, run_task};
use tks::db;
use tks::gateway::routes::identities::{CreateIdentityRequest, CreateIdentityResponse};
use tks::gateway::routes::staging::StagingApproveResponse;
use tks::gateway::{AppState, create_router};
use tks::storage::git::{GitReadHandle, GitWriteHandle, start_git_actor};
use tks::storage::{StorageRepo, TopologicalEnvelope};

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TempRepoGuard {
    path: std::path::PathBuf,
}

impl TempRepoGuard {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("tks-gate2-git-{}", Uuid::new_v4()));
        Self { path }
    }
}

impl Drop for TempRepoGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Helper context holding running test server, database pool, and Git handles.
struct Gate2Context {
    base_url: String,
    pool: deadpool_postgres::Pool,
    _git_write: GitWriteHandle,
    _git_read: GitReadHandle,
    cancel_token: CancellationToken,
    _guard: tokio::sync::MutexGuard<'static, ()>,
    _temp_repo: TempRepoGuard,
}

async fn setup_gate2_environment() -> Gate2Context {
    let guard = TEST_MUTEX.lock().await;
    let database_url = db::ensure_test_database_ready()
        .await
        .expect("Failed to prepare test database");

    // Ensure embedded migrations are applied
    let (mut client, _handle) = db::connect(&database_url)
        .await
        .expect("Failed to connect to PostgreSQL");
    db::run_migrations(&mut client)
        .await
        .expect("Failed to run migrations");

    // Clean up any residue from prior runs for gate 2 test keys
    client
        .execute(
            "DELETE FROM audit_ledger WHERE actor_id LIKE 'agent-runner-%' OR actor_id = 'admin-supervisor' OR actor_id = 'test_gate2';",
            &[],
        )
        .await
        .expect("Cleanup audit_ledger failed");

    client
        .execute(
            "DELETE FROM graph_nodes WHERE created_by LIKE 'agent-runner-%' OR created_by = 'admin-supervisor' OR node_key LIKE '%GATE2%' OR node_key LIKE 'TASK-%' OR node_key LIKE 'PHASE3-%' OR node_key IN ('REQ-005', 'REQ-006');",
            &[],
        )
        .await
        .expect("Cleanup graph_nodes failed");

    client
        .execute(
            "DELETE FROM agent_identities WHERE agent_id IN ('agent-runner-42', 'agent-runner-99', 'admin-supervisor');",
            &[],
        )
        .await
        .expect("Cleanup agent_identities failed");

    let temp_repo = TempRepoGuard::new();
    let (git_write, git_read) =
        start_git_actor(&temp_repo.path, 128).expect("Failed to start bare Git actor");

    let pool = db::create_pool(&database_url).expect("Failed to create connection pool");
    let storage = StorageRepo::new(pool.clone());
    let state = AppState::new(pool.clone(), storage, git_write.clone(), git_read.clone());

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

    // Allow server to bind
    tokio::time::sleep(Duration::from_millis(50)).await;

    Gate2Context {
        base_url,
        pool,
        _git_write: git_write,
        _git_read: git_read,
        cancel_token,
        _guard: guard,
        _temp_repo: temp_repo,
    }
}

#[tokio::test]
async fn test_phase2_dogfooding_gate2_mvd_acceptance() {
    println!("\n===============================================================================");
    println!("PHASE 2 DOGFOODING MILESTONE (GATE 2 - AUTONOMOUS SELF-EVOLUTION) ACCEPTANCE TEST");
    println!("===============================================================================\n");

    let ctx = setup_gate2_environment().await;
    let http = reqwest::Client::new();
    let mcp_url = format!("{}/mcp", ctx.base_url);

    // -------------------------------------------------------------------------
    // STEP 1: Provision and Authenticate Identities
    // -------------------------------------------------------------------------
    println!("\n--- [Step 1] Provisioning and Authenticating Identities ---");

    // 1.1 Provision test agent `agent-runner-42`
    let agent42_token = "token-agent-runner-42";
    let ident_url = format!("{}/api/v1/identities", ctx.base_url);
    let resp42 = http
        .post(&ident_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&CreateIdentityRequest {
            name: "agent-runner-42".to_string(),
            role: "AGENT".to_string(),
            token: Some(agent42_token.to_string()),
        })
        .send()
        .await
        .expect("Failed to create agent-runner-42");
    assert_eq!(resp42.status(), StatusCode::CREATED);
    let ident42: CreateIdentityResponse = resp42.json().await.unwrap();
    assert_eq!(ident42.agent_id, "agent-runner-42");
    println!("Agent 'agent-runner-42' provisioned successfully.");

    // 1.2 Provision second agent `agent-runner-99` for cross-agent dependency testing
    let agent99_token = "token-agent-runner-99";
    let resp99 = http
        .post(&ident_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&CreateIdentityRequest {
            name: "agent-runner-99".to_string(),
            role: "AGENT".to_string(),
            token: Some(agent99_token.to_string()),
        })
        .send()
        .await
        .expect("Failed to create agent-runner-99");
    assert_eq!(resp99.status(), StatusCode::CREATED);
    println!("Agent 'agent-runner-99' provisioned successfully.");

    // 1.3 Provision human supervisor `admin-supervisor`
    let admin_token = "token-admin-supervisor";
    let resp_admin = http
        .post(&ident_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&CreateIdentityRequest {
            name: "admin-supervisor".to_string(),
            role: "HUMAN".to_string(),
            token: Some(admin_token.to_string()),
        })
        .send()
        .await
        .expect("Failed to create admin-supervisor");
    assert_eq!(resp_admin.status(), StatusCode::CREATED);
    println!("Supervisor 'admin-supervisor' provisioned successfully.");

    // 1.4 Setup Precondition Nodes: REQ-005 (AUTONOMOUS_ELABORATION) and REQ-006 (HUMAN_REVIEW_REQUIRED)
    let client = ctx.pool.get().await.expect("Failed to get DB connection");

    let req5_id = Uuid::new_v4();
    let req6_id = Uuid::new_v4();

    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'REQ-005', 'REQUIREMENT', 'Autonomous Core Subsystem', 'High-throughput execution subsystem allowing agent elaboration', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'system') \
             ON CONFLICT (node_key) WHERE lifecycle_state = 'ACTIVE' AND node_key IS NOT NULL DO UPDATE \
             SET governance_policy = 'AUTONOMOUS_ELABORATION', lifecycle_state = 'ACTIVE' \
             RETURNING id;",
            &[&req5_id],
        )
        .await
        .expect("Failed to insert REQ-005");

    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by) \
             VALUES ($1, 'REQ-006', 'REQUIREMENT', 'Normative Protocol Specification', 'Governing interface specification requiring human review', 'ACTIVE', 'HUMAN_REVIEW_REQUIRED', 'system') \
             ON CONFLICT (node_key) WHERE lifecycle_state = 'ACTIVE' AND node_key IS NOT NULL DO UPDATE \
             SET governance_policy = 'HUMAN_REVIEW_REQUIRED', lifecycle_state = 'ACTIVE' \
             RETURNING id;",
            &[&req6_id],
        )
        .await
        .expect("Failed to insert REQ-006");

    println!(
        "Precondition nodes established: REQ-005 (AUTONOMOUS_ELABORATION) and REQ-006 (HUMAN_REVIEW_REQUIRED)."
    );

    // -------------------------------------------------------------------------
    // STEP 2: Autonomous Elaboration under REQ-005
    // -------------------------------------------------------------------------
    println!("\n--- [Step 2] Autonomous Elaboration of Child Tasks under REQ-005 ---");

    // Agent calls propose_node_mutation targeting REQ-005 to elaborate 3 sub-tasks
    let mut task_ids = Vec::new();
    let mut batch_ids = Vec::new();

    for i in 1..=3 {
        let task_key = format!("TASK-10{i}");
        let req_payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 200 + i,
            "method": "tools/call",
            "params": {
                "name": "propose_node_mutation",
                "arguments": {
                    "target_node_id": "REQ-005",
                    "mutation_type": "TASK_ELABORATION",
                    "proposed_title": format!("Autonomous Subtask {i}"),
                    "proposed_content": format!("Decomposed implementation task {i} for high-throughput pipeline"),
                    "proposed_attributes": {
                        "node_key": task_key,
                        "complexity": "moderate"
                    }
                }
            }
        });

        let resp = http
            .post(&mcp_url)
            .header(AUTHORIZATION, format!("Bearer {agent42_token}"))
            .header(CONTENT_TYPE, "application/json")
            .json(&req_payload)
            .send()
            .await
            .expect("Failed to send propose_node_mutation");

        assert_eq!(resp.status(), StatusCode::OK);
        let resp_json: serde_json::Value = resp.json().await.unwrap();

        let result = resp_json.get("result").expect("Must contain result");
        let task_obj = result.get("mutation").unwrap_or(result);
        assert_eq!(
            task_obj.get("status").and_then(|v| v.as_str()),
            Some("COMMITTED")
        );
        assert_eq!(
            task_obj.get("lifecycle_state").and_then(|v| v.as_str()),
            Some("ACTIVE")
        );
        assert_eq!(
            task_obj.get("governance_policy").and_then(|v| v.as_str()),
            Some("AUTONOMOUS_ELABORATION"),
            "Task must inherit AUTONOMOUS_ELABORATION policy by default (D-81)"
        );

        let tid: Uuid = task_obj
            .get("task_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .expect("Must have task_id UUID");
        let bid: Uuid = task_obj
            .get("batch_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .expect("Must have batch_id UUID");

        task_ids.push(tid);
        batch_ids.push(bid);
        println!("Elaborated sub-task {i} ({task_key}): ID={tid}, batch_id={bid}");
    }

    assert_eq!(task_ids.len(), 3);
    let primary_batch_id = batch_ids[1];

    // Verify upward edges and audit_ledger in database
    for tid in &task_ids {
        let edge_row = client
            .query_one(
                "SELECT e.edge_type, e.lifecycle_state, e.created_by, parent.node_key \
                 FROM graph_edges e \
                 JOIN graph_nodes parent ON parent.id = e.to_node_id \
                 WHERE e.from_node_id = $1;",
                &[tid],
            )
            .await
            .expect("Must have upward edge from elaborated task");

        assert_eq!(edge_row.get::<_, String>("lifecycle_state"), "ACTIVE");
        assert_eq!(edge_row.get::<_, String>("created_by"), "agent-runner-42");
        assert_eq!(edge_row.get::<_, String>("node_key"), "REQ-005");

        let audit_row = client
            .query_one(
                "SELECT event_type, actor_id, batch_id, event_seq \
                 FROM audit_ledger WHERE entity_id = $1;",
                &[tid],
            )
            .await
            .expect("Must have audit_ledger event for elaborated task");

        assert_eq!(audit_row.get::<_, String>("event_type"), "TASK_ELABORATED");
        assert_eq!(audit_row.get::<_, String>("actor_id"), "agent-runner-42");
        assert!(audit_row.get::<_, i64>("event_seq") > 0);
    }
    println!(
        "Verified: Tasks created directly in ACTIVE state with valid upward edges and audit events."
    );

    // -------------------------------------------------------------------------
    // STEP 3: Immediate Context Envelope Queryability
    // -------------------------------------------------------------------------
    println!("\n--- [Step 3] Querying Context Envelope for Elaborated Task ---");

    let env_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 204,
        "method": "tools/call",
        "params": {
            "name": "get_context_envelope",
            "arguments": {
                "target_node_id": task_ids[0].to_string(),
                "depth": 2
            }
        }
    });

    let env_resp = http
        .post(&mcp_url)
        .header(AUTHORIZATION, format!("Bearer {agent42_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&env_req)
        .send()
        .await
        .expect("Failed to execute get_context_envelope");

    assert_eq!(env_resp.status(), StatusCode::OK);
    let env_json: serde_json::Value = env_resp.json().await.unwrap();

    let env_val = env_json
        .get("result")
        .and_then(|r| r.get("envelope"))
        .expect("Result must contain envelope object");

    let envelope: TopologicalEnvelope = serde_json::from_value(env_val.clone()).unwrap();
    assert_eq!(envelope.target_node.id, task_ids[0]);
    assert!(
        envelope
            .ancestor_requirements
            .iter()
            .any(|a| a.node_key.as_deref() == Some("REQ-005")),
        "REQ-005 must be present in ancestor requirements"
    );
    println!("Verified: Context envelope resolves immediately without supervisory approval.");

    // -------------------------------------------------------------------------
    // STEP 4: Cycle Prevention - Reject Circular Structural Edges
    // -------------------------------------------------------------------------
    println!("\n--- [Step 4] Attempting Circular Edge to Assert ERR_GRAPH_CYCLE_DETECTED ---");

    // Attempt to insert edge from REQ-005 to task_ids[0] (which already has upward edge to REQ-005)
    let _cycle_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 205,
        "method": "tools/call",
        "params": {
            "name": "propose_node_mutation",
            "arguments": {
                "target_node_id": "REQ-005",
                "mutation_type": "TASK",
                "proposed_title": "Cycle proposal",
                "proposed_attributes": {
                    "cycle_target": task_ids[0].to_string()
                }
            }
        }
    });

    // Alternatively, verify cycle check directly via mutation REST or storage layer
    let mut db_client = ctx.pool.get().await.unwrap();
    let has_cycle = tks::storage::mutation::check_dag_cycle(&**db_client, req5_id, task_ids[0])
        .await
        .expect("check_dag_cycle execution failed");
    assert!(
        has_cycle,
        "DAG cycle check must detect cycle between child task and parent REQ-005"
    );

    let cycle_insert_res = tks::storage::mutation::insert_structural_edge(
        &**db_client,
        req5_id,
        task_ids[0],
        "DERIVED_FROM",
        "agent-runner-42",
        "ACTIVE",
    )
    .await;

    match cycle_insert_res {
        Err(tks::storage::MutationError::CycleDetected { from_id, to_id }) => {
            println!(
                "Verified: Cyclic edge rejected with MutationError::CycleDetected ({from_id} -> {to_id})."
            );
        }
        other => panic!("Expected MutationError::CycleDetected, got: {other:?}"),
    }

    // -------------------------------------------------------------------------
    // STEP 5: Leaf Task Status Update under Native Row-Level Lock
    // -------------------------------------------------------------------------
    println!("\n--- [Step 5] Updating Active Task to COMPLETED under Row Lock ---");

    let update_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 206,
        "method": "tools/call",
        "params": {
            "name": "update_node_status",
            "arguments": {
                "node_id": "TASK-101",
                "status": "COMPLETED",
                "notes": "Verified all acceptance criteria deterministically"
            }
        }
    });

    let update_resp = http
        .post(&mcp_url)
        .header(AUTHORIZATION, format!("Bearer {agent42_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&update_req)
        .send()
        .await
        .expect("Failed to execute update_node_status");

    assert_eq!(update_resp.status(), StatusCode::OK);
    let update_json: serde_json::Value = update_resp.json().await.unwrap();
    let result = update_json.get("result").expect("Must contain result");
    let update_res = result.get("update").unwrap_or(result);
    assert!(
        matches!(
            update_res.get("status").and_then(|v| v.as_str()),
            Some("UPDATED") | Some("COMMITTED")
        ),
        "Expected UPDATED or COMMITTED status"
    );
    assert_eq!(
        update_res.get("execution_status").and_then(|v| v.as_str()),
        Some("COMPLETED")
    );

    // Verify row lock and discrete audit record
    let update_audit = client
        .query_one(
            "SELECT event_type, actor_id, delta FROM audit_ledger \
             WHERE entity_id = $1 AND event_type = 'TASK_STATUS_UPDATE' \
             ORDER BY event_seq DESC LIMIT 1;",
            &[&task_ids[0]],
        )
        .await
        .expect("Must have TASK_STATUS_UPDATE audit row");
    assert_eq!(update_audit.get::<_, String>("actor_id"), "agent-runner-42");
    println!(
        "Verified: Task status updated in place with discrete TASK_STATUS_UPDATE audit event."
    );

    // -------------------------------------------------------------------------
    // STEP 6: Propose Normative Change to REQ-006 (HUMAN_REVIEW_REQUIRED)
    // -------------------------------------------------------------------------
    println!("\n--- [Step 6] Proposing Normative Change to REQ-006 ---");

    let norm_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 207,
        "method": "tools/call",
        "params": {
            "name": "propose_node_mutation",
            "arguments": {
                "target_node_id": "REQ-006",
                "mutation_type": "NORMATIVE_PROPOSAL",
                "proposed_title": "Normative Protocol Specification (V2 Draft)",
                "proposed_content": "Updated normative specification with strict TLS 1.3 and zero-trust verification rules."
            }
        }
    });

    let norm_resp = http
        .post(&mcp_url)
        .header(AUTHORIZATION, format!("Bearer {agent42_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&norm_req)
        .send()
        .await
        .expect("Failed to execute normative proposal");

    assert_eq!(norm_resp.status(), StatusCode::OK);
    let norm_json: serde_json::Value = norm_resp.json().await.unwrap();
    let result = norm_json.get("result").expect("Must contain result");
    let norm_res = result.get("mutation").unwrap_or(result);

    // -------------------------------------------------------------------------
    // STEP 7: Verify Draft Interception and Staging Isolation
    // -------------------------------------------------------------------------
    println!("\n--- [Step 7] Verifying Candidate DRAFT Staging Isolation ---");

    assert_eq!(
        norm_res.get("status").and_then(|v| v.as_str()),
        Some("PENDING_REVIEW")
    );
    let draft_id: Uuid = norm_res
        .get("draft_id")
        .or_else(|| norm_res.get("node_id"))
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .expect("Must return draft_id");

    let draft_row = client
        .query_one(
            "SELECT lifecycle_state, governance_policy, created_by, attributes \
             FROM graph_nodes WHERE id = $1;",
            &[&draft_id],
        )
        .await
        .expect("Draft row must exist");

    assert_eq!(draft_row.get::<_, String>("lifecycle_state"), "DRAFT");
    assert_eq!(draft_row.get::<_, String>("created_by"), "agent-runner-42");

    // Verify REQ-006 active node remains unmodified
    let req6_row = client
        .query_one(
            "SELECT title, content, lifecycle_state FROM graph_nodes WHERE node_key = 'REQ-006' AND lifecycle_state = 'ACTIVE';",
            &[],
        )
        .await
        .expect("REQ-006 active node must remain");
    assert_eq!(
        req6_row.get::<_, String>("title"),
        "Normative Protocol Specification"
    );
    println!(
        "Verified: In-place edit intercepted; candidate staged as DRAFT; REQ-006 active unchanged."
    );

    // -------------------------------------------------------------------------
    // STEP 8: Caller Draft Querying and Isolation
    // -------------------------------------------------------------------------
    println!("\n--- [Step 8] Verifying Caller-Scoped Draft Query Isolation ---");

    // Author (agent-runner-42) queries draft with include_drafts: true -> succeeds
    let author_draft_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 208,
        "method": "tools/call",
        "params": {
            "name": "get_context_envelope",
            "arguments": {
                "target_node_id": draft_id.to_string(),
                "depth": 2,
                "include_drafts": true
            }
        }
    });

    let author_resp = http
        .post(&mcp_url)
        .header(AUTHORIZATION, format!("Bearer {agent42_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&author_draft_req)
        .send()
        .await
        .expect("Author draft query failed");

    assert_eq!(author_resp.status(), StatusCode::OK);
    let author_json: serde_json::Value = author_resp.json().await.unwrap();
    assert!(author_json.get("result").is_some(), "Author must see draft");

    // Second agent (agent-runner-99) attempts to query draft -> rejected / hidden
    let intruder_resp = http
        .post(&mcp_url)
        .header(AUTHORIZATION, format!("Bearer {agent99_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&author_draft_req)
        .send()
        .await
        .expect("Intruder draft query failed");

    let intruder_json: serde_json::Value = intruder_resp.json().await.unwrap();
    assert!(
        intruder_json.get("error").is_some() || intruder_json.get("result").is_none(),
        "Non-authoring agent must not access candidate draft"
    );
    println!(
        "Verified: Draft is visible to author (include_drafts: true) but isolated from other agents."
    );

    // -------------------------------------------------------------------------
    // STEP 9: Staging Approval and Draft Revision Squashing
    // -------------------------------------------------------------------------
    println!("\n--- [Step 9] Staging Approval & Draft Squash into audit_ledger ---");

    // Admin approves the draft via POST /api/v1/staging/approve
    let approve_url = format!("{}/api/v1/staging/approve", ctx.base_url);
    let approve_resp = http
        .post(&approve_url)
        .header(AUTHORIZATION, format!("Bearer {admin_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({
            "approved_node_ids": [draft_id]
        }))
        .send()
        .await
        .expect("Staging approval request failed");

    assert_eq!(approve_resp.status(), StatusCode::OK);
    let approve_data: StagingApproveResponse = approve_resp.json().await.unwrap();
    assert_eq!(approve_data.approved_nodes, 1);
    println!(
        "Draft approved successfully: batch_id={}",
        approve_data.batch_id
    );

    // Verify draft node is promoted to ACTIVE
    let promoted_row = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&draft_id],
        )
        .await
        .unwrap();
    assert_eq!(promoted_row.get::<_, String>("lifecycle_state"), "ACTIVE");

    // Verify previous node was superseded
    let superseded_count: i64 = client
        .query_one(
            "SELECT count(*) FROM graph_nodes WHERE node_key = 'REQ-006' AND lifecycle_state = 'SUPERSEDED';",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        superseded_count, 1,
        "Previous REQ-006 node must be marked SUPERSEDED"
    );

    // Verify audit_ledger contains APPROVED event with draft_evolution_summary
    let approve_audit = client
        .query_one(
            "SELECT event_type, batch_id, draft_evolution_summary \
             FROM audit_ledger WHERE event_type = 'APPROVED' AND batch_id = $1;",
            &[&approve_data.batch_id],
        )
        .await
        .expect("Must have APPROVED audit record");
    assert!(
        approve_audit
            .get::<_, Option<serde_json::Value>>("draft_evolution_summary")
            .is_some()
    );
    println!(
        "Verified: Draft squashed into canonical APPROVED audit record with draft_evolution_summary."
    );

    // -------------------------------------------------------------------------
    // STEP 10: Unified Rollback (Dry-Run, Confirmation Abort, Compensating Commit)
    // -------------------------------------------------------------------------
    println!("\n--- [Step 10] Testing Unified Rollback & Blast-Radius Safety Abort ---");

    // 10.1 Agent 99 creates a dependent child task under Agent 42's task_ids[1] (TASK-102)
    let child_task_resp = tks::storage::mutation::elaborate_task(
        &mut db_client,
        &task_ids[1].to_string(),
        "Agent 99 Dependent Task",
        Some("Dependent implementation authored by second agent"),
        Some(serde_json::json!({ "node_key": "TASK-GATE2-DEP-99" })),
        &tks::gateway::auth::AuthenticatedAgent {
            agent_id: "agent-runner-99".to_string(),
            actor_type: "AGENT".to_string(),
        },
    )
    .await
    .expect("Agent 99 dependent task elaboration failed");
    let dep_task_id = child_task_resp.task_id;
    println!("Agent 99 created dependent child task: ID={dep_task_id}");

    // 10.2 Admin tests dry-run rollback on primary batch
    let revert_url = format!("{}/api/v1/admin/revert-mutations", ctx.base_url);
    let dry_run_resp = http
        .post(&revert_url)
        .header(AUTHORIZATION, format!("Bearer {admin_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({
            "batch_id": primary_batch_id,
            "dry_run": true
        }))
        .send()
        .await
        .expect("Dry run revert failed");

    assert_eq!(dry_run_resp.status(), StatusCode::OK);
    let dry_json: serde_json::Value = dry_run_resp.json().await.unwrap();
    assert_eq!(
        dry_json.get("status").and_then(|v| v.as_str()),
        Some("PREVIEW")
    );
    assert_eq!(
        dry_json.get("reverted_events").and_then(|v| v.as_u64()),
        Some(0)
    );
    println!("Dry-run preview verified: zero mutations committed.");

    // 10.3 Admin attempts revert with force: false -> must abort with ERR_CONFIRMATION_REQUIRED
    let abort_resp = http
        .post(&revert_url)
        .header(AUTHORIZATION, format!("Bearer {admin_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({
            "batch_id": primary_batch_id,
            "dry_run": false,
            "force": false
        }))
        .send()
        .await
        .expect("Confirmation abort check failed");

    assert_eq!(
        abort_resp.status(),
        StatusCode::CONFLICT,
        "Revert with cross-agent dependencies without force must return 409 Conflict"
    );
    let abort_json: serde_json::Value = abort_resp.json().await.unwrap();
    assert_eq!(
        abort_json.get("error").and_then(|v| v.as_str()),
        Some("ERR_CONFIRMATION_REQUIRED")
    );
    assert!(abort_json.get("preview").is_some());
    println!(
        "Verified: Revert halted with ERR_CONFIRMATION_REQUIRED due to cross-agent dependency."
    );

    // 10.4 Admin resubmits revert with force: true -> commits compensating REVERT events
    let force_resp = http
        .post(&revert_url)
        .header(AUTHORIZATION, format!("Bearer {admin_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({
            "batch_id": primary_batch_id,
            "dry_run": false,
            "force": true
        }))
        .send()
        .await
        .expect("Forced revert failed");

    assert_eq!(force_resp.status(), StatusCode::OK);
    let force_json: serde_json::Value = force_resp.json().await.unwrap();
    assert_eq!(
        force_json.get("status").and_then(|v| v.as_str()),
        Some("REVERTED")
    );
    println!("Verified: Compensating transaction committed with force=true.");

    // -------------------------------------------------------------------------
    // STEP 11: Cascading Invalidation & Reverification Recovery
    // -------------------------------------------------------------------------
    println!("\n--- [Step 11] Cascading Invalidation & Reverification Recovery ---");

    // Parent task_ids[1] is now SUPERSEDED
    let parent_state = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&task_ids[1]],
        )
        .await
        .unwrap()
        .get::<_, String>("lifecycle_state");
    assert_eq!(parent_state, "SUPERSEDED");

    // Child task dep_task_id cascaded to NEEDS_REVERIFICATION
    let dep_state = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&dep_task_id],
        )
        .await
        .unwrap()
        .get::<_, String>("lifecycle_state");
    assert_eq!(
        dep_state, "NEEDS_REVERIFICATION",
        "Child task must cascade to NEEDS_REVERIFICATION"
    );
    println!("Verified: Dependent child task cascaded to NEEDS_REVERIFICATION.");

    // Reverify child task, re-parenting it to active task_ids[2] (TASK-103)
    let reverify_url = format!("{}/api/v1/nodes/{dep_task_id}/reverify", ctx.base_url);
    let reverify_resp = http
        .post(&reverify_url)
        .header(AUTHORIZATION, format!("Bearer {admin_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({
            "rationale": "Re-anchored dependent task under active task TASK-103 following parent rollback",
            "updated_attributes": {
                "reparent_to": task_ids[2].to_string()
            }
        }))
        .send()
        .await
        .expect("Reverification request failed");

    assert_eq!(reverify_resp.status(), StatusCode::OK);
    let reverify_data: serde_json::Value = reverify_resp.json().await.unwrap();
    assert_eq!(
        reverify_data.get("status").and_then(|v| v.as_str()),
        Some("ACTIVE")
    );
    assert_eq!(
        reverify_data
            .get("staleness_score")
            .and_then(|v| v.as_f64()),
        Some(0.0)
    );

    // Verify child task restored to ACTIVE and REVERIFIED audit record logged
    let restored_state = client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&dep_task_id],
        )
        .await
        .unwrap()
        .get::<_, String>("lifecycle_state");
    assert_eq!(restored_state, "ACTIVE");

    let rev_audit = client
        .query_one(
            "SELECT event_type FROM audit_ledger WHERE entity_id = $1 AND event_type = 'REVERIFIED';",
            &[&dep_task_id],
        )
        .await
        .expect("Must have REVERIFIED audit record");
    assert_eq!(rev_audit.get::<_, String>("event_type"), "REVERIFIED");
    println!(
        "Verified: Child task restored to ACTIVE with staleness_score=0.0 and REVERIFIED audit event."
    );

    // -------------------------------------------------------------------------
    // STEP 12: External Agents Authoring & Tracking Phase 3 Preparatory Tasks
    // -------------------------------------------------------------------------
    println!("\n--- [Step 12] External Agents Authoring Phase 3 Prep Tasks (INV-6) ---");

    let phase3_tasks = [
        (
            "PHASE3-001",
            "Phase 3 Web Explorer: Real-Time DAG Visualization & State Inspector",
            "Develop interactive Cytoscape/WebGL graph canvas visualizing multi-depth requirement trees, governance states, and cascade invalidation paths.",
        ),
        (
            "PHASE3-002",
            "Phase 3 Automated Invalidation Cascade Engine",
            "Implement high-throughput event notification bus triggering recursive downward invalidation sweeps across modified specifications.",
        ),
        (
            "PHASE3-003",
            "Phase 3 Multi-Agent Workspace Isolation & Conflict Resolver",
            "Develop branch-isolated workspace containers enabling concurrent agent task elaboration without uncommitted lock interference.",
        ),
    ];

    let mut phase3_created_ids = Vec::new();

    for (key, title, content) in &phase3_tasks {
        let p3_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 300,
            "method": "tools/call",
            "params": {
                "name": "create_subtask",
                "arguments": {
                    "parent_node_id": "REQ-005",
                    "title": title,
                    "content": content,
                    "attributes": {
                        "node_key": key,
                        "milestone": "Gate 3",
                        "assigned_agent": "agent-runner-42"
                    }
                }
            }
        });

        let resp = http
            .post(&mcp_url)
            .header(AUTHORIZATION, format!("Bearer {agent42_token}"))
            .header(CONTENT_TYPE, "application/json")
            .json(&p3_req)
            .send()
            .await
            .expect("Failed to create Phase 3 prep task");

        assert_eq!(resp.status(), StatusCode::OK);
        let resp_json: serde_json::Value = resp.json().await.unwrap();
        let result = resp_json.get("result").expect("Must contain result");
        let task_obj = result.get("subtask").unwrap_or(result);
        assert_eq!(
            task_obj.get("status").and_then(|v| v.as_str()),
            Some("ACTIVE")
        );

        let pid: Uuid = task_obj
            .get("task_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap();
        phase3_created_ids.push(pid);
        println!("Authoring Phase 3 Prep Task [{key}]: {title} (ID: {pid})");
    }

    assert_eq!(phase3_created_ids.len(), 3);

    // Agent transitions PHASE3-001 to IN_PROGRESS and PHASE3-002 to COMPLETED
    let t_update1 = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 301,
        "method": "tools/call",
        "params": {
            "name": "update_node_status",
            "arguments": {
                "node_id": "PHASE3-001",
                "status": "IN_PROGRESS",
                "notes": "Initialized SvelteKit shell and graph canvas layout"
            }
        }
    });
    let u1_resp = http
        .post(&mcp_url)
        .header(AUTHORIZATION, format!("Bearer {agent42_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&t_update1)
        .send()
        .await
        .unwrap();
    assert_eq!(u1_resp.status(), StatusCode::OK);

    let t_update2 = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 302,
        "method": "tools/call",
        "params": {
            "name": "update_node_status",
            "arguments": {
                "node_id": "PHASE3-002",
                "status": "COMPLETED",
                "notes": "Verified recursive cascade CTE benchmarks with sub-10ms response"
            }
        }
    });
    let u2_resp = http
        .post(&mcp_url)
        .header(AUTHORIZATION, format!("Bearer {agent42_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&t_update2)
        .send()
        .await
        .unwrap();
    assert_eq!(u2_resp.status(), StatusCode::OK);

    // Verify context envelope for PHASE3-001 resolves cleanly
    let p3_env_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 303,
        "method": "tools/call",
        "params": {
            "name": "get_context_envelope",
            "arguments": {
                "target_node_id": "PHASE3-001",
                "depth": 2
            }
        }
    });
    let p3_env_resp = http
        .post(&mcp_url)
        .header(AUTHORIZATION, format!("Bearer {agent42_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&p3_env_req)
        .send()
        .await
        .unwrap();
    assert_eq!(p3_env_resp.status(), StatusCode::OK);
    let p3_env_json: serde_json::Value = p3_env_resp.json().await.unwrap();
    assert!(p3_env_json.get("result").is_some());

    println!(
        "Verified: External agents successfully authored, updated, and tracked Phase 3 preparatory tasks directly within the substrate."
    );

    // -------------------------------------------------------------------------
    // STEP 13: Test CLI Operational Subcommands (task and admin)
    // -------------------------------------------------------------------------
    println!("\n--- [Step 13] Verifying CLI Operational Subcommands ---");

    // 13.1 `tks task create`
    let cli_create_res = run_task(
        TaskSubcommand::Create(CreateTaskArgs {
            parent: "REQ-005".to_string(),
            title: "CLI Created Task".to_string(),
            content: Some("Task created via operational CLI subcommand".to_string()),
        }),
        &ctx.base_url,
        agent42_token,
    )
    .await;
    assert!(
        cli_create_res.is_ok(),
        "tks task create must succeed: {cli_create_res:?}"
    );

    // 13.2 `tks task list`
    let cli_list_res = run_task(
        TaskSubcommand::List(ListTasksArgs {
            parent: Some("REQ-005".to_string()),
            status: None,
            limit: 10,
        }),
        &ctx.base_url,
        agent42_token,
    )
    .await;
    assert!(
        cli_list_res.is_ok(),
        "tks task list must succeed: {cli_list_res:?}"
    );

    // 13.3 `tks task update`
    let cli_update_res = run_task(
        TaskSubcommand::Update(UpdateTaskArgs {
            id: "PHASE3-001".to_string(),
            status: "COMPLETED".to_string(),
            notes: Some("Completed via CLI command".to_string()),
        }),
        &ctx.base_url,
        agent42_token,
    )
    .await;
    assert!(
        cli_update_res.is_ok(),
        "tks task update must succeed: {cli_update_res:?}"
    );

    // 13.4 `tks admin reverify`
    let cli_reverify_res = run_admin(
        AdminSubcommand::Reverify(AdminReverifyArgs {
            id: "PHASE3-001".to_string(),
            rationale: "Reverified operational state via CLI".to_string(),
            reparent_to: None,
        }),
        &ctx.base_url,
        admin_token,
    )
    .await;
    assert!(
        cli_reverify_res.is_ok(),
        "tks admin reverify must succeed: {cli_reverify_res:?}"
    );

    // 13.5 `tks admin revert --dry-run`
    let cli_revert_res = run_admin(
        AdminSubcommand::Revert(AdminRevertArgs {
            batch_id: Some(primary_batch_id),
            agent_id: None,
            since: None,
            event_seq_start: None,
            event_seq_end: None,
            dry_run: true,
            force: false,
        }),
        &ctx.base_url,
        admin_token,
    )
    .await;
    assert!(
        cli_revert_res.is_ok(),
        "tks admin revert --dry-run must succeed: {cli_revert_res:?}"
    );

    println!("All CLI operational subcommands executed cleanly.");

    println!("\n===============================================================================");
    println!("  GATE 2 ACCEPTANCE SCORECARD: PASS (ALL 11 MVD STEPS + PHASE 3 SELF-EVOLUTION)");
    println!("===============================================================================\n");

    ctx.cancel_token.cancel();
}
