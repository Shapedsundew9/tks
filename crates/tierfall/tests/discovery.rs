//! Live model discovery, ranking, caching and fallbacks against mock `/models` endpoints.

mod common;

use common::*;
use serde_json::json;
use tierfall::ProviderKind::{Gemini, Groq, Openrouter};
use tierfall::config::Env;
use tierfall::discovery::DiscoverySource;
use tierfall::{Config, Tierfall};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gemini_models() -> serde_json::Value {
    let ids = [
        "gemini-2.5-flash",
        "gemini-3.8-flash",
        "gemini-3.6-flash",
        "gemini-3.5-flash-lite",
        "gemini-2.5-flash-lite",
        "gemini-3-pro-preview",
        "gemini-embedding-001",
        "gemma-4-31b-it",
        "gemini-flash-latest",
        "imagen-4.0-generate-001",
    ];
    json!({"object": "list", "data": ids.iter().map(|id| json!({"id": format!("models/{id}"), "object": "model", "owned_by": "google"})).collect::<Vec<_>>()})
}

async fn mock_models(server: &MockServer, prefix: &str, response: ResponseTemplate, times: u64) {
    Mock::given(method("GET"))
        .and(path(format!("{prefix}/models")))
        .respond_with(response)
        .expect(times)
        .mount(server)
        .await;
}

fn discovered(
    id: &str,
    provider: tierfall::ProviderKind,
    uri: &str,
) -> tierfall::config::AccountConfig {
    account(id, provider, uri, &[])
}

#[tokio::test]
async fn ranks_quality_first_across_accounts() {
    let (gem, groq, or) = (
        MockServer::start().await,
        MockServer::start().await,
        MockServer::start().await,
    );
    mock_models(&gem, "", json_response(200, gemini_models()), 2).await;
    mock_models(
        &groq,
        "",
        json_response(
            200,
            json!({"object": "list", "data": [
                {"id": "openai/gpt-oss-120b", "context_window": 131072, "active": true},
                {"id": "openai/gpt-oss-20b", "context_window": 131072, "active": true},
                {"id": "whisper-large-v3", "active": true},
                {"id": "qwen/qwen3-32b", "active": false}
            ]}),
        ),
        1,
    )
    .await;
    mock_models(
        &or,
        "",
        json_response(200, json!({"data": [
            {"id": "nvidia/nemotron-3-ultra-550b-a55b:free", "context_length": 1000000},
            {"id": "openrouter/free", "pricing": {"prompt": "0", "completion": "0"}},
            {"id": "anthropic/claude-x", "pricing": {"prompt": "0.00001", "completion": "0.00002"}}
        ]})),
        1,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            discovered("gemini-ss", Gemini, &gem.uri()),
            discovered("gemini-am", Gemini, &gem.uri()),
            discovered("groq", Groq, &groq.uri()),
            discovered("openrouter", Openrouter, &or.uri()),
        ],
    ))
    .await;
    let auto: Vec<String> = tf.routes()["auto"]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        auto,
        [
            "gemini-3.8-flash@gemini-ss",
            "gemini-3.8-flash@gemini-am",
            "gemini-3.6-flash@gemini-ss",
            "gemini-3.6-flash@gemini-am",
            "openai/gpt-oss-120b@groq",
            "nvidia/nemotron-3-ultra-550b-a55b:free@openrouter",
            "gemini-3.5-flash-lite@gemini-ss",
            "gemini-3.5-flash-lite@gemini-am",
            "gemini-2.5-flash-lite@gemini-ss",
            "gemini-2.5-flash-lite@gemini-am",
            "openai/gpt-oss-20b@groq",
            "gemma-4-31b-it@gemini-ss",
            "gemma-4-31b-it@gemini-am",
            "openrouter/free@openrouter",
        ]
    );
    let fast: Vec<String> = tf.routes()["fast"]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        &fast[..3],
        [
            "openai/gpt-oss-120b@groq",
            "openai/gpt-oss-20b@groq",
            "gemini-3.5-flash-lite@gemini-ss"
        ]
    );
    // Listed but unranked models remain routable by name.
    let (_, chain) = tf.resolve("gemini-2.5-flash").unwrap();
    assert_eq!(chain.len(), 2);
    let models = tf.models();
    let ids: Vec<&str> = models["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(&ids[..3], ["auto", "fast", "gemini-3.8-flash"]);
    assert!(
        tf.catalogs()
            .values()
            .all(|c| c.source == DiscoverySource::Live)
    );
}

#[tokio::test]
async fn discovery_failure_uses_static_defaults() {
    let gem = MockServer::start().await;
    mock_models(&gem, "", ResponseTemplate::new(503), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![discovered("gemini-ss", Gemini, &gem.uri())],
    ))
    .await;
    let catalog = &tf.catalogs()["gemini-ss"];
    assert_eq!(catalog.source, DiscoverySource::Static);
    assert!(catalog.error.as_deref().unwrap().contains("503"));
    let auto: Vec<String> = tf.routes()["auto"]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        auto,
        [
            "gemini-flash-latest@gemini-ss",
            "gemini-flash-lite-latest@gemini-ss"
        ]
    );
}

