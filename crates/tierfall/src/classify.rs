//! Classifies upstream responses into routing outcomes and extracts retry/reset timing.

use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use reqwest::header::HeaderMap;
use serde::Serialize;
use serde_json::Value;

use crate::provider::ProviderKind;

/// Granularity of a rate limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LimitScope {
    /// Short window (per-minute, per-second, token buckets).
    Minute,
    /// Daily quota exhausted.
    Day,
}

/// What a single upstream attempt means for routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Success,
    RateLimited {
        scope: LimitScope,
        until: DateTime<Utc>,
    },
    /// 5xx, timeouts, connection errors: retry elsewhere, cool down briefly.
    Transient {
        reason: String,
    },
    /// Invalid/forbidden key or unusable account: disable the account.
    AuthFailed {
        reason: String,
    },
    /// The model does not exist (any more) on this account.
    ModelGone {
        reason: String,
    },
    /// Input too large for this model: another model may accept it.
    ContextTooLong {
        reason: String,
    },
    /// The request itself is bad: return it to the caller without fallback.
    ClientError {
        reason: String,
    },
}

impl Outcome {
    /// Short machine-readable class name.
    #[must_use]
    pub fn class(&self) -> &'static str {
        match self {
            Outcome::Success => "success",
            Outcome::RateLimited {
                scope: LimitScope::Minute,
                ..
            } => "rate_limited",
            Outcome::RateLimited {
                scope: LimitScope::Day,
                ..
            } => "exhausted",
            Outcome::Transient { .. } => "transient",
            Outcome::AuthFailed { .. } => "auth_failed",
            Outcome::ModelGone { .. } => "model_gone",
            Outcome::ContextTooLong { .. } => "context_too_long",
            Outcome::ClientError { .. } => "client_error",
        }
    }
}

const DEFAULT_MINUTE_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_MINUTE_COOLDOWN: Duration = Duration::from_secs(3600);
const MAX_DAY_COOLDOWN: Duration = Duration::from_secs(26 * 3600);

/// Classifies an HTTP response from `provider`.
#[must_use]
pub fn classify(
    provider: ProviderKind,
    status: u16,
    headers: &HeaderMap,
    body: &[u8],
    now: DateTime<Utc>,
) -> Outcome {
    let payload: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    if (200..300).contains(&status) {
        // OpenRouter (and some proxies) can return 200 with an error payload.
        if payload.get("choices").is_none() && error_object(&payload).is_some() {
            return classify_error_payload(provider, &payload, headers, now);
        }
        return Outcome::Success;
    }
    let text = if payload.is_null() {
        String::from_utf8_lossy(body).into_owned()
    } else {
        String::new()
    };
    classify_status(provider, status, &payload, &text, headers, now)
}

/// Classifies an error payload delivered with HTTP 200 or inside an SSE stream.
#[must_use]
pub fn classify_error_payload(
    provider: ProviderKind,
    payload: &Value,
    headers: &HeaderMap,
    now: DateTime<Utc>,
) -> Outcome {
    let code = error_object(payload)
        .and_then(|e| e.get("code"))
        .and_then(|c| {
            c.as_u64()
                .or_else(|| c.as_str().and_then(|s| s.parse().ok()))
        })
        .and_then(|c| u16::try_from(c).ok())
        // An error object without a usable status is treated as a transient upstream failure.
        .unwrap_or(502);
    classify_status(provider, code, payload, "", headers, now)
}

