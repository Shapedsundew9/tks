//! Integration test suite for the CLI Facilitator Provider and Dual-Role Session subsystem.
//!
//! Verifies:
//! - Test 1: Capabilities & dynamic backend/model selection (`GET /api/v1/elicitation/capabilities`
//!   and `PUT /api/v1/elicitation/facilitator`).
//! - Test 2: Dual-role persistent CLI sessions, lazy spawning, incremental context maturation,
//!   mid-session model switching, and session cleanup on close.
//! - Test 3: Live `agy` multi-turn validation using `gpt-oss-120b-medium`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use tks::db;
use tks::elicitation::cli_provider::CliFacilitatorProvider;
use tks::elicitation::{
    AddTurnRequest, CreateSessionRequest, ElicitationConfig, ElicitationEvent, ElicitationStore,
    FACILITATOR_SYSTEM_PROMPT_V2, FacilitatorContext, FacilitatorProvider, FacilitatorProviderKind,
    GenerationMetadata, INSPIRATION_SYSTEM_PROMPT_V3, InputMode, InspirationIdea, TurnRole,
    parse_inspiration_ideas,
};
use tks::gateway::{AppState, create_router};
use tks::storage::StorageRepo;
use tks::storage::git::start_git_actor;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Temporary executable mock `agy` script that records spawns and stdin lines.
struct MockAgyHarness {
    _bin_path: PathBuf,
    log_path: PathBuf,
    temp_dir: PathBuf,
}

impl MockAgyHarness {
    fn new() -> Self {
        let temp_dir = std::env::temp_dir().join(format!("test-mock-agy-{}", Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).expect("failed to create temp dir for mock agy");
        let bin_path = temp_dir.join("mock_agy");
        let log_path = temp_dir.join("mock_agy_log.jsonl");

        let script = r#"#!/usr/bin/env python3
import sys
import os
import json

log_file = os.environ.get("MOCK_AGY_LOG_FILE")

def log(entry):
    if log_file:
        with open(log_file, "a", encoding="utf-8") as f:
            f.write(json.dumps(entry) + "\n")

if len(sys.argv) > 1 and sys.argv[1] == "models":
    sys.stdout.write(
        "gemini-4-argon-medium     Gemini 4 Argon (Medium)\n"
        "gemini-3.8-flash-low      Gemini 3.8 Flash (Low)\n"
        "gpt-oss-120b-medium       GPT-OSS 120B (Medium)\n"
        "claude-opus-5-5-high      Claude Opus 5.5 (High)\n"
    )
    sys.stdout.flush()
    sys.exit(0)

model = "unknown"
if "--model" in sys.argv:
    idx = sys.argv.index("--model")
    if idx + 1 < len(sys.argv):
        model = sys.argv[idx + 1]

pid = os.getpid()
conv_id = f"mock-conv-{pid}"

log({
    "event": "spawn",
    "pid": pid,
    "model": model,
    "conv_id": conv_id,
    "args": sys.argv[1:]
})

init_event = {
    "event": "init",
    "conversation_id": conv_id,
    "init": {
        "model": model,
        "cwd": os.getcwd()
    }
}
sys.stdout.write(json.dumps(init_event) + "\n")
sys.stdout.flush()

while True:
    line = sys.stdin.readline()
    if not line:
        break
    line = line.strip()
    if not line:
        continue
    try:
        data = json.loads(line)
    except Exception:
        continue

    prompt = ""
    if data.get("event") == "user":
        prompt = data.get("message", {}).get("content", "")

    log({
        "event": "stdin",
        "pid": pid,
        "model": model,
        "conv_id": conv_id,
        "prompt": prompt
    })

    if "unexamined gaps" in prompt or "INSPIRATION" in prompt or "missing boundaries" in prompt:
        resp_text = json.dumps({
            "ideas": [
                {
                    "category": "Equipment Safety",
                    "title": "Mandatory Tool Safety Certification",
                    "statement": "Require members to complete a 10-minute digital safety module before borrowing high-risk power tools."
                },
                {
                    "category": "Inventory Lifecycle",
                    "title": "Automated Maintenance Tracking",
                    "statement": "Log cumulative run hours per power tool and schedule automatic blade sharpening and motor inspection."
                }
            ]
        })
    else:
        resp_text = "What criteria will determine eligibility for borrowing tools?"

    step_event = {
        "event": "step_update",
        "step_update": {
            "conversation_id": conv_id,
            "step_index": 1,
            "state": "DONE",
            "step_type": "agent_response",
            "text_delta": resp_text,
            "usage": {
                "input_tokens": 15,
                "output_tokens": 25,
                "total_tokens": 40
            }
        }
    }
    sys.stdout.write(json.dumps(step_event) + "\n")

    result_event = {
        "event": "result",
        "result": {
            "conversation_id": conv_id,
            "status": "SUCCESS",
            "response": resp_text,
            "duration_seconds": 0.05,
            "num_turns": 1,
            "usage": {
                "input_tokens": 15,
                "output_tokens": 25,
                "total_tokens": 40
            }
        }
    }
    sys.stdout.write(json.dumps(result_event) + "\n")
    sys.stdout.flush()
"#;

        fs::write(&bin_path, script).expect("failed to write mock agy script");
        let mut perms = fs::metadata(&bin_path)
            .expect("metadata failed")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin_path, perms).expect("set permissions failed");

        unsafe {
            std::env::set_var("TKS_AGY_BIN", &bin_path);
            std::env::set_var("MOCK_AGY_LOG_FILE", &log_path);
        }

        Self {
            _bin_path: bin_path,
            log_path,
            temp_dir,
        }
    }

