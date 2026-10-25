//! OpenAI-compatible HTTP proxy (`POST /v1/chat/completions`, `GET /v1/models`, `GET /status`).

use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::net::TcpListener;

use crate::error::{Error, Result};
use crate::router::{ChatStream, Tierfall};
use crate::sse::SseEvent;
use crate::types::ServedBy;

const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// Builds the axum router. `/health` is always unauthenticated.
pub fn router(tf: Tierfall) -> Router {
    let key: Option<Arc<String>> = tf.server_api_key().map(Arc::new);
    Router::new()
        .route("/v1/chat/completions", post(chat))
        .route("/chat/completions", post(chat))
        .route("/v1/models", get(models))
        .route("/models", get(models))
        .route("/status", get(status))
        .route("/v1/status", get(status))
        .layer(middleware::from_fn_with_state(key, auth))
        .route("/health", get(health))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(tf)
}

/// Binds `bind` and serves until Ctrl-C / SIGTERM, then flushes quota state.
pub async fn serve(tf: Tierfall, bind: &str) -> Result<()> {
    let listener = TcpListener::bind(bind).await?;
    serve_with_listener(tf, listener, shutdown_signal()).await
}

/// Serves on an existing listener until `shutdown` resolves.
pub async fn serve_with_listener(
    tf: Tierfall,
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let addr = listener.local_addr()?;
    if tf.server_api_key().is_none() && !addr.ip().is_loopback() {
        tracing::warn!(
            target: "tierfall",
            "listening on non-loopback {addr} without authentication; set {} to require a key",
            tf.config().server.api_key_env
        );
    }
    tracing::info!(target: "tierfall", "OpenAI-compatible API on http://{addr}/v1");
    axum::serve(listener, router(tf.clone()))
        .with_graceful_shutdown(shutdown)
        .await?;
    tf.flush_state()?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!(target: "tierfall", "shutting down");
}

async fn auth(State(key): State<Option<Arc<String>>>, request: Request, next: Next) -> Response {
    if let Some(expected) = key {
        let presented = bearer(request.headers());
        if !presented.is_some_and(|p| constant_time_eq(p.as_bytes(), expected.as_bytes())) {
            return error_response(
                StatusCode::UNAUTHORIZED,
                json!({"message": "invalid or missing API key", "type": "invalid_request_error", "code": "invalid_api_key"}),
            );
        }
    }
    next.run(request).await
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    if let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let (scheme, token) = value.split_once(' ')?;
        return scheme.eq_ignore_ascii_case("bearer").then(|| token.trim());
    }
    headers.get("x-api-key").and_then(|v| v.to_str().ok())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn health() -> Response {
    axum::Json(json!({"status": "ok"})).into_response()
}

async fn models(State(tf): State<Tierfall>) -> Response {
    axum::Json(tf.models()).into_response()
}

async fn status(State(tf): State<Tierfall>) -> Response {
    axum::Json(tf.status()).into_response()
}

async fn chat(State(tf): State<Tierfall>, body: Bytes) -> Response {
    let map = match serde_json::from_slice::<Value>(&body) {
        Ok(Value::Object(map)) => map,
        Ok(_) => {
            return error_to_response(Error::InvalidRequest(
                "request body must be a JSON object".into(),
            ));
        }
        Err(e) => {
            return error_to_response(Error::InvalidRequest(format!("invalid JSON body: {e}")));
        }
    };
    let stream = map.get("stream").and_then(Value::as_bool).unwrap_or(false);
    if stream {
        match tf.stream_json(map).await {
            Ok(s) => stream_response(s),
            Err(e) => error_to_response(e),
        }
    } else {
        match tf.complete_json(map).await {
            Ok(raw) => {
                let mut resp = (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json")],
                    raw.body,
                )
                    .into_response();
                add_served_headers(resp.headers_mut(), &raw.served_by);
                resp
            }
            Err(e) => error_to_response(e),
        }
    }
}

fn stream_response(stream: ChatStream) -> Response {
    let (served_by, _attempts, events) = stream.into_events();
    let body = futures_util::stream::unfold(
        (events, false, false),
        |(mut events, mut done, finished)| async move {
            if finished {
                return None;
            }
            match events.next().await {
                Some(Ok(event)) => {
                    done |= event.is_done();
                    Some((
                        Ok::<Bytes, Infallible>(event.encode()),
                        (events, done, false),
                    ))
                }
                Some(Err(e)) => {
                    let payload =
                        json!({"error": {"message": e.to_string(), "type": "stream_error"}});
                    let bytes = SseEvent::data(payload.to_string()).encode();
                    Some((Ok(bytes), (events, done, false)))
                }
                None if done => None,
                None => Some((Ok(SseEvent::data("[DONE]").encode()), (events, true, true))),
            }
        },
    );
    let mut resp = Response::new(Body::from_stream(body));
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    add_served_headers(headers, &served_by);
    resp
}

