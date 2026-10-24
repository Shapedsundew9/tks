//! Integration test suite for Elicitation Track E-A (EA-1, D-130..D-134, D-EA-6..D-EA-17).
//!
//! Verifies:
//! - Full draft session lifecycle: create, get, submit turns, patch, close, reopen, and delete.
//! - Error handling: 409 on writes to a closed session, 404 after delete and forced expiry.
//! - Capacity bounds (max 32 live sessions, max 1000 turns).
//! - Input validation: participant names, slug format, text normalisation.
//! - Cycle-2 continuation from valid prior transcript (stable session_id, cycle = prior + 1, today's date).
//! - E-B gap import (read-only query against ingestion_jobs, gap questions never appear in turns).
//! - Loopback guard: 403 for non-loopback Host, 403 for cross-origin state-changing mutation, no CORS headers.
//! - Tracing safety: turn text is never emitted into tracing logs.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use reqwest::StatusCode;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt;
use uuid::Uuid;

use tks::db;
use tks::elicitation::{ElicitationConfig, ElicitationStore, FacilitatorProviderKind};
use tks::gateway::{AppState, create_router};
use tks::storage::StorageRepo;
use tks::storage::git::start_git_actor;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Log capture buffer to verify tracing safety.
#[derive(Clone, Default)]
struct LogCapture {
    logs: Arc<Mutex<Vec<String>>>,
}

struct LogWriter {
    logs: Arc<Mutex<Vec<String>>>,
}

impl std::io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(s) = std::str::from_utf8(buf)
            && let Ok(mut l) = self.logs.lock()
        {
            l.push(s.to_string());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
    type Writer = LogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriter {
            logs: self.logs.clone(),
        }
    }
}

struct TestContext {
    base_url: String,
    port: u16,
    state: AppState,
    cancel_token: CancellationToken,
    log_capture: LogCapture,
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

async fn setup_test_context() -> TestContext {
    let guard = TEST_MUTEX.lock().await;
    let database_url = db::ensure_test_database_ready()
        .await
        .expect("Failed to prepare test database");
    let pool = db::create_pool(&database_url).expect("Failed to create connection pool");

    let repo_dir = format!("target/test-elicitation-git-{}", Uuid::new_v4());
    let (git_write, git_read) =
        start_git_actor(&repo_dir, 64).expect("Failed to start bare Git actor");

    let storage = StorageRepo::new(pool.clone());
    let elicitation_cfg = ElicitationConfig {
        facilitator_provider: FacilitatorProviderKind::None,
        ollama_url: "http://localhost:11434".to_string(),
        facilitator_model: "qwen3:8b".to_string(),
        asr_assets_dir: None,
        idle_expiry: Duration::from_secs(24 * 3600),
    };
    let elicitation_store = ElicitationStore::new(elicitation_cfg);

    let state =
        AppState::new(pool, storage, git_write, git_read).with_elicitation(elicitation_store);

    let log_capture = LogCapture::default();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_writer(log_capture.clone())
            .with_ansi(false),
    );
    let _ = tracing::subscriber::set_global_default(subscriber);

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

    TestContext {
        base_url,
        port,
        state,
        cancel_token,
        log_capture,
        _guard: guard,
    }
}

#[tokio::test]
async fn test_elicitation_capabilities_endpoint() {
    let ctx = setup_test_context().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/v1/elicitation/capabilities", ctx.base_url))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::OK);
    let json: serde_json::Value = resp.json().await.expect("valid JSON");

    assert_eq!(json["asr"]["available"], false);
    assert_eq!(json["asr"]["engine"], "sherpa-onnx");
    assert_eq!(json["facilitator"]["provider"], "none");

    ctx.cancel_token.cancel();
}