    fn read_logs(&self) -> Vec<Value> {
        if !self.log_path.exists() {
            return Vec::new();
        }
        let content = fs::read_to_string(&self.log_path).unwrap_or_default();
        content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }
}

impl Drop for MockAgyHarness {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("TKS_AGY_BIN");
            std::env::remove_var("MOCK_AGY_LOG_FILE");
        }
        let _ = fs::remove_dir_all(&self.temp_dir);
    }
}

fn is_pid_dead(pid: u64) -> bool {
    let stat_path = format!("/proc/{pid}/stat");
    match fs::read_to_string(&stat_path) {
        Ok(content) => {
            if let Some(idx) = content.rfind(')') {
                let rest = content[idx + 1..].trim_start();
                if let Some(state) = rest.chars().next() {
                    return state == 'Z' || state == 'X';
                }
            }
            false
        }
        Err(_) => true,
    }
}

async fn wait_for_pid_termination(pid: u64, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if is_pid_dead(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    is_pid_dead(pid)
}

async fn wait_for_proposal(
    events: &mut tokio::sync::broadcast::Receiver<ElicitationEvent>,
    expected_mode: &str,
) -> (GenerationMetadata, String, Vec<InspirationIdea>) {
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("timeout waiting for proposal event")
            .expect("event channel closed");
        match ev {
            ElicitationEvent::FacilitatorProposal {
                generation,
                text,
                ideas,
                mode,
                ..
            } => {
                if mode == expected_mode {
                    return (generation, text, ideas);
                }
            }
            ElicitationEvent::FacilitatorError { error } => {
                panic!("Received unexpected FacilitatorError: {error}");
            }
            _ => {}
        }
    }
}

