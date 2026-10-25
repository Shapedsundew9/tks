//! Live smoke tests against the real providers configured in this environment.
//!
//! Skipped unless `TIERFALL_LIVE=1`. Uses the process environment (and a `tierfall.toml`
//! if one is discoverable) with a throwaway state directory. Each run spends one request per
//! cloud account plus two on the `auto` route; an account's next-ranked models (up to
//! [`MODELS_PER_ACCOUNT`]) are tried only after a transient failure or quota response.
//! Only actual upstream quota responses count as a soft pass; missing keys, disabled
//! accounts, failed discovery and accounts that only fail transiently do not.
//! `TIERFALL_LIVE_ACCOUNTS` can require a comma-separated set of account IDs, so an
//! incomplete environment cannot silently reduce coverage.

use std::time::Duration;

mod common;

#[cfg(feature = "server")]
use serde_json::Value;
use serde_json::json;
use tierfall::config::{Config, Env, resolve_accounts};
use tierfall::discovery::DiscoverySource;
use tierfall::{ChatRequest, Error, ProviderKind, Target, Tierfall};

const PROMPT: &str = "Reply with exactly one word: pong";

/// Account-pinned models tried per cloud account, in `auto` order. Provider overloads such as
/// Gemini's "high demand" 503 hit a model on every key at once, so a single pinned model
/// cannot distinguish an unhealthy account from a busy model.
const MODELS_PER_ACCOUNT: usize = 3;

fn live() -> bool {
    let on = std::env::var("TIERFALL_LIVE").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("skipping live test (set TIERFALL_LIVE=1 to run)");
    }
    on
}

async fn live_tierfall(dir: &std::path::Path) -> Tierfall {
    let env = Env::from_process();
    let (mut cfg, _) = Config::load(None, &env).expect("config loads");
    cfg.router.state_dir = Some(dir.to_path_buf());
    let expected = cloud_accounts(&cfg, &env).unwrap_or_else(|e| panic!("live preflight: {e}"));
    let tf = Tierfall::with_env(cfg, env)
        .await
        .expect("provider accounts initialize");
    for w in tf.warnings() {
        eprintln!("warning: {w}");
    }
    cloud_targets(&tf, &expected).unwrap_or_else(|e| panic!("live preflight: {e}"));
    tf
}

fn cloud_accounts(cfg: &Config, env: &Env) -> Result<Vec<String>, String> {
    let (accounts, _) = resolve_accounts(cfg, env);
    let cloud: Vec<String> = accounts
        .iter()
        .filter(|a| a.provider != ProviderKind::Ollama)
        .map(|a| a.id.clone())
        .collect();
    let mut required: Vec<&str> = cfg
        .accounts
        .iter()
        .filter(|a| a.enabled && a.provider != ProviderKind::Ollama)
        .map(|a| a.id.as_str())
        .collect();
    if env.is_declared("TIERFALL_LIVE_ACCOUNTS") {
        for id in env.get("TIERFALL_LIVE_ACCOUNTS").unwrap_or("").split(',') {
            let id = id.trim();
            if id.is_empty() {
                return Err(
                    "TIERFALL_LIVE_ACCOUNTS must contain nonempty cloud account IDs".into(),
                );
            }
            required.push(id);
        }
    }
    let missing: Vec<&str> = required
        .into_iter()
        .filter(|id| !cloud.iter().any(|a| a == id))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "required cloud accounts are unavailable: {}. Load their key environment variables \
             (rebuild/reopen the devcontainer if needed); no live requests were sent",
            missing.join(", ")
        ));
    }
    if cloud.is_empty() {
        return Err(
            "no cloud accounts configured; an Ollama-only run is not cloud verification".into(),
        );
    }
    Ok(cloud)
}