fn add_served_headers(headers: &mut HeaderMap, served_by: &ServedBy) {
    let pairs = [
        ("x-tierfall-account", served_by.account.clone()),
        (
            "x-tierfall-provider",
            served_by.provider.as_str().to_string(),
        ),
        ("x-tierfall-model", served_by.model.clone()),
        ("x-tierfall-attempts", served_by.attempts.to_string()),
    ];
    for (name, value) in pairs {
        if let Ok(v) = HeaderValue::from_str(&value) {
            headers.insert(name, v);
        }
    }
}

fn error_response(status: StatusCode, error: Value) -> Response {
    (status, axum::Json(json!({ "error": error }))).into_response()
}

/// Maps a tierfall error onto an OpenAI-style error response.
#[must_use]
pub fn error_to_response(err: Error) -> Response {
    match err {
        Error::InvalidRequest(message) => error_response(
            StatusCode::BAD_REQUEST,
            json!({"message": message, "type": "invalid_request_error"}),
        ),
        Error::UnknownModel(model) => error_response(
            StatusCode::NOT_FOUND,
            json!({
                "message": format!("unknown model or route '{model}' (see GET /v1/models)"),
                "type": "invalid_request_error",
                "code": "model_not_found",
            }),
        ),
        Error::AllExhausted {
            route,
            retry_after,
            attempts,
        } => {
            let secs = retry_after.map(|d| d.as_secs_f64().ceil().max(1.0) as u64);
            let mut resp = error_response(
                StatusCode::TOO_MANY_REQUESTS,
                json!({
                    "message": format!("all providers for '{route}' are rate-limited or unavailable"),
                    "type": "all_providers_exhausted",
                    "code": "all_providers_exhausted",
                    "retry_after_seconds": secs,
                    "attempts": attempts,
                }),
            );
            if let Some(secs) = secs
                && let Ok(v) = HeaderValue::from_str(&secs.to_string())
            {
                resp.headers_mut().insert(header::RETRY_AFTER, v);
            }
            resp
        }
        Error::Upstream {
            status,
            body,
            served_by,
        } => {
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
            let mut error = upstream_error_object(&body);
            if let Value::Object(map) = &mut error {
                map.insert("tierfall_served_by".into(), json!(served_by));
            }
            let mut resp = error_response(status, error);
            add_served_headers(resp.headers_mut(), &served_by);
            resp
        }
        Error::Config(message) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"message": message, "type": "configuration_error"}),
        ),
        other => error_response(
            StatusCode::BAD_GATEWAY,
            json!({"message": other.to_string(), "type": "upstream_error"}),
        ),
    }
}

fn upstream_error_object(body: &[u8]) -> Value {
    let parsed: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let candidate = match &parsed {
        Value::Array(items) => items.first().and_then(|v| v.get("error")).cloned(),
        _ => parsed.get("error").cloned(),
    };
    match candidate {
        Some(Value::Object(mut map)) => {
            if !map.get("message").is_some_and(Value::is_string) {
                map.insert(
                    "message".into(),
                    Value::String(crate::classify::error_message(body)),
                );
            }
            map.entry("type").or_insert_with(|| json!("upstream_error"));
            Value::Object(map)
        }
        _ => json!({"message": crate::classify::error_message(body), "type": "upstream_error"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_parsing_and_compare() {
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc"),
        );
        assert_eq!(bearer(&h), Some("abc"));
        let mut h = HeaderMap::new();
        h.insert("x-api-key", HeaderValue::from_static("xyz"));
        assert_eq!(bearer(&h), Some("xyz"));
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn normalises_upstream_errors() {
        let gemini =
            br#"[{"error": {"code": 400, "message": "bad thing", "status": "INVALID_ARGUMENT"}}]"#;
        let obj = upstream_error_object(gemini);
        assert_eq!(obj["message"], "bad thing");
        assert_eq!(obj["status"], "INVALID_ARGUMENT");
        let text = upstream_error_object(b"plain failure");
        assert_eq!(text["type"], "upstream_error");
    }
}