#[tokio::test]
async fn test_cli_capabilities_and_dynamic_backend_selection() {
    let _guard = TEST_MUTEX.lock().await;
    let harness = MockAgyHarness::new();

    let database_url = db::ensure_test_database_ready()
        .await
        .expect("test db ready");
    let pool = db::create_pool(&database_url).expect("pool creation failed");
    let git_dir = format!("target/test-cli-git-{}", Uuid::new_v4());
    let (write, read) = start_git_actor(&git_dir, 8).expect("start git actor");
    let storage = StorageRepo::new(pool.clone());

    let elicitation_cfg = ElicitationConfig {
        facilitator_provider: FacilitatorProviderKind::None,
        cli_tool: "agy".to_string(),
        cli_model: None,
        ..Default::default()
    };
    let store = ElicitationStore::new(elicitation_cfg);
    let state = AppState::new(pool, storage, write, read).with_elicitation(store);
    let app = create_router(state);

    // 1. GET /api/v1/elicitation/capabilities
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/elicitation/capabilities")
                .header("host", "localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let caps: Value =
        serde_json::from_slice(&to_bytes(res.into_body(), 100_000).await.unwrap()).unwrap();

    // Verify initial active backend is "none"
    assert_eq!(caps["backends"]["active"], "none");
    assert_eq!(caps["facilitator"]["provider"], "none");

    // Verify backends.cli reports available: true, status: "green", default_model: "gpt-oss-120b-medium"
    let cli = &caps["backends"]["cli"];
    assert_eq!(cli["available"], true);
    assert_eq!(cli["status"], "green");
    assert_eq!(cli["active_tool"], "agy");
    assert_eq!(cli["active_model"], "gpt-oss-120b-medium");

    let tools = cli["tools"].as_array().expect("tools must be array");
    assert_eq!(tools.len(), 1);
    let agy_tool = &tools[0];
    assert_eq!(agy_tool["id"], "agy");
    assert_eq!(agy_tool["available"], true);
    assert_eq!(agy_tool["status"], "green");
    assert_eq!(agy_tool["default_model"], "gpt-oss-120b-medium");

    // Verify models sorted by tier rank (gpt-oss-120b-medium first, then gemini-3.8-flash-low, etc.)
    let models = agy_tool["models"].as_array().expect("models must be array");
    assert_eq!(models.len(), 4);
    assert_eq!(models[0]["id"], "gpt-oss-120b-medium");
    assert_eq!(models[0]["tier_rank"], 0);
    assert_eq!(models[1]["id"], "gemini-3.8-flash-low");
    assert_eq!(models[1]["tier_rank"], 10);
    assert_eq!(models[2]["id"], "gemini-4-argon-medium");
    assert_eq!(models[2]["tier_rank"], 30);
    assert_eq!(models[3]["id"], "claude-opus-5-5-high");
    assert_eq!(models[3]["tier_rank"], 45);

    // 2. PUT /api/v1/elicitation/facilitator to switch to "cli" with model override
    let put_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/elicitation/facilitator")
                .header("host", "localhost")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "provider": "cli",
                        "cli_tool": "agy",
                        "cli_model": "gemini-3.8-flash-low"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_res.status(), StatusCode::OK);
    let updated_caps: Value =
        serde_json::from_slice(&to_bytes(put_res.into_body(), 100_000).await.unwrap()).unwrap();

    assert_eq!(updated_caps["backends"]["active"], "cli");
    assert_eq!(updated_caps["facilitator"]["provider"], "cli");
    assert_eq!(updated_caps["facilitator"]["model"], "gemini-3.8-flash-low");
    assert_eq!(
        updated_caps["backends"]["cli"]["active_model"],
        "gemini-3.8-flash-low"
    );

    // Verify requesting unadvertised model returns 409 Conflict
    let bad_model_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/elicitation/facilitator")
                .header("host", "localhost")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "provider": "cli",
                        "cli_model": "unadvertised-model-999"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_model_res.status(), StatusCode::CONFLICT);

    // 3. PUT /api/v1/elicitation/facilitator to switch back to "none"
    let switch_none_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/elicitation/facilitator")
                .header("host", "localhost")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "provider": "none" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(switch_none_res.status(), StatusCode::OK);
    let none_caps: Value = serde_json::from_slice(
        &to_bytes(switch_none_res.into_body(), 100_000)
            .await
            .unwrap(),
    )
    .unwrap();

    assert_eq!(none_caps["backends"]["active"], "none");
    assert_eq!(none_caps["facilitator"]["provider"], "none");

    drop(harness);
}

