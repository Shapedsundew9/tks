//! Integration and property tests for Transcript Rendering, Export Validation, and E-B Interoperability (EA-4, §6.5).
//!
//! Verifies:
//! - Property tests: 100 randomly generated sessions (1..200 turns, quotes, colons, #, *, >, unicode, leading -)
//!   always render to canonical `tks-transcript/v1` text that `ParsedTranscript::parse` accepts, with byte-exact utterances.
//! - Refusal of close on empty session or uncleared uncertainty flags.
//! - Accurate provenance computation (`manual` vs `asr`).
//! - E-B round-trip verification:
//!   1. Export a typed/dictated session.
//!   2. Ingest via `handle_open_extraction_session` with inline `content`.
//!   3. Verify every `SourceExcerpt` from `to_source_excerpts` matches the stored turn text byte for byte.
//! - Cycle-2 export preservation of `session_id` and `cycle = 2`.

use chrono::Utc;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use tks::db;
use tks::elicitation::session::{
    ElicitationParticipant, ElicitationSession, ElicitationTurn, InputMode, SessionState, TurnRole,
    UncertainFlag, normalize_turn_text,
};
use tks::elicitation::transcript::{render_transcript, validate_transcript_for_close};
use tks::gateway::AppState;
use tks::gateway::auth::AuthenticatedAgent;
use tks::gateway::mcp::tools::{
    handle_get_gap_report, handle_open_extraction_session, handle_stage_goals,
    handle_submit_episodes, handle_submit_goal_candidates,
};
use tks::ingest::transcript::ParsedTranscript;
use tks::storage::StorageRepo;
use tks::storage::git::start_git_actor;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[test]
fn test_property_transcript_rendering_and_parser_roundtrip() {
    // Deterministic pseudo-random session generator to thoroughly stress the renderer & parser
    let test_strings = [
        "Simple turn content.",
        "Contains \"double quotes\" and 'single quotes' and `backticks`.",
        "Prefix with # Heading attempt and --- hr and > blockquote and * list item and - dash.",
        "Contains : colons: multiple : of : them.",
        "Multilingual: 🚀 Привет, мир! こんにちは 🌟 €100 £50 100% complete.",
        "Leading and trailing space attempt:    spaced out text   ",
        "Punctuation galore: !?.,:;()[]{}-_=+*&^%$#@~<>|\\/",
        "Long technical text: We need PostgreSQL 16+ with pgvector, HNSW cosine index, and sub-15ms CPU inference.",
    ];

    for seed in 1..=100 {
        let turn_count = 1 + (seed * 7 % 200); // 1 to 200 turns
        let mut turns = Vec::with_capacity(turn_count);

        let mut has_dictated = false;
        for t in 1..=turn_count {
            let role = if t % 2 == 1 {
                TurnRole::Stakeholder
            } else {
                TurnRole::Facilitator
            };
            let input_mode = if (seed + t) % 3 == 0 {
                has_dictated = true;
                InputMode::Dictated
            } else {
                InputMode::Typed
            };

            let base_str = test_strings[(seed + t) % test_strings.len()];
            let raw_text = format!("Turn {t} (seed {seed}): {base_str}");
            let norm_text = normalize_turn_text(&raw_text).expect("valid normalized text");

            turns.push(ElicitationTurn {
                number: t as u32,
                role,
                text: norm_text,
                input_mode,
                uncertain_flags: Vec::new(),
                committed_at: Utc::now(),
                edited_at: None,
            });
        }

        let session = ElicitationSession {
            elicitation_id: Uuid::new_v4(),
            state: SessionState::Closed,
            title: format!("Property Session {seed}"),
            session_id: format!("prop-sess-{seed}"),
            cycle: (seed % 5) as u32 + 1,
            date: "2026-10-24".to_string(),
            language: "en".to_string(),
            participants: vec![
                ElicitationParticipant {
                    name: "Stakeholder One".to_string(),
                    role: "stakeholder".to_string(),
                },
                ElicitationParticipant {
                    name: "Facilitator One".to_string(),
                    role: "facilitator".to_string(),
                },
            ],
            turns: turns.clone(),
            imported_gap_questions: Vec::new(),
            explored_items: Vec::new(),
            created_at: Utc::now(),
            last_activity_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::days(1),
            source: if has_dictated {
                "asr".to_string()
            } else {
                "manual".to_string()
            },
        };

        // Export validation must pass
        assert!(
            validate_transcript_for_close(&session).is_ok(),
            "Session validation failed for seed {seed}"
        );

        // Render to canonical Markdown
        let rendered = render_transcript(&session);

        // Parse with ParsedTranscript::parse
        let parsed = ParsedTranscript::parse(&rendered).unwrap_or_else(|e| {
            panic!(
                "Failed to parse rendered transcript for seed {seed}: {e}\nRendered:\n{rendered}"
            )
        });

        assert_eq!(parsed.turns.len(), turns.len());
        assert_eq!(parsed.frontmatter.title, session.title);
        assert_eq!(parsed.frontmatter.session_id, session.session_id);
        assert_eq!(parsed.frontmatter.cycle, session.cycle);
        assert_eq!(parsed.frontmatter.source, session.compute_source());

        for (stored, parsed_turn) in turns.iter().zip(parsed.turns.iter()) {
            assert_eq!(stored.number, parsed_turn.number);
            assert_eq!(stored.text, parsed_turn.utterance_text);
        }
    }
}

