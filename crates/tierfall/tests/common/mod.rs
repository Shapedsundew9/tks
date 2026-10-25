//! Shared helpers for tierfall integration tests (mock upstreams via wiremock).
#![allow(dead_code)]

use std::path::Path;

use serde_json::{Value, json};
use tierfall::config::{AccountConfig, Config, Env};
use tierfall::{ProviderKind, Tierfall};

/// An account pinned to `models` that talks to `base_url` (a mock server).
pub fn account(id: &str, provider: ProviderKind, base_url: &str, models: &[&str]) -> AccountConfig {
    AccountConfig {
        id: id.to_string(),
        provider,
        api_key_env: Some(format!("KEY_{}", id.to_ascii_uppercase().replace('-', "_"))),
        base_url: Some(base_url.to_string()),
        models: models.iter().map(ToString::to_string).collect(),
        preferred_models: Vec::new(),
        tier: None,
        drop_params: Vec::new(),
        enabled: true,
    }
}

/// A config with auto-detection and Ollama disabled and state stored under `dir`.
pub fn base_config(dir: &Path, accounts: Vec<AccountConfig>) -> Config {
    let mut cfg = Config::default();
    cfg.router.auto_detect = false;
    cfg.router.include_ollama = false;
    cfg.router.state_dir = Some(dir.to_path_buf());
    cfg.router.request_timeout_secs = 10;
    cfg.accounts = accounts;
    cfg
}

/// Environment with a dummy key for every configured account.
pub fn env_for(cfg: &Config) -> Env {
    Env::from_pairs(
        cfg.accounts
            .iter()
            .filter_map(|a| a.api_key_env.clone())
            .map(|k| (k, "test-key".to_string())),
    )
}

pub async fn build(cfg: Config) -> Tierfall {
    let env = env_for(&cfg);
    Tierfall::with_env(cfg, env).await.expect("tierfall builds")
}

/// A minimal OpenAI chat completion body.
pub fn completion(model: &str, text: &str) -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1,
        "model": model,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": text}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8}
    })
}

/// An SSE body streaming `parts` as content deltas, optionally followed by usage and `[DONE]`.
pub fn sse(parts: &[&str], usage_total: Option<u64>, done: bool) -> String {
    let mut out = String::from(": keep-alive comment\n\n");
    for p in parts {
        let chunk = json!({
            "id": "c", "object": "chat.completion.chunk", "created": 1, "model": "m",
            "choices": [{"index": 0, "delta": {"content": p}, "finish_reason": null}]
        });
        out.push_str(&format!("data: {chunk}\n\n"));
    }
    if let Some(total) = usage_total {
        let chunk = json!({
            "id": "c", "object": "chat.completion.chunk", "created": 1, "model": "m",
            "choices": [], "usage": {"prompt_tokens": 1, "completion_tokens": total - 1, "total_tokens": total}
        });
        out.push_str(&format!("data: {chunk}\n\n"));
    }
    if done {
        out.push_str("data: [DONE]\n\n");
    }
    out
}

pub fn sse_response(body: String) -> wiremock::ResponseTemplate {
    wiremock::ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

pub fn json_response(status: u16, body: Value) -> wiremock::ResponseTemplate {
    wiremock::ResponseTemplate::new(status).set_body_json(body)
}

pub fn request(model: &str) -> tierfall::ChatRequest {
    tierfall::ChatRequest::new(model).user("ping")
}

/// Gemini's daily-quota 429 payload (OpenAI-compat endpoint wraps it in an array).
pub fn gemini_daily_quota() -> Value {
    json!([{"error": {
        "code": 429,
        "message": "You exceeded your current quota. Quota exceeded for metric: generativelanguage.googleapis.com/generate_content_free_tier_requests, limit: 20, model: gemini-3.8-flash",
        "status": "RESOURCE_EXHAUSTED",
        "details": [
            {"@type": "type.googleapis.com/google.rpc.QuotaFailure", "violations": [{
                "quotaMetric": "generativelanguage.googleapis.com/generate_content_free_tier_requests",
                "quotaId": "GenerateRequestsPerDayPerProjectPerModel-FreeTier"
            }]},
            {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "43s"}
        ]
    }}])
}