#[tokio::test]
async fn rejected_key_during_discovery_disables_account() {
    let (gem, groq) = (MockServer::start().await, MockServer::start().await);
    let body = json!([{"error": {"code": 400, "message": "API key not valid. Please pass a valid API key.", "status": "INVALID_ARGUMENT"}}]);
    mock_models(&gem, "", json_response(400, body), 1).await;
    mock_models(
        &groq,
        "",
        json_response(200, json!({"data": [{"id": "openai/gpt-oss-120b"}]})),
        1,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            discovered("gemini-ss", Gemini, &gem.uri()),
            discovered("groq", Groq, &groq.uri()),
        ],
    ))
    .await;
    let status = tf.status();
    assert!(
        status.accounts[0]
            .disabled
            .as_deref()
            .unwrap()
            .contains("API key not valid")
    );
    let auto: Vec<String> = tf.routes()["auto"]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(auto, ["openai/gpt-oss-120b@groq"]);
}

#[tokio::test]
async fn cache_is_reused_within_ttl_and_refresh_forces_fetch() {
    let groq = MockServer::start().await;
    mock_models(
        &groq,
        "",
        json_response(200, json!({"data": [{"id": "openai/gpt-oss-120b"}]})),
        2,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let accounts = vec![discovered("groq", Groq, &groq.uri())];
    {
        let tf = build(base_config(dir.path(), accounts.clone())).await;
        assert_eq!(tf.catalogs()["groq"].source, DiscoverySource::Live);
    }
    assert!(dir.path().join("models.json").is_file());
    let tf = build(base_config(dir.path(), accounts)).await;
    assert_eq!(tf.catalogs()["groq"].source, DiscoverySource::Cache);
    tf.refresh_models().await;
    assert_eq!(tf.catalogs()["groq"].source, DiscoverySource::Live);
}

#[tokio::test]
async fn ollama_is_last_resort_and_optional() {
    let (groq, ollama) = (MockServer::start().await, MockServer::start().await);
    mock_models(
        &groq,
        "",
        json_response(200, json!({"data": [{"id": "openai/gpt-oss-20b"}]})),
        1,
    )
    .await;
    mock_models(
        &ollama,
        "/v1",
        json_response(
            200,
            json!({"object": "list", "data": [
                {"id": "llama3.2:3b"}, {"id": "qwen3:8b"}, {"id": "nomic-embed-text:latest"}
            ]}),
        ),
        1,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = base_config(dir.path(), vec![discovered("groq", Groq, &groq.uri())]);
    cfg.router.include_ollama = true;
    let env = Env::from_pairs([("KEY_GROQ", "k".to_string()), ("OLLAMA_HOST", ollama.uri())]);
    let tf = Tierfall::with_env(cfg.clone(), env).await.unwrap();
    let auto: Vec<String> = tf.routes()["auto"]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        auto,
        [
            "openai/gpt-oss-20b@groq",
            "qwen3:8b@ollama",
            "llama3.2:3b@ollama"
        ]
    );

    // Unreachable Ollama: no error, simply absent from routes.
    let env = Env::from_pairs([("KEY_GROQ", "k"), ("OLLAMA_HOST", "http://127.0.0.1:9")]);
    let tf = Tierfall::with_env(cfg, env).await.unwrap();
    assert_eq!(tf.catalogs()["ollama"].source, DiscoverySource::Failed);
    assert_eq!(tf.routes()["auto"].len(), 1);
}

#[tokio::test]
async fn auto_detects_accounts_from_env_in_requested_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.router.discovery = false;
    cfg.router.include_ollama = false;
    cfg.router.state_dir = Some(dir.path().to_path_buf());
    let env = Env::from_pairs([
        ("GEMINI_API_KEY_JB", "k1"),
        ("GEMINI_API_KEY_AM", "k2"),
        ("GEMINI_API_KEY_SS", "k3"),
        ("GEMINI_API_KEY", ""),
        ("GROQ_API_KEY", "k4"),
        ("TIERFALL_ACCOUNT_ORDER", "gemini-ss,gemini-am"),
    ]);
    let tf = Tierfall::with_env(cfg, env).await.unwrap();
    let ids: Vec<&str> = tf.accounts().iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["gemini-ss", "gemini-am", "gemini-jb", "groq"]);
    let auto: Vec<String> = tf.routes()["auto"]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        &auto[..4],
        [
            "gemini-flash-latest@gemini-ss",
            "gemini-flash-latest@gemini-am",
            "gemini-flash-latest@gemini-jb",
            "openai/gpt-oss-120b@groq"
        ]
    );
    assert!(format!("{tf:?}").contains("<redacted>"));
    assert!(!format!("{tf:?}").contains("k3"));
}