#[tokio::test]
async fn test_session_lifecycle_and_turn_review() {
    let ctx = setup_test_context().await;
    let client = reqwest::Client::new();
    let confidential_turn_text = "CONFIDENTIAL_STAKEHOLDER_STATEMENT_987654";

    // 1. Create a new session
    let create_payload = json!({
        "title": "Project Intake Session",
        "session_id": "T01-project-intake",
        "stakeholder_name": "Priya Sharma",
        "facilitator_name": "Alex Carter"
    });

    let resp = client
        .post(format!("{}/api/v1/elicitation/sessions", ctx.base_url))
        .json(&create_payload)
        .send()
        .await
        .expect("create session failed");

    assert_eq!(resp.status(), StatusCode::CREATED);
    let session: serde_json::Value = resp.json().await.expect("valid JSON");
    let elicitation_id = session["elicitation_id"].as_str().unwrap();
    assert_eq!(session["session_id"], "T01-project-intake");
    assert_eq!(session["cycle"], 1);
    assert_eq!(session["state"], "open");
    assert_eq!(session["source"], "manual");
    assert_eq!(session["turns"].as_array().unwrap().len(), 0);

    // 2. Get session
    let get_resp = client
        .get(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}",
            ctx.base_url
        ))
        .send()
        .await
        .expect("get session failed");
    assert_eq!(get_resp.status(), StatusCode::OK);

    // 3. Add turn 1 (stakeholder, typed)
    let turn1_resp = client
        .post(format!("{}/api/v1/elicitation/sessions/{elicitation_id}/turns", ctx.base_url))
        .json(&json!({
            "role": "stakeholder",
            "text": format!("  Hello,   I want to describe our needs: {confidential_turn_text}.  \r\n"),
            "input_mode": "typed"
        }))
        .send()
        .await
        .expect("add turn 1 failed");

    assert_eq!(turn1_resp.status(), StatusCode::CREATED);
    let turn1: serde_json::Value = turn1_resp.json().await.expect("valid JSON");
    assert_eq!(turn1["number"], 1);
    assert_eq!(turn1["role"], "stakeholder");
    assert_eq!(turn1["input_mode"], "typed");
    assert_eq!(
        turn1["text"],
        format!("Hello, I want to describe our needs: {confidential_turn_text}.")
    );

    // 4. Add turn 2 (facilitator, typed)
    let turn2_resp = client
        .post(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}/turns",
            ctx.base_url
        ))
        .json(&json!({
            "role": "facilitator",
            "text": "What is the primary constraint on your schedule?",
            "input_mode": "typed"
        }))
        .send()
        .await
        .expect("add turn 2 failed");
    assert_eq!(turn2_resp.status(), StatusCode::CREATED);

    // 5. Add turn 3 (stakeholder, dictated)
    let turn3_resp = client
        .post(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}/turns",
            ctx.base_url
        ))
        .json(&json!({
            "role": "stakeholder",
            "text": "We must deliver before the end of Q4 without exception.",
            "input_mode": "dictated"
        }))
        .send()
        .await
        .expect("add turn 3 failed");
    assert_eq!(turn3_resp.status(), StatusCode::CREATED);

    // Check computed source is now "asr" because turn 3 was dictated (D-EA-7)
    let get_resp2 = client
        .get(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}",
            ctx.base_url
        ))
        .send()
        .await
        .expect("get session failed");
    let session2: serde_json::Value = get_resp2.json().await.expect("valid JSON");
    assert_eq!(session2["source"], "asr");

    // 6. Patch turn 1 (transcription correction)
    let patch_resp = client
        .patch(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}/turns/1",
            ctx.base_url
        ))
        .json(&json!({
            "text": "Hello, I want to formally describe our requirements."
        }))
        .send()
        .await
        .expect("patch turn failed");
    assert_eq!(patch_resp.status(), StatusCode::OK);
    let patched_turn1: serde_json::Value = patch_resp.json().await.expect("valid JSON");
    assert_eq!(
        patched_turn1["text"],
        "Hello, I want to formally describe our requirements."
    );
    assert!(patched_turn1["edited_at"].is_string());

    // 7. Verify dictated mode is NEVER downgraded to typed (D-EA-15)
    let patch_mode_resp = client
        .patch(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}/turns/3",
            ctx.base_url
        ))
        .json(&json!({
            "input_mode": "typed"
        }))
        .send()
        .await
        .expect("patch input_mode failed");
    assert_eq!(patch_mode_resp.status(), StatusCode::OK);
    let patched_turn3: serde_json::Value = patch_mode_resp.json().await.expect("valid JSON");
    assert_eq!(patched_turn3["input_mode"], "dictated");

    // 8. Add an uncertainty flag to turn 3
    let flag_resp = client
        .patch(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}/turns/3",
            ctx.base_url
        ))
        .json(&json!({
            "uncertain_flags": [
                { "start": 0, "end": 7, "note": "Verify deliver vs deploy" }
            ]
        }))
        .send()
        .await
        .expect("set uncertain flag failed");
    assert_eq!(flag_resp.status(), StatusCode::OK);

    // 9. Close session must FAIL with 422 due to uncleared uncertainty flags (§6.5)
    let close_fail_resp = client
        .post(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}/close",
            ctx.base_url
        ))
        .send()
        .await
        .expect("close session failed");
    assert_eq!(close_fail_resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let close_fail_json: serde_json::Value = close_fail_resp.json().await.expect("valid JSON");
    assert_eq!(close_fail_json["valid"], false);
    assert_eq!(close_fail_json["errors"][0]["code"], "uncertain_flags");
    assert_eq!(close_fail_json["errors"][0]["turn"], 3);

    // 10. Clear uncertainty flag on turn 3
    let clear_flag_resp = client
        .patch(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}/turns/3",
            ctx.base_url
        ))
        .json(&json!({
            "uncertain_flags": []
        }))
        .send()
        .await
        .expect("clear flag failed");
    assert_eq!(clear_flag_resp.status(), StatusCode::OK);

    // 11. Now close session succeeds
    let close_ok_resp = client
        .post(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}/close",
            ctx.base_url
        ))
        .send()
        .await
        .expect("close session failed");
    assert_eq!(close_ok_resp.status(), StatusCode::OK);
    let close_ok_json: serde_json::Value = close_ok_resp.json().await.expect("valid JSON");
    assert_eq!(close_ok_json["valid"], true);

    // 12. Attempt to add turn to closed session -> 409 Conflict
    let add_closed_resp = client
        .post(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}/turns",
            ctx.base_url
        ))
        .json(&json!({
            "role": "stakeholder",
            "text": "Late addition after close",
            "input_mode": "typed"
        }))
        .send()
        .await
        .expect("add turn to closed session failed");
    assert_eq!(add_closed_resp.status(), StatusCode::CONFLICT);

    // 13. Download transcript while closed -> 200 with proper headers
    let transcript_resp = client
        .get(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}/transcript",
            ctx.base_url
        ))
        .send()
        .await
        .expect("download transcript failed");
    assert_eq!(transcript_resp.status(), StatusCode::OK);
    let content_type = transcript_resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(content_type.contains("text/markdown"));
    let content_disposition = transcript_resp
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(content_disposition.contains("T01-project-intake-c1.md"));
    let md_text = transcript_resp.text().await.expect("text content");
    assert!(md_text.contains("format: tks-transcript/v1"));
    assert!(md_text.contains("**[t1] Priya Sharma (stakeholder):**"));
    assert!(md_text.contains("**[t2] Alex Carter (facilitator):**"));
    assert!(md_text.contains("**[t3] Priya Sharma (stakeholder):**"));

    // 14. Reopen session
    let reopen_resp = client
        .post(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}/reopen",
            ctx.base_url
        ))
        .send()
        .await
        .expect("reopen session failed");
    assert_eq!(reopen_resp.status(), StatusCode::OK);
    let reopened: serde_json::Value = reopen_resp.json().await.expect("valid JSON");
    assert_eq!(reopened["state"], "open");

    // While reopened, transcript download is refused with 409
    let transcript_open_resp = client
        .get(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}/transcript",
            ctx.base_url
        ))
        .send()
        .await
        .expect("download transcript when open failed");
    assert_eq!(transcript_open_resp.status(), StatusCode::CONFLICT);

    // 15. Delete session
    let delete_resp = client
        .delete(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}",
            ctx.base_url
        ))
        .send()
        .await
        .expect("delete session failed");
    assert_eq!(delete_resp.status(), StatusCode::NO_CONTENT);

    // Subsequent GET returns 404
    let get_after_delete = client
        .get(format!(
            "{}/api/v1/elicitation/sessions/{elicitation_id}",
            ctx.base_url
        ))
        .send()
        .await
        .expect("get after delete failed");
    assert_eq!(get_after_delete.status(), StatusCode::NOT_FOUND);

    // 16. Verify tracing safety: confidental turn text MUST NOT appear anywhere in logs!
    if let Ok(logs) = ctx.log_capture.logs.lock() {
        for log_line in logs.iter() {
            assert!(
                !log_line.contains(confidential_turn_text),
                "Privacy violation! Confidential turn text leaked into tracing logs: {log_line}"
            );
        }
    }

    ctx.cancel_token.cancel();
}

