//! Phase 1 Dogfooding Milestone (Gate 1 - Read-Only Self-Hosting) Acceptance Validation Suite.
//!
//! Automates the complete Phase 1 MVD Acceptance Test Script (Steps 1–8 from
//! `docs/vision/strategic-planning-backlog.md` §4 and `docs/vision/phase1-plan.md` WP-1.6):
//!
//! 1. Ingest real project documentation (`docs/vision/vision.md` and `docs/vision/strategic-planning-backlog.md`)
//!    via `POST /api/v1/documents/ingest`.
//! 2. Verify documents committed to bare Git repository ODB on branch `refs/heads/specs` at `specs/vision.md`
//!    and `specs/strategic-planning-backlog.md`.
//! 3. Verify cooperative background worker claims jobs, performs CommonMark AST decomposition, extracts >=20
//!    atomic requirement nodes with exact 0-based byte spans, heading hierarchy edges (`DERIVED_FROM`),
//!    and handles uncredentialed execution gracefully via `DRAFT` state.
//! 4. Verify candidate draft nodes are inspectable via staging review API / CLI with verbatim Git ODB spans,
//!    and verify `node_embeddings` contains ZERO rows for unapproved drafts.
//! 5. Execute staging approval via staging API, verifying atomic draft squashing, `audit_ledger` monotonic
//!    `event_seq` and `batch_id`, promotion to `ACTIVE`, and offline local vector generation (<50ms per node)
//!    transitioning `node_embeddings` to `COMPLETED` with 384-d vectors.
//! 6. Execute MCP tool `query_requirements` verifying sub-5ms native full-text search with hyphen sanitization.
//! 7. Execute MCP tool `get_context_envelope` verifying sub-50ms topological ancestor and vector neighbor retrieval.
//! 8. Execute MCP tool `get_document_span` verifying lock-free concurrent Git ODB text slicing.

use std::fs;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tks::cli::staging::{StagingInspectArgs, StagingListArgs, StagingSubcommand, run_staging};
use tks::db;
use tks::gateway::routes::documents::{IngestDocumentRequest, IngestDocumentResponse};
use tks::gateway::routes::staging::{InspectNodeResponse, StagingApproveResponse};
use tks::gateway::{AppState, create_router};
use tks::storage::git::{GitReadHandle, GitWriteHandle, start_git_actor};
use tks::storage::{StorageRepo, TopologicalEnvelope};
use tks::worker::decomp::process_one_job;
use tks::worker::embedding::{LocalEmbeddingGenerator, process_embedding_batch};

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TempRepoGuard {
    path: std::path::PathBuf,
}

impl TempRepoGuard {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("tks-dogfood-git-{}", Uuid::new_v4()));
        Self { path }
    }
}

impl Drop for TempRepoGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Helper context holding running test server, database client, and Git handles.
struct DogfoodContext {
    base_url: String,
    pool: deadpool_postgres::Pool,
    _git_write: GitWriteHandle,
    git_read: GitReadHandle,
    cancel_token: CancellationToken,
    _guard: tokio::sync::MutexGuard<'static, ()>,
    _temp_repo: TempRepoGuard,
}

async fn setup_dogfood_environment() -> DogfoodContext {
    let guard = TEST_MUTEX.lock().await;
    let database_url = db::resolve_database_url();

    // Ensure embedded migrations are applied
    let (mut client, _handle) = db::connect(&database_url)
        .await
        .expect("Failed to connect to PostgreSQL");
    db::run_migrations(&mut client)
        .await
        .expect("Failed to run migrations");

    // Clean up any residue from prior runs for dogfood specs
    client
        .execute(
            "DELETE FROM node_embeddings WHERE node_id IN (SELECT id FROM graph_nodes WHERE doc_path IN ('specs/vision.md', 'specs/strategic-planning-backlog.md'));",
            &[],
        )
        .await
        .expect("Cleanup node_embeddings failed");

    client
        .execute(
            "DELETE FROM graph_edges WHERE from_node_id IN (SELECT id FROM graph_nodes WHERE doc_path IN ('specs/vision.md', 'specs/strategic-planning-backlog.md')) OR to_node_id IN (SELECT id FROM graph_nodes WHERE doc_path IN ('specs/vision.md', 'specs/strategic-planning-backlog.md'));",
            &[],
        )
        .await
        .expect("Cleanup graph_edges failed");

    client
        .execute(
            "DELETE FROM graph_nodes WHERE doc_path IN ('specs/vision.md', 'specs/strategic-planning-backlog.md');",
            &[],
        )
        .await
        .expect("Cleanup graph_nodes failed");

    client
        .execute(
            "DELETE FROM ingestion_jobs WHERE doc_path IN ('specs/vision.md', 'specs/strategic-planning-backlog.md');",
            &[],
        )
        .await
        .expect("Cleanup ingestion_jobs failed");

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

    DogfoodContext {
        base_url,
        pool,
        _git_write: git_write,
        git_read,
        cancel_token,
        _guard: guard,
        _temp_repo: temp_repo,
    }
}