#[tokio::test]
async fn test_cli_dual_role_persistent_sessions_and_incremental_maturation() {
    let _guard = TEST_MUTEX.lock().await;
    let harness = MockAgyHarness::new();

    let database_url = db::ensure_test_database_ready()
        .await
        .expect("test db ready");
    let pool = db::create_pool(&database_url).expect("pool creation failed");
    let git_dir = format!("target/test-cli-git-{}", Uuid::new_v4());
    let (write, read) = start_git_actor(&git_dir, 8).expect("start git actor");
    let storage = StorageRepo::new(pool.clone());

    let elicitation_cfg = ElicitationConfig {
        facilitator_provider: FacilitatorProviderKind::Cli,
        cli_tool: "agy".to_string(),
        cli_model: Some("gpt-oss-120b-medium".to_string()),
        ..Default::default()
    };
    let store = ElicitationStore::new(elicitation_cfg);
    let state = AppState::new(pool, storage, write, read).with_elicitation(store.clone());
    let app = create_router(state);

    // 1. Create elicitation session
    let session = store
        .create_session(
            CreateSessionRequest {
                title: "Tool Shed Lending".to_string(),
                session_id: Some("tool-shed-lending".to_string()),
                stakeholder_name: "Alice".to_string(),
                facilitator_name: "Bob".to_string(),
                prior_transcript: None,
                prior_extraction_session_id: None,
            },
            None,
        )
        .await
        .expect("create session failed");
    let elicitation_id = session.elicitation_id;
    let mut events = store
        .subscribe_events(elicitation_id)
        .await
        .expect("subscribe events failed");

    // 2. Add stakeholder turn t1
    store
        .add_turn(
            elicitation_id,
            AddTurnRequest {
                role: TurnRole::Stakeholder,
                text: "Members should be able to check out drills and saws for up to 3 days."
                    .to_string(),
                input_mode: InputMode::Typed,
                insert_after_turn: None,
            },
        )
        .await
        .expect("add turn t1 failed");

    // 3. Request a socratic proposal
    let prop_res1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/elicitation/sessions/{elicitation_id}/facilitator/proposals"
                ))
                .header("host", "localhost")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "mode": "socratic" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(prop_res1.status(), StatusCode::ACCEPTED);

    // Wait for FacilitatorProposal event
    let (p1_generation, p1_text, _) = wait_for_proposal(&mut events, "socratic").await;
    assert_eq!(p1_generation.provider, "cli:agy");
    assert_eq!(p1_generation.model, "gpt-oss-120b-medium");
    assert!(!p1_text.trim().is_empty());
    let socratic_conv_id1 = p1_generation
        .account
        .expect("conv_id required on generation");

    // Inspect logs: verify socratic CLI session lazily spawned, received persona seed + [t1]
    let logs1 = harness.read_logs();
    let spawns1: Vec<&Value> = logs1.iter().filter(|l| l["event"] == "spawn").collect();
    assert_eq!(
        spawns1.len(),
        1,
        "Expected exactly 1 CLI process spawned so far"
    );
    let pid_socratic = spawns1[0]["pid"].as_u64().unwrap();
    assert_eq!(spawns1[0]["conv_id"], socratic_conv_id1);
    assert_eq!(spawns1[0]["model"], "gpt-oss-120b-medium");

    let stdins1: Vec<&Value> = logs1.iter().filter(|l| l["event"] == "stdin").collect();
    assert_eq!(stdins1.len(), 1);
    let prompt1 = stdins1[0]["prompt"].as_str().unwrap();
    assert!(prompt1.contains("Role Persona & Rules:"));
    assert!(prompt1.contains("Socratic facilitator"));
    assert!(prompt1.contains(
        "[t1] (stakeholder): Members should be able to check out drills and saws for up to 3 days."
    ));

    // 4. Request an inspiration proposal on the same elicitation session
    let prop_res2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/elicitation/sessions/{elicitation_id}/facilitator/proposals"
                ))
                .header("host", "localhost")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "mode": "inspiration" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(prop_res2.status(), StatusCode::ACCEPTED);

    let (p2_generation, p2_text, p2_ideas) = wait_for_proposal(&mut events, "inspiration").await;
    assert_eq!(p2_generation.provider, "cli:agy");
    assert!(!p2_text.trim().is_empty());
    assert!(!p2_ideas.is_empty(), "Inspiration ideas must be parsed");
    let inspire_conv_id = p2_generation
        .account
        .expect("conv_id required on inspiration generation");
    assert_ne!(
        inspire_conv_id, socratic_conv_id1,
        "Inspiration session must use distinct conversation_id"
    );

    // Inspect logs: verify a SECOND separate CLI session spawned for inspiration role
    let logs2 = harness.read_logs();
    let spawns2: Vec<&Value> = logs2.iter().filter(|l| l["event"] == "spawn").collect();
    assert_eq!(
        spawns2.len(),
        2,
        "Expected exactly 2 CLI processes spawned (socratic + inspiration)"
    );
    let pid_inspiration = spawns2[1]["pid"].as_u64().unwrap();
    assert_ne!(
        pid_inspiration, pid_socratic,
        "Inspiration PID must be distinct from socratic PID"
    );
    assert_eq!(spawns2[1]["conv_id"], inspire_conv_id);

    let stdins2: Vec<&Value> = logs2.iter().filter(|l| l["event"] == "stdin").collect();
    assert_eq!(stdins2.len(), 2);
    let prompt2 = stdins2[1]["prompt"].as_str().unwrap();
    assert!(prompt2.contains("Role Persona & Rules:"));
    assert!(prompt2.contains("unexamined gaps, negative space, and missing boundaries"));

    // 5. Add a second stakeholder turn t2 and request another socratic proposal
    store
        .add_turn(
            elicitation_id,
            AddTurnRequest {
                role: TurnRole::Stakeholder,
                text: "Late returns incur a 5 dollar daily fee.".to_string(),
                input_mode: InputMode::Typed,
                insert_after_turn: None,
            },
        )
        .await
        .expect("add turn t2 failed");

    let prop_res3 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/elicitation/sessions/{elicitation_id}/facilitator/proposals"
                ))
                .header("host", "localhost")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "mode": "socratic" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(prop_res3.status(), StatusCode::ACCEPTED);

    let (p3_generation, _, _) = wait_for_proposal(&mut events, "socratic").await;
    assert_eq!(
        p3_generation.account.as_deref(),
        Some(socratic_conv_id1.as_str()),
        "Must reuse existing socratic conversation_id"
    );

    // Inspect logs: verify existing socratic CLI process reused, and prompt contains ONLY incremental update
    let logs3 = harness.read_logs();
    let spawns3: Vec<&Value> = logs3.iter().filter(|l| l["event"] == "spawn").collect();
    assert_eq!(
        spawns3.len(),
        2,
        "No new process spawned for second socratic turn"
    );

    let stdins3: Vec<&Value> = logs3.iter().filter(|l| l["event"] == "stdin").collect();
    assert_eq!(stdins3.len(), 3);
    assert_eq!(
        stdins3[2]["pid"].as_u64().unwrap(),
        pid_socratic,
        "Turn 2 delivered to existing socratic PID"
    );
    let prompt3 = stdins3[2]["prompt"].as_str().unwrap();
    assert!(prompt3.contains("New Statements Recorded Since Your Last Turn:"));
    assert!(prompt3.contains("[t2] (stakeholder): Late returns incur a 5 dollar daily fee."));
    assert!(
        !prompt3.contains("Role Persona & Rules:"),
        "Incremental turn must not contain persona seed rules again"
    );
    assert!(
        !prompt3.contains("[t1] (stakeholder)"),
        "Incremental turn must not re-send [t1]"
    );

    // 6. Switch cli_model mid-session via PUT /api/v1/elicitation/facilitator and request another socratic proposal
    let put_switch = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/elicitation/facilitator")
                .header("host", "localhost")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "provider": "cli",
                        "cli_model": "gemini-3.8-flash-low"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_switch.status(), StatusCode::OK);

    let prop_res4 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/elicitation/sessions/{elicitation_id}/facilitator/proposals"
                ))
                .header("host", "localhost")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "mode": "socratic" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(prop_res4.status(), StatusCode::ACCEPTED);

    let (p4_generation, _, _) = wait_for_proposal(&mut events, "socratic").await;
    assert_eq!(p4_generation.model, "gemini-3.8-flash-low");
    let socratic_conv_id2 = p4_generation
        .account
        .expect("conv_id required for new model");
    assert_ne!(
        socratic_conv_id2, socratic_conv_id1,
        "New model must use a new conversation_id"
    );

    // Inspect logs: verify a new CLI session spawned with the new model and seeded with full transcript ([t1] and [t2])
    let logs4 = harness.read_logs();
    let spawns4: Vec<&Value> = logs4.iter().filter(|l| l["event"] == "spawn").collect();
    assert_eq!(
        spawns4.len(),
        3,
        "New CLI process spawned for switched model"
    );
    let pid_socratic_new = spawns4[2]["pid"].as_u64().unwrap();
    assert_ne!(pid_socratic_new, pid_socratic);
    assert_eq!(spawns4[2]["model"], "gemini-3.8-flash-low");

    let stdins4: Vec<&Value> = logs4.iter().filter(|l| l["event"] == "stdin").collect();
    assert_eq!(stdins4.len(), 4);
    assert_eq!(stdins4[3]["pid"].as_u64().unwrap(), pid_socratic_new);
    let prompt4 = stdins4[3]["prompt"].as_str().unwrap();
    assert!(prompt4.contains("Role Persona & Rules:"));
    assert!(prompt4.contains(
        "[t1] (stakeholder): Members should be able to check out drills and saws for up to 3 days."
    ));
    assert!(prompt4.contains("[t2] (stakeholder): Late returns incur a 5 dollar daily fee."));

    // Verify older socratic process pid_socratic was terminated upon model switch
    assert!(
        wait_for_pid_termination(pid_socratic, Duration::from_secs(3)).await,
        "Original socratic process {pid_socratic} should have terminated on model switch"
    );

    // Verify active processes are currently running before close
    assert!(
        !is_pid_dead(pid_inspiration),
        "Inspiration process {pid_inspiration} should still be alive before close"
    );
    assert!(
        !is_pid_dead(pid_socratic_new),
        "Socratic process {pid_socratic_new} should still be alive before close"
    );

    // 7. Close the elicitation session and verify open CLI sessions are cleaned up
    let close_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/elicitation/sessions/{elicitation_id}/close"
                ))
                .header("host", "localhost")
                .header("content-type", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(close_res.status(), StatusCode::OK);

    // Verify remaining open sessions (pid_inspiration and pid_socratic_new) are terminated
    assert!(
        wait_for_pid_termination(pid_inspiration, Duration::from_secs(3)).await,
        "Inspiration CLI process {pid_inspiration} should have been terminated on session close"
    );
    assert!(
        wait_for_pid_termination(pid_socratic_new, Duration::from_secs(3)).await,
        "Socratic CLI process {pid_socratic_new} should have been terminated on session close"
    );

    drop(harness);
}