fn classify_status(
    provider: ProviderKind,
    status: u16,
    payload: &Value,
    raw_text: &str,
    headers: &HeaderMap,
    now: DateTime<Utc>,
) -> Outcome {
    let message = error_message_from(payload, raw_text);
    let lower = message.to_ascii_lowercase();
    let reason = truncate(&message, 300);
    let err_code = error_object(payload)
        .and_then(|e| e.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();

    match status {
        429 => rate_limited(provider, payload, &lower, headers, now),
        401 | 403 => {
            if provider == ProviderKind::Openrouter
                && (lower.contains("moderation") || lower.contains("flagged"))
            {
                Outcome::ClientError { reason }
            } else {
                Outcome::AuthFailed { reason }
            }
        }
        402 => Outcome::AuthFailed {
            reason: format!("payment required: {reason}"),
        },
        404 => Outcome::ModelGone { reason },
        400 | 413 | 422 => {
            if is_auth_message(&lower) {
                Outcome::AuthFailed { reason }
            } else if is_model_gone(&lower, &err_code) {
                Outcome::ModelGone { reason }
            } else if status == 413 || is_context_too_long(&lower, &err_code) {
                Outcome::ContextTooLong { reason }
            } else {
                Outcome::ClientError { reason }
            }
        }
        408 | 409 | 425 | 500..=599 => Outcome::Transient {
            reason: format!("HTTP {status}: {reason}"),
        },
        _ => Outcome::ClientError { reason },
    }
}

fn rate_limited(
    provider: ProviderKind,
    payload: &Value,
    lower_message: &str,
    headers: &HeaderMap,
    now: DateTime<Utc>,
) -> Outcome {
    let quota_ids = gemini_quota_ids(payload).to_ascii_lowercase();
    let daily = quota_ids.contains("perday")
        || lower_message.contains("per day")
        || lower_message.contains("perday")
        || lower_message.contains("per_day")
        || lower_message.contains("-per-day")
        || lower_message.contains("daily")
        || lower_message.contains("limit: 0,")
        || lower_message.contains("limit: 0 ");

    let hinted = gemini_retry_delay(payload)
        .or_else(|| header_retry_after(headers, now))
        .or_else(|| provider_reset_header(provider, headers, now))
        .or_else(|| retry_in_from_message(lower_message));

    if daily {
        let until = match (provider, hinted) {
            (ProviderKind::Gemini, _) | (_, None) => next_daily_reset(provider, now),
            (_, Some(d)) => now + chrono_dur(d.min(MAX_DAY_COOLDOWN)),
        };
        return Outcome::RateLimited {
            scope: LimitScope::Day,
            until,
        };
    }
    let wait = hinted
        .unwrap_or(DEFAULT_MINUTE_COOLDOWN)
        .clamp(Duration::from_millis(250), MAX_MINUTE_COOLDOWN);
    Outcome::RateLimited {
        scope: LimitScope::Minute,
        until: now + chrono_dur(wait),
    }
}

/// Learns limits from headers on a successful response (e.g. Groq remaining-requests = 0).
#[must_use]
pub fn learn_from_success(
    provider: ProviderKind,
    headers: &HeaderMap,
    now: DateTime<Utc>,
) -> Option<(LimitScope, DateTime<Utc>)> {
    match provider {
        ProviderKind::Groq => {
            if header_str(headers, "x-ratelimit-remaining-requests") == Some("0") {
                let reset = header_str(headers, "x-ratelimit-reset-requests")
                    .and_then(parse_duration)
                    .unwrap_or(DEFAULT_MINUTE_COOLDOWN);
                return Some((
                    LimitScope::Day,
                    now + chrono_dur(reset.min(MAX_DAY_COOLDOWN)),
                ));
            }
            if header_str(headers, "x-ratelimit-remaining-tokens") == Some("0") {
                let reset = header_str(headers, "x-ratelimit-reset-tokens")
                    .and_then(parse_duration)
                    .unwrap_or(DEFAULT_MINUTE_COOLDOWN);
                return Some((
                    LimitScope::Minute,
                    now + chrono_dur(reset.min(MAX_MINUTE_COOLDOWN)),
                ));
            }
            None
        }
        ProviderKind::Openrouter => {
            if header_str(headers, "x-ratelimit-remaining") == Some("0") {
                let until = header_str(headers, "x-ratelimit-reset")
                    .and_then(|v| parse_epoch(v, now))
                    .unwrap_or_else(|| now + chrono_dur(DEFAULT_MINUTE_COOLDOWN));
                let scope = if until - now > chrono::Duration::hours(2) {
                    LimitScope::Day
                } else {
                    LimitScope::Minute
                };
                return Some((scope, until));
            }
            None
        }
        _ => None,
    }
}

/// Next midnight in the provider's quota timezone, as UTC.
#[must_use]
pub fn next_daily_reset(provider: ProviderKind, now: DateTime<Utc>) -> DateTime<Utc> {
    let tz = provider.daily_reset_tz();
    let local = now.with_timezone(&tz);
    let next_day = local
        .date_naive()
        .succ_opt()
        .unwrap_or_else(|| local.date_naive());
    let midnight = next_day.and_hms_opt(0, 0, 0).unwrap_or_default();
    tz.from_local_datetime(&midnight).earliest().map_or_else(
        || now + chrono::Duration::hours(24),
        |t| t.with_timezone(&Utc),
    )
}

/// Calendar day (in the provider's quota timezone) for usage accounting.
#[must_use]
pub fn quota_day(provider: ProviderKind, now: DateTime<Utc>) -> String {
    now.with_timezone(&provider.daily_reset_tz())
        .format("%Y-%m-%d")
        .to_string()
}

/// Parses Go/Gemini-style durations: `43s`, `0.5s`, `1m26.4s`, `2h`, `120ms`, or bare seconds.
#[must_use]
pub fn parse_duration(text: &str) -> Option<Duration> {
    let s = text.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(secs) = s.parse::<f64>() {
        return (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs));
    }
    let mut total = 0.0_f64;
    let mut rest = s;
    let mut matched = false;
    while !rest.is_empty() {
        let num_len = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        if num_len == 0 {
            return None;
        }
        let value: f64 = rest[..num_len].parse().ok()?;
        rest = &rest[num_len..];
        let unit_len = rest
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(rest.len());
        let factor = match &rest[..unit_len] {
            "h" => 3600.0,
            "m" => 60.0,
            "s" => 1.0,
            "ms" => 1e-3,
            "us" | "\u{b5}s" => 1e-6,
            "ns" => 1e-9,
            _ => return None,
        };
        rest = &rest[unit_len..];
        total += value * factor;
        matched = true;
    }
    matched.then(|| Duration::from_secs_f64(total))
}