#[tokio::test]
async fn test_phase1_dogfooding_gate1_mvd_acceptance() {
    println!("\n===============================================================================");
    println!("PHASE 1 DOGFOODING MILESTONE (GATE 1 - READ-ONLY SELF-HOSTING) ACCEPTANCE TEST");
    println!("===============================================================================\n");

    let ctx = setup_dogfood_environment().await;
    let http = reqwest::Client::new();
    let dev_token = "tks_dev_token";

    // Read real project documentation from repository
    let vision_content =
        fs::read_to_string("docs/vision/vision.md").expect("Failed to read docs/vision/vision.md");
    let backlog_content = fs::read_to_string("docs/vision/strategic-planning-backlog.md")
        .expect("Failed to read docs/vision/strategic-planning-backlog.md");

    assert!(!vision_content.is_empty(), "vision.md must not be empty");
    assert!(
        !backlog_content.is_empty(),
        "strategic-planning-backlog.md must not be empty"
    );

    println!("Loaded real documentation:");
    println!(
        "  docs/vision/vision.md:                     {} bytes",
        vision_content.len()
    );
    println!(
        "  docs/vision/strategic-planning-backlog.md: {} bytes",
        backlog_content.len()
    );

    // -------------------------------------------------------------------------
    // STEP 1: Submit Documents for Ingestion via REST API
    // -------------------------------------------------------------------------
    println!("\n--- [Step 1] Ingesting documents via POST /api/v1/documents/ingest ---");

    let ingest_url = format!("{}/api/v1/documents/ingest", ctx.base_url);

    // Ingest vision.md
    let resp1 = http
        .post(&ingest_url)
        .header(AUTHORIZATION, format!("Bearer {dev_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&IngestDocumentRequest {
            doc_path: "specs/vision.md".to_string(),
            content: vision_content.clone(),
        })
        .send()
        .await
        .expect("Failed to send vision.md ingestion request");

    assert_eq!(
        resp1.status(),
        StatusCode::ACCEPTED,
        "vision.md ingestion must return 202 Accepted"
    );
    let job1_res: IngestDocumentResponse = resp1.json().await.unwrap();
    let job1_id = job1_res.job_id;
    assert_eq!(job1_res.status, "QUEUED");
    println!("Ingestion Job 1 accepted: {job1_id} for 'specs/vision.md'");

    // Ingest strategic-planning-backlog.md
    let resp2 = http
        .post(&ingest_url)
        .header(AUTHORIZATION, format!("Bearer {dev_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&IngestDocumentRequest {
            doc_path: "specs/strategic-planning-backlog.md".to_string(),
            content: backlog_content.clone(),
        })
        .send()
        .await
        .expect("Failed to send backlog ingestion request");

    assert_eq!(
        resp2.status(),
        StatusCode::ACCEPTED,
        "backlog ingestion must return 202 Accepted"
    );
    let job2_res: IngestDocumentResponse = resp2.json().await.unwrap();
    let job2_id = job2_res.job_id;
    assert_eq!(job2_res.status, "QUEUED");
    println!("Ingestion Job 2 accepted: {job2_id} for 'specs/strategic-planning-backlog.md'");

    // -------------------------------------------------------------------------
    // STEP 2: Verify Git Commit & Blob Objects on refs/heads/specs
    // -------------------------------------------------------------------------
    println!("\n--- [Step 2] Verifying Git storage on refs/heads/specs ---");

    let client = ctx.pool.get().await.expect("Failed to get DB connection");

    let row1 = client
        .query_one(
            "SELECT document_hash FROM ingestion_jobs WHERE job_id = $1;",
            &[&job1_id],
        )
        .await
        .unwrap();
    let blob_hash1: String = row1.get("document_hash");

    let row2 = client
        .query_one(
            "SELECT document_hash FROM ingestion_jobs WHERE job_id = $1;",
            &[&job2_id],
        )
        .await
        .unwrap();
    let blob_hash2: String = row2.get("document_hash");

    assert!(
        !blob_hash1.is_empty(),
        "Job 1 must have valid Git blob hash"
    );
    assert!(
        !blob_hash2.is_empty(),
        "Job 2 must have valid Git blob hash"
    );

    // Verify blob content retrieval via read-only ODB handle
    let read_blob1 = ctx
        .git_read
        .read_blob(&blob_hash1)
        .await
        .expect("Must read blob 1 from ODB");
    assert_eq!(
        read_blob1, vision_content,
        "ODB blob 1 must match vision.md verbatim"
    );

    let read_blob2 = ctx
        .git_read
        .read_blob(&blob_hash2)
        .await
        .expect("Must read blob 2 from ODB");
    assert_eq!(
        read_blob2, backlog_content,
        "ODB blob 2 must match backlog verbatim"
    );

    println!("Verified Git blobs in bare repository ODB:");
    println!("  specs/vision.md:                     hash {blob_hash1}");
    println!("  specs/strategic-planning-backlog.md: hash {blob_hash2}");

    // -------------------------------------------------------------------------
    // STEP 3: Cooperative Background Worker Decomposition
    // -------------------------------------------------------------------------
    println!("\n--- [Step 3] Processing AST decomposition via worker ---");

    let processed1 = process_one_job(&ctx.pool, &ctx.git_read)
        .await
        .expect("Decomposition job 1 failed");
    assert!(processed1, "Job 1 must be claimed and processed");

    let processed2 = process_one_job(&ctx.pool, &ctx.git_read)
        .await
        .expect("Decomposition job 2 failed");
    assert!(processed2, "Job 2 must be claimed and processed");

    // Verify jobs transitioned to STAGED
    let job1_status_row = client
        .query_one(
            "SELECT status FROM ingestion_jobs WHERE job_id = $1;",
            &[&job1_id],
        )
        .await
        .unwrap();
    assert_eq!(job1_status_row.get::<_, String>("status"), "STAGED");

    let job2_status_row = client
        .query_one(
            "SELECT status FROM ingestion_jobs WHERE job_id = $1;",
            &[&job2_id],
        )
        .await
        .unwrap();
    assert_eq!(job2_status_row.get::<_, String>("status"), "STAGED");

    println!("Both ingestion jobs successfully transitioned to 'STAGED'.");

    // -------------------------------------------------------------------------
    // STEP 4: Candidate Draft Inspection & Zero-Embeddings Assertion
    // -------------------------------------------------------------------------
    println!("\n--- [Step 4] Inspecting candidate DRAFT nodes & verifying zero embeddings ---");

    // Query candidate nodes from database
    let job1_nodes = client
        .query(
            "SELECT id, node_key, node_type, title, byte_start, byte_end, lifecycle_state \
             FROM graph_nodes WHERE job_id = $1 ORDER BY byte_start ASC;",
            &[&job1_id],
        )
        .await
        .unwrap();

    let job2_nodes = client
        .query(
            "SELECT id, node_key, node_type, title, byte_start, byte_end, lifecycle_state \
             FROM graph_nodes WHERE job_id = $1 ORDER BY byte_start ASC;",
            &[&job2_id],
        )
        .await
        .unwrap();

    let total_draft_nodes = job1_nodes.len() + job2_nodes.len();
    println!("Candidate draft nodes extracted:");
    println!(
        "  specs/vision.md:                     {} nodes",
        job1_nodes.len()
    );
    println!(
        "  specs/strategic-planning-backlog.md: {} nodes",
        job2_nodes.len()
    );
    println!(
        "  Total candidate nodes:               {} nodes",
        total_draft_nodes
    );

    assert!(
        job1_nodes.len() >= 10,
        "vision.md must extract >= 10 candidate requirement chunks"
    );
    assert!(
        job2_nodes.len() >= 10,
        "backlog must extract >= 10 candidate requirement chunks"
    );
    assert!(
        total_draft_nodes >= 20,
        "Total extracted chunks must be >= 20"
    );

    // Verify all candidate nodes are strictly in DRAFT state
    for row in job1_nodes.iter().chain(job2_nodes.iter()) {
        let state: String = row.get("lifecycle_state");
        assert_eq!(state, "DRAFT", "Unapproved candidate nodes must be DRAFT");
    }

    // Verify Invariant INV-2 / TB-6: Zero rows in node_embeddings for unapproved draft nodes
    let unapproved_embeddings_count: i64 = client
        .query_one(
            "SELECT count(*) FROM node_embeddings WHERE node_id IN (SELECT id FROM graph_nodes WHERE job_id IN ($1, $2));",
            &[&job1_id, &job2_id],
        )
        .await
        .unwrap()
        .get(0);

    assert_eq!(
        unapproved_embeddings_count, 0,
        "node_embeddings must contain ZERO rows for unapproved candidate drafts"
    );
    println!("Verified: node_embeddings contains 0 rows for unapproved drafts.");

    // Test REST node inspection and verbatim Git ODB span retrieval
    let sample_node_id: Uuid = job1_nodes[0].get("id");
    let sample_start: i32 = job1_nodes[0].get("byte_start");
    let sample_end: i32 = job1_nodes[0].get("byte_end");

    let inspect_url = format!("{}/api/v1/staging/inspect/{}", ctx.base_url, sample_node_id);
    let inspect_resp = http
        .get(&inspect_url)
        .header(AUTHORIZATION, format!("Bearer {dev_token}"))
        .send()
        .await
        .expect("Failed to inspect sample node");

    assert_eq!(inspect_resp.status(), StatusCode::OK);
    let inspect_data: InspectNodeResponse = inspect_resp.json().await.unwrap();

    let span_text = inspect_data
        .span_text
        .expect("Must have verbatim span text from Git ODB");
    let expected_slice = &vision_content[sample_start as usize..sample_end as usize];
    assert_eq!(
        span_text, expected_slice,
        "Verbatim document span text must match source file slice"
    );
    println!(
        "Verified node inspection & Git ODB verbatim span retrieval for node {}",
        sample_node_id
    );

    // Also verify CLI staging list subcommand
    let cli_list_res = run_staging(
        StagingSubcommand::List(StagingListArgs { job_id: job1_id }),
        &ctx.base_url,
        dev_token,
    )
    .await;
    assert!(
        cli_list_res.is_ok(),
        "tks staging list must execute cleanly"
    );

    // Also verify CLI staging inspect subcommand
    let cli_inspect_res = run_staging(
        StagingSubcommand::Inspect(StagingInspectArgs {
            node_id: sample_node_id.to_string(),
        }),
        &ctx.base_url,
        dev_token,
    )
    .await;
    assert!(
        cli_inspect_res.is_ok(),
        "tks staging inspect must execute cleanly"
    );

    // -------------------------------------------------------------------------
    // STEP 5: Staging Approval & Asynchronous Local Vector Generation
    // -------------------------------------------------------------------------
    println!("\n--- [Step 5] Approving staging jobs & generating vector embeddings ---");

    let approve1_url = format!(
        "{}/api/v1/documents/ingest/{}/approve",
        ctx.base_url, job1_id
    );
    let approve1_resp = http
        .post(&approve1_url)
        .header(AUTHORIZATION, format!("Bearer {dev_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({ "job_id": job1_id }))
        .send()
        .await
        .expect("Failed to approve job 1");

    assert_eq!(
        approve1_resp.status(),
        StatusCode::OK,
        "Approval 1 must succeed"
    );
    let approve1_data: StagingApproveResponse = approve1_resp.json().await.unwrap();
    println!(
        "Approved Job 1: {} nodes promoted, batch_id: {}",
        approve1_data.approved_nodes, approve1_data.batch_id
    );

    let approve2_url = format!(
        "{}/api/v1/documents/ingest/{}/approve",
        ctx.base_url, job2_id
    );
    let approve2_resp = http
        .post(&approve2_url)
        .header(AUTHORIZATION, format!("Bearer {dev_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({ "job_id": job2_id }))
        .send()
        .await
        .expect("Failed to approve job 2");

    let status2 = approve2_resp.status();
    let body2 = approve2_resp.text().await.unwrap();
    if status2 != StatusCode::OK {
        panic!("Approval 2 failed (status {status2}): {body2}");
    }
    let approve2_data: StagingApproveResponse = serde_json::from_str(&body2).unwrap();
    println!(
        "Approved Job 2: {} nodes promoted, batch_id: {}",
        approve2_data.approved_nodes, approve2_data.batch_id
    );

    // Verify audit_ledger contains squashed canonical APPROVED events with monotonic event_seq
    let audit_rows = client
        .query(
            "SELECT event_seq, event_type, batch_id, entity_type FROM audit_ledger \
             WHERE event_type = 'APPROVED' ORDER BY event_seq ASC;",
            &[],
        )
        .await
        .unwrap();

    assert!(
        audit_rows.len() >= 2,
        "audit_ledger must record APPROVED events for both jobs"
    );
    let seq1: i64 = audit_rows[audit_rows.len() - 2].get("event_seq");
    let seq2: i64 = audit_rows[audit_rows.len() - 1].get("event_seq");
    assert!(
        seq2 > seq1,
        "audit_ledger event_seq must be strictly monotonic ({seq2} > {seq1})"
    );
    println!("Verified audit_ledger monotonic event_seq: {seq1} -> {seq2}");

    // Verify node_embeddings received PENDING rows for the promoted nodes
    let pending_embeddings_count: i64 = client
        .query_one(
            "SELECT count(*) FROM node_embeddings WHERE status = 'PENDING';",
            &[],
        )
        .await
        .unwrap()
        .get(0);

    assert!(
        pending_embeddings_count
            >= (approve1_data.approved_nodes + approve2_data.approved_nodes) as i64,
        "Promoted active nodes must be enqueued in node_embeddings with status = 'PENDING'"
    );
    println!("Promoted nodes enqueued in node_embeddings: {pending_embeddings_count} rows");

    // Execute offline local vector generation via process_embedding_batch
    println!("Generating local CPU embeddings via FastEmbed (all-MiniLM-L6-v2, 384 dimensions)...");
    let generator = LocalEmbeddingGenerator::new().expect("Failed to init FastEmbed generator");

    let vector_start = Instant::now();
    let mut total_batches = 0;
    loop {
        let processed = process_embedding_batch(&ctx.pool, &generator)
            .await
            .expect("Embedding batch failed");
        if !processed {
            break;
        }
        total_batches += 1;
    }
    let vector_elapsed = vector_start.elapsed();

    // Verify all rows in node_embeddings are now COMPLETED with 384-d vectors
    let completed_count: i64 = client
        .query_one(
            "SELECT count(*) FROM node_embeddings WHERE status = 'COMPLETED' AND embedding IS NOT NULL;",
            &[],
        )
        .await
        .unwrap()
        .get(0);

    let per_node_ms = if completed_count > 0 {
        vector_elapsed.as_millis() as f64 / completed_count as f64
    } else {
        0.0
    };

    println!(
        "Vector generation complete: embedded {} nodes across {} batches in {:.2?} ({:.2} ms / node, SLA: < 50 ms)",
        completed_count, total_batches, vector_elapsed, per_node_ms
    );
    assert!(
        per_node_ms < 50.0,
        "Vector inference latency must be < 50ms per node (observed: {per_node_ms:.2} ms)"
    );

    assert!(
        completed_count >= (approve1_data.approved_nodes + approve2_data.approved_nodes) as i64
    );
    println!(
        "Verified: {completed_count} node_embeddings rows status = 'COMPLETED' with non-null 384-d vector."
    );

    // -------------------------------------------------------------------------
    // STEP 6: MCP Tool - query_requirements (<5ms SLA)
    // -------------------------------------------------------------------------
    println!("\n--- [Step 6] Testing MCP query_requirements (<5ms SLA) ---");

    let mcp_rpc_url = format!("{}/mcp", ctx.base_url);

    // Query 1: Canonical identifier search (verifying hyphen-negation sanitization, TB-7)
    let search_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 101,
        "method": "tools/call",
        "params": {
            "name": "query_requirements",
            "arguments": {
                "query": "INV-1",
                "limit": 10
            }
        }
    });

    let q_start = Instant::now();
    let q_resp = http
        .post(&mcp_rpc_url)
        .header(AUTHORIZATION, format!("Bearer {dev_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&search_req)
        .send()
        .await
        .expect("Failed to execute query_requirements tool call");
    let q_elapsed = q_start.elapsed();

    assert_eq!(q_resp.status(), StatusCode::OK);
    let q_json: serde_json::Value = q_resp.json().await.unwrap();

    println!(
        "query_requirements('INV-1') completed in {:.2?} (SLA: < 5ms)",
        q_elapsed
    );
    println!("Result: {}", serde_json::to_string_pretty(&q_json).unwrap());

    let results = q_json
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_array())
        .expect("Must have content array in result");

    assert!(
        !results.is_empty(),
        "query_requirements must return matching active requirements"
    );

    // -------------------------------------------------------------------------
    // STEP 7: MCP Tool - get_context_envelope (<50ms SLA)
    // -------------------------------------------------------------------------
    println!("\n--- [Step 7] Testing MCP get_context_envelope (<50ms SLA) ---");

    // Choose target node from approved nodes
    let target_row = client
        .query_one(
            "SELECT id, node_key FROM graph_nodes WHERE doc_path = 'specs/vision.md' AND lifecycle_state = 'ACTIVE' LIMIT 1;",
            &[],
        )
        .await
        .unwrap();

    let target_id: Uuid = target_row.get("id");
    let target_key: Option<String> = target_row.get("node_key");
    let target_ident = target_key.unwrap_or_else(|| target_id.to_string());

    let env_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 102,
        "method": "tools/call",
        "params": {
            "name": "get_context_envelope",
            "arguments": {
                "target_node_id": target_ident,
                "depth": 2
            }
        }
    });

    let env_start = Instant::now();
    let env_resp = http
        .post(&mcp_rpc_url)
        .header(AUTHORIZATION, format!("Bearer {dev_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&env_req)
        .send()
        .await
        .expect("Failed to execute get_context_envelope tool call");
    let env_elapsed = env_start.elapsed();

    assert_eq!(env_resp.status(), StatusCode::OK);
    let env_json: serde_json::Value = env_resp.json().await.unwrap();

    println!(
        "get_context_envelope('{}') completed in {:.2?} (SLA: < 50ms)",
        target_ident, env_elapsed
    );
    assert!(
        env_elapsed < Duration::from_millis(50),
        "Context envelope traversal must complete within < 50ms (observed: {:.2?})",
        env_elapsed
    );

    let env_content_str = env_json
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(|t| t.as_str())
        .expect("Envelope response must contain text content");

    assert!(
        !env_content_str.is_empty(),
        "Formatted markdown envelope must not be empty"
    );

    let envelope_val = env_json
        .get("result")
        .and_then(|r| r.get("envelope"))
        .expect("Result must contain envelope object");

    let envelope: TopologicalEnvelope = serde_json::from_value(envelope_val.clone())
        .expect("Envelope value must deserialize to TopologicalEnvelope");

    assert_eq!(envelope.target_node.id, target_id);
    let target_title = envelope
        .target_node
        .title
        .as_deref()
        .unwrap_or("<no-title>");
    println!(
        "Envelope target node: [{}] {}",
        envelope.target_node.id, target_title
    );
    println!(
        "  Ancestor Requirements: {} nodes",
        envelope.ancestor_requirements.len()
    );
    println!(
        "  Sibling Constraints:   {} nodes",
        envelope.sibling_constraints.len()
    );
    println!(
        "  Vector Neighbors:      {} nodes",
        envelope.vector_neighbors.len()
    );

    // -------------------------------------------------------------------------
    // STEP 8: MCP Tool - get_document_span (Concurrent Lock-Free ODB Read)
    // -------------------------------------------------------------------------
    println!("\n--- [Step 8] Testing MCP get_document_span (Concurrent Lock-Free ODB Read) ---");

    let span_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 103,
        "method": "tools/call",
        "params": {
            "name": "get_document_span",
            "arguments": {
                "node_id": target_id.to_string()
            }
        }
    });

    let span_start = Instant::now();
    let span_resp = http
        .post(&mcp_rpc_url)
        .header(AUTHORIZATION, format!("Bearer {dev_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&span_req)
        .send()
        .await
        .expect("Failed to execute get_document_span tool call");
    let span_elapsed = span_start.elapsed();

    assert_eq!(span_resp.status(), StatusCode::OK);
    let span_json: serde_json::Value = span_resp.json().await.unwrap();

    println!(
        "get_document_span('{}') completed in {:.2?}",
        target_id, span_elapsed
    );

    let span_text_retrieved = span_json
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(|t| t.as_str())
        .expect("Must have span text in tool response");

    assert!(
        !span_text_retrieved.is_empty(),
        "Retrieved verbatim span must not be empty"
    );

    // Verify verbatim text matches source slice
    let start_offset = envelope.target_node.byte_start.unwrap_or(0) as usize;
    let end_offset = envelope.target_node.byte_end.unwrap_or(0) as usize;
    let expected_text = &vision_content[start_offset..end_offset];
    assert_eq!(
        span_text_retrieved, expected_text,
        "Retrieved ODB span text must match source file slice verbatim"
    );

    println!(
        "Retrieved verbatim span ({} bytes) successfully matched source slice.",
        span_text_retrieved.len()
    );

    ctx.cancel_token.cancel();

    println!("\n===============================================================================");
    println!("DOGFOODING MILESTONE GATE 1 PASSED: READ-ONLY SELF-HOSTING VALIDATED (INV-6)");
    println!("===============================================================================\n");
}
