//! Streaming: commit after the first event, fall back before it.

mod common;

use common::*;
use futures_util::StreamExt;
use serde_json::json;
use tierfall::ProviderKind::{Openai, Openrouter};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn mock_stream(server: &MockServer, response: ResponseTemplate, times: u64) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({"stream": true})))
        .respond_with(response)
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test]
async fn streams_chunks_and_records_usage() {
    let a = MockServer::start().await;
    mock_stream(
        &a,
        sse_response(sse(&["Hel", "lo", "!"], Some(42), true)),
        1,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![account("a", Openai, &a.uri(), &["m-a"])],
    ))
    .await;
    let stream = tf.chat_stream(request("auto")).await.unwrap();
    assert_eq!(stream.served_by().account, "a");
    assert!(stream.attempts().is_empty());
    assert_eq!(stream.collect_text().await.unwrap(), "Hello!");
    let model = &tf.status().accounts[0].models[0];
    assert_eq!((model.requests_today, model.tokens_today), (1, 42));
}

#[tokio::test]
async fn falls_back_before_the_first_event() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    mock_stream(
        &a,
        json_response(429, json!({"error": {"message": "Rate limit reached"}}))
            .insert_header("retry-after", "10"),
        1,
    )
    .await;
    mock_stream(&b, sse_response(sse(&["from ", "b"], None, true)), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("a", Openai, &a.uri(), &["m-a"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    ))
    .await;
    let stream = tf.chat_stream(request("auto")).await.unwrap();
    assert_eq!(stream.served_by().account, "b");
    assert_eq!(stream.attempts()[0].outcome, "rate_limited");
    assert_eq!(stream.collect_text().await.unwrap(), "from b");
}

#[tokio::test]
async fn error_as_first_event_triggers_fallback() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    let error =
        json!({"error": {"code": 429, "message": "Rate limit exceeded: free-models-per-day"}});
    mock_stream(
        &a,
        sse_response(format!(": OPENROUTER PROCESSING\n\ndata: {error}\n\n")),
        1,
    )
    .await;
    mock_stream(&b, sse_response(sse(&["fine"], None, false)), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("or", Openrouter, &a.uri(), &["x/y:free"]),
            account("b", Openrouter, &b.uri(), &["x/z:free"]),
        ],
    ))
    .await;
    let stream = tf.chat_stream(request("auto")).await.unwrap();
    assert_eq!(stream.served_by().account, "b");
    assert_eq!(stream.attempts()[0].outcome, "exhausted");
    assert_eq!(stream.collect_text().await.unwrap(), "fine");
    assert_eq!(tf.status().accounts[0].models[0].availability, "exhausted");
}

#[tokio::test]
async fn empty_stream_is_transient() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    mock_stream(&a, sse_response(String::new()), 1).await;
    mock_stream(&b, sse_response(sse(&["ok"], None, true)), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![
            account("a", Openai, &a.uri(), &["m-a"]),
            account("b", Openai, &b.uri(), &["m-b"]),
        ],
    ))
    .await;
    let stream = tf.chat_stream(request("auto")).await.unwrap();
    assert_eq!(stream.attempts()[0].outcome, "transient");
    assert_eq!(stream.collect_text().await.unwrap(), "ok");
}

#[tokio::test]
async fn raw_events_preserve_the_wire_stream() {
    let a = MockServer::start().await;
    mock_stream(&a, sse_response(sse(&["x", "y"], None, true)), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![account("a", Openai, &a.uri(), &["m-a"])],
    ))
    .await;
    let (served, attempts, events) = tf.chat_stream(request("auto")).await.unwrap().into_events();
    assert_eq!(served.model, "m-a");
    assert!(attempts.is_empty());
    let events: Vec<_> = events.map(Result::unwrap).collect().await;
    assert_eq!(events.len(), 3);
    assert!(events[2].is_done());
}

#[tokio::test]
async fn mid_stream_errors_surface_as_errors() {
    let a = MockServer::start().await;
    let chunk = json!({"id": "c", "object": "chat.completion.chunk", "created": 1, "model": "m",
        "choices": [{"index": 0, "delta": {"content": "part"}}]});
    let error = json!({"error": {"code": 502, "message": "provider disconnected"}});
    mock_stream(
        &a,
        sse_response(format!("data: {chunk}\n\ndata: {error}\n\n")),
        1,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let tf = build(base_config(
        dir.path(),
        vec![account("a", Openai, &a.uri(), &["m-a"])],
    ))
    .await;
    let mut stream = tf.chat_stream(request("auto")).await.unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap().text(), Some("part"));
    let err = stream.next().await.unwrap().unwrap_err();
    assert!(err.to_string().contains("provider disconnected"), "{err}");
    assert!(stream.next().await.is_none());
    assert_eq!(tf.status().accounts[0].models[0].availability, "cooldown");
}
