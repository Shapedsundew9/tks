//! Embedded HTML5/JS/CSS assets and route handlers for Real-Time Substrate Web Explorer (WP-3.4, PHASE3-001).

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

pub const INDEX_HTML: &str = include_str!("index.html");
pub const APP_JS: &str = include_str!("app.js");
pub const STYLE_CSS: &str = include_str!("style.css");
pub const CYTOSCAPE_JS: &str = include_str!("cytoscape.min.js");
pub const DAGRE_JS: &str = include_str!("dagre.min.js");
pub const CYTOSCAPE_DAGRE_JS: &str = include_str!("cytoscape-dagre.min.js");

/// Serves the interactive web explorer HTML interface (`GET /explorer`).
pub async fn serve_explorer_index() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        INDEX_HTML,
    )
        .into_response()
}

/// Serves the web explorer JavaScript application logic (`GET /explorer/app.js`).
pub async fn serve_explorer_js() -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        APP_JS,
    )
        .into_response()
}

/// Serves the web explorer CSS stylesheet (`GET /explorer/style.css`).
pub async fn serve_explorer_css() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLE_CSS,
    )
        .into_response()
}

/// Serves bundled Cytoscape.js core library (`GET /explorer/cytoscape.min.js`).
pub async fn serve_cytoscape() -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        CYTOSCAPE_JS,
    )
        .into_response()
}

/// Serves bundled Dagre layout library (`GET /explorer/dagre.min.js`).
pub async fn serve_dagre() -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        DAGRE_JS,
    )
        .into_response()
}

/// Serves bundled Cytoscape-Dagre layout plugin (`GET /explorer/cytoscape-dagre.min.js`).
pub async fn serve_cytoscape_dagre() -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        CYTOSCAPE_DAGRE_JS,
    )
        .into_response()
}