/// Extracts a human-readable error message from a JSON or text body (truncated, no prompt data).
#[must_use]
pub fn error_message(body: &[u8]) -> String {
    let payload: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let text = if payload.is_null() {
        String::from_utf8_lossy(body).into_owned()
    } else {
        String::new()
    };
    truncate(&error_message_from(&payload, &text), 300)
}

fn error_message_from(payload: &Value, raw_text: &str) -> String {
    if let Some(err) = error_object(payload) {
        let mut msg = match err {
            Value::String(s) => s.clone(),
            _ => err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        };
        if let Some(raw) = err
            .get("metadata")
            .and_then(|m| m.get("raw"))
            .and_then(Value::as_str)
        {
            msg.push_str(" | ");
            msg.push_str(raw);
        }
        if !msg.is_empty() {
            return msg;
        }
    }
    if let Some(m) = payload.get("message").and_then(Value::as_str) {
        return m.to_string();
    }
    raw_text.trim().to_string()
}

fn error_object(payload: &Value) -> Option<&Value> {
    let v = match payload {
        Value::Array(items) => items.first()?,
        other => other,
    };
    v.get("error")
}

fn gemini_details(payload: &Value) -> impl Iterator<Item = &Value> {
    error_object(payload)
        .and_then(|e| e.get("details"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn gemini_quota_ids(payload: &Value) -> String {
    let mut ids = String::new();
    for detail in gemini_details(payload) {
        for v in detail
            .get("violations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for key in ["quotaId", "quotaMetric"] {
                if let Some(id) = v.get(key).and_then(Value::as_str) {
                    ids.push_str(id);
                    ids.push(' ');
                }
            }
        }
    }
    ids
}

fn gemini_retry_delay(payload: &Value) -> Option<Duration> {
    gemini_details(payload)
        .find_map(|d| d.get("retryDelay").and_then(Value::as_str))
        .and_then(parse_duration)
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
}

fn header_retry_after(headers: &HeaderMap, now: DateTime<Utc>) -> Option<Duration> {
    let value = header_str(headers, "retry-after")?;
    if let Some(d) = parse_duration(value) {
        return Some(d);
    }
    let at = DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&Utc);
    (at - now).to_std().ok()
}

fn provider_reset_header(
    provider: ProviderKind,
    headers: &HeaderMap,
    now: DateTime<Utc>,
) -> Option<Duration> {
    match provider {
        ProviderKind::Groq => ["x-ratelimit-reset-requests", "x-ratelimit-reset-tokens"]
            .iter()
            .filter_map(|h| header_str(headers, h).and_then(parse_duration))
            .min(),
        ProviderKind::Openrouter => header_str(headers, "x-ratelimit-reset")
            .and_then(|v| parse_epoch(v, now))
            .and_then(|at| (at - now).to_std().ok()),
        _ => None,
    }
}

fn parse_epoch(value: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let n: i64 = value.trim().parse().ok()?;
    let at = if n > 10_000_000_000 {
        Utc.timestamp_millis_opt(n).single()?
    } else {
        Utc.timestamp_opt(n, 0).single()?
    };
    (at > now).then_some(at)
}

fn retry_in_from_message(lower: &str) -> Option<Duration> {
    for marker in ["try again in ", "retry in ", "retry after "] {
        if let Some(idx) = lower.find(marker) {
            let tail = &lower[idx + marker.len()..];
            let token: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.')
                .collect();
            let token = token.trim_end_matches('.');
            if let Some(d) = parse_duration(token) {
                return Some(d);
            }
        }
    }
    None
}

fn is_auth_message(lower: &str) -> bool {
    lower.contains("api key not valid")
        || lower.contains("valid api key")
        || lower.contains("api key expired")
        || lower.contains("api_key_invalid")
        || lower.contains("invalid api key")
        || lower.contains("location is not supported")
}

fn is_model_gone(lower: &str, code: &str) -> bool {
    matches!(code, "model_not_found" | "model_decommissioned")
        || lower.contains("not a valid model")
        || lower.contains("no endpoints found")
        || (lower.contains("model")
            && (lower.contains("not found")
                || lower.contains("does not exist")
                || lower.contains("decommissioned")))
}

fn is_context_too_long(lower: &str, code: &str) -> bool {
    code == "context_length_exceeded"
        || [
            "context length",
            "context_length",
            "context window",
            "maximum context",
            "too many tokens",
            "reduce the length",
            "input token count",
            "exceeds the maximum number of tokens",
            "prompt is too long",
            "request too large",
            "maximum input length",
        ]
        .iter()
        .any(|m| lower.contains(m))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn chrono_dur(d: Duration) -> chrono::Duration {
    chrono::Duration::from_std(d).unwrap_or_else(|_| chrono::Duration::hours(24))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 25, 16, 0, 0).unwrap()
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    fn body(v: &Value) -> Vec<u8> {
        serde_json::to_vec(v).unwrap()
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("43s"), Some(Duration::from_secs(43)));
        assert_eq!(parse_duration("0.5s"), Some(Duration::from_millis(500)));
        assert_eq!(
            parse_duration("1m26.4s"),
            Some(Duration::from_secs_f64(86.4))
        );
        assert_eq!(parse_duration("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_duration("120ms"), Some(Duration::from_millis(120)));
        assert_eq!(parse_duration("7"), Some(Duration::from_secs(7)));
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration(""), None);
    }

    #[test]
    fn gemini_per_minute_uses_retry_delay() {
        let b = json!([{"error": {"code": 429, "status": "RESOURCE_EXHAUSTED",
        "message": "You exceeded your current quota",
        "details": [
            {"@type": "type.googleapis.com/google.rpc.QuotaFailure",
             "violations": [{"quotaId": "GenerateRequestsPerMinutePerProjectPerModel-FreeTier"}]},
            {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "17s"}
        ]}}]);
        let out = classify(
            ProviderKind::Gemini,
            429,
            &HeaderMap::new(),
            &body(&b),
            now(),
        );
        assert_eq!(
            out,
            Outcome::RateLimited {
                scope: LimitScope::Minute,
                until: now() + chrono::Duration::seconds(17)
            }
        );
    }

    #[test]
    fn gemini_per_day_resets_at_pacific_midnight() {
        let b = json!({"error": {"code": 429, "message": "quota",
            "details": [{"violations": [{"quotaId": "GenerateRequestsPerDayPerProjectPerModel-FreeTier"}]},
                        {"retryDelay": "5s"}]}});
        let out = classify(
            ProviderKind::Gemini,
            429,
            &HeaderMap::new(),
            &body(&b),
            now(),
        );
        // 2026-10-25 is PDT (UTC-7): next midnight PT = 2026-10-26T07:00Z.
        assert_eq!(
            out,
            Outcome::RateLimited {
                scope: LimitScope::Day,
                until: Utc.with_ymd_and_hms(2026, 10, 26, 7, 0, 0).unwrap()
            }
        );
    }

    #[test]
    fn pacific_reset_handles_dst_end() {
        // 2026-11-01 is the DST switch; after it PT is UTC-8.
        let t = Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap();
        assert_eq!(
            next_daily_reset(ProviderKind::Gemini, t),
            Utc.with_ymd_and_hms(2026, 11, 2, 8, 0, 0).unwrap()
        );
        assert_eq!(
            next_daily_reset(ProviderKind::Groq, t),
            Utc.with_ymd_and_hms(2026, 11, 2, 0, 0, 0).unwrap()
        );
        assert_eq!(
            quota_day(
                ProviderKind::Gemini,
                Utc.with_ymd_and_hms(2026, 10, 26, 3, 0, 0).unwrap()
            ),
            "2026-10-25"
        );
    }

    #[test]
    fn groq_daily_uses_retry_after_header() {
        let b = json!({"error": {"message": "Rate limit reached for model `x` on requests per day (RPD): Limit 1000, Used 1000. Please try again in 1m26.4s.", "type": "requests", "code": "rate_limit_exceeded"}});
        let out = classify(
            ProviderKind::Groq,
            429,
            &headers(&[("retry-after", "87")]),
            &body(&b),
            now(),
        );
        assert_eq!(
            out,
            Outcome::RateLimited {
                scope: LimitScope::Day,
                until: now() + chrono::Duration::seconds(87)
            }
        );
    }

    #[test]
    fn openrouter_daily_and_upstream() {
        let reset_ms = (now() + chrono::Duration::hours(8))
            .timestamp_millis()
            .to_string();
        let b = json!({"error": {"message": "Rate limit exceeded: free-models-per-day. Add 10 credits", "code": 429}});
        let out = classify(
            ProviderKind::Openrouter,
            429,
            &headers(&[("x-ratelimit-reset", &reset_ms)]),
            &body(&b),
            now(),
        );
        assert_eq!(
            out,
            Outcome::RateLimited {
                scope: LimitScope::Day,
                until: now() + chrono::Duration::hours(8)
            }
        );
        let b = json!({"error": {"message": "Provider returned error", "code": 429, "metadata": {"raw": "x is temporarily rate-limited upstream"}}});
        let out = classify(
            ProviderKind::Openrouter,
            429,
            &HeaderMap::new(),
            &body(&b),
            now(),
        );
        assert!(matches!(
            out,
            Outcome::RateLimited {
                scope: LimitScope::Minute,
                ..
            }
        ));
    }

    #[test]
    fn auth_and_client_errors() {
        let invalid = json!([{"error": {"code": 400, "message": "API key not valid. Please pass a valid API key.", "status": "INVALID_ARGUMENT"}}]);
        assert!(matches!(
            classify(
                ProviderKind::Gemini,
                400,
                &HeaderMap::new(),
                &body(&invalid),
                now()
            ),
            Outcome::AuthFailed { .. }
        ));
        // Exact payloads observed from the live Gemini OpenAI-compat endpoints.
        for live in [
            json!({"error": {"code": 400, "message": "Please pass a valid API key", "status": "INVALID_ARGUMENT"}}),
            json!([{"error": {"code": 400, "message": "Please pass a valid API key", "status": "INVALID_ARGUMENT"}}]),
        ] {
            assert!(matches!(
                classify(
                    ProviderKind::Gemini,
                    400,
                    &HeaderMap::new(),
                    &body(&live),
                    now()
                ),
                Outcome::AuthFailed { .. }
            ));
        }
        assert!(matches!(
            classify(ProviderKind::Groq, 401, &HeaderMap::new(), b"nope", now()),
            Outcome::AuthFailed { .. }
        ));
        let bad =
            json!({"error": {"message": "temperature must be <= 2", "code": "invalid_value"}});
        assert!(matches!(
            classify(
                ProviderKind::Groq,
                400,
                &HeaderMap::new(),
                &body(&bad),
                now()
            ),
            Outcome::ClientError { .. }
        ));
        let flagged = json!({"error": {"message": "input was flagged by moderation", "code": 403}});
        assert!(matches!(
            classify(
                ProviderKind::Openrouter,
                403,
                &HeaderMap::new(),
                &body(&flagged),
                now()
            ),
            Outcome::ClientError { .. }
        ));
    }

    #[test]
    fn context_model_gone_and_transient() {
        let ctx = json!({"error": {"message": "The input token count (2000000) exceeds the maximum number of tokens allowed (1048576)."}});
        assert!(matches!(
            classify(
                ProviderKind::Gemini,
                400,
                &HeaderMap::new(),
                &body(&ctx),
                now()
            ),
            Outcome::ContextTooLong { .. }
        ));
        assert!(matches!(
            classify(ProviderKind::Groq, 413, &HeaderMap::new(), b"{}", now()),
            Outcome::ContextTooLong { .. }
        ));
        let gone = json!({"error": {"message": "The model `x` has been decommissioned", "code": "model_decommissioned"}});
        assert!(matches!(
            classify(
                ProviderKind::Groq,
                400,
                &HeaderMap::new(),
                &body(&gone),
                now()
            ),
            Outcome::ModelGone { .. }
        ));
        assert!(matches!(
            classify(ProviderKind::Gemini, 404, &HeaderMap::new(), b"", now()),
            Outcome::ModelGone { .. }
        ));
        assert!(matches!(
            classify(
                ProviderKind::Gemini,
                503,
                &HeaderMap::new(),
                b"overloaded",
                now()
            ),
            Outcome::Transient { .. }
        ));
    }

    #[test]
    fn ok_with_error_payload_is_not_success() {
        let b = json!({"error": {"message": "upstream busy", "code": 429}});
        assert!(matches!(
            classify(
                ProviderKind::Openrouter,
                200,
                &HeaderMap::new(),
                &body(&b),
                now()
            ),
            Outcome::RateLimited { .. }
        ));
        let weird = json!({"error": {"message": "???"}});
        assert!(matches!(
            classify(
                ProviderKind::Openrouter,
                200,
                &HeaderMap::new(),
                &body(&weird),
                now()
            ),
            Outcome::Transient { .. }
        ));
        let ok = json!({"choices": []});
        assert_eq!(
            classify(
                ProviderKind::Groq,
                200,
                &HeaderMap::new(),
                &body(&ok),
                now()
            ),
            Outcome::Success
        );
    }

    #[test]
    fn learns_exhaustion_from_success_headers() {
        let h = headers(&[
            ("x-ratelimit-remaining-requests", "0"),
            ("x-ratelimit-reset-requests", "2m59.56s"),
        ]);
        let (scope, until) = learn_from_success(ProviderKind::Groq, &h, now()).unwrap();
        assert_eq!(scope, LimitScope::Day);
        assert_eq!(until, now() + chrono::Duration::milliseconds(179_560));
        assert!(learn_from_success(ProviderKind::Gemini, &h, now()).is_none());
    }

    #[test]
    fn message_retry_hint_and_default() {
        let b = json!({"error": {"message": "Please retry in 2.5s."}});
        let out = classify(
            ProviderKind::Openai,
            429,
            &HeaderMap::new(),
            &body(&b),
            now(),
        );
        assert_eq!(
            out,
            Outcome::RateLimited {
                scope: LimitScope::Minute,
                until: now() + chrono::Duration::milliseconds(2500)
            }
        );
        let out = classify(ProviderKind::Openai, 429, &HeaderMap::new(), b"", now());
        assert_eq!(
            out,
            Outcome::RateLimited {
                scope: LimitScope::Minute,
                until: now() + chrono::Duration::seconds(60)
            }
        );
    }
}
