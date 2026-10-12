//! Integration test suite for MCP Extraction Session Protocol & Prompts Capability (EB-4, TB-14).
//!
//! Verifies:
//! 1. MCP `prompts/list` and `prompts/get` capability returning versioned pass instructions (TB-14).
//! 2. Full end-to-end extraction session lifecycle:
//!    - `open_extraction_session` parses transcript into `ingestion_jobs.attributes`.
//!    - `submit_episodes` enforces contiguity and turn coverage.
//!    - `submit_goal_candidates` enforces verbatim mechanical grounding and technology linting.
//!    - `submit_verification_verdicts` records entailment verdicts.
//!    - `get_gap_report` evaluates the 8 canonical gap detectors.
//!    - `stage_goals` atomically stages `GOAL` and `SOURCE_EXCERPT` nodes and edges into `graph_nodes`.

use reqwest::StatusCode;
use reqwest::header::CONTENT_TYPE;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tks::db;
use tks::gateway::auth::compute_token_hash;
use tks::gateway::{AppState, create_router};
use tks::storage::StorageRepo;
use tks::storage::envelope::validate_ancestor_path;
use tks::storage::git::start_git_actor;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

    {
        let client = pool.get().await.expect("Failed to get DB client");
        let _ = client
            .execute(
                "DELETE FROM graph_nodes WHERE node_key LIKE 'GOAL-%' OR node_key LIKE 'EXC-%';",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM ingestion_jobs WHERE doc_path LIKE '%T01%';",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "DELETE FROM agent_identities WHERE agent_id = 'test_eb4_agent';",
                &[],
            )
            .await;

        let token_hash = compute_token_hash("test_eb4_token");
        let _ = client
            .execute(
                "INSERT INTO agent_identities (agent_id, actor_type, token_hash, is_active) \
                 VALUES ('test_eb4_agent', 'AGENT', $1, true) \
                 ON CONFLICT (agent_id) DO UPDATE SET token_hash = $1, is_active = true;",
                &[&token_hash],
            )
            .await;
    }

    let repo_dir = format!("target/test-eb4-git-{}", Uuid::new_v4());
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

    (base_url, cancel_token, state, guard)
}

