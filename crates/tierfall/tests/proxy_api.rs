//! The OpenAI-compatible HTTP proxy end to end: real axum server, mock upstreams.

mod common;

use common::*;
use serde_json::{Value, json};
use tierfall::ProviderKind::Openai;
use tierfall::Tierfall;
use tierfall::config::Env;
use tokio::sync::oneshot;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer};

struct Proxy {
    base: String,
    http: reqwest::Client,
    stop: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Proxy {
    async fn start(tf: Tierfall) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (stop, rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            tierfall::server::serve_with_listener(tf, listener, async {
                let _ = rx.await;
            })
            .await
            .unwrap();
        });
        Self {
            base,
            http: reqwest::Client::new(),
            stop: Some(stop),
            task: Some(task),
        }
    }

    async fn chat(&self, body: Value) -> reqwest::Response {
        self.http
            .post(format!("{}/v1/chat/completions", self.base))
            .json(&body)
            .send()
            .await
            .unwrap()
    }

    async fn shutdown(mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.task.take().unwrap().await.unwrap();
    }
}

async fn mock_chat(server: &MockServer, response: wiremock::ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(response)
        .mount(server)
        .await;
}

fn header<'a>(resp: &'a reqwest::Response, name: &str) -> &'a str {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
}

fn ping(model: &str) -> Value {
    json!({"model": model, "messages": [{"role": "user", "content": "ping"}]})
}

#[tokio::test]
async fn non_streaming_completion_with_fallback_headers() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    mock_chat(
        &a,
        json_response(429, json!({"error": {"message": "Rate limit reached"}})),
    )
    .await;
    let mut upstream = completion("m-b", "pong");
    upstream["system_fingerprint"] = json!("fp_x");
    mock_chat(&b, json_response(200, upstream.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("a", Openai, &a.uri(), &["m-a"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    ))
    .await;
    let proxy = Proxy::start(tf).await;
    let resp = proxy.chat(ping("auto")).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(header(&resp, "x-tierfall-account"), "b");
    assert_eq!(header(&resp, "x-tierfall-provider"), "openai");
    assert_eq!(header(&resp, "x-tierfall-model"), "m-b");
    assert_eq!(header(&resp, "x-tierfall-attempts"), "2");
    // The upstream body is passed through verbatim.
    assert_eq!(resp.json::<Value>().await.unwrap(), upstream);
    proxy.shutdown().await;
    // Shutdown flushes quota state.
    assert!(dir.path().join("state.json").is_file());
}

#[tokio::test]
async fn streaming_appends_done_when_upstream_omits_it() {
    let a = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(
            json!({"stream": true, "temperature": 0.2}),
        ))
        .respond_with(sse_response(sse(&["po", "ng"], None, false)))
        .expect(1)
        .mount(&a)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![account("a", Openai, &a.uri(), &["m-a"])],
    ))
    .await;
    let proxy = Proxy::start(tf).await;
    let mut body = ping("m-a");
    body["stream"] = json!(true);
    body["temperature"] = json!(0.2);
    let resp = proxy.chat(body).await;
    assert_eq!(resp.status(), 200);
    assert!(header(&resp, "content-type").starts_with("text/event-stream"));
    assert_eq!(header(&resp, "x-tierfall-model"), "m-a");
    let text = resp.text().await.unwrap();
    let data: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .collect();
    assert_eq!(data.len(), 3, "{text}");
    assert_eq!(
        serde_json::from_str::<Value>(data[0]).unwrap()["choices"][0]["delta"]["content"],
        "po"
    );
    assert_eq!(data[2], "[DONE]");
    proxy.shutdown().await;
}

#[tokio::test]
async fn models_and_status_endpoints() {
    let a = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![account("a", Openai, &a.uri(), &["m-a", "m-a2"])],
    ))
    .await;
    let proxy = Proxy::start(tf).await;
    for route in ["/v1/models", "/models"] {
        let models: Value = proxy
            .http
            .get(format!("{}{route}", proxy.base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(models["object"], "list");
        let ids: Vec<&str> = models["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["auto", "fast", "m-a", "m-a2"]);
    }
    let status: Value = proxy
        .http
        .get(format!("{}/status", proxy.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["default_route"], "auto");
    assert_eq!(status["accounts"][0]["id"], "a");
    assert_eq!(status["accounts"][0]["models"][0]["availability"], "ready");
    assert!(!status.to_string().contains("test-key"));
    proxy.shutdown().await;
}

#[tokio::test]
async fn api_key_is_enforced_when_configured() {
    let a = MockServer::start().await;
    mock_chat(&a, json_response(200, completion("m-a", "ok"))).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = base_config(dir.path(), vec![account("a", Openai, &a.uri(), &["m-a"])]);
    let env = Env::from_pairs([("KEY_A", "upstream"), ("TIERFALL_API_KEY", "s3cret")]);
    let proxy = Proxy::start(Tierfall::with_env(cfg, env).await.unwrap()).await;
    let url = format!("{}/v1/chat/completions", proxy.base);

    let resp = proxy
        .http
        .post(&url)
        .json(&ping("auto"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_api_key"
    );
    let resp = proxy
        .http
        .post(&url)
        .bearer_auth("wrong")
        .json(&ping("auto"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    let resp = proxy
        .http
        .post(&url)
        .bearer_auth("s3cret")
        .json(&ping("auto"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let resp = proxy
        .http
        .post(&url)
        .header("x-api-key", "s3cret")
        .json(&ping("auto"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let resp = proxy
        .http
        .get(format!("{}/v1/models", proxy.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    let resp = proxy
        .http
        .get(format!("{}/health", proxy.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    proxy.shutdown().await;
}

#[tokio::test]
async fn error_mapping() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    mock_chat(
        &a,
        json_response(429, json!({"error": {"message": "Rate limit reached"}}))
            .insert_header("retry-after", "30"),
    )
    .await;
    mock_chat(
        &b,
        json_response(400, json!({"error": {"message": "'messages' must not be empty", "type": "invalid_request_error"}})),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("a", Openai, &a.uri(), &["m-a"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    ))
    .await;
    let proxy = Proxy::start(tf).await;

    // All exhausted → 429 with Retry-After and the attempt log.
    let resp = proxy.chat(ping("m-a")).await;
    assert_eq!(resp.status(), 429);
    let retry: u64 = header(&resp, "retry-after").parse().unwrap();
    assert!((29..=31).contains(&retry), "{retry}");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["type"], "all_providers_exhausted");
    assert_eq!(body["error"]["attempts"][0]["outcome"], "rate_limited");

    // Upstream client errors pass through with their status.
    let resp = proxy.chat(ping("m-b")).await;
    assert_eq!(resp.status(), 400);
    assert_eq!(header(&resp, "x-tierfall-account"), "b");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["message"], "'messages' must not be empty");
    assert_eq!(body["error"]["tierfall_served_by"]["account"], "b");

    let resp = proxy.chat(ping("no-such-model")).await;
    assert_eq!(resp.status(), 404);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["code"],
        "model_not_found"
    );

    let resp = proxy
        .http
        .post(format!("{}/v1/chat/completions", proxy.base))
        .header("content-type", "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["type"],
        "invalid_request_error"
    );

    let resp = proxy.chat(json!([1, 2])).await;
    assert_eq!(resp.status(), 400);
    proxy.shutdown().await;
}
