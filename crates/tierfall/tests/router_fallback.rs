//! Fallback behaviour of the router against mock upstreams.

mod common;

use std::time::{Duration, Instant};

use common::*;
use serde_json::json;
use tierfall::ProviderKind::{Gemini, Groq, Openai, Openrouter};
use tierfall::config::LimitConfig;
use tierfall::{Config, Error};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer};

async fn mock_chat(server: &MockServer, response: wiremock::ResponseTemplate, times: u64) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(response)
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test]
async fn minute_rate_limit_falls_back_and_cools_down() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer test-key"))
        .and(body_partial_json(json!({"model": "m-a"})))
        .respond_with(
            json_response(
                429,
                json!({"error": {"message": "Rate limit reached for requests"}}),
            )
            .insert_header("retry-after", "30"),
        )
        .expect(1)
        .mount(&a)
        .await;
    mock_chat(&b, json_response(200, completion("m-b", "hello from b")), 2).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("a", Openai, &a.uri(), &["m-a"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    ))
    .await;

    let reply = tf.chat(request("auto")).await.unwrap();
    assert_eq!(reply.text().as_deref(), Some("hello from b"));
    let served = reply.served_by.unwrap();
    assert_eq!((served.account.as_str(), served.attempts), ("b", 2));

    let raw = tf.chat_raw(request("auto")).await.unwrap();
    assert_eq!(raw.served_by.account, "b");
    assert_eq!(raw.attempts[0].outcome, "skipped:cooldown");
    let status = tf.status();
    assert_eq!(status.accounts[0].models[0].availability, "cooldown");
    assert_eq!(status.accounts[1].models[0].tokens_today, 16);
}

#[tokio::test]
async fn gemini_daily_quota_is_exhausted_and_persisted() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    mock_chat(&a, json_response(429, gemini_daily_quota()), 1).await;
    mock_chat(&b, json_response(200, completion("m-b", "ok")), 2).await;
    let dir = tempfile::tempdir().unwrap();
    let accounts = vec![
        account("gem", Gemini, &a.uri(), &["gemini-3.8-flash"]),
        account("b", Openai, &b.uri(), &["m-b"]),
    ];
    {
        let tf = build(base_config(dir.path(), accounts.clone())).await;
        let raw = tf.chat_raw(request("auto")).await.unwrap();
        assert_eq!(raw.served_by.account, "b");
        assert_eq!(raw.attempts[0].outcome, "exhausted");
        tf.flush_state().unwrap();
    }
    let tf = build(base_config(dir.path(), accounts)).await;
    let gem = &tf.status().accounts[0].models[0];
    assert_eq!(gem.availability, "exhausted");
    assert!(gem.until.unwrap() > chrono::Utc::now() + chrono::Duration::minutes(1));
    let raw = tf.chat_raw(request("auto")).await.unwrap();
    assert_eq!(raw.served_by.account, "b");
    assert_eq!(raw.attempts[0].outcome, "skipped:exhausted");
}

