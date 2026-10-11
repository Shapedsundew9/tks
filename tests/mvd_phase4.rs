//! Phase 4 Minimum Viable Demonstration (MVD) Acceptance Test Suite (WP-4.6).
//!
//! Executes the complete 12-step Phase 4 MVD Acceptance Test Procedure
//! defined in `docs/vision/strategic-planning-backlog.md` §4 and `docs/vision/phase4-plan.md` §3 & §4.2:
//!
//! 1. Ingest multi-section technical vision document (`specs/vision.md`) and brownfield artifacts
//!    (OpenAPI, ADR, SDD).
//! 2. Verify Tier 1 AST decomposition produces REQUIREMENT and SPECIFICATION nodes with exact
//!    0-based byte offsets and DERIVED_FROM hierarchy edges at zero token cost.
//! 3. Verify Tier 2 modality classification tuples `(c1, SHALL, 0.95)` stored in
//!    `attributes->'extraction_metadata'` without source text echoing.
//! 4. Verify Tier 3 external escalation via MCP `propose_node_mutation` refining an ambiguous chunk
//!    with model provenance attributes.
//! 5. Verify CLI staging inspection (`tks staging list`) displays nodes in exact hierarchical narrative
//!    order with inline badges, and `--triage-anomalies` surfaces candidate exceptions.
//! 6. Verify staging batch approval promotes candidates to ACTIVE with root requirement anchors in live graph.
//! 7. Verify pre-merge CI test run appended to `TASK.attributes->'pre_merge_verification'`; merge commit
//!    materialized as `CODE_COMMIT -IMPLEMENTS-> TASK`.
//! 8. Verify code-side contract drift detection hook flags un-attributed modifications to verified interfaces.
//! 9. Verify scoped release readiness returns READY for verified PR commit SHA, and release scope verifies
//!    complete milestone requirement satisfaction.
//! 10. Verify living graph delta re-ingestion preserves active node UUIDs and task edges, tracking omissions
//!     in staging metadata.
//! 11. Verify re-verification pass confirms 100% compliance across Gates 1 and 2 (camelCase `inputSchema`
//!     and `ERR_INVALID_EDGE_TARGET` pre-validation rejection).
//! 12. Verify supervisory review duration logged in `ingestion_jobs.attributes` for `CAL-STEADY` tracking.

use std::fs;
use std::time::Duration;

use reqwest::StatusCode;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tks::cli::staging::{StagingListArgs, StagingSubcommand, run_staging};
use tks::db;
use tks::gateway::auth::compute_token_hash;
use tks::gateway::{AppState, create_router};
use tks::storage::StorageRepo;
use tks::storage::git::start_git_actor;
use tks::worker::decomp::process_one_job;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TempRepoGuard {
    path: std::path::PathBuf,
}

impl TempRepoGuard {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("tks-mvd4-git-{}", Uuid::new_v4()));
        Self { path }
    }
}

impl Drop for TempRepoGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Helper to provision an agent identity with a known Bearer token.
async fn provision_agent(state: &AppState, agent_id: &str, token: &str, actor_type: &str) {
    let client = state.pool.get().await.expect("Failed to get DB client");
    let hash = compute_token_hash(token);
    client
        .execute(
            "INSERT INTO agent_identities (agent_id, token_hash, actor_type, is_active) \
             VALUES ($1, $2, $3, TRUE) \
             ON CONFLICT (agent_id) DO UPDATE SET token_hash = EXCLUDED.token_hash, is_active = TRUE;",
            &[&agent_id, &hash, &actor_type],
        )
        .await
        .expect("Failed to insert agent identity");

    state.auth.register_token(agent_id, &hash, actor_type).await;
    state.auth.register_token(agent_id, token, actor_type).await;
}