#[tokio::test]
async fn test_mcp_prompts_protocol_capability() {
    let (base_url, cancel_token, _state, _guard) = spawn_test_server().await;
    let http = reqwest::Client::new();
    let mcp_url = format!("{base_url}/mcp");

    // 1. initialize advertises prompts capability (TB-14)
    let init_resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(init_resp.status(), StatusCode::OK);
    let init_body: Value = init_resp.json().await.unwrap();
    assert!(
        init_body["result"]["capabilities"]["prompts"].is_object(),
        "MCP capabilities must advertise 'prompts'"
    );

    // 2. prompts/list returns all 4 versioned pass instructions
    let list_resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "prompts/list",
            "params": {}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(list_resp.status(), StatusCode::OK);
    let list_body: Value = list_resp.json().await.unwrap();
    let prompts = list_body["result"]["prompts"]
        .as_array()
        .expect("prompts array");
    assert_eq!(
        prompts.len(),
        4,
        "Must expose exactly 4 canonical prompt templates"
    );

    let prompt_names: Vec<&str> = prompts
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert!(prompt_names.contains(&"elicitation/episode-segmentation/v1"));
    assert!(prompt_names.contains(&"elicitation/goal-extraction/v1"));
    assert!(prompt_names.contains(&"elicitation/entailment-judge/v1"));
    assert!(prompt_names.contains(&"elicitation/gap-questions/v1"));

    // 3. prompts/get renders parameterized prompt
    let get_resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "prompts/get",
            "params": {
                "name": "elicitation/episode-segmentation/v1",
                "arguments": {
                    "transcript_text": "Sample transcript content here"
                }
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(get_resp.status(), StatusCode::OK);
    let get_body: Value = get_resp.json().await.unwrap();
    let messages = get_body["result"]["messages"]
        .as_array()
        .expect("messages array");
    assert_eq!(messages.len(), 1);
    let prompt_text = messages[0]["content"]["text"].as_str().unwrap();
    assert!(prompt_text.contains("Sample transcript content here"));
    assert!(prompt_text.contains("Episode Segmentation"));

    // 4. prompts/get rejects missing required argument
    let err_resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "prompts/get",
            "params": {
                "name": "elicitation/episode-segmentation/v1",
                "arguments": {}
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(err_resp.status(), StatusCode::OK);
    let err_body: Value = err_resp.json().await.unwrap();
    assert_eq!(err_body["error"]["code"], -32602);

    cancel_token.cancel();
}

#[tokio::test]
async fn test_elicitation_extraction_session_end_to_end() {
    let (base_url, cancel_token, state, _guard) = spawn_test_server().await;
    let http = reqwest::Client::new();
    let mcp_url = format!("{base_url}/mcp");
    let auth_token = "test_eb4_token";

    // 1. open_extraction_session
    let open_resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "auth_token": auth_token,
            "params": {
                "name": "open_extraction_session",
                "arguments": {
                    "doc_path": "tests/fixtures/elicitation/transcripts/T01-capacity.md"
                }
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(open_resp.status(), StatusCode::OK);
    let open_body: Value = open_resp.json().await.unwrap();
    assert!(
        open_body["error"].is_null(),
        "open_extraction_session must succeed: {open_body:?}"
    );

    let session_id_str = open_body["result"]["session"]["session_id"]
        .as_str()
        .unwrap();
    let session_id = Uuid::parse_str(session_id_str).expect("Valid UUID");
    let turns_count = open_body["result"]["session"]["turns_count"]
        .as_u64()
        .unwrap();
    assert!(turns_count >= 15, "T01 has >= 15 turns");

    // 2. submit_episodes - test gap failure first
    let invalid_episodes = json!([
        { "id": "E1", "turns": ["t1", "t3"], "topic": "Intro" },
        { "id": "E2", "turns": ["t5", format!("t{turns_count}")], "topic": "Gap" }
    ]);

    let ep_fail_resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "auth_token": auth_token,
            "params": {
                "name": "submit_episodes",
                "arguments": {
                    "session_id": session_id_str,
                    "episodes": invalid_episodes
                }
            }
        }))
        .send()
        .await
        .unwrap();

    let ep_fail_body: Value = ep_fail_resp.json().await.unwrap();
    assert!(
        ep_fail_body["error"].is_object(),
        "Non-contiguous episode submission must fail"
    );

    // 2b. submit_episodes - valid full coverage
    let valid_episodes = json!([
        { "id": "E1", "turns": ["t1", "t5"], "topic": "Pain points" },
        { "id": "E2", "turns": ["t6", format!("t{turns_count}")], "topic": "Proposed vision" }
    ]);

    let ep_ok_resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "auth_token": auth_token,
            "params": {
                "name": "submit_episodes",
                "arguments": {
                    "session_id": session_id_str,
                    "episodes": valid_episodes
                }
            }
        }))
        .send()
        .await
        .unwrap();

    let ep_ok_body: Value = ep_ok_resp.json().await.unwrap();
    assert!(
        ep_ok_body["error"].is_null(),
        "submit_episodes must succeed: {ep_ok_body:?}"
    );
    assert_eq!(ep_ok_body["result"]["result"]["episodes_accepted"], 2);

    // 3. submit_goal_candidates - perturbed quote failure first
    let perturbed_goals = json!([
        {
            "id": "G1",
            "kind": "ACHIEVE",
            "statement": "Managers can see sprint progress independently.",
            "stakeholder": "Priya",
            "status": "CONFIRMED",
            "grounding": [
                { "turn": "t2", "quote": "I spend all my Tuesday chasing people on Discord" }
            ]
        }
    ]);

    let goal_fail_resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "auth_token": auth_token,
            "params": {
                "name": "submit_goal_candidates",
                "arguments": {
                    "session_id": session_id_str,
                    "goals": perturbed_goals
                }
            }
        }))
        .send()
        .await
        .unwrap();

    let goal_fail_body: Value = goal_fail_resp.json().await.unwrap();
    assert!(
        goal_fail_body["error"].is_object(),
        "Perturbed grounding quote MUST be rejected"
    );

    // 3b. submit_goal_candidates - true verbatim quotes from T01 turn t2
    let valid_goals = json!([
        {
            "id": "G1",
            "kind": "ACHIEVE",
            "statement": "Project managers can independently track sprint progress without chasing updates on Slack.",
            "stakeholder": "Priya",
            "status": "CONFIRMED",
            "episode": "E1",
            "grounding": [
                { "turn": "t2", "quote": "I spend half my Monday chasing people on Slack just to figure out who has bandwidth." }
            ],
            "links": [
                { "type": "CONTRIBUTES_TO", "target": "G2" }
            ]
        },
        {
            "id": "G2",
            "kind": "SOFTGOAL",
            "statement": "High team autonomy with minimal status reporting friction.",
            "stakeholder": "Priya",
            "status": "CONFIRMED",
            "episode": "E1",
            "grounding": [
                { "turn": "t2", "quote": "assess sprint progress without interrupting developers constantly" }
            ]
        },
        {
            "id": "G3",
            "kind": "AVOID",
            "statement": "Avoid manual spreadsheet tracking.",
            "stakeholder": "Priya",
            "status": "CONFIRMED",
            "rationale": "Spreadsheets desync immediately",
            "episode": "E2",
            "grounding": [
                { "turn": "t2", "quote": "chasing people on Slack" }
            ]
        }
    ]);

    let goal_ok_resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "auth_token": auth_token,
            "params": {
                "name": "submit_goal_candidates",
                "arguments": {
                    "session_id": session_id_str,
                    "goals": valid_goals
                }
            }
        }))
        .send()
        .await
        .unwrap();

    let goal_ok_body: Value = goal_ok_resp.json().await.unwrap();
    assert!(
        goal_ok_body["error"].is_null(),
        "submit_goal_candidates must succeed: {goal_ok_body:?}"
    );
    assert_eq!(goal_ok_body["result"]["result"]["goals_accepted"], 3);

    // 4. submit_verification_verdicts
    let verdicts = json!([
        { "goal_id": "G1", "forward": "ENTAILED", "reverse_overconstrained": false },
        { "goal_id": "G2", "forward": "ENTAILED", "reverse_overconstrained": false },
        { "goal_id": "G3", "forward": "ENTAILED", "reverse_overconstrained": false }
    ]);

    let ver_resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "auth_token": auth_token,
            "params": {
                "name": "submit_verification_verdicts",
                "arguments": {
                    "session_id": session_id_str,
                    "verdicts": verdicts
                }
            }
        }))
        .send()
        .await
        .unwrap();

    let ver_body: Value = ver_resp.json().await.unwrap();
    assert!(
        ver_body["error"].is_null(),
        "submit_verification_verdicts must succeed: {ver_body:?}"
    );
    assert_eq!(ver_body["result"]["result"]["verdicts_recorded"], 3);

    // 5. get_gap_report
    let gap_resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "auth_token": auth_token,
            "params": {
                "name": "get_gap_report",
                "arguments": {
                    "session_id": session_id_str
                }
            }
        }))
        .send()
        .await
        .unwrap();

    let gap_body: Value = gap_resp.json().await.unwrap();
    assert!(
        gap_body["error"].is_null(),
        "get_gap_report must succeed: {gap_body:?}"
    );

    // 6. stage_goals
    let stage_resp = http
        .post(&mcp_url)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "auth_token": auth_token,
            "params": {
                "name": "stage_goals",
                "arguments": {
                    "session_id": session_id_str
                }
            }
        }))
        .send()
        .await
        .unwrap();

    let stage_body: Value = stage_resp.json().await.unwrap();
    assert!(
        stage_body["error"].is_null(),
        "stage_goals must succeed: {stage_body:?}"
    );
    assert_eq!(stage_body["result"]["staging"]["status"], "STAGED");
    assert_eq!(stage_body["result"]["staging"]["staged_goals_count"], 3);

    // 7. Verify Invariant INV-1 for staged goals in DB
    let client = state.pool.get().await.expect("Pool client");
    let staged_rows = client
        .query(
            "SELECT id, node_key, node_type, lifecycle_state, attributes FROM graph_nodes WHERE job_id = $1;",
            &[&session_id],
        )
        .await
        .expect("Query staged nodes");

    let goal_ids: Vec<Uuid> = staged_rows
        .iter()
        .filter(|r| r.get::<_, String>("node_type") == "GOAL")
        .map(|r| r.get("id"))
        .collect();

    assert_eq!(goal_ids.len(), 3, "Exactly 3 GOAL nodes staged");

    for r in staged_rows
        .iter()
        .filter(|r| r.get::<_, String>("node_type") == "GOAL")
    {
        let attrs: Value = r.get("attributes");
        assert!(
            attrs.get("goal_kind").is_some(),
            "GOAL node must specify goal_kind in attributes"
        );
    }

    // Lineage root validation: GOAL nodes satisfy INV-1 ancestor validation
    let is_valid = validate_ancestor_path(&**client, &goal_ids, true)
        .await
        .expect("validate_ancestor_path");
    assert!(is_valid, "Staged GOAL roots must satisfy Invariant INV-1");

    cancel_token.cancel();
}