#[test]
fn test_close_validation_refusal_rules() {
    // 1. Refusal on empty session (0 turns)
    let empty_session = ElicitationSession {
        elicitation_id: Uuid::new_v4(),
        state: SessionState::Open,
        title: "Empty Session".to_string(),
        session_id: "empty-session".to_string(),
        cycle: 1,
        date: "2026-10-24".to_string(),
        language: "en".to_string(),
        participants: vec![
            ElicitationParticipant {
                name: "Priya".to_string(),
                role: "stakeholder".to_string(),
            },
            ElicitationParticipant {
                name: "Alex".to_string(),
                role: "facilitator".to_string(),
            },
        ],
        turns: Vec::new(),
        imported_gap_questions: Vec::new(),
        explored_items: Vec::new(),
        created_at: Utc::now(),
        last_activity_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::days(1),
        source: "manual".to_string(),
    };

    let errs = validate_transcript_for_close(&empty_session).unwrap_err();
    assert!(errs.iter().any(|e| e.code == "no_turns"));

    // 2. Refusal on uncleared uncertainty flag
    let mut session_with_flags = empty_session.clone();
    session_with_flags.turns.push(ElicitationTurn {
        number: 1,
        role: TurnRole::Stakeholder,
        text: "We need 99.9% availability.".to_string(),
        input_mode: InputMode::Typed,
        uncertain_flags: vec![UncertainFlag {
            start: 8,
            end: 13,
            note: Some("Check SLA agreement".to_string()),
        }],
        committed_at: Utc::now(),
        edited_at: None,
    });

    let errs2 = validate_transcript_for_close(&session_with_flags).unwrap_err();
    assert!(
        errs2
            .iter()
            .any(|e| e.code == "uncertain_flags" && e.turn == Some(1)),
        "Expected error for uncleared uncertainty flag on turn 1"
    );
}