#[tokio::test]
async fn test_live_agy_cli_multi_turn_validation() {
    let _guard = TEST_MUTEX.lock().await;

    // Ensure we run against real `agy`
    unsafe {
        std::env::remove_var("TKS_AGY_BIN");
        std::env::remove_var("MOCK_AGY_LOG_FILE");
    }

    let provider =
        CliFacilitatorProvider::new("agy".to_string(), Some("gpt-oss-120b-medium".to_string()));
    let status = provider.get_tool_status("agy").await;
    if !status.as_ref().map(|s| s.available).unwrap_or(false) {
        eprintln!(
            "Skipping live agy test: agy tool is not available ({:?})",
            status.map(|s| s.message)
        );
        return;
    }

    let elicitation_id = Uuid::new_v4();

    // Turn 1 (socratic): opening or initial stakeholder turn
    let ctx1 = FacilitatorContext {
        elicitation_id: Some(elicitation_id),
        system_prompt: FACILITATOR_SYSTEM_PROMPT_V2.to_string(),
        session_title: "Community Tool Library".to_string(),
        is_inspiration: false,
        focus_topic: None,
        gap_questions: Vec::new(),
        explored_items: Vec::new(),
        turns: vec![(
            "stakeholder".to_string(),
            "We want members to borrow tools from our community shed.".to_string(),
        )],
    };

    let p1 = provider
        .propose(ctx1)
        .await
        .expect("Turn 1 Socratic proposal failed");
    assert!(
        !p1.text.trim().is_empty(),
        "Socratic question must not be empty"
    );
    assert_eq!(p1.generation.model, "gpt-oss-120b-medium");
    assert_eq!(p1.generation.provider, "cli:agy");
    let socratic_conv_id = p1
        .generation
        .account
        .expect("Turn 1 must have conversation_id");

    // Turn 2 (socratic): follow-up stakeholder turn
    let ctx2 = FacilitatorContext {
        elicitation_id: Some(elicitation_id),
        system_prompt: FACILITATOR_SYSTEM_PROMPT_V2.to_string(),
        session_title: "Community Tool Library".to_string(),
        is_inspiration: false,
        focus_topic: None,
        gap_questions: Vec::new(),
        explored_items: vec![p1.text.clone()],
        turns: vec![
            (
                "stakeholder".to_string(),
                "We want members to borrow tools from our community shed.".to_string(),
            ),
            ("facilitator".to_string(), p1.text.clone()),
            (
                "stakeholder".to_string(),
                "Borrowing is limited to 7 days for registered residents.".to_string(),
            ),
        ],
    };

    let p2 = provider
        .propose(ctx2)
        .await
        .expect("Turn 2 Socratic proposal failed");
    assert!(
        !p2.text.trim().is_empty(),
        "Turn 2 Socratic question must not be empty"
    );
    assert_eq!(p2.generation.model, "gpt-oss-120b-medium");
    assert_eq!(
        p2.generation.account.as_ref(),
        Some(&socratic_conv_id),
        "Same conversation_id must be preserved across Socratic turns"
    );

    // Turn 1 (inspiration): inspiration brainstorm on the same elicitation_id
    let ctx_inspire = FacilitatorContext {
        elicitation_id: Some(elicitation_id),
        system_prompt: INSPIRATION_SYSTEM_PROMPT_V3.to_string(),
        session_title: "Community Tool Library".to_string(),
        is_inspiration: true,
        focus_topic: None,
        gap_questions: Vec::new(),
        explored_items: vec![p1.text.clone(), p2.text.clone()],
        turns: vec![
            (
                "stakeholder".to_string(),
                "We want members to borrow tools from our community shed.".to_string(),
            ),
            ("facilitator".to_string(), p1.text.clone()),
            (
                "stakeholder".to_string(),
                "Borrowing is limited to 7 days for registered residents.".to_string(),
            ),
        ],
    };

    let p_inspire = provider
        .propose(ctx_inspire)
        .await
        .expect("Inspiration proposal failed");
    let inspire_conv_id = p_inspire
        .generation
        .account
        .expect("Inspiration must have conversation_id");
    assert_ne!(
        inspire_conv_id, socratic_conv_id,
        "Distinct conversation_id must be used for Inspiration role"
    );

    let ideas = parse_inspiration_ideas(&p_inspire.text);
    assert!(
        !ideas.is_empty(),
        "parse_inspiration_ideas should extract valid ideas from: {}",
        p_inspire.text
    );

    provider.close_session(elicitation_id).await;
}