/// Per expected account, its first [`MODELS_PER_ACCOUNT`] `auto` targets (never empty).
fn cloud_targets(tf: &Tierfall, expected: &[String]) -> Result<Vec<Vec<Target>>, String> {
    let status = tf.status();
    let routes = tf.routes();
    let auto = routes.get("auto").ok_or("no auto route")?;
    let mut targets = Vec::new();
    for id in expected {
        let account = status
            .accounts
            .iter()
            .find(|a| &a.id == id)
            .ok_or_else(|| format!("{id}: missing account"))?;
        if let Some(reason) = &account.disabled {
            return Err(format!("{id}: disabled ({reason}); check its key"));
        }
        if !matches!(
            account.discovery,
            Some(DiscoverySource::Live | DiscoverySource::Pinned)
        ) || account.discovery_error.is_some()
        {
            return Err(format!(
                "{id}: models were not discovered live or explicitly pinned \
                 (source={:?}, error={:?})",
                account.discovery, account.discovery_error
            ));
        }
        let chain: Vec<Target> = auto
            .iter()
            .filter(|t| &t.account == id)
            .take(MODELS_PER_ACCOUNT)
            .cloned()
            .collect();
        if chain.is_empty() {
            return Err(format!("{id}: no routable models"));
        }
        targets.push(chain);
    }
    Ok(targets)
}

fn quota_only<'a>(attempts: impl IntoIterator<Item = (&'a str, Option<u64>)>) -> bool {
    let mut seen = false;
    for (outcome, status) in attempts {
        if !matches!(
            outcome,
            "rate_limited" | "rate_limited:waited" | "exhausted"
        ) || !matches!(status, Some(200 | 429))
        {
            return false;
        }
        seen = true;
    }
    seen
}

/// True when every attempt was a transient upstream failure (overload, 5xx, timeout).
fn transient_only<'a>(outcomes: impl IntoIterator<Item = &'a str>) -> bool {
    let mut seen = false;
    for outcome in outcomes {
        if outcome != "transient" {
            return false;
        }
        seen = true;
    }
    seen
}

/// A live result that did not fail the test outright.
enum Verdict<T> {
    Answered(T),
    /// A genuine upstream quota response; the completion itself is not verified.
    Quota,
    /// Only transient upstream failures; this verifies nothing.
    Unavailable,
}

/// Classifies a live result; panics on anything other than a completion, a verified quota
/// response or a purely transient upstream failure.
fn verdict<T>(what: &str, result: tierfall::Result<T>) -> Verdict<T> {
    match result {
        Ok(v) => Verdict::Answered(v),
        Err(Error::AllExhausted {
            attempts,
            retry_after,
            ..
        }) if quota_only(
            attempts
                .iter()
                .map(|a| (a.outcome.as_str(), a.status.map(u64::from))),
        ) =>
        {
            eprintln!(
                "{what}: QUOTA VERIFIED, completion not verified \
                 (retry after {retry_after:?}): {attempts:?}"
            );
            Verdict::Quota
        }
        Err(Error::AllExhausted { attempts, .. })
            if transient_only(attempts.iter().map(|a| a.outcome.as_str())) =>
        {
            eprintln!("{what}: transient upstream failure, not verification: {attempts:?}");
            Verdict::Unavailable
        }
        Err(Error::AllExhausted { attempts, .. }) => {
            panic!("{what}: no verified completion or quota response: {attempts:?}")
        }
        Err(e) => panic!("{what}: {e}"),
    }
}

/// Returns the response, or None for verified upstream quota responses; panics otherwise.
fn soft<T>(what: &str, result: tierfall::Result<T>) -> Option<T> {
    match verdict(what, result) {
        Verdict::Answered(v) => Some(v),
        Verdict::Quota => None,
        Verdict::Unavailable => panic!("{what}: no verified completion or quota response"),
    }
}

