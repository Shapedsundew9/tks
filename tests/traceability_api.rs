//! Integration test suite for Pre-Merge CI Traceability, VCS Commit Lineage & Contract Drift Detection (WP-4.5).
//!
//! Verifies:
//! - Pre-merge CI test run ingestion via `POST /api/v1/verification/test-run` into `TASK.attributes->'pre_merge_verification'` (D-107, TB-12.1).
//! - Confirmation of zero provisional commit nodes and zero relationally invalid graph edges created during pre-merge verification (D-107).
//! - Disambiguated release readiness inspection `GET /api/v1/release/readiness?scope=pre-merge` gating PR readiness (D-113, D-120).
//! - VCS merge commit materialization `POST /api/v1/vcs/commits` creating `CODE_COMMIT` nodes and upward directed `IMPLEMENTS` edges (`CODE_COMMIT -IMPLEMENTS-> TASK`, D-108, INV-1).
//! - Pre-transaction edge pre-validation rejection (`ERR_INVALID_EDGE_TARGET`, HTTP 422) for raw Git commit SHAs and non-existent task targets (INV-8, INV-9, D-96, D-97).
//! - Post-merge release readiness inspection `GET /api/v1/release/readiness?scope=release` verifying complete milestone requirement satisfaction (D-113, TB-12.3).
//! - Code-side contract drift detection hooks (`POST /api/v1/verification/check-drift`).

use std::time::Duration;

use reqwest::StatusCode;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tks::db;
use tks::gateway::auth::compute_token_hash;
use tks::gateway::{AppState, create_router};
use tks::storage::StorageRepo;
use tks::storage::git::start_git_actor;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Spawns a test gateway server on an ephemeral TCP port.
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
    let pool = db::create_pool(&test_db_url).expect("Failed to create PostgreSQL connection pool");

    // Clean test residue
    {
        let client = pool.get().await.expect("Failed to get DB client");
        let _ = client
            .execute(
                "DELETE FROM graph_edges WHERE created_by LIKE 'test_wp45_%';",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM graph_nodes WHERE created_by LIKE 'test_wp45_%' OR node_key LIKE 'REQ-WP45-%' OR node_key LIKE 'TASK-WP45-%' OR node_key LIKE 'COMMIT-%';",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM audit_ledger WHERE actor_id LIKE 'test_wp45_%';",
                &[],
            )
            .await;
    }

    let repo_dir = format!("target/test-traceability-git-{}", Uuid::new_v4());
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
             VALUES ($1, $2, 'HUMAN', TRUE) \
             ON CONFLICT (agent_id) DO UPDATE SET token_hash = EXCLUDED.token_hash, is_active = TRUE;",
            &[&agent_id, &hash],
        )
        .await
        .expect("Failed to insert agent identity");

    state.auth.register_token(agent_id, &hash, "HUMAN").await;
    state.auth.register_token(agent_id, token, "HUMAN").await;
}

/// Helper to seed a valid root requirement and child tasks.
async fn seed_requirement_and_tasks(
    state: &AppState,
    prefix: &str,
    task_count: usize,
) -> (Uuid, Vec<Uuid>) {
    let client = state.pool.get().await.expect("Failed to get DB client");
    let req_id = Uuid::new_v4();
    let req_key = format!("REQ-WP45-{prefix}");

    client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, trust_tier) \
             VALUES ($1, $2, 'REQUIREMENT', 'Root Requirement', 'Requirement content', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'test_wp45_author', 'HUMAN_APPROVED');",
            &[&req_id, &req_key],
        )
        .await
        .expect("Failed to insert root requirement");

    let mut task_ids = Vec::with_capacity(task_count);
    for i in 1..=task_count {
        let task_id = Uuid::new_v4();
        let task_key = format!("TASK-WP45-{prefix}-{i}");

        client
            .execute(
                "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, trust_tier) \
                 VALUES ($1, $2, 'TASK', 'Subtask', 'Task intent', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'test_wp45_author', 'HUMAN_APPROVED');",
                &[&task_id, &task_key],
            )
            .await
            .expect("Failed to insert task");

        // Insert upward FULFILLS edge from task to requirement
        client
            .execute(
                "INSERT INTO graph_edges (from_node_id, to_node_id, edge_type, created_by, lifecycle_state) \
                 VALUES ($1, $2, 'FULFILLS', 'test_wp45_author', 'ACTIVE');",
                &[&task_id, &req_id],
            )
            .await
            .expect("Failed to insert upward edge");

        task_ids.push(task_id);
    }

    (req_id, task_ids)
}