#[tokio::test]
async fn auth_failure_disables_the_whole_account() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    mock_chat(
        &a,
        json_response(401, json!({"error": {"message": "Invalid API Key"}})),
        1,
    )
    .await;
    mock_chat(&b, json_response(200, completion("m-b", "ok")), 2).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("a", Openai, &a.uri(), &["m-a1", "m-a2"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    ))
    .await;
    assert_eq!(
        tf.chat_raw(request("auto"))
            .await
            .unwrap()
            .served_by
            .account,
        "b"
    );
    let raw = tf.chat_raw(request("auto")).await.unwrap();
    assert_eq!(raw.attempts[0].outcome, "skipped:disabled");
    assert!(tf.status().accounts[0].disabled.is_some());
    match tf.chat(request("m-a2@a")).await {
        Err(Error::AllExhausted {
            attempts,
            retry_after,
            ..
        }) => {
            assert_eq!(attempts[0].outcome, "skipped:disabled");
            assert!(retry_after.is_none());
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn gemini_invalid_key_400_disables_account() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    let body = json!([{"error": {"code": 400, "message": "API key not valid. Please pass a valid API key.", "status": "INVALID_ARGUMENT"}}]);
    mock_chat(&a, json_response(400, body), 1).await;
    mock_chat(&b, json_response(200, completion("m-b", "ok")), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("gem", Gemini, &a.uri(), &["gemini-3.8-flash"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    ))
    .await;
    let raw = tf.chat_raw(request("auto")).await.unwrap();
    assert_eq!(raw.attempts[0].outcome, "auth_failed");
    assert!(tf.status().accounts[0].disabled.is_some());
}

#[tokio::test]
async fn client_errors_are_returned_without_fallback() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    let body = json!({"error": {"message": "'messages.0.role' must be one of system, user", "type": "invalid_request_error"}});
    mock_chat(&a, json_response(400, body), 1).await;
    mock_chat(&b, json_response(200, completion("m-b", "ok")), 0).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("a", Openai, &a.uri(), &["m-a"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    ))
    .await;
    match tf.chat(request("auto")).await {
        Err(Error::Upstream {
            status,
            body,
            served_by,
        }) => {
            assert_eq!(status, 400);
            assert_eq!(served_by.account, "a");
            assert!(String::from_utf8_lossy(&body).contains("must be one of"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn context_too_long_falls_back_and_surfaces_when_alone() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    let body = json!({"error": {"message": "This model's maximum context length is 8192 tokens", "code": "context_length_exceeded"}});
    mock_chat(&a, json_response(400, body), 2).await;
    mock_chat(&b, json_response(200, completion("m-b", "long ok")), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("a", Openai, &a.uri(), &["m-a"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    ))
    .await;
    let raw = tf.chat_raw(request("auto")).await.unwrap();
    assert_eq!(
        (
            raw.served_by.account.as_str(),
            raw.attempts[0].outcome.as_str()
        ),
        ("b", "context_too_long")
    );
    // Context errors do not cool the entry down, and a lone context error is returned as-is.
    match tf.chat(request("m-a@a")).await {
        Err(Error::Upstream { status: 400, .. }) => {}
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn server_errors_fall_back_with_cooldown() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    mock_chat(
        &a,
        wiremock::ResponseTemplate::new(503).set_body_string("upstream overloaded"),
        1,
    )
    .await;
    mock_chat(&b, json_response(200, completion("m-b", "ok")), 2).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("a", Openai, &a.uri(), &["m-a"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    ))
    .await;
    assert_eq!(
        tf.chat_raw(request("auto")).await.unwrap().attempts[0].outcome,
        "transient"
    );
    assert_eq!(
        tf.chat_raw(request("auto")).await.unwrap().attempts[0].outcome,
        "skipped:cooldown"
    );
}

#[tokio::test]
async fn unreachable_upstream_is_transient() {
    let b = MockServer::start().await;
    mock_chat(&b, json_response(200, completion("m-b", "ok")), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("a", Openai, "http://127.0.0.1:9", &["m-a"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    ))
    .await;
    let raw = tf.chat_raw(request("auto")).await.unwrap();
    assert_eq!(
        (
            raw.served_by.account.as_str(),
            raw.attempts[0].outcome.as_str()
        ),
        ("b", "transient")
    );
}

#[tokio::test]
async fn missing_model_is_marked_gone() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    let body =
        json!({"error": {"message": "The model `m-a` does not exist", "code": "model_not_found"}});
    mock_chat(&a, json_response(404, body), 1).await;
    mock_chat(&b, json_response(200, completion("m-b", "ok")), 2).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("a", Openai, &a.uri(), &["m-a"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    ))
    .await;
    assert_eq!(
        tf.chat_raw(request("auto")).await.unwrap().attempts[0].outcome,
        "model_gone"
    );
    assert_eq!(
        tf.chat_raw(request("auto")).await.unwrap().attempts[0].outcome,
        "skipped:gone"
    );
}

#[tokio::test]
async fn everything_exhausted_reports_earliest_retry() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    let limited = |secs: &str| {
        json_response(429, json!({"error": {"message": "Too many requests"}}))
            .insert_header("retry-after", secs)
    };
    mock_chat(&a, limited("20"), 1).await;
    mock_chat(&b, limited("40"), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("a", Openai, &a.uri(), &["m-a"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    ))
    .await;
    for expected in ["rate_limited", "skipped:cooldown"] {
        match tf.chat(request("auto")).await {
            Err(Error::AllExhausted {
                route,
                retry_after,
                attempts,
            }) => {
                assert_eq!(route, "auto");
                assert_eq!(attempts.len(), 2);
                assert_eq!(attempts[0].outcome, expected);
                let secs = retry_after.unwrap().as_secs_f64();
                assert!((15.0..=20.5).contains(&secs), "retry_after {secs}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[tokio::test]
async fn proactive_rpm_limit_skips_before_calling() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    mock_chat(&a, json_response(200, completion("m-a", "a")), 1).await;
    mock_chat(&b, json_response(200, completion("m-b", "b")), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = base_config(
        dir.path(),
        vec![
            account("a", Openai, &a.uri(), &["m-a"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    );
    cfg.limits = vec![LimitConfig {
        account: Some("a".into()),
        rpm: Some(1),
        ..LimitConfig::default()
    }];
    let tf = build(cfg).await;
    assert_eq!(
        tf.chat_raw(request("auto"))
            .await
            .unwrap()
            .served_by
            .account,
        "a"
    );
    let raw = tf.chat_raw(request("auto")).await.unwrap();
    assert_eq!(
        (
            raw.served_by.account.as_str(),
            raw.attempts[0].outcome.as_str()
        ),
        ("b", "skipped:limited")
    );
    assert_eq!(tf.status().accounts[0].models[0].rpm, Some(1));
}

#[tokio::test]
async fn short_wait_retries_the_same_entry() {
    let a = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            json_response(429, json!({"error": {"message": "slow down"}}))
                .insert_header("retry-after", "1"),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&a)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(json_response(200, completion("m-a", "after wait")))
        .with_priority(2)
        .mount(&a)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = base_config(dir.path(), vec![account("a", Openai, &a.uri(), &["m-a"])]);
    cfg.router.short_wait_secs = 3;
    let tf = build(cfg).await;
    let started = Instant::now();
    let raw = tf.chat_raw(request("auto")).await.unwrap();
    assert!(started.elapsed() >= Duration::from_millis(900));
    assert_eq!(raw.served_by.attempts, 2);
    assert_eq!(raw.attempts[0].outcome, "rate_limited:waited");
}

#[tokio::test]
async fn openrouter_error_inside_200_is_classified() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    let body = json!({"error": {"code": 429, "message": "Rate limit exceeded: free-models-per-day. Add 10 credits to unlock 1000 free model requests per day"}});
    mock_chat(&a, json_response(200, body), 1).await;
    mock_chat(&b, json_response(200, completion("m-b", "ok")), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("or", Openrouter, &a.uri(), &["x/y:free"]),
            account("b", Openrouter, &b.uri(), &["x/z:free"]),
        ],
    ))
    .await;
    let raw = tf.chat_raw(request("auto")).await.unwrap();
    assert_eq!(
        (
            raw.served_by.account.as_str(),
            raw.attempts[0].outcome.as_str()
        ),
        ("b", "exhausted")
    );
}

#[tokio::test]
async fn groq_remaining_zero_header_is_learned() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    mock_chat(
        &a,
        json_response(200, completion("openai/gpt-oss-120b", "last one"))
            .insert_header("x-ratelimit-remaining-requests", "0")
            .insert_header("x-ratelimit-reset-requests", "2m59.56s"),
        1,
    )
    .await;
    mock_chat(&b, json_response(200, completion("m-b", "ok")), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("groq", Groq, &a.uri(), &["openai/gpt-oss-120b"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    ))
    .await;
    assert_eq!(
        tf.chat(request("auto")).await.unwrap().text().as_deref(),
        Some("last one")
    );
    let raw = tf.chat_raw(request("auto")).await.unwrap();
    assert_eq!(
        (
            raw.served_by.account.as_str(),
            raw.attempts[0].outcome.as_str()
        ),
        ("b", "skipped:exhausted")
    );
}

#[tokio::test]
async fn model_resolution_forms() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(json_response(200, completion("shared", "from a")))
        .mount(&a)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(json_response(200, completion("shared", "from b")))
        .mount(&b)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("a", Openai, &a.uri(), &["shared", "m-a"]),
            account("b", Openai, &b.uri(), &["shared"]),
        ],
    ))
    .await;
    let (_, chain) = tf.resolve("shared").unwrap();
    let chain: Vec<String> = chain.iter().map(ToString::to_string).collect();
    assert_eq!(chain, ["shared@a", "shared@b"]);
    assert_eq!(
        tf.chat(request("shared")).await.unwrap().text().as_deref(),
        Some("from a")
    );
    assert_eq!(
        tf.chat(request("shared@b"))
            .await
            .unwrap()
            .text()
            .as_deref(),
        Some("from b")
    );
    assert_eq!(
        tf.chat(request("")).await.unwrap().text().as_deref(),
        Some("from a")
    );
    assert!(matches!(tf.chat(request("nope")).await, Err(Error::UnknownModel(m)) if m == "nope"));
    assert!(matches!(
        tf.chat(tierfall::ChatRequest::new("auto")).await,
        Err(Error::InvalidRequest(_))
    ));
}

#[tokio::test]
async fn configured_routes_and_default_route() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    mock_chat(&a, json_response(200, completion("m-a", "from a")), 0).await;
    mock_chat(&b, json_response(200, completion("m-b", "from b")), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::from_toml_str(
        r#"
        [router]
        default_route = "cheap"
        [[routes]]
        name = "cheap"
        chain = [{ account = "b", model = "m-b" }, { route = "auto" }]
        "#,
    )
    .unwrap();
    let base = base_config(
        dir.path(),
        vec![
            account("a", Openai, &a.uri(), &["m-a"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    );
    cfg.router.auto_detect = false;
    cfg.router.include_ollama = false;
    cfg.router.state_dir = base.router.state_dir.clone();
    cfg.accounts = base.accounts;
    let tf = build(cfg).await;
    let routes = tf.routes();
    let cheap: Vec<String> = routes["cheap"].iter().map(ToString::to_string).collect();
    assert_eq!(cheap, ["m-b@b", "m-a@a"]);
    assert!(routes.contains_key("auto") && routes.contains_key("fast"));
    assert_eq!(
        tf.chat(request("default")).await.unwrap().text().as_deref(),
        Some("from b")
    );
}

#[tokio::test]
async fn request_parameters_pass_through() {
    let a = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({
            "model": "m-a",
            "temperature": 0.2,
            "tools": [{"type": "function", "function": {"name": "lookup"}}],
            "response_format": {"type": "json_object"},
        })))
        .respond_with(json_response(200, completion("m-a", "{}")))
        .expect(1)
        .mount(&a)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![account("a", Openai, &a.uri(), &["m-a"])],
    ))
    .await;
    let req = request("auto")
        .param("temperature", 0.2)
        .param(
            "tools",
            json!([{"type": "function", "function": {"name": "lookup"}}]),
        )
        .param("response_format", json!({"type": "json_object"}));
    assert_eq!(tf.chat(req).await.unwrap().text().as_deref(), Some("{}"));
}
