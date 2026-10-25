//! Provider kinds and their protocol defaults.

use std::fmt;

use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

/// The upstream API family. All of them are spoken to via the OpenAI chat-completions protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Google Gemini API (OpenAI-compatible endpoint).
    Gemini,
    /// Groq Cloud.
    Groq,
    /// OpenRouter (only `:free` / zero-priced models are auto-selected).
    Openrouter,
    /// Any other OpenAI-compatible endpoint (`base_url` required).
    Openai,
    /// Local Ollama via its OpenAI-compatible `/v1` endpoint.
    Ollama,
}

impl ProviderKind {
    /// All kinds in the default priority order.
    pub const ALL: [ProviderKind; 5] = [
        ProviderKind::Gemini,
        ProviderKind::Groq,
        ProviderKind::Openrouter,
        ProviderKind::Openai,
        ProviderKind::Ollama,
    ];

    /// Lowercase identifier used in config and account IDs.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Gemini => "gemini",
            ProviderKind::Groq => "groq",
            ProviderKind::Openrouter => "openrouter",
            ProviderKind::Openai => "openai",
            ProviderKind::Ollama => "ollama",
        }
    }

    /// Default OpenAI-compatible base URL (without trailing slash).
    #[must_use]
    pub fn default_base_url(self) -> Option<&'static str> {
        match self {
            ProviderKind::Gemini => Some("https://generativelanguage.googleapis.com/v1beta/openai"),
            ProviderKind::Groq => Some("https://api.groq.com/openai/v1"),
            ProviderKind::Openrouter => Some("https://openrouter.ai/api/v1"),
            ProviderKind::Openai | ProviderKind::Ollama => None,
        }
    }

    /// Environment variable prefix used for auto-detection (`PREFIX` or `PREFIX_<SUFFIX>`).
    #[must_use]
    pub fn key_env_prefix(self) -> Option<&'static str> {
        match self {
            ProviderKind::Gemini => Some("GEMINI_API_KEY"),
            ProviderKind::Groq => Some("GROQ_API_KEY"),
            ProviderKind::Openrouter => Some("OPENROUTER_API_KEY"),
            ProviderKind::Openai | ProviderKind::Ollama => None,
        }
    }

    /// Whether requests need an API key.
    #[must_use]
    pub fn requires_key(self) -> bool {
        !matches!(self, ProviderKind::Ollama)
    }

    /// Timezone in which daily quotas reset.
    #[must_use]
    pub fn daily_reset_tz(self) -> Tz {
        match self {
            ProviderKind::Gemini => chrono_tz::America::Los_Angeles,
            _ => chrono_tz::UTC,
        }
    }

    /// Extra static headers sent with every request.
    #[must_use]
    pub fn extra_headers(self) -> &'static [(&'static str, &'static str)] {
        match self {
            ProviderKind::Openrouter => &[("X-Title", "tierfall")],
            _ => &[],
        }
    }

    /// Tier assigned to models that match no ranking rule; `None` excludes them.
    #[must_use]
    pub fn unmatched_tier(self) -> Option<u8> {
        match self {
            ProviderKind::Gemini => None,
            ProviderKind::Groq | ProviderKind::Openrouter => Some(3),
            ProviderKind::Openai => Some(2),
            ProviderKind::Ollama => Some(4),
        }
    }

    /// Whether only zero-priced models should be auto-selected from discovery.
    #[must_use]
    pub fn free_models_only(self) -> bool {
        matches!(self, ProviderKind::Openrouter)
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ProviderKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ProviderKind::ALL
            .into_iter()
            .find(|k| k.as_str().eq_ignore_ascii_case(s.trim()))
            .ok_or_else(|| format!("unknown provider '{s}'"))
    }
}

/// Normalises a base URL: adds `http://` if no scheme, trims trailing slashes.
#[must_use]
pub fn normalize_base_url(raw: &str) -> String {
    let trimmed = raw.trim();
    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    with_scheme.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display() {
        assert_eq!(
            "OpenRouter".parse::<ProviderKind>().unwrap(),
            ProviderKind::Openrouter
        );
        assert_eq!(ProviderKind::Gemini.to_string(), "gemini");
        assert!("nope".parse::<ProviderKind>().is_err());
    }

    #[test]
    fn normalizes_urls() {
        assert_eq!(normalize_base_url("ollama:11434/"), "http://ollama:11434");
        assert_eq!(normalize_base_url("https://x/v1//"), "https://x/v1");
    }
}