#[tokio::test]
async fn test_eb_extraction_roundtrip() {
    let _guard = TEST_MUTEX.lock().await;
    let database_url = db::ensure_test_database_ready()
        .await
        .expect("Failed to prepare test database");
    let pool = db::create_pool(&database_url).expect("Failed to create connection pool");

    let repo_dir = format!("target/test-eb-interop-git-{}", Uuid::new_v4());
    let (git_write, git_read) =
        start_git_actor(&repo_dir, 64).expect("Failed to start test git actor");

    let storage = StorageRepo::new(pool.clone());
    let state = AppState::new(pool, storage, git_write, git_read);

    // 1. Build a realistic typed & spoken elicitation session
    let turn1_text =
        "We are building an automated customer intake workflow to eliminate manual PDF re-keying.";
    let turn2_text = "Who are the primary users affected by the re-keying delay?";
    let turn3_text =
        "The loan underwriters, who currently wait 48 hours for document verification.";

    let session = ElicitationSession {
        elicitation_id: Uuid::new_v4(),
        state: SessionState::Closed,
        title: "Loan Processing Intake".to_string(),
        session_id: "T12-loan-processing".to_string(),
        cycle: 1,
        date: "2026-10-24".to_string(),
        language: "en".to_string(),
        participants: vec![
            ElicitationParticipant {
                name: "Maria Gonzales".to_string(),
                role: "stakeholder".to_string(),
            },
            ElicitationParticipant {
                name: "David Kim".to_string(),
                role: "facilitator".to_string(),
            },
        ],
        turns: vec![
            ElicitationTurn {
                number: 1,
                role: TurnRole::Stakeholder,
                text: turn1_text.to_string(),
                input_mode: InputMode::Typed,
                uncertain_flags: Vec::new(),
                committed_at: Utc::now(),
                edited_at: None,
            },
            ElicitationTurn {
                number: 2,
                role: TurnRole::Facilitator,
                text: turn2_text.to_string(),
                input_mode: InputMode::Typed,
                uncertain_flags: Vec::new(),
                committed_at: Utc::now(),
                edited_at: None,
            },
            ElicitationTurn {
                number: 3,
                role: TurnRole::Stakeholder,
                text: turn3_text.to_string(),
                input_mode: InputMode::Dictated,
                uncertain_flags: Vec::new(),
                committed_at: Utc::now(),
                edited_at: None,
            },
        ],
        imported_gap_questions: Vec::new(),
        explored_items: Vec::new(),
        created_at: Utc::now(),
        last_activity_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::days(1),
        source: "asr".to_string(),
    };

    // 2. Validate and render canonical transcript
    assert!(validate_transcript_for_close(&session).is_ok());
    let rendered_transcript = render_transcript(&session);
    assert_eq!(session.compute_source(), "asr");

    // 3. Open E-B extraction session via open_extraction_session with inline content
    let doc_path = "docs/transcripts/T12-loan-processing-c1.md";
    let args = json!({
        "doc_path": doc_path,
        "content": rendered_transcript
    });

    let open_res = handle_open_extraction_session(&state, args, None)
        .await
        .expect("open_extraction_session failed");

    let eb_session_id = open_res["session_id"].as_str().unwrap();
    assert_eq!(open_res["doc_path"], doc_path);
    assert_eq!(open_res["cycle"], 1);

    // 4. Compute doc_sha256 and parse transcript
    let mut hasher = Sha256::new();
    hasher.update(rendered_transcript.as_bytes());
    let doc_sha256 = format!("{:x}", hasher.finalize());

    let parsed = ParsedTranscript::parse(&rendered_transcript)
        .expect("ParsedTranscript::parse should succeed on rendered transcript");

    let excerpts = parsed.to_source_excerpts(doc_path, &doc_sha256);
    assert_eq!(excerpts.len(), 3);

    // 5. Verify byte-exact match between every SourceExcerpt and stored turn text
    assert_eq!(excerpts[0].excerpt_id, "T12-loan-processing:t1");
    assert_eq!(excerpts[0].speaker.as_deref(), Some("Maria Gonzales"));
    assert_eq!(excerpts[0].role.as_deref(), Some("stakeholder"));
    assert_eq!(excerpts[0].content, turn1_text);

    assert_eq!(excerpts[1].excerpt_id, "T12-loan-processing:t2");
    assert_eq!(excerpts[1].speaker.as_deref(), Some("David Kim"));
    assert_eq!(excerpts[1].role.as_deref(), Some("facilitator"));
    assert_eq!(excerpts[1].content, turn2_text);

    assert_eq!(excerpts[2].excerpt_id, "T12-loan-processing:t3");
    assert_eq!(excerpts[2].speaker.as_deref(), Some("Maria Gonzales"));
    assert_eq!(excerpts[2].role.as_deref(), Some("stakeholder"));
    assert_eq!(excerpts[2].content, turn3_text);

    println!("E-B round trip verified with session ID {eb_session_id}");
}

#[tokio::test]
async fn test_cycle_2_export_preserves_session_id_and_cycle() {
    let session = ElicitationSession {
        elicitation_id: Uuid::new_v4(),
        state: SessionState::Closed,
        title: "Elicitation Cycle Two".to_string(),
        session_id: "T99-cycle-two".to_string(),
        cycle: 2,
        date: "2026-10-24".to_string(),
        language: "en".to_string(),
        participants: vec![
            ElicitationParticipant {
                name: "Dr. Alex".to_string(),
                role: "stakeholder".to_string(),
            },
            ElicitationParticipant {
                name: "Morgan".to_string(),
                role: "facilitator".to_string(),
            },
        ],
        turns: vec![ElicitationTurn {
            number: 1,
            role: TurnRole::Stakeholder,
            text: "Continuing with cycle 2 clarifications.".to_string(),
            input_mode: InputMode::Typed,
            uncertain_flags: Vec::new(),
            committed_at: Utc::now(),
            edited_at: None,
        }],
        imported_gap_questions: Vec::new(),
        explored_items: Vec::new(),
        created_at: Utc::now(),
        last_activity_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::days(1),
        source: "manual".to_string(),
    };

    assert!(validate_transcript_for_close(&session).is_ok());
    let rendered = render_transcript(&session);
    let parsed = ParsedTranscript::parse(&rendered).expect("valid parse");

    assert_eq!(parsed.frontmatter.session_id, "T99-cycle-two");
    assert_eq!(parsed.frontmatter.cycle, 2);
}