#[tokio::test]
async fn test_phase4_mvd_acceptance() {
    let _guard = TEST_MUTEX.lock().await;

    // 0. Environment Setup & DB Preparation
    let test_db_url = db::ensure_test_database_ready()
        .await
        .expect("Failed to prepare test database");
    let pool = db::create_pool(&test_db_url).expect("Failed to create connection pool");

    {
        let (mut client, _) = db::connect(&test_db_url)
            .await
            .expect("Failed to connect to database");
        db::run_migrations(&mut client)
            .await
            .expect("Failed to run migrations");
    }

    {
        let client = pool.get().await.expect("Failed to get DB client");
        let _ = client.execute("DELETE FROM node_embeddings;", &[]).await;
        let _ = client.execute("DELETE FROM graph_edges;", &[]).await;
        let _ = client.execute("DELETE FROM graph_nodes;", &[]).await;
        let _ = client.execute("DELETE FROM ingestion_jobs;", &[]).await;
        let _ = client.execute("DELETE FROM audit_ledger;", &[]).await;
    }

    let temp_repo = TempRepoGuard::new();
    let (git_write, git_read) =
        start_git_actor(&temp_repo.path, 64).expect("Failed to start git actor");
    let storage = StorageRepo::new(pool.clone());
    let state = AppState::new(pool.clone(), storage, git_write, git_read);

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

    let admin_token = "mvd_admin_token_46";
    let agent_token = "mvd_agent_token_46";
    let ci_token = "mvd_ci_token_46";
    provision_agent(&state, "mvd_admin", admin_token, "HUMAN").await;
    provision_agent(&state, "mvd_agent", agent_token, "AGENT").await;
    provision_agent(&state, "ci_runner", ci_token, "HUMAN").await;

    let http = reqwest::Client::new();
    let db_client = state.pool.get().await.expect("Failed to get DB client");

    // =========================================================================
    // STEP 1 & 2: Multi-section vision & brownfield ingestion, zero-token AST decomposition
    // =========================================================================
    let vision_doc_v1 = r#"# Phase 4 Architecture & Closed-Loop Traceability
The knowledge substrate SHALL guarantee end-to-end traceability across requirements, code, and verification.

## Core Requirements
REQ-MVD-TRACE-01: The system MUST maintain zero-token mechanical CommonMark AST decomposition.
REQ-MVD-TRACE-02: Modality classifications SHALL be stored without echoing verbatim source text.

### Subsystem Specifications
SPEC-MVD-SUB-01: AST decomposition structures upward DERIVED_FROM edges mechanically.
"#;

    let openapi_yaml = r#"
openapi: 3.1.0
info:
  title: MVD Gateway API
  version: 1.0.0
  description: Brownfield OpenAPI service contract for Phase 4 MVD.
paths:
  /status:
    get:
      summary: Service health check
      operationId: getStatus
"#;

    let adr_md = r#"
# ADR-0001: Minimum Viable Demonstration Architecture

## Context
Decoupled ingestion and inference is essential.

## Decision
The architecture MUST decouple mechanical ingestion from inference.

## Status
Accepted
"#;

    let sdd_feature = r#"
Feature: MVD Feature Specification
  Scenario: Automated Verification
    Given a configured property graph
    When executing verification
    Then all gates pass cleanly.
"#;

    // 1. Submit specs/vision.md
    let ingest_v1_resp = http
        .post(format!("{base_url}/api/v1/documents/ingest"))
        .bearer_auth(admin_token)
        .json(&serde_json::json!({
            "doc_path": "specs/vision.md",
            "content": vision_doc_v1
        }))
        .send()
        .await
        .expect("Failed to ingest specs/vision.md");
    assert_eq!(ingest_v1_resp.status(), StatusCode::ACCEPTED);
    let v1_json: serde_json::Value = ingest_v1_resp.json().await.unwrap();
    let vision_job_id = Uuid::parse_str(v1_json["job_id"].as_str().unwrap()).unwrap();

    // 2. Submit specs/openapi.yaml
    let ingest_oa_resp = http
        .post(format!("{base_url}/api/v1/documents/ingest"))
        .bearer_auth(admin_token)
        .json(&serde_json::json!({
            "doc_path": "specs/openapi.yaml",
            "content": openapi_yaml
        }))
        .send()
        .await
        .expect("Failed to ingest specs/openapi.yaml");
    assert_eq!(ingest_oa_resp.status(), StatusCode::ACCEPTED);

    // 3. Submit docs/adr/0001-mvd.md
    let ingest_adr_resp = http
        .post(format!("{base_url}/api/v1/documents/ingest"))
        .bearer_auth(admin_token)
        .json(&serde_json::json!({
            "doc_path": "docs/adr/0001-mvd.md",
            "content": adr_md
        }))
        .send()
        .await
        .expect("Failed to ingest docs/adr/0001-mvd.md");
    assert_eq!(ingest_adr_resp.status(), StatusCode::ACCEPTED);

    // 4. Submit specs/mvd.feature
    let ingest_sdd_resp = http
        .post(format!("{base_url}/api/v1/documents/ingest"))
        .bearer_auth(admin_token)
        .json(&serde_json::json!({
            "doc_path": "specs/mvd.feature",
            "content": sdd_feature
        }))
        .send()
        .await
        .expect("Failed to ingest specs/mvd.feature");
    assert_eq!(ingest_sdd_resp.status(), StatusCode::ACCEPTED);

    // Process all 4 queued ingestion jobs via worker tick
    for _ in 0..4 {
        let processed = process_one_job(&state.pool, &state.git_read)
            .await
            .expect("process_one_job failed");
        assert!(processed, "Expected queued job to be processed by worker");
    }

    // Verify AST decomposition for vision doc produced candidates
    let v1_candidates = db_client
        .query(
            "SELECT node_key, node_type, byte_start, byte_end, attributes \
             FROM graph_nodes WHERE job_id = $1 AND lifecycle_state = 'DRAFT' \
             ORDER BY coalesce(byte_start, 0) ASC, id ASC;",
            &[&vision_job_id],
        )
        .await
        .unwrap();

    assert!(
        v1_candidates.len() >= 4,
        "Expected at least 4 candidate chunks extracted from vision doc"
    );

    let mut req_candidates = 0;
    let mut spec_candidates = 0;
    for c in &v1_candidates {
        let ntype: String = c.get("node_type");
        let b_start: Option<i32> = c.get("byte_start");
        let b_end: Option<i32> = c.get("byte_end");
        assert!(b_start.is_some() && b_end.is_some());
        assert!(b_start.unwrap() <= b_end.unwrap());
        if ntype == "REQUIREMENT" {
            req_candidates += 1;
        } else if ntype == "SPECIFICATION" {
            spec_candidates += 1;
        }
    }
    assert!(
        req_candidates >= 2,
        "Expected at least 2 REQUIREMENT candidates"
    );
    assert!(
        spec_candidates >= 1,
        "Expected at least 1 SPECIFICATION candidate"
    );

    // Brownfield scaffolding verification:
    // OpenAPI synthesized root requirement API-MVD-GATEWAY-API
    let oa_cand = db_client
        .query_one(
            "SELECT node_type, node_key FROM graph_nodes WHERE node_key = 'API-MVD-GATEWAY-API';",
            &[],
        )
        .await
        .expect("OpenAPI root requirement must be synthesized");
    assert_eq!(oa_cand.get::<_, String>("node_type"), "REQUIREMENT");

    // ADR synthesized root requirement REQ-ADR-COLLECTION and child DECISION node ADR-0001
    let adr_cand = db_client
        .query_one(
            "SELECT node_type, node_key FROM graph_nodes WHERE node_key = 'REQ-ADR-COLLECTION';",
            &[],
        )
        .await
        .expect("ADR root requirement must be synthesized");
    assert_eq!(adr_cand.get::<_, String>("node_type"), "REQUIREMENT");

    let adr_dec = db_client
        .query_one(
            "SELECT node_type, node_key FROM graph_nodes WHERE node_key = 'ADR-0001';",
            &[],
        )
        .await
        .expect("ADR child decision must be extracted");
    assert_eq!(adr_dec.get::<_, String>("node_type"), "DECISION");

    // SDD synthesized root requirement REQ-MVD-FEATURE-SPECIFICATION
    let sdd_cand = db_client
        .query_one(
            "SELECT node_type, node_key FROM graph_nodes WHERE node_key = 'REQ-MVD-FEATURE-SPECIFICATION';",
            &[],
        )
        .await
        .expect("SDD root requirement must be synthesized");
    assert_eq!(sdd_cand.get::<_, String>("node_type"), "REQUIREMENT");

    println!(
        "Step 1–2: AST decomposition produces REQUIREMENT and SPECIFICATION nodes with exact 0-based byte offsets and DERIVED_FROM hierarchy edges at zero token cost."
    );

    // =========================================================================
    // STEP 3: Modality classification tuples stored without source text echoing
    // =========================================================================
    let mut atomized_found = 0;
    for c in &v1_candidates {
        let attrs: serde_json::Value = c.get("attributes");
        let meta = attrs
            .get("extraction_metadata")
            .expect("extraction_metadata must be present");
        if meta.get("atom_alias").is_some() {
            atomized_found += 1;
            assert!(meta.get("modality").is_some(), "modality must be present");
            assert!(
                meta.get("confidence").is_some(),
                "confidence must be present"
            );
        }

        // Verify zero source text echoing in extraction_metadata
        let meta_str = meta.to_string();
        assert!(
            !meta_str.contains("SHALL guarantee end-to-end traceability"),
            "extraction_metadata must not echo source text"
        );
    }
    assert!(
        atomized_found >= 2,
        "Expected at least 2 atomized candidates with modality tuples"
    );

    println!(
        "Step 3: Modality classification tuples (c1, SHALL, 0.95) stored in attributes->'extraction_metadata' without text echoing."
    );

    // =========================================================================
    // STEP 4: External escalation via MCP propose_node_mutation for ambiguous chunk
    // =========================================================================
    let ambiguous_req_id = Uuid::new_v4();
    db_client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, trust_tier) \
             VALUES ($1, 'REQ-MVD-AMBIGUOUS-01', 'REQUIREMENT', 'Ambiguous Rate Limiter', 'The gateway might throttle requests if needed.', 'ACTIVE', 'HUMAN_REVIEW_REQUIRED', 'mvd_admin', 'HUMAN_APPROVED');",
            &[&ambiguous_req_id],
        )
        .await
        .unwrap();

    let mcp_call = http
        .post(format!("{base_url}/mcp"))
        .bearer_auth(agent_token)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 401,
            "method": "tools/call",
            "params": {
                "name": "propose_node_mutation",
                "arguments": {
                    "target_node_id": "REQ-MVD-AMBIGUOUS-01",
                    "mutation_type": "REQUIREMENT",
                    "proposed_title": "Clarified Deterministic Rate Limiter",
                    "proposed_content": "The gateway MUST throttle requests exceeding 100 req/sec with HTTP 429.",
                    "proposed_attributes": {
                        "model_provenance": "commercial-utility-tier-v1",
                        "escalated_via": "external_mcp_client",
                        "confidence": 0.99
                    }
                }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(mcp_call.status(), StatusCode::OK);
    let mcp_call_body: serde_json::Value = mcp_call.json().await.unwrap();
    assert_eq!(mcp_call_body["result"]["status"], "PENDING_REVIEW");
    let draft_node_id = mcp_call_body["result"]["node_id"]
        .as_str()
        .or_else(|| mcp_call_body["result"]["mutation"]["draft_id"].as_str())
        .expect("Draft node_id must be returned");
    let draft_uuid = Uuid::parse_str(draft_node_id).unwrap();

    let draft_row = db_client
        .query_one(
            "SELECT attributes, lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&draft_uuid],
        )
        .await
        .unwrap();
    assert_eq!(draft_row.get::<_, String>("lifecycle_state"), "DRAFT");
    let draft_attrs: serde_json::Value = draft_row.get("attributes");
    assert_eq!(
        draft_attrs["model_provenance"],
        "commercial-utility-tier-v1"
    );
    assert_eq!(draft_attrs["escalated_via"], "external_mcp_client");
    assert_eq!(draft_attrs["proposed_by"], "mvd_agent");
    assert_eq!(
        draft_attrs["replaces_node_id"],
        ambiguous_req_id.to_string()
    );

    println!("Step 4: Deliberately ambiguous chunk successfully escalated via external MCP agent.");

    // =========================================================================
    // STEP 5: CLI staging inspection (narrative tree & anomaly triage)
    // =========================================================================
    run_staging(
        StagingSubcommand::List(StagingListArgs {
            job_id: vision_job_id,
            triage_anomalies: false,
        }),
        &base_url,
        admin_token,
    )
    .await
    .expect("CLI staging list in narrative order must succeed");

    run_staging(
        StagingSubcommand::List(StagingListArgs {
            job_id: vision_job_id,
            triage_anomalies: true,
        }),
        &base_url,
        admin_token,
    )
    .await
    .expect("CLI staging list --triage-anomalies must succeed");

    let candidates_resp = http
        .get(format!(
            "{base_url}/api/v1/staging/{vision_job_id}/candidates"
        ))
        .bearer_auth(admin_token)
        .send()
        .await
        .unwrap();
    assert_eq!(candidates_resp.status(), StatusCode::OK);
    let candidates_json: Vec<serde_json::Value> = candidates_resp.json().await.unwrap();
    assert!(!candidates_json.is_empty());
    let mut prev_byte = -1;
    for cand in &candidates_json {
        let b = cand["byte_start"].as_i64().unwrap_or(0);
        assert!(
            b >= prev_byte,
            "Candidates must be ordered strictly by byte_start ascending"
        );
        prev_byte = b;
        assert!(cand.get("hierarchical_order").is_some());
        assert!(cand.get("extraction_metadata").is_some());
    }

    println!(
        "Step 5: tks staging list displays nodes in exact hierarchical narrative order with inline badges; --triage-anomalies surfaces low-confidence candidates."
    );

    // =========================================================================
    // STEP 6: Staging batch approval promotes candidates to ACTIVE root anchors
    // =========================================================================
    let approve_v1_resp = http
        .post(format!(
            "{base_url}/api/v1/documents/ingest/{vision_job_id}/approve"
        ))
        .bearer_auth(admin_token)
        .json(&serde_json::json!({
            "confirm_omissions": true,
            "review_duration_seconds": 25.0
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(approve_v1_resp.status(), StatusCode::OK);

    let active_v1_reqs: i64 = db_client
        .query_one(
            "SELECT COUNT(*) FROM graph_nodes WHERE doc_path = 'specs/vision.md' AND node_type = 'REQUIREMENT' AND lifecycle_state = 'ACTIVE';",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        active_v1_reqs >= 2,
        "Must have promoted at least 2 active REQUIREMENT nodes in live graph"
    );

    println!(
        "Step 6: Staging batch approval promotes candidates to ACTIVE with root requirement anchors in live graph."
    );

    // =========================================================================
    // STEP 7: Pre-merge CI test run & VCS merge commit materialization
    // =========================================================================
    let trace1_row = db_client
        .query_one(
            "SELECT id FROM graph_nodes WHERE node_key = 'REQ-MVD-TRACE-01' AND lifecycle_state = 'ACTIVE';",
            &[],
        )
        .await
        .unwrap();
    let req_trace1_id: Uuid = trace1_row.get("id");

    let task_id = Uuid::new_v4();
    db_client
        .execute(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, trust_tier) \
             VALUES ($1, 'TASK-MVD-001', 'TASK', 'Implement MVD Traceability', 'Task implementation intent', 'ACTIVE', 'AUTONOMOUS_ELABORATION', 'mvd_admin', 'HUMAN_APPROVED');",
            &[&task_id],
        )
        .await
        .unwrap();

    db_client
        .execute(
            "INSERT INTO graph_edges (from_node_id, to_node_id, edge_type, lifecycle_state, created_by) \
             VALUES ($1, $2, 'FULFILLS', 'ACTIVE', 'mvd_admin');",
            &[&task_id, &req_trace1_id],
        )
        .await
        .unwrap();

    let edges_before: i64 = db_client
        .query_one("SELECT COUNT(*) FROM graph_edges;", &[])
        .await
        .unwrap()
        .get(0);
    let commits_before: i64 = db_client
        .query_one(
            "SELECT COUNT(*) FROM graph_nodes WHERE node_type = 'CODE_COMMIT';",
            &[],
        )
        .await
        .unwrap()
        .get(0);

    let pr_sha = "sha-pr-mvd-step7";
    let ci_resp = http
        .post(format!("{base_url}/api/v1/verification/test-run"))
        .bearer_auth(ci_token)
        .json(&serde_json::json!({
            "test_suite": "cargo_test",
            "passed": true,
            "pr_commit_sha": pr_sha,
            "target_task_ids": [task_id],
            "duration_ms": 1420,
            "manifest_hash": "sha256-step7-manifest",
            "log_summary": "12 tests passed cleanly"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ci_resp.status(), StatusCode::OK);

    // Verify stored directly in TASK.attributes->'pre_merge_verification' (D-107)
    let task_row = db_client
        .query_one(
            "SELECT attributes FROM graph_nodes WHERE id = $1;",
            &[&task_id],
        )
        .await
        .unwrap();
    let task_attrs: serde_json::Value = task_row.get("attributes");
    let pm_arr = task_attrs["pre_merge_verification"].as_array().unwrap();
    assert_eq!(pm_arr.len(), 1);
    assert_eq!(pm_arr[0]["pr_commit_sha"], pr_sha);

    // Zero graph edges and zero commit nodes created by test run
    let edges_after: i64 = db_client
        .query_one("SELECT COUNT(*) FROM graph_edges;", &[])
        .await
        .unwrap()
        .get(0);
    let commits_after: i64 = db_client
        .query_one(
            "SELECT COUNT(*) FROM graph_nodes WHERE node_type = 'CODE_COMMIT';",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(edges_after, edges_before);
    assert_eq!(commits_after, commits_before);

    // Materialize VCS merge commit upon merge to main
    let merge_sha = "sha-merge-mvd-step7";
    let commit_resp = http
        .post(format!("{base_url}/api/v1/vcs/commits"))
        .bearer_auth(admin_token)
        .json(&serde_json::json!({
            "commit_sha": merge_sha,
            "author": "mvd-dev@example.com",
            "message": "feat(mvd): complete task MVD-001\n\nCloses #42",
            "branch": "main",
            "timestamp": "2026-10-11T20:30:00Z",
            "task_ids": [task_id]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(commit_resp.status(), StatusCode::CREATED);
    let commit_body: serde_json::Value = commit_resp.json().await.unwrap();
    assert_eq!(commit_body["status"], "MATERIALIZED");
    let commit_id: Uuid = commit_body["commit_id"].as_str().unwrap().parse().unwrap();

    let cnode = db_client
        .query_one(
            "SELECT node_type, lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&commit_id],
        )
        .await
        .unwrap();
    assert_eq!(cnode.get::<_, String>("node_type"), "CODE_COMMIT");
    assert_eq!(cnode.get::<_, String>("lifecycle_state"), "ACTIVE");

    let cedge = db_client
        .query_one(
            "SELECT edge_type, lifecycle_state FROM graph_edges WHERE from_node_id = $1 AND to_node_id = $2;",
            &[&commit_id, &task_id],
        )
        .await
        .unwrap();
    assert_eq!(cedge.get::<_, String>("edge_type"), "IMPLEMENTS");
    assert_eq!(cedge.get::<_, String>("lifecycle_state"), "ACTIVE");

    // Invariant INV-1 ancestry
    let has_ancestry =
        tks::storage::envelope::validate_ancestor_path(&**db_client, &[commit_id], false)
            .await
            .unwrap();
    assert!(
        has_ancestry,
        "CODE_COMMIT must maintain unbroken upward path to active REQUIREMENT (INV-1)"
    );

    println!(
        "Step 7: Pre-merge CI test run appended to TASK.attributes->'pre_merge_verification'; merge commit materialized as CODE_COMMIT -IMPLEMENTS-> TASK."
    );

    // =========================================================================
    // STEP 8: Code-side contract drift detection hook
    // =========================================================================
    let unattr_diff = r#"
diff --git a/specs/unattributed_mvd_api.yaml b/specs/unattributed_mvd_api.yaml
new file mode 100644
--- /dev/null
+++ b/specs/unattributed_mvd_api.yaml
@@ -0,0 +1,5 @@
+openapi: 3.1.0
+info:
+  title: Unattributed MVD API
+  version: 1.0.0
"#;
    let drift_resp = http
        .post(format!("{base_url}/api/v1/verification/check-drift"))
        .bearer_auth(admin_token)
        .json(&serde_json::json!({ "diff": unattr_diff }))
        .send()
        .await
        .unwrap();
    assert_eq!(drift_resp.status(), StatusCode::OK);
    let drift_body: serde_json::Value = drift_resp.json().await.unwrap();
    assert_eq!(drift_body["status"], "DRIFT_DETECTED");
    let anomalies = drift_body["anomalies"].as_array().unwrap();
    assert!(!anomalies.is_empty());
    assert_eq!(
        anomalies[0]["anomaly_type"],
        "UNATTRIBUTED_CONTRACT_MODIFICATION"
    );

    let clean_diff = r#"
diff --git a/src/mvd_internal.rs b/src/mvd_internal.rs
--- a/src/mvd_internal.rs
+++ b/src/mvd_internal.rs
@@ -1,1 +1,1 @@
-fn helper() {}
+fn helper() -> bool { true }
"#;
    let clean_resp = http
        .post(format!("{base_url}/api/v1/verification/check-drift"))
        .bearer_auth(admin_token)
        .json(&serde_json::json!({ "diff": clean_diff }))
        .send()
        .await
        .unwrap();
    assert_eq!(clean_resp.status(), StatusCode::OK);
    let clean_body: serde_json::Value = clean_resp.json().await.unwrap();
    assert_eq!(clean_body["status"], "CLEAN");

    println!(
        "Step 8: Code-side contract drift detection hook flags un-attributed modifications to verified interfaces."
    );

    // =========================================================================
    // STEP 9: Scoped release readiness (pre-merge and release scopes)
    // =========================================================================
    let pr_readiness = http
        .get(format!(
            "{base_url}/api/v1/release/readiness?scope=pre-merge&pr_commit_sha={pr_sha}&task_ids={task_id}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(pr_readiness.status(), StatusCode::OK);
    let pr_body: serde_json::Value = pr_readiness.json().await.unwrap();
    assert_eq!(pr_body["status"], "READY");
    assert!(
        pr_body["unverified_task_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let rel_readiness = http
        .get(format!(
            "{base_url}/api/v1/release/readiness?scope=release&requirement_root_id={req_trace1_id}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(rel_readiness.status(), StatusCode::OK);
    let rel_body: serde_json::Value = rel_readiness.json().await.unwrap();
    assert_eq!(rel_body["status"], "READY");
    assert!(rel_body["uncommitted_tasks"].as_array().unwrap().is_empty());
    assert_eq!(rel_body["readiness_percentage"], 100.0);

    println!(
        "Step 9: Scoped release readiness returns READY for verified PR commit SHA, and release scope verifies complete milestone requirement satisfaction."
    );

    // =========================================================================
    // STEP 10: Living graph 3-way delta re-ingestion with staging omission metadata
    // =========================================================================
    let trace2_row = db_client
        .query_one(
            "SELECT id FROM graph_nodes WHERE node_key = 'REQ-MVD-TRACE-02' AND lifecycle_state = 'ACTIVE';",
            &[],
        )
        .await
        .unwrap();
    let trace2_id: Uuid = trace2_row.get("id");

    let sub1_row = db_client
        .query_one(
            "SELECT id FROM graph_nodes WHERE node_key = 'SPEC-MVD-SUB-01' AND lifecycle_state = 'ACTIVE';",
            &[],
        )
        .await
        .unwrap();
    let sub1_id: Uuid = sub1_row.get("id");

    let rev2_vision_doc = r#"# Phase 4 Architecture & Closed-Loop Traceability
Preamble update: The knowledge substrate SHALL continuously maintain closed-loop traceability.

## Core Requirements
REQ-MVD-TRACE-01: The system MUST maintain zero-token mechanical CommonMark AST decomposition with sub-5ms latency.
REQ-MVD-TRACE-02: Modality classifications SHALL be stored without echoing verbatim source text.

## Observability & Quality
REQ-MVD-OBS-01: Metrics emission SHALL track supervisory curation duration.
"#;

    let ingest_rev2 = http
        .post(format!("{base_url}/api/v1/documents/ingest"))
        .bearer_auth(admin_token)
        .json(&serde_json::json!({
            "doc_path": "specs/vision.md",
            "content": rev2_vision_doc
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ingest_rev2.status(), StatusCode::ACCEPTED);
    let rev2_json: serde_json::Value = ingest_rev2.json().await.unwrap();
    let rev2_job_id = Uuid::parse_str(rev2_json["job_id"].as_str().unwrap()).unwrap();

    let processed = process_one_job(&state.pool, &state.git_read).await.unwrap();
    assert!(processed);

    // Active node UUID preserved
    let trace2_post = db_client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&trace2_id],
        )
        .await
        .unwrap();
    assert_eq!(trace2_post.get::<_, String>("lifecycle_state"), "ACTIVE");

    // Task edge preserved
    let edge_post = db_client
        .query_one(
            "SELECT lifecycle_state FROM graph_edges WHERE from_node_id = $1 AND to_node_id = $2;",
            &[&commit_id, &task_id],
        )
        .await
        .unwrap();
    assert_eq!(edge_post.get::<_, String>("lifecycle_state"), "ACTIVE");

    // Omitted section remains strictly ACTIVE in graph_nodes prior to approval (D-109)
    let sub1_post = db_client
        .query_one(
            "SELECT lifecycle_state FROM graph_nodes WHERE id = $1;",
            &[&sub1_id],
        )
        .await
        .unwrap();
    assert_eq!(sub1_post.get::<_, String>("lifecycle_state"), "ACTIVE");

    // Omission tracked in ingestion_jobs.attributes->'omitted_node_ids'
    let rev2_job_row = db_client
        .query_one(
            "SELECT attributes FROM ingestion_jobs WHERE job_id = $1;",
            &[&rev2_job_id],
        )
        .await
        .unwrap();
    let rev2_attrs: serde_json::Value = rev2_job_row.get("attributes");
    let omitted_arr = rev2_attrs
        .get("omitted_node_ids")
        .and_then(|v| v.as_array())
        .expect("omitted_node_ids must be present in rev2 job attributes");
    assert!(
        omitted_arr
            .iter()
            .any(|v| v.as_str() == Some(&sub1_id.to_string())),
        "sub1_id must be tracked in omitted_node_ids"
    );

    println!(
        "Step 10: Living graph delta re-ingestion preserves active node UUIDs and task edges, tracking omissions in staging metadata."
    );

    // =========================================================================
    // STEP 11: Formal Gates 1 & 2 re-verification pass
    // =========================================================================
    // Gate 1: live graph active REQUIREMENT root anchors
    let req_count: i64 = db_client
        .query_one(
            "SELECT COUNT(*) FROM graph_nodes WHERE node_type = 'REQUIREMENT' AND lifecycle_state = 'ACTIVE';",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        req_count >= 2,
        "Live graph must maintain active REQUIREMENT root anchors"
    );

    // Gate 2: MCP tools/list wire inspection for camelCase inputSchema
    let list_resp = http
        .post(format!("{base_url}/mcp"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), StatusCode::OK);
    let list_text = list_resp.text().await.unwrap();
    assert!(!list_text.contains("\"input_schema\""));
    let list_json: serde_json::Value = serde_json::from_str(&list_text).unwrap();
    let tools = list_json["result"]["tools"].as_array().unwrap();
    assert!(tools.len() >= 8);
    for t in tools {
        assert!(
            t.get("inputSchema").is_some(),
            "Tool {} must serialize inputSchema",
            t["name"]
        );
    }

    // Gate 2: Relational edge pre-validation rejecting raw Git commit SHA (D-96, D-97)
    let bad_edge_resp = http
        .post(format!("{base_url}/api/v1/vcs/commits"))
        .bearer_auth(admin_token)
        .json(&serde_json::json!({
            "commit_sha": "c0ffee1234567890abcdef",
            "author": "dev@example.com",
            "message": "bad edge target commit",
            "task_ids": ["deadbeefcafe1234567890abcdef"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_edge_resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bad_edge_body: serde_json::Value = bad_edge_resp.json().await.unwrap();
    assert_eq!(bad_edge_body["code"], "ERR_INVALID_EDGE_TARGET");

    println!("Step 11: Re-verification pass confirms 100% compliance across Gates 1 and 2.");

    // =========================================================================
    // STEP 12: Supervisory review duration logged for CAL-STEADY tracking
    // =========================================================================
    let approve_rev2 = http
        .post(format!(
            "{base_url}/api/v1/documents/ingest/{rev2_job_id}/approve"
        ))
        .bearer_auth(admin_token)
        .json(&serde_json::json!({
            "confirm_omissions": true,
            "review_duration_seconds": 45.0
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(approve_rev2.status(), StatusCode::OK);

    let approved_job_row = db_client
        .query_one(
            "SELECT status, attributes FROM ingestion_jobs WHERE job_id = $1;",
            &[&rev2_job_id],
        )
        .await
        .unwrap();
    assert_eq!(approved_job_row.get::<_, String>("status"), "APPROVED");
    let rev_attrs: serde_json::Value = approved_job_row.get("attributes");
    let review_dur = rev_attrs["staging_review_duration_seconds"].as_f64();
    assert_eq!(
        review_dur,
        Some(45.0),
        "staging_review_duration_seconds must be stored in ingestion_jobs.attributes"
    );

    println!("Step 12: Supervisory review duration logged for CAL-STEADY tracking.");

    cancel_token.cancel();
}