#[tokio::test]
async fn test_pre_merge_verification_ingestion() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    provision_agent(&state, "ci_runner", "ci_token_wp45").await;
    let (_req_id, task_ids) = seed_requirement_and_tasks(&state, "TEST1", 1).await;
    let target_task_id = task_ids[0];

    let client = reqwest::Client::new();

    // 1. Unauthenticated request should fail with 401
    let unauth_resp = client
        .post(format!("{base_url}/api/v1/verification/test-run"))
        .json(&json!({
            "test_suite": "cargo_test",
            "passed": true,
            "pr_commit_sha": "sha-pr-1234",
            "target_task_ids": [target_task_id],
        }))
        .send()
        .await
        .expect("Failed to send unauth request");
    assert_eq!(unauth_resp.status(), StatusCode::UNAUTHORIZED);

    // Count edges and commit nodes before ingestion
    let db_client = state.pool.get().await.expect("Failed to get DB client");
    let edges_before: i64 = db_client
        .query_one("SELECT COUNT(*) FROM graph_edges;", &[])
        .await
        .unwrap()
        .get(0);
    let commit_nodes_before: i64 = db_client
        .query_one(
            "SELECT COUNT(*) FROM graph_nodes WHERE node_type = 'CODE_COMMIT';",
            &[],
        )
        .await
        .unwrap()
        .get(0);

    // 2. Ingest passing CI test run with valid Bearer token
    let resp = client
        .post(format!("{base_url}/api/v1/verification/test-run"))
        .header("Authorization", "Bearer ci_token_wp45")
        .json(&json!({
            "test_suite": "cargo_test",
            "passed": true,
            "pr_commit_sha": "sha-pr-1234",
            "target_task_ids": [target_task_id],
            "duration_ms": 1240,
            "manifest_hash": "sha256-abcdef",
            "log_summary": "All 42 tests passed cleanly"
        }))
        .send()
        .await
        .expect("Failed to send test-run request");

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("Failed to parse JSON");
    assert_eq!(body["status"], "RECORDED");
    assert_eq!(body["test_suite"], "cargo_test");
    assert_eq!(body["passed"], true);
    assert_eq!(body["pr_commit_sha"], "sha-pr-1234");

    // 3. Verify test run stored directly in TASK.attributes->'pre_merge_verification' (D-107)
    let row = db_client
        .query_one(
            "SELECT attributes FROM graph_nodes WHERE id = $1;",
            &[&target_task_id],
        )
        .await
        .expect("Failed to fetch task");
    let attrs: serde_json::Value = row.get("attributes");
    let verification_array = attrs
        .get("pre_merge_verification")
        .and_then(|v| v.as_array())
        .expect("Expected pre_merge_verification array in attributes");

    assert_eq!(verification_array.len(), 1);
    assert_eq!(verification_array[0]["test_suite"], "cargo_test");
    assert_eq!(verification_array[0]["passed"], true);
    assert_eq!(verification_array[0]["pr_commit_sha"], "sha-pr-1234");
    assert_eq!(verification_array[0]["duration_ms"], 1240);
    assert_eq!(verification_array[0]["manifest_hash"], "sha256-abcdef");

    // 4. Verify ZERO new graph edges and ZERO provisional commit nodes were created (D-107)
    let edges_after: i64 = db_client
        .query_one("SELECT COUNT(*) FROM graph_edges;", &[])
        .await
        .unwrap()
        .get(0);
    let commit_nodes_after: i64 = db_client
        .query_one(
            "SELECT COUNT(*) FROM graph_nodes WHERE node_type = 'CODE_COMMIT';",
            &[],
        )
        .await
        .unwrap()
        .get(0);

    assert_eq!(
        edges_after, edges_before,
        "Pre-merge test run must not create graph edges (D-107)"
    );
    assert_eq!(
        commit_nodes_after, commit_nodes_before,
        "Pre-merge test run must not create provisional commit nodes (D-107)"
    );

    cancel.cancel();
}