#[tokio::test]
async fn test_ea7_dogfood_eb_pipeline() {
    let _guard = TEST_MUTEX.lock().await;
    let database_url = db::ensure_test_database_ready()
        .await
        .expect("Failed to prepare test database");
    let pool = db::create_pool(&database_url).expect("Failed to create connection pool");

    let repo_dir = format!("target/test-dogfood-git-{}", Uuid::new_v4());
    let (git_write, git_read) =
        start_git_actor(&repo_dir, 64).expect("Failed to start test git actor");

    let storage = StorageRepo::new(pool.clone());
    let state = AppState::new(pool, storage, git_write, git_read);

    let auth = AuthenticatedAgent {
        agent_id: "test_dogfood_agent".to_string(),
        actor_type: "HUMAN".to_string(),
    };

    // 1. Read real dogfood transcript docs/transcripts/ea7-dogfood-c1.md
    let doc_path = "docs/transcripts/ea7-dogfood-c1.md";
    let content = tokio::fs::read_to_string(doc_path)
        .await
        .expect("Failed to read ea7-dogfood-c1.md");

    let parsed = ParsedTranscript::parse(&content)
        .expect("ParsedTranscript::parse should accept dogfood transcript");
    assert_eq!(parsed.frontmatter.session_id, "ea7-dogfood");
    assert_eq!(parsed.frontmatter.cycle, 1);
    assert_eq!(parsed.frontmatter.source, "asr");
    assert_eq!(parsed.turns.len(), 5);

    // 2. Open E-B extraction session
    let open_res = handle_open_extraction_session(
        &state,
        json!({
            "doc_path": doc_path,
            "content": content
        }),
        Some(&auth),
    )
    .await
    .expect("open_extraction_session should succeed");

    let session_id = open_res["session_id"].as_str().unwrap();

    // 3. Submit episodes covering turns t1 to t5
    let episodes_arg = json!({
        "session_id": session_id,
        "episodes": [
            {
                "id": "E1",
                "turns": ["t1", "t5"],
                "topic": "Release verification and cascade prevention"
            }
        ]
    });
    let ep_res = handle_submit_episodes(&state, episodes_arg, Some(&auth))
        .await
        .expect("submit_episodes should succeed");
    assert_eq!(ep_res["episodes_accepted"], 1);

    // 4. Submit goal candidates with verbatim quotes
    let goals_arg = json!({
        "session_id": session_id,
        "goals": [
            {
                "id": "G1",
                "kind": "ACHIEVE",
                "statement": "Release engineers independently verify that all dependent services are healthy and stable before initiating any production rollout.",
                "stakeholder": "Priya Sharma",
                "status": "CONFIRMED",
                "episode": "E1",
                "grounding": [
                    {
                        "turn": "t5",
                        "quote": "Release engineers can independently verify that all dependent services are healthy and stable before initiating any production rollout."
                    }
                ]
            },
            {
                "id": "G2",
                "kind": "AVOID",
                "statement": "Outages cascade across services and cause customer transactions to fail during manual rollbacks.",
                "stakeholder": "Priya Sharma",
                "status": "CONFIRMED",
                "episode": "E1",
                "grounding": [
                    {
                        "turn": "t3",
                        "quote": "Outages cascade across services and we spend hours manually rolling back changes while customer transactions fail."
                    }
                ]
            }
        ]
    });

    let goals_res = handle_submit_goal_candidates(&state, goals_arg, Some(&auth))
        .await
        .expect("submit_goal_candidates should succeed");
    assert_eq!(goals_res["goals_accepted"], 2);

    // 5. Query gap report
    let gap_res = handle_get_gap_report(&state, json!({ "session_id": session_id }), Some(&auth))
        .await
        .expect("get_gap_report should succeed");

    assert!(gap_res.get("gaps").is_some());

    // 6. Stage goals atomically
    let stage_res = handle_stage_goals(&state, json!({ "session_id": session_id }), Some(&auth))
        .await
        .expect("stage_goals should succeed");

    assert_eq!(stage_res["staged_goals_count"], 2);
    assert_eq!(stage_res["status"], "STAGED");
}
