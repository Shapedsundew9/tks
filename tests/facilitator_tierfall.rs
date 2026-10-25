//! Isolated facilitator adapter tests: no credentials, discovery, or external calls.

use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use tierfall::config::{AccountConfig, Config, Env};
use tierfall::{ProviderKind, Tierfall};
use tks::elicitation::{
    FacilitatorContext, FacilitatorProvider, TierfallFacilitatorProvider,
    build_facilitator_user_prompt,
};

struct Stub {
    url: String,
    status: Arc<AtomicU16>,
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Stub {
    async fn new(status_code: u16, body: Value) -> Self {
        let status = Arc::new(AtomicU16::new(status_code));
        let hits = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (s, h, r) = (status.clone(), hits.clone(), requests.clone());
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move |Json(request): Json<Value>| {
                let (s, h, r, body) = (s.clone(), h.clone(), r.clone(), body.clone());
                async move {
                    h.fetch_add(1, Ordering::SeqCst);
                    r.lock().unwrap().push(request);
                    (
                        axum::http::StatusCode::from_u16(s.load(Ordering::SeqCst)).unwrap(),
                        Json(body),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            url,
            status,
            hits,
            requests,
            task,
        }
    }
}

fn completion(text: &str) -> Value {
    json!({"choices":[{"message":{"role":"assistant","content":text}}],
        "usage":{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}})
}

async fn client(stubs: &[(&Stub, ProviderKind)]) -> Tierfall {
    let mut cfg = Config::default();
    cfg.router.auto_detect = false;
    cfg.router.include_ollama = false;
    cfg.router.discovery = false;
    cfg.router.persist_state = false;
    cfg.router.short_wait_secs = 0;
    cfg.router.request_timeout_secs = 2;
    cfg.accounts = stubs
        .iter()
        .enumerate()
        .map(|(index, (stub, kind))| AccountConfig {
            id: format!("account-{index}"),
            provider: *kind,
            base_url: Some(format!("{}/v1", stub.url)),
            api_key_env: if *kind == ProviderKind::Ollama {
                None
            } else {
                Some("STUB_KEY".to_string())
            },
            models: vec![format!("model-{index}")],
            preferred_models: Vec::new(),
            tier: Some(if *kind == ProviderKind::Ollama {
                4
            } else {
                (index + 1) as u8
            }),
            drop_params: Vec::new(),
            enabled: true,
        })
        .collect();
    Tierfall::with_env(cfg, Env::from_pairs([("STUB_KEY", "dummy")]))
        .await
        .unwrap()
}

fn context(inspiration: bool) -> FacilitatorContext {
    FacilitatorContext {
        system_prompt: "synthetic system prompt".to_string(),
        session_title: "Synthetic tool library".to_string(),
        is_inspiration: inspiration,
        focus_topic: Some("Loan boundaries".to_string()),
        gap_questions: vec!["Who may extend a loan?".to_string()],
        explored_items: vec!["Loan length".to_string()],
        turns: vec![(
            "stakeholder".to_string(),
            "Members borrow tools.".to_string(),
        )],
    }
}

#[tokio::test]
async fn preserves_context_modes_and_provenance() {
    let stub = Stub::new(
        200,
        completion("  Which member may authorize an extension?  "),
    )
    .await;
    let provider = TierfallFacilitatorProvider {
        client: client(&[(&stub, ProviderKind::Openai)]).await,
        route: "auto".to_string(),
    };
    for inspiration in [false, true] {
        let ctx = context(inspiration);
        let expected_prompt = build_facilitator_user_prompt(&ctx);
        let reply = provider.propose(ctx).await.unwrap();
        assert_eq!(reply.text, "Which member may authorize an extension?");
        assert_eq!(reply.generation.provider, "openai");
        assert_eq!(reply.generation.account.as_deref(), Some("account-0"));
        assert_eq!(reply.generation.model, "model-0");
        assert_eq!(reply.generation.attempts, 1);
        assert_eq!(reply.generation.usage.unwrap().total(), 8);
        let requests = stub.requests.lock().unwrap();
        let request = requests.last().unwrap();
        assert_eq!(request["messages"][0]["content"], "synthetic system prompt");
        assert_eq!(request["messages"][1]["content"], expected_prompt);
        assert!(request.get("options").is_none());
        assert!(request.get("think").is_none());
    }
}

#[tokio::test]
async fn quota_falls_back_to_local_and_shared_client_remembers_cooldown() {
    let hosted = Stub::new(429, json!({"error":{"message":"rate limit exceeded"}})).await;
    let local = Stub::new(200, completion("Who may borrow the tool?")).await;
    let provider = TierfallFacilitatorProvider {
        client: client(&[
            (&hosted, ProviderKind::Openai),
            (&local, ProviderKind::Ollama),
        ])
        .await,
        route: "auto".to_string(),
    };
    let first = provider.propose(context(false)).await.unwrap();
    assert_eq!(first.generation.provider, "ollama");
    assert_eq!(first.generation.attempts, 2);
    let second = provider.clone().propose(context(true)).await.unwrap();
    assert_eq!(second.generation.provider, "ollama");
    assert_eq!(second.generation.attempts, 1);
    assert_eq!(hosted.hits.load(Ordering::SeqCst), 1);
    assert_eq!(local.hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn transient_failure_falls_back_but_bad_request_does_not() {
    let first = Stub::new(503, json!({"error":{"message":"unavailable"}})).await;
    let second = Stub::new(200, completion("A or B?")).await;
    let provider = TierfallFacilitatorProvider {
        client: client(&[
            (&first, ProviderKind::Openai),
            (&second, ProviderKind::Openai),
        ])
        .await,
        route: "auto".to_string(),
    };
    assert_eq!(
        provider
            .propose(context(false))
            .await
            .unwrap()
            .generation
            .model,
        "model-1"
    );
    first.status.store(400, Ordering::SeqCst);
    let fresh = TierfallFacilitatorProvider {
        client: client(&[
            (&first, ProviderKind::Openai),
            (&second, ProviderKind::Openai),
        ])
        .await,
        route: "auto".to_string(),
    };
    assert!(
        fresh
            .propose(context(false))
            .await
            .unwrap_err()
            .contains("HTTP 400")
    );
    assert_eq!(second.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn rejects_empty_and_non_text_completions() {
    for body in [
        completion(" \n "),
        json!({"choices":[]}),
        json!({"choices":[{"message":{"content":{"unexpected":"value"}}}]}),
        json!({"choices":"invalid"}),
    ] {
        let stub = Stub::new(200, body).await;
        let provider = TierfallFacilitatorProvider {
            client: client(&[(&stub, ProviderKind::Openai)]).await,
            route: "auto".to_string(),
        };
        assert!(provider.propose(context(false)).await.is_err());
    }
}

#[tokio::test]
async fn exhausted_errors_do_not_include_upstream_echoes() {
    let stub = Stub::new(
        503,
        json!({"error":{"message":"sensitive echoed transcript"}}),
    )
    .await;
    let provider = TierfallFacilitatorProvider {
        client: client(&[(&stub, ProviderKind::Openai)]).await,
        route: "auto".to_string(),
    };
    let error = provider.propose(context(false)).await.unwrap_err();
    assert!(error.contains("all providers exhausted"));
    assert!(!error.contains("sensitive"));
}

#[tokio::test]
async fn gateway_capabilities_async_proposals_and_errors() {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tks::elicitation::{
        CreateSessionRequest, ElicitationConfig, ElicitationEvent, ElicitationStore,
    };
    use tower::ServiceExt;

    let database_url = tks::db::ensure_test_database_ready().await.unwrap();
    let pool = tks::db::create_pool(&database_url).unwrap();
    let stub = Stub::new(200, completion("Who may borrow a tool?")).await;
    let router = client(&[(&stub, ProviderKind::Openai)]).await;
    let mut store = ElicitationStore::new(ElicitationConfig::default());
    store.set_tierfall(router).unwrap();
    let session = store
        .create_session(
            CreateSessionRequest {
                title: "Synthetic tool library".to_string(),
                session_id: Some("synthetic-tierfall".to_string()),
                stakeholder_name: "Member".to_string(),
                facilitator_name: "Volunteer".to_string(),
                prior_transcript: None,
                prior_extraction_session_id: None,
            },
            None,
        )
        .await
        .unwrap();
    let mut events = store
        .subscribe_events(session.elicitation_id)
        .await
        .unwrap();
    let git_dir = format!("target/test-tierfall-git-{}", uuid::Uuid::new_v4());
    let (write, read) = tks::storage::git::start_git_actor(&git_dir, 8).unwrap();
    let state = tks::gateway::AppState::new(
        pool.clone(),
        tks::storage::StorageRepo::new(pool),
        write,
        read,
    )
    .with_elicitation(store.clone());
    let app = tks::gateway::create_router(state);
    let capabilities = app
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
    assert_eq!(capabilities.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(capabilities.into_body(), 100_000).await.unwrap())
            .unwrap();
    assert_eq!(
        body["facilitator"],
        json!({"provider":"tierfall","route":"auto","available":true})
    );
    for inspiration in [false, true] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/elicitation/sessions/{}/facilitator/proposals",
                        session.elicitation_id
                    ))
                    .header("host", "localhost")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"mode":if inspiration {"inspiration"} else {"socratic"}})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let event = tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        if inspiration {
            match event {
                ElicitationEvent::FacilitatorError { error } => {
                    assert!(error.contains("exhausted"))
                }
                other => panic!("expected error, got {other:?}"),
            }
        } else {
            let serialized = serde_json::to_value(&event).unwrap();
            assert_eq!(serialized["type"], "facilitator_proposal");
            assert_eq!(serialized["data"]["generation"]["model"], "model-0");
            assert_eq!(serialized["data"]["text"], "Who may borrow a tool?");
            assert!(serialized["data"]["proposal_id"].is_string());
            assert!(serialized["data"]["flags"].is_array());
            stub.status.store(503, Ordering::SeqCst);
        }
    }
    assert!(
        !store
            .get_session(session.elicitation_id)
            .await
            .unwrap()
            .explored_items
            .is_empty()
    );
    let invalid = TierfallFacilitatorProvider {
        client: client(&[(&stub, ProviderKind::Openai)]).await,
        route: "nonexistent-route".to_string(),
    };
    let mut uninitialized = ElicitationStore::new(ElicitationConfig {
        facilitator_route: invalid.route,
        ..Default::default()
    });
    assert!(uninitialized.set_tierfall(invalid.client).is_err());
    assert!(uninitialized.facilitator.is_none());
}
