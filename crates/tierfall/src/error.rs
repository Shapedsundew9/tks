//! Error types for tierfall.

use std::time::Duration;

use bytes::Bytes;
use serde::Serialize;

use crate::types::ServedBy;

/// Result alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// One step of a routing attempt, recorded for diagnostics (never contains prompt text).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Attempt {
    pub account: String,
    pub model: String,
    /// Short outcome class, e.g. `rate_limited`, `exhausted`, `skipped:cooldown`, `transient`.
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<chrono::DateTime<chrono::Utc>>,
}

/// Errors returned by tierfall.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("unknown model or route '{0}'")]
    UnknownModel(String),

    #[error("all providers exhausted for '{route}' after {} attempt(s)", attempts.len())]
    AllExhausted {
        route: String,
        /// Time until the earliest entry is expected to become available again.
        retry_after: Option<Duration>,
        attempts: Vec<Attempt>,
    },

    #[error("upstream {}/{} rejected the request with HTTP {status}", served_by.account, served_by.model)]
    Upstream {
        status: u16,
        body: Bytes,
        served_by: ServedBy,
    },

    #[error("stream error: {0}")]
    Stream(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Describes a reqwest error with its cause chain, without the request URL.
pub(crate) fn describe_error(e: &reqwest::Error) -> String {
    let mut text = e.to_string();
    if let Some(url) = e.url() {
        text = text.replace(url.as_str(), "<url>");
    }
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}