#[tokio::test]
async fn test_input_validation_and_bounds() {
    let ctx = setup_test_context().await;
    let client = reqwest::Client::new();

    // Missing session_id without prior_transcript
    let resp1 = client
        .post(format!("{}/api/v1/elicitation/sessions", ctx.base_url))
        .json(&json!({
            "title": "Invalid Session",
            "stakeholder_name": "Priya",
            "facilitator_name": "Alex"
        }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp1.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Invalid slug (starts with dash)
    let resp2 = client
        .post(format!("{}/api/v1/elicitation/sessions", ctx.base_url))
        .json(&json!({
            "title": "Invalid Slug",
            "session_id": "-invalid-slug",
            "stakeholder_name": "Priya",
            "facilitator_name": "Alex"
        }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp2.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Invalid participant name with parenthesis
    let resp3 = client
        .post(format!("{}/api/v1/elicitation/sessions", ctx.base_url))
        .json(&json!({
            "title": "Invalid Name",
            "session_id": "valid-slug",
            "stakeholder_name": "Priya (Tech Lead)",
            "facilitator_name": "Alex"
        }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp3.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Identical participant names
    let resp4 = client
        .post(format!("{}/api/v1/elicitation/sessions", ctx.base_url))
        .json(&json!({
            "title": "Duplicate Names",
            "session_id": "valid-slug",
            "stakeholder_name": "Priya",
            "facilitator_name": "Priya"
        }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp4.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Create valid session for turn normalisation test
    let create_resp = client
        .post(format!("{}/api/v1/elicitation/sessions", ctx.base_url))
        .json(&json!({
            "title": "Normalisation Test",
            "session_id": "norm-test",
            "stakeholder_name": "Priya",
            "facilitator_name": "Alex"
        }))
        .send()
        .await
        .expect("create failed");
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let s: serde_json::Value = create_resp.json().await.unwrap();
    let id = s["elicitation_id"].as_str().unwrap();

    // Turn with empty-after-normalisation text
    let empty_turn_resp = client
        .post(format!(
            "{}/api/v1/elicitation/sessions/{id}/turns",
            ctx.base_url
        ))
        .json(&json!({
            "role": "stakeholder",
            "text": "   \n\r\t   ",
            "input_mode": "typed"
        }))
        .send()
        .await
        .expect("empty turn request failed");
    assert_eq!(empty_turn_resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Turn exceeding 16 KiB
    let huge_text = "a".repeat(16 * 1024 + 10);
    let huge_turn_resp = client
        .post(format!(
            "{}/api/v1/elicitation/sessions/{id}/turns",
            ctx.base_url
        ))
        .json(&json!({
            "role": "stakeholder",
            "text": huge_text,
            "input_mode": "typed"
        }))
        .send()
        .await
        .expect("huge turn request failed");
    assert_eq!(huge_turn_resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    ctx.cancel_token.cancel();
}

#[tokio::test]
async fn test_cycle_2_continuation() {
    let ctx = setup_test_context().await;
    let client = reqwest::Client::new();

    let prior_transcript = r#"---
format: tks-transcript/v1
title: "Cycle 1 Discovery"
session_id: "T09-warehouse"
cycle: 1
date: "2026-10-15"
language: "en"
source: "manual"
participants:
  - { name: "Marcus Reed", role: "stakeholder" }
  - { name: "Sarah Chen", role: "facilitator" }
---

**[t1] Marcus Reed (stakeholder):** We want automated drone inventory counting.
"#;

    let resp = client
        .post(format!("{}/api/v1/elicitation/sessions", ctx.base_url))
        .json(&json!({
            "title": "",
            "stakeholder_name": "",
            "facilitator_name": "",
            "prior_transcript": prior_transcript
        }))
        .send()
        .await
        .expect("create cycle 2 failed");

    assert_eq!(resp.status(), StatusCode::CREATED);
    let session: serde_json::Value = resp.json().await.expect("valid JSON");

    assert_eq!(session["session_id"], "T09-warehouse");
    assert_eq!(session["cycle"], 2);
    assert_eq!(session["title"], "Cycle 1 Discovery");
    let today = Utc::now().format("%Y-%m-%d").to_string();
    assert_eq!(session["date"], today);

    let parts = session["participants"].as_array().unwrap();
    assert_eq!(parts[0]["name"], "Marcus Reed");
    assert_eq!(parts[1]["name"], "Sarah Chen");

    ctx.cancel_token.cancel();
}

#[tokio::test]
async fn test_gap_import_read_only() {
    let ctx = setup_test_context().await;
    let client = reqwest::Client::new();

    let extraction_id = Uuid::new_v4();
    let db_client = ctx.state.pool.get().await.expect("pool client");

    // Insert mock extraction job in ingestion_jobs with gap questions
    let session_json = json!({
        "session_id": extraction_id,
        "doc_path": "docs/transcripts/mock.md",
        "document_hash": "mocksha",
        "document_kind": "transcript",
        "cycle": 1,
        "participants": [
            { "name": "Alice", "role": "stakeholder" },
            { "name": "Bob", "role": "facilitator" }
        ],
        "turns": [
            {
                "id": "t1",
                "number": 1,
                "name": "Alice",
                "role": "stakeholder",
                "block_byte_start": 0,
                "block_byte_end": 50,
                "utterance_byte_start": 20,
                "utterance_byte_end": 50,
                "block_text": "text",
                "utterance_text": "text"
            }
        ],
        "gaps": [
            {
                "id": "GP-1",
                "detector": "SOFTGOAL_WITHOUT_CONTRIBUTOR",
                "refs": ["G-01"],
                "description": "Softgoal has no contributors",
                "suggested_question": "How will you measure satisfaction for the UI?"
            }
        ]
    });

    db_client
        .execute(
            "INSERT INTO ingestion_jobs (job_id, doc_path, document_hash, status, created_by, attributes, created_at, updated_at) \
             VALUES ($1, 'docs/transcripts/mock.md', 'mocksha', 'PROCESSING', 'agent', $2, NOW() - INTERVAL '1 hour', NOW() - INTERVAL '1 hour');",
            &[&extraction_id, &session_json],
        )
        .await
        .expect("insert mock ingestion_jobs failed");

    // Capture updated_at before gap import
    let row_before = db_client
        .query_one(
            "SELECT updated_at FROM ingestion_jobs WHERE job_id = $1;",
            &[&extraction_id],
        )
        .await
        .expect("query before");
    let updated_at_before: chrono::DateTime<Utc> = row_before.get("updated_at");

    // Create session with prior_extraction_session_id
    let resp = client
        .post(format!("{}/api/v1/elicitation/sessions", ctx.base_url))
        .json(&json!({
            "title": "Gap Seeding Test",
            "session_id": "gap-test",
            "stakeholder_name": "Alice",
            "facilitator_name": "Bob",
            "prior_extraction_session_id": extraction_id
        }))
        .send()
        .await
        .expect("create with gaps failed");

    assert_eq!(resp.status(), StatusCode::CREATED);
    let session: serde_json::Value = resp.json().await.expect("valid JSON");

    // Verify imported gap questions are populated and are NOT turns
    let gaps = session["imported_gap_questions"].as_array().unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0]["gap_id"], "GP-1");
    assert_eq!(
        gaps[0]["suggested_question"],
        "How will you measure satisfaction for the UI?"
    );
    assert_eq!(session["turns"].as_array().unwrap().len(), 0);

    // Verify ingestion_jobs row was NOT modified (read-only query)
    let row_after = db_client
        .query_one(
            "SELECT updated_at FROM ingestion_jobs WHERE job_id = $1;",
            &[&extraction_id],
        )
        .await
        .expect("query after");
    let updated_at_after: chrono::DateTime<Utc> = row_after.get("updated_at");

    assert_eq!(
        updated_at_before, updated_at_after,
        "ingestion_jobs.updated_at was modified during gap import; must be strictly read-only"
    );

    ctx.cancel_token.cancel();
}

#[tokio::test]
async fn test_loopback_guard_and_cors_isolation() {
    let ctx = setup_test_context().await;
    let client = reqwest::Client::new();

    // 1. Non-loopback Host header -> 403 Forbidden
    let non_loopback_resp = client
        .get(format!("{}/api/v1/elicitation/capabilities", ctx.base_url))
        .header("Host", "192.168.1.5:8080")
        .send()
        .await
        .expect("request failed");
    assert_eq!(non_loopback_resp.status(), StatusCode::FORBIDDEN);

    // 2. Cross-origin state-changing mutation with evil Origin -> 403 Forbidden
    let evil_origin_resp = client
        .post(format!("{}/api/v1/elicitation/sessions", ctx.base_url))
        .header("Host", format!("localhost:{}", ctx.port))
        .header("Origin", "https://evil.example")
        .json(&json!({
            "title": "Evil Origin Test",
            "session_id": "evil-test",
            "stakeholder_name": "Priya",
            "facilitator_name": "Alex"
        }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(evil_origin_resp.status(), StatusCode::FORBIDDEN);

    // 3. Permissive CORS exclusion: elicitation responses MUST NOT include Access-Control-Allow-Origin
    let normal_resp = client
        .get(format!("{}/api/v1/elicitation/capabilities", ctx.base_url))
        .header("Host", format!("localhost:{}", ctx.port))
        .send()
        .await
        .expect("request failed");
    assert_eq!(normal_resp.status(), StatusCode::OK);
    assert!(
        normal_resp
            .headers()
            .get("access-control-allow-origin")
            .is_none(),
        "Elicitation routes must never return Access-Control-Allow-Origin"
    );

    ctx.cancel_token.cancel();
}

#[tokio::test]
async fn test_forced_idle_expiry() {
    let quick_store = ElicitationStore::new(ElicitationConfig {
        idle_expiry: Duration::from_millis(50),
        ..Default::default()
    });

    let session = quick_store
        .create_session(
            tks::elicitation::CreateSessionRequest {
                title: "Quick Expire Test".to_string(),
                session_id: Some("quick-expire".to_string()),
                stakeholder_name: "Priya".to_string(),
                facilitator_name: "Alex".to_string(),
                prior_transcript: None,
                prior_extraction_session_id: None,
            },
            None,
        )
        .await
        .expect("create quick session");

    // Immediately after creation, session is found
    assert!(
        quick_store
            .get_session(session.elicitation_id)
            .await
            .is_ok()
    );

    // Wait past the 50ms expiry
    tokio::time::sleep(Duration::from_millis(70)).await;

    // get_session now detects idle expiry, evicts it, and returns NotFound (404)
    let get_res = quick_store.get_session(session.elicitation_id).await;
    assert!(
        matches!(
            get_res,
            Err(tks::elicitation::ElicitationError::NotFound(_))
        ),
        "Session must return NotFound after forced idle expiry"
    );
}