#[tokio::test]
async fn every_cloud_account_discovers_models_and_answers() {
    if !live() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let tf = live_tierfall(dir.path()).await;
    let status = tf.status();
    let expected: Vec<String> = tf
        .accounts()
        .iter()
        .filter(|a| a.provider != ProviderKind::Ollama)
        .map(|a| a.id.clone())
        .collect();
    let targets = cloud_targets(&tf, &expected).expect("all cloud accounts are routable");
    let mut answered = 0;
    let mut quota_accounts = Vec::new();
    for chain in &targets {
        let account = status
            .accounts
            .iter()
            .find(|a| a.id == chain[0].account)
            .unwrap();
        let models: Vec<&str> = chain.iter().map(|t| t.model.as_str()).collect();
        eprintln!(
            "{}: discovery={:?}, {} models, trying {models:?} in order",
            account.id,
            account.discovery,
            account.models.len()
        );
        let mut quota = false;
        let mut done = false;
        for target in chain {
            let request = ChatRequest::new(target.to_string()).user(PROMPT);
            match verdict(&target.to_string(), tf.chat(request).await) {
                Verdict::Answered(resp) => {
                    let served = resp.served_by.clone().unwrap();
                    assert_eq!(served.account, account.id);
                    assert_eq!(served.provider, account.provider);
                    assert_eq!(served.model, target.model);
                    let text = resp.text().unwrap_or_default();
                    eprintln!("{target}: ANSWERED {:?}", text.trim());
                    assert!(!text.trim().is_empty(), "{target}: no answer text");
                    done = true;
                    break;
                }
                Verdict::Quota => quota = true,
                Verdict::Unavailable => {}
            }
        }
        if done {
            answered += 1;
        } else if quota {
            quota_accounts.push(account.id.clone());
        } else {
            panic!(
                "{}: none of {models:?} answered or returned a quota response; \
                 transient upstream failures are not verification",
                account.id
            );
        }
    }
    eprintln!(
        "{answered}/{} cloud accounts answered; quota verified only (completion not \
         verified): {quota_accounts:?}",
        targets.len()
    );
}

#[tokio::test]
async fn auto_route_streams() {
    if !live() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let tf = live_tierfall(dir.path()).await;
    let request =
        ChatRequest::new("auto").user("Count from 1 to 5 using digits separated by spaces.");
    if let Some(stream) = soft("auto stream", tf.chat_stream(request).await) {
        let served = stream.served_by().clone();
        let attempts = stream.attempts().to_vec();
        let text = tokio::time::timeout(Duration::from_secs(180), stream.collect_text())
            .await
            .expect("stream finishes")
            .expect("stream succeeds");
        eprintln!(
            "served by {}/{} after {attempts:?}: {:?}",
            served.account,
            served.model,
            text.trim()
        );
        assert!(text.contains('3'), "unexpected answer {text:?}");
    }
}

#[tokio::test]
#[cfg(feature = "server")]
async fn proxy_round_trip() {
    if !live() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let tf = live_tierfall(dir.path()).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let key = tf.server_api_key();
    let (stop, rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(tierfall::server::serve_with_listener(tf, listener, async {
        let _ = rx.await;
    }));
    let http = reqwest::Client::new();
    let mut req = http
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({"model": "auto", "messages": [{"role": "user", "content": PROMPT}]}));
    if let Some(key) = key {
        req = req.bearer_auth(key);
    }
    let resp = req.send().await.unwrap();
    let status = resp.status().as_u16();
    let header = |name: &str| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(ToString::to_string)
    };
    let served = header("x-tierfall-account");
    let model = header("x-tierfall-model");
    let calls = header("x-tierfall-attempts");
    let body: Value = resp.json().await.unwrap();
    let _ = stop.send(());
    server.await.unwrap().unwrap();
    match status {
        200 => {
            assert!(served.is_some());
            eprintln!(
                "proxy served by {served:?}/{model:?} after {calls:?} upstream calls: {}",
                body["choices"][0]["message"]["content"]
            );
            assert!(body["choices"][0]["message"].is_object(), "{body}");
        }
        429 => {
            assert_eq!(body["error"]["type"], "all_providers_exhausted");
            let attempts = body["error"]["attempts"].as_array().expect("attempt log");
            assert!(
                quota_only(
                    attempts
                        .iter()
                        .map(|a| (a["outcome"].as_str().unwrap_or(""), a["status"].as_u64()))
                ),
                "proxy: no verified quota response: {body}"
            );
            eprintln!("proxy: QUOTA VERIFIED, completion not verified: {body}");
        }
        other => panic!("proxy returned {other}: {body}"),
    }
}