#[tokio::test]
async fn test_pre_merge_release_readiness_gating() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    provision_agent(&state, "ci_runner", "ci_token_wp45").await;
    let (_req_id, task_ids) = seed_requirement_and_tasks(&state, "GATE", 2).await;
    let task_1 = task_ids[0];
    let task_2 = task_ids[1];

    let client = reqwest::Client::new();
    let pr_sha = "sha-pr-gating-test";

    // 1. Missing pr_commit_sha returns 400 Bad Request
    let bad_resp = client
        .get(format!(
            "{base_url}/api/v1/release/readiness?scope=pre-merge"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_resp.status(), StatusCode::BAD_REQUEST);

    // 2. Before any test runs: checking readiness with task_ids returns BLOCKED with both tasks
    let resp = client
        .get(format!(
            "{base_url}/api/v1/release/readiness?scope=pre-merge&pr_commit_sha={pr_sha}&task_ids={task_1},{task_2}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["scope"], "pre-merge");
    assert_eq!(body["status"], "BLOCKED");
    let unverified = body["unverified_task_ids"].as_array().unwrap();
    assert_eq!(unverified.len(), 2);

    // 3. Ingest passing test run for task_1 only
    let _ = client
        .post(format!("{base_url}/api/v1/verification/test-run"))
        .header("Authorization", "Bearer ci_token_wp45")
        .json(&json!({
            "test_suite": "cargo_test",
            "passed": true,
            "pr_commit_sha": pr_sha,
            "target_task_ids": [task_1],
        }))
        .send()
        .await
        .unwrap();

    // Query readiness again: should still be BLOCKED with only task_2 unverified
    let resp2 = client
        .get(format!(
            "{base_url}/api/v1/release/readiness?scope=pre-merge&pr_commit_sha={pr_sha}&task_ids={task_1},{task_2}"
        ))
        .send()
        .await
        .unwrap();
    let body2: serde_json::Value = resp2.json().await.unwrap();
    assert_eq!(body2["status"], "BLOCKED");
    let unverified2 = body2["unverified_task_ids"].as_array().unwrap();
    assert_eq!(unverified2.len(), 1);
    assert_eq!(unverified2[0], task_2.to_string());

    // 4. Ingest passing test run for task_2
    let _ = client
        .post(format!("{base_url}/api/v1/verification/test-run"))
        .header("Authorization", "Bearer ci_token_wp45")
        .json(&json!({
            "test_suite": "cargo_test",
            "passed": true,
            "pr_commit_sha": pr_sha,
            "target_task_ids": [task_2],
        }))
        .send()
        .await
        .unwrap();

    // Query readiness: both tasks verified -> returns READY with empty unverified list!
    let resp3 = client
        .get(format!(
            "{base_url}/api/v1/release/readiness?scope=pre-merge&pr_commit_sha={pr_sha}&task_ids={task_1},{task_2}"
        ))
        .send()
        .await
        .unwrap();
    let body3: serde_json::Value = resp3.json().await.unwrap();
    assert_eq!(body3["status"], "READY");
    let unverified3 = body3["unverified_task_ids"].as_array().unwrap();
    assert!(unverified3.is_empty());

    cancel.cancel();
}

#[tokio::test]
async fn test_vcs_merge_commit_edge_prevalidation_rejection() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    provision_agent(&state, "vcs_bot", "vcs_token_wp45").await;

    let client = reqwest::Client::new();

    // 1. Submit raw Git commit SHA string as task ID target (INV-8, INV-9, D-96, D-97)
    let resp = client
        .post(format!("{base_url}/api/v1/vcs/commits"))
        .header("Authorization", "Bearer vcs_token_wp45")
        .json(&json!({
            "commit_sha": "c0ffee1234567890abcdef",
            "author": "developer@example.com",
            "message": "merge pull request #42",
            "task_ids": ["c0ffee1234deadbeef"], // invalid UUID format
        }))
        .send()
        .await
        .unwrap();

    // Must return HTTP 422 Unprocessable Entity with ERR_INVALID_EDGE_TARGET envelope
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "ERR_INVALID_EDGE_TARGET");
    assert_eq!(body["invalid_field"], "task_ids");
    assert_eq!(body["invalid_value"], "c0ffee1234deadbeef");
    assert!(body["suggested_repairs"].as_array().is_some());

    // 2. Submit non-existent UUID
    let non_existent = Uuid::new_v4();
    let resp_nonexistent = client
        .post(format!("{base_url}/api/v1/vcs/commits"))
        .header("Authorization", "Bearer vcs_token_wp45")
        .json(&json!({
            "commit_sha": "1234567890abcdef",
            "author": "developer@example.com",
            "message": "merge pull request #43",
            "task_ids": [non_existent.to_string()],
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_nonexistent.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body_ne: serde_json::Value = resp_nonexistent.json().await.unwrap();
    assert_eq!(body_ne["code"], "ERR_INVALID_EDGE_TARGET");

    cancel.cancel();
}

#[tokio::test]
async fn test_vcs_merge_commit_materialization_and_ancestry() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    provision_agent(&state, "vcs_bot", "vcs_token_wp45").await;
    let (_req_id, task_ids) = seed_requirement_and_tasks(&state, "COMMIT", 2).await;
    let task_1 = task_ids[0];
    let task_2 = task_ids[1];

    let client = reqwest::Client::new();
    let commit_sha = "abc789def01234567890abcdef";

    // Materialize canonical merge commit linking upward to task_1 and task_2 (D-108)
    let resp = client
        .post(format!("{base_url}/api/v1/vcs/commits"))
        .header("Authorization", "Bearer vcs_token_wp45")
        .json(&json!({
            "commit_sha": commit_sha,
            "author": "lead-dev@example.com",
            "message": "feat: resolve task 1 and task 2\n\nCloses issues #1 and #2",
            "branch": "main",
            "timestamp": "2026-10-11T20:15:00Z",
            "task_ids": [task_1, task_2],
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "MATERIALIZED");
    let commit_id: Uuid = body["commit_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(body["commit_sha"], commit_sha);

    // Verify CODE_COMMIT node created in graph_nodes
    let db_client = state.pool.get().await.expect("Failed to get DB client");
    let commit_row = db_client
        .query_one(
            "SELECT node_type, lifecycle_state, governance_policy, node_key, title, content, attributes \
             FROM graph_nodes WHERE id = $1;",
            &[&commit_id],
        )
        .await
        .expect("Failed to query commit node");

    let node_type: String = commit_row.get("node_type");
    let state_str: String = commit_row.get("lifecycle_state");
    let policy: String = commit_row.get("governance_policy");
    let title: Option<String> = commit_row.get("title");

    assert_eq!(node_type, "CODE_COMMIT");
    assert_eq!(state_str, "ACTIVE");
    assert_eq!(policy, "LOCKED");
    assert_eq!(title.as_deref(), Some("feat: resolve task 1 and task 2"));

    // Verify upward directed IMPLEMENTS edges pointing from commit_id to task_1 and task_2 (D-108, INV-1)
    let edge_rows = db_client
        .query(
            "SELECT from_node_id, to_node_id, edge_type, lifecycle_state \
             FROM graph_edges WHERE from_node_id = $1 ORDER BY to_node_id;",
            &[&commit_id],
        )
        .await
        .expect("Failed to query edges");

    assert_eq!(edge_rows.len(), 2);
    for row in edge_rows {
        let from_id: Uuid = row.get("from_node_id");
        let to_id: Uuid = row.get("to_node_id");
        let edge_type: String = row.get("edge_type");
        let state: String = row.get("lifecycle_state");

        assert_eq!(from_id, commit_id);
        assert!(to_id == task_1 || to_id == task_2);
        assert_eq!(edge_type, "IMPLEMENTS");
        assert_eq!(state, "ACTIVE");
    }

    // Verify Invariant INV-1 ancestry: CODE_COMMIT reaches active REQUIREMENT root upward
    let has_ancestry =
        tks::storage::envelope::validate_ancestor_path(&**db_client, &[commit_id], false)
            .await
            .expect("Failed to validate ancestor path");
    assert!(
        has_ancestry,
        "CODE_COMMIT node must maintain unbroken upward path to active REQUIREMENT (INV-1)"
    );

    // Verify tasks have commit_sha reflected in attributes->'vcs_commits'
    let task_row = db_client
        .query_one(
            "SELECT attributes FROM graph_nodes WHERE id = $1;",
            &[&task_1],
        )
        .await
        .unwrap();
    let task_attrs: serde_json::Value = task_row.get("attributes");
    let commits = task_attrs["vcs_commits"]
        .as_array()
        .expect("Expected vcs_commits array");
    assert!(commits.iter().any(|v| v.as_str() == Some(commit_sha)));

    cancel.cancel();
}

#[tokio::test]
async fn test_release_scope_readiness_verification() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    provision_agent(&state, "release_manager", "rel_token_wp45").await;
    let (req_id, task_ids) = seed_requirement_and_tasks(&state, "REL", 1).await;
    let task_id = task_ids[0];

    let client = reqwest::Client::new();

    // 1. Before commit materialization: release scope readiness returns BLOCKED with uncommitted task
    let resp = client
        .get(format!(
            "{base_url}/api/v1/release/readiness?scope=release&requirement_root_id={req_id}"
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["scope"], "release");
    assert_eq!(body["status"], "BLOCKED");
    let uncommitted = body["uncommitted_tasks"].as_array().unwrap();
    assert_eq!(uncommitted.len(), 1);
    assert_eq!(uncommitted[0], task_id.to_string());

    // 2. Materialize merge commit for the task
    let _ = client
        .post(format!("{base_url}/api/v1/vcs/commits"))
        .header("Authorization", "Bearer rel_token_wp45")
        .json(&json!({
            "commit_sha": "release-commit-sha-45",
            "author": "lead-dev@example.com",
            "message": "chore: complete task for release",
            "task_ids": [task_id],
        }))
        .send()
        .await
        .unwrap();

    // 3. After commit materialization: release scope readiness returns READY!
    let resp_after = client
        .get(format!(
            "{base_url}/api/v1/release/readiness?scope=release&requirement_root_id={req_id}"
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_after.status(), StatusCode::OK);
    let body_after: serde_json::Value = resp_after.json().await.unwrap();
    assert_eq!(body_after["status"], "READY");
    let uncommitted_after = body_after["uncommitted_tasks"].as_array().unwrap();
    assert!(uncommitted_after.is_empty());
    let satisfied = body_after["satisfied_requirements"].as_array().unwrap();
    assert_eq!(satisfied.len(), 1);
    assert_eq!(body_after["readiness_percentage"], 100.0);

    cancel.cancel();
}

#[tokio::test]
async fn test_contract_drift_detection() {
    let (base_url, cancel, state, _guard) = spawn_test_server().await;
    provision_agent(&state, "developer", "dev_token_wp45").await;

    let client = reqwest::Client::new();

    // 1. Diff modifying public contract/interface without attribution
    let unattributed_diff = r#"
diff --git a/specs/unattributed_api.yaml b/specs/unattributed_api.yaml
new file mode 100644
--- /dev/null
+++ b/specs/unattributed_api.yaml
@@ -0,0 +1,5 @@
+openapi: 3.1.0
+info:
+  title: Unattributed API
+  version: 1.0.0
"#;

    let resp = client
        .post(format!("{base_url}/api/v1/verification/check-drift"))
        .header("Authorization", "Bearer dev_token_wp45")
        .json(&json!({
            "diff": unattributed_diff,
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "DRIFT_DETECTED");
    let anomalies = body["anomalies"].as_array().unwrap();
    assert!(!anomalies.is_empty());
    assert_eq!(anomalies[0]["file"], "specs/unattributed_api.yaml");
    assert_eq!(
        anomalies[0]["anomaly_type"],
        "UNATTRIBUTED_CONTRACT_MODIFICATION"
    );

    // 2. Diff modifying internal unexported non-contract code -> clean
    let internal_diff = r#"
diff --git a/src/internal_helper.rs b/src/internal_helper.rs
--- a/src/internal_helper.rs
+++ b/src/internal_helper.rs
@@ -1,3 +1,3 @@
-fn helper() {}
+fn helper() -> bool { true }
"#;

    let resp_internal = client
        .post(format!("{base_url}/api/v1/verification/check-drift"))
        .header("Authorization", "Bearer dev_token_wp45")
        .json(&json!({
            "diff": internal_diff,
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_internal.status(), StatusCode::OK);
    let body_internal: serde_json::Value = resp_internal.json().await.unwrap();
    assert_eq!(body_internal["status"], "CLEAN");
    assert!(body_internal["anomalies"].as_array().unwrap().is_empty());

    // 3. Attributing the contract file to an active requirement node -> clean
    let db_client = state.pool.get().await.expect("Failed to get DB client");
    db_client
        .execute(
            "INSERT INTO graph_nodes (node_key, node_type, title, lifecycle_state, governance_policy, created_by, doc_path, trust_tier) \
             VALUES ('SPEC-ATTRIBUTED', 'SPECIFICATION', 'Attributed Spec', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'test_wp45_author', 'specs/unattributed_api.yaml', 'HUMAN_APPROVED');",
            &[],
        )
        .await
        .unwrap();

    let resp_attributed = client
        .post(format!("{base_url}/api/v1/verification/check-drift"))
        .header("Authorization", "Bearer dev_token_wp45")
        .json(&json!({
            "diff": unattributed_diff,
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_attributed.status(), StatusCode::OK);
    let body_att: serde_json::Value = resp_attributed.json().await.unwrap();
    assert_eq!(body_att["status"], "CLEAN");
    assert!(body_att["anomalies"].as_array().unwrap().is_empty());

    cancel.cancel();
}
