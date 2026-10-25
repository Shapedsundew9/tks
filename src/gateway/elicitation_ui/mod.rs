//! Embedded HTML5/JS/CSS assets and route handlers for Intent Elicitation Studio (Track E-A, §6.3, §6.7).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::http::header::{self, HeaderName, HeaderValue};
use axum::response::{IntoResponse, Response};

use crate::elicitation::AsrLockManifest;
use crate::gateway::AppState;

pub const INDEX_HTML: &str = include_str!("index.html");
pub const ELICITATION_JS: &str = include_str!("elicitation.js");
pub const ASR_WORKER_JS: &str = include_str!("asr-worker.js");
pub const AUDIO_WORKLET_JS: &str = include_str!("audio-worklet.js");
pub const ELICITATION_CSS: &str = include_str!("elicitation.css");

/// Security headers middleware applied to elicitation UI and API endpoints (EA-6, D-EA-10).
pub async fn elicitation_security_headers_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        ),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("microphone=(self)"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// Serves the interactive elicitation studio HTML interface (`GET /explorer/elicitation`).
pub async fn serve_elicitation_index() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        INDEX_HTML,
    )
        .into_response()
}

/// Serves the elicitation studio JavaScript logic (`GET /explorer/elicitation/elicitation.js`).
pub async fn serve_elicitation_js() -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        ELICITATION_JS,
    )
        .into_response()
}

/// Serves the ASR streaming WebWorker (`GET /explorer/elicitation/asr-worker.js`).
pub async fn serve_asr_worker_js() -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        ASR_WORKER_JS,
    )
        .into_response()
}

/// Serves the AudioWorklet processor script (`GET /explorer/elicitation/audio-worklet.js`).
pub async fn serve_audio_worklet_js() -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        AUDIO_WORKLET_JS,
    )
        .into_response()
}

/// Serves the elicitation studio CSS stylesheet (`GET /explorer/elicitation/elicitation.css`).
pub async fn serve_elicitation_css() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        ELICITATION_CSS,
    )
        .into_response()
}

/// Serves pre-downloaded ASR assets with cache headers and strict lockfile whitelisting.
pub async fn serve_asr_asset(
    State(state): State<AppState>,
    Path((version, file)): Path<(String, String)>,
) -> Result<Response, StatusCode> {
    let manifest = AsrLockManifest::load_default();
    if version != manifest.version {
        return Err(StatusCode::NOT_FOUND);
    }

    // Strict whitelist: unlisted files and path traversals return 404
    if !manifest.is_file_allowed(&file) {
        return Err(StatusCode::NOT_FOUND);
    }

    let asr_dir = match &state.elicitation.config.asr_assets_dir {
        Some(dir) => dir,
        None => return Err(StatusCode::NOT_FOUND),
    };

    let file_path = asr_dir.join(&file);
    if !file_path.is_file() {
        return Err(StatusCode::NOT_FOUND);
    }

    let file_handle = tokio::fs::File::open(&file_path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let stream = tokio_util::io::ReaderStream::new(file_handle);
    let body = axum::body::Body::from_stream(stream);

    let content_type = if file.ends_with(".wasm") {
        "application/wasm"
    } else if file.ends_with(".js") {
        "application/javascript; charset=utf-8"
    } else if file.ends_with(".txt") {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    };

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(response)
}