#[cfg(test)]
mod verification_tests {
    use super::*;
    use common::{account, base_config, build, json_response};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer};

    #[test]
    fn ollama_only_is_not_cloud_verification() {
        assert!(
            cloud_accounts(&Config::default(), &Env::default())
                .unwrap_err()
                .contains("Ollama-only")
        );
    }

    #[test]
    fn required_accounts_cannot_be_silently_skipped() {
        let env = Env::from_pairs([
            ("GEMINI_API_KEY_SS", "test-key"),
            ("GROQ_API_KEY", ""),
            ("TIERFALL_LIVE_ACCOUNTS", "gemini-ss,groq,openrouter"),
        ]);
        let error = cloud_accounts(&Config::default(), &env).unwrap_err();
        assert!(error.contains("groq, openrouter"), "{error}");
        assert!(!error.contains("test-key"));
    }

    #[test]
    fn configured_accounts_with_missing_keys_are_required() {
        let cfg = Config::from_toml_str(
            r#"
            [[accounts]]
            id = "custom"
            provider = "gemini"
            api_key_env = "CUSTOM_KEY"
            "#,
        )
        .unwrap();
        let env = Env::from_pairs([("GROQ_API_KEY", "test-key")]);
        assert!(cloud_accounts(&cfg, &env).unwrap_err().contains("custom"));
    }

    #[test]
    fn all_required_cloud_accounts_are_selected() {
        let env = Env::from_pairs([
            ("GEMINI_API_KEY_SS", "test-key"),
            ("GEMINI_API_KEY_AM", "test-key"),
            ("GEMINI_API_KEY_JB", "test-key"),
            ("GROQ_API_KEY", "test-key"),
            ("OPENROUTER_API_KEY", "test-key"),
            (
                "TIERFALL_LIVE_ACCOUNTS",
                "gemini-ss, gemini-am, gemini-jb, groq, openrouter",
            ),
        ]);
        assert_eq!(cloud_accounts(&Config::default(), &env).unwrap().len(), 5);
    }

    #[test]
    fn empty_required_account_lists_are_rejected() {
        for list in ["", " ", ",", "gemini,", ",gemini"] {
            let env = Env::from_pairs([
                ("GEMINI_API_KEY", "test-key"),
                ("TIERFALL_LIVE_ACCOUNTS", list),
            ]);
            assert!(
                cloud_accounts(&Config::default(), &env).is_err(),
                "{list:?}"
            );
        }
    }

    #[tokio::test]
    async fn failed_cloud_discovery_is_not_hidden_by_a_healthy_account() {
        for (status, body, expected_error) in [
            (
                401,
                json!({"error": {"message": "invalid API key"}}),
                "disabled",
            ),
            (
                503,
                json!({"error": {"message": "unavailable"}}),
                "source=Some(Static)",
            ),
            (
                200,
                json!({"data": [{"id": "gemini-2.5-pro"}]}),
                "no listed model matched",
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/models"))
                .respond_with(json_response(status, body))
                .expect(1)
                .mount(&server)
                .await;
            let dir = tempfile::tempdir().unwrap();
            let tf = build(base_config(
                dir.path(),
                vec![
                    account(
                        "healthy",
                        ProviderKind::Ollama,
                        &server.uri(),
                        &["qwen3:8b"],
                    ),
                    account("cloud", ProviderKind::Gemini, &server.uri(), &[]),
                ],
            ))
            .await;
            let error = cloud_targets(&tf, &["cloud".into()]).unwrap_err();
            assert!(error.contains(expected_error), "{error}");
        }
    }

    #[tokio::test]
    async fn live_discovery_and_explicit_pins_are_verified() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(json_response(
                200,
                json!({"data": [
                    {"id": "gemini-2.5-flash"},
                    {"id": "gemini-2.5-flash-lite"},
                    {"id": "gemma-3-27b-it"},
                    {"id": "gemma-3-12b-it"}
                ]}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let tf = build(base_config(
            dir.path(),
            vec![
                account("discovered", ProviderKind::Gemini, &server.uri(), &[]),
                account(
                    "pinned",
                    ProviderKind::Groq,
                    &server.uri(),
                    &["openai/gpt-oss-120b"],
                ),
            ],
        ))
        .await;
        let targets = cloud_targets(&tf, &["discovered".into(), "pinned".into()]).unwrap();
        let names = |chain: &[Target]| chain.iter().map(ToString::to_string).collect::<Vec<_>>();
        let auto: Vec<Target> = tf.routes()["auto"]
            .iter()
            .filter(|t| t.account == "discovered")
            .take(MODELS_PER_ACCOUNT)
            .cloned()
            .collect();
        assert_eq!(targets[0].len(), MODELS_PER_ACCOUNT);
        assert_eq!(names(&targets[0]), names(&auto));
        assert_eq!(targets[0][0].to_string(), "gemini-2.5-flash@discovered");
        assert_eq!(names(&targets[1]), ["openai/gpt-oss-120b@pinned"]);
    }

    #[test]
    fn only_transient_failures_allow_trying_the_next_model() {
        assert!(transient_only(["transient"]));
        assert!(transient_only(["transient", "transient"]));
        assert!(!transient_only([]));
        for outcome in [
            "rate_limited",
            "exhausted",
            "auth_failed",
            "model_gone",
            "context_too_long",
            "client_error",
            "skipped:disabled",
            "skipped:cooldown",
        ] {
            assert!(!transient_only([outcome]), "{outcome}");
            assert!(!transient_only(["transient", outcome]), "{outcome}");
        }
    }

    #[test]
    fn non_transient_failures_still_fail_the_account() {
        let attempt = |outcome: &str, status| tierfall::Attempt {
            account: "cloud".into(),
            model: "m".into(),
            outcome: outcome.into(),
            status,
            until: None,
        };
        let exhausted = |attempts| Error::AllExhausted {
            route: "m@cloud".into(),
            attempts,
            retry_after: None,
        };
        assert!(matches!(
            verdict::<()>("t", Err(exhausted(vec![attempt("transient", Some(503))]))),
            Verdict::Unavailable
        ));
        assert!(matches!(
            verdict::<()>(
                "q",
                Err(exhausted(vec![attempt("rate_limited", Some(429))]))
            ),
            Verdict::Quota
        ));
        for outcome in ["auth_failed", "model_gone", "skipped:disabled"] {
            let result = std::panic::catch_unwind(|| {
                verdict::<()>(outcome, Err(exhausted(vec![attempt(outcome, Some(401))])))
            });
            assert!(result.is_err(), "{outcome} must fail the run");
        }
        assert!(
            std::panic::catch_unwind(|| {
                soft::<()>("t", Err(exhausted(vec![attempt("transient", Some(503))])))
            })
            .is_err(),
            "a transient-only auto route is not verification"
        );
    }

    #[test]
    fn only_real_quota_responses_are_soft_passes() {
        assert!(quota_only([("rate_limited", Some(429))]));
        assert!(quota_only([("exhausted", Some(429))]));
        assert!(quota_only([("rate_limited", Some(200))]));
        assert!(!quota_only([]));
        for outcome in ["transient", "auth_failed", "model_gone", "skipped:disabled"] {
            assert!(!quota_only([(outcome, Some(503))]), "{outcome}");
            assert!(!quota_only([("rate_limited", Some(429)), (outcome, None)]));
        }
        assert!(!quota_only([("rate_limited", None)]));
        assert!(!quota_only([("rate_limited", Some(503))]));
        assert!(!quota_only([("exhausted", Some(401))]));
    }
}
