//! Model discovery (`GET /models`), ranking into quality tiers, caching and static fallbacks.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{Account, RankRule};
use crate::error::describe_error;
use crate::provider::ProviderKind;

/// A model as reported by a provider's model list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u64>,
    /// `Some(true)` when the provider reports the model as zero-priced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free: Option<bool>,
}

impl RawModel {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            context_length: None,
            free: None,
        }
    }
}

/// A model selected for routing, with its quality tier (1 = best).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RankedModel {
    pub id: String,
    pub tier: u8,
}

/// Where an account's model list came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DiscoverySource {
    Pinned,
    Live,
    Cache,
    Static,
    Failed,
}

/// The models known for one account.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AccountCatalog {
    pub source: DiscoverySource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<DateTime<Utc>>,
    /// Every model ID the provider reported (used for concrete-model routing).
    #[serde(skip)]
    pub available: Vec<String>,
    /// Models selected for automatic routes, ordered by tier then rank.
    pub ranked: Vec<RankedModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AccountCatalog {
    /// True when `model` can be requested from this account.
    #[must_use]
    pub fn offers(&self, model: &str) -> bool {
        self.available.iter().any(|m| m == model) || self.ranked.iter().any(|m| m.id == model)
    }
}

/// On-disk cache of discovery results.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelCache {
    pub accounts: BTreeMap<String, CachedModels>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedModels {
    pub fetched_at: DateTime<Utc>,
    pub base_url: String,
    pub models: Vec<RawModel>,
}

impl ModelCache {
    #[must_use]
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        crate::state::write_atomic(path, &json)
    }
}

/// Discovery failure.
#[derive(Debug, Clone)]
pub struct FetchError {
    /// The key was rejected (the account should be disabled).
    pub auth: bool,
    pub message: String,
}

/// Fetches `GET {base_url}/models` for an account.
pub async fn fetch_models(
    http: &reqwest::Client,
    account: &Account,
    timeout: Duration,
) -> Result<Vec<RawModel>, FetchError> {
    let url = format!("{}/models", account.base_url);
    let mut req = http.get(&url).timeout(timeout);
    if let Some(key) = &account.api_key {
        req = req.bearer_auth(key);
    }
    for (name, value) in account.provider.extra_headers() {
        req = req.header(*name, *value);
    }
    let resp = req.send().await.map_err(|e| FetchError {
        auth: false,
        message: format!("request failed: {}", describe_error(&e)),
    })?;
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let body = resp.bytes().await.map_err(|e| FetchError {
        auth: false,
        message: format!("reading body failed: {}", describe_error(&e)),
    })?;
    if !(200..300).contains(&status) {
        let outcome =
            crate::classify::classify(account.provider, status, &headers, &body, Utc::now());
        return Err(FetchError {
            auth: matches!(outcome, crate::classify::Outcome::AuthFailed { .. }),
            message: format!("HTTP {status}: {}", crate::classify::error_message(&body)),
        });
    }
    let json: Value = serde_json::from_slice(&body).map_err(|e| FetchError {
        auth: false,
        message: format!("invalid JSON: {e}"),
    })?;
    Ok(parse_models(account.provider, &json))
}

/// Parses OpenAI-style (`data[]`) or native Gemini (`models[]`) model lists.
#[must_use]
pub fn parse_models(_provider: ProviderKind, json: &Value) -> Vec<RawModel> {
    let mut out = Vec::new();
    if let Some(items) = json.get("data").and_then(Value::as_array) {
        for item in items {
            let Some(id) = item.get("id").and_then(Value::as_str) else {
                continue;
            };
            if item.get("active").and_then(Value::as_bool) == Some(false) {
                continue;
            }
            let context_length = item
                .get("context_length")
                .or_else(|| item.get("context_window"))
                .and_then(Value::as_u64);
            let free = item.get("pricing").map(|p| {
                let zero = |k: &str| {
                    p.get(k)
                        .and_then(|v| {
                            v.as_str()
                                .map(str::to_string)
                                .or_else(|| v.as_f64().map(|f| f.to_string()))
                        })
                        .is_some_and(|s| s.parse::<f64>().is_ok_and(|f| f == 0.0))
                };
                zero("prompt") && zero("completion")
            });
            let free = if id.ends_with(":free") {
                Some(true)
            } else {
                free
            };
            out.push(RawModel {
                id: id.strip_prefix("models/").unwrap_or(id).to_string(),
                context_length,
                free,
            });
        }
    } else if let Some(items) = json.get("models").and_then(Value::as_array) {
        for item in items {
            let Some(name) = item.get("name").and_then(Value::as_str) else {
                continue;
            };
            let generates = item
                .get("supportedGenerationMethods")
                .and_then(Value::as_array)
                .is_none_or(|m| m.iter().any(|x| x.as_str() == Some("generateContent")));
            if !generates {
                continue;
            }
            out.push(RawModel {
                id: name.strip_prefix("models/").unwrap_or(name).to_string(),
                context_length: item.get("inputTokenLimit").and_then(Value::as_u64),
                free: None,
            });
        }
    }
    out
}

fn rule(provider: Option<ProviderKind>, pattern: &str, tier: Option<u8>) -> RankRule {
    RankRule {
        provider,
        pattern: pattern.to_string(),
        tier,
        exclude: tier.is_none(),
        min_size_b: None,
    }
}

fn sized(provider: ProviderKind, tier: u8, min: f64) -> RankRule {
    RankRule {
        provider: Some(provider),
        pattern: "*".to_string(),
        tier: Some(tier),
        exclude: false,
        min_size_b: Some(min),
    }
}

/// Built-in ranking rules (evaluated after user rules; first match wins).
#[must_use]
pub fn builtin_rules() -> Vec<RankRule> {
    use ProviderKind::{Gemini, Groq, Openrouter};
    let mut rules: Vec<RankRule> = [
        "*embed*",
        "*tts*",
        "*whisper*",
        "*guard*",
        "*content-safety*",
        "*moderation*",
        "*transcribe*",
        "*audio*",
        "*-live*",
        "*image*",
        "imagen*",
        "veo*",
        "*lyria*",
        "*aqa*",
        "*robotics*",
        "*computer-use*",
        "*orpheus*",
        "*playai*",
        "*-latest",
    ]
    .iter()
    .map(|p| rule(None, p, None))
    .collect();
    rules.extend([
        rule(Some(Gemini), "*-pro*", None),
        rule(Some(Gemini), "*thinking*", None),
        rule(Some(Gemini), "learnlm*", None),
        rule(Some(Gemini), "gemini-*-flash", Some(1)),
        rule(Some(Gemini), "gemini-*-flash-preview*", Some(1)),
        rule(Some(Gemini), "gemini-*-flash-lite", Some(2)),
        rule(Some(Gemini), "gemini-*-flash-lite-preview*", Some(2)),
        rule(Some(Gemini), "gemma-*", Some(3)),
        rule(Some(Groq), "*compound*", None),
        rule(Some(Groq), "*gpt-oss-120b*", Some(1)),
        rule(Some(Groq), "moonshotai/kimi-k*", Some(1)),
        sized(Groq, 1, 200.0),
        rule(Some(Groq), "*gpt-oss-20b*", Some(2)),
        rule(Some(Groq), "qwen/*", Some(2)),
        sized(Groq, 2, 30.0),
        rule(Some(Openrouter), "openrouter/auto", None),
        sized(Openrouter, 1, 100.0),
        sized(Openrouter, 2, 20.0),
        rule(Some(Openrouter), "openrouter/free", Some(3)),
    ]);
    rules
}

/// Models used when discovery is disabled or fails and nothing is cached.
#[must_use]
pub fn static_models(account: &Account) -> Vec<RankedModel> {
    let list: Vec<(String, u8)> = match account.provider {
        ProviderKind::Gemini => vec![
            ("gemini-flash-latest".into(), 1),
            ("gemini-flash-lite-latest".into(), 2),
        ],
        ProviderKind::Groq => vec![
            ("openai/gpt-oss-120b".into(), 1),
            ("openai/gpt-oss-20b".into(), 2),
        ],
        ProviderKind::Openrouter => vec![("openrouter/free".into(), 3)],
        ProviderKind::Openai => Vec::new(),
        ProviderKind::Ollama => account
            .preferred_models
            .iter()
            .map(|m| (m.clone(), 4))
            .collect(),
    };
    let mut models: Vec<RankedModel> = list
        .into_iter()
        .map(|(id, tier)| RankedModel {
            id,
            tier: account.tier_override.unwrap_or(tier),
        })
        .collect();
    models.sort_by_key(|m| m.tier);
    models
}

/// Ranks pinned models: their order is kept, tiers come from rules (excluded/unmatched → 3).
#[must_use]
pub fn rank_pinned(account: &Account, rules: &[RankRule]) -> Vec<RankedModel> {
    let mut models: Vec<RankedModel> = account
        .pinned_models
        .iter()
        .map(|id| {
            let tier = account.tier_override.unwrap_or_else(|| {
                match match_rule(account.provider, id, rules) {
                    Some((_, r)) if !r.exclude => r.tier.unwrap_or(3),
                    _ => account.provider.unmatched_tier().unwrap_or(3),
                }
            });
            RankedModel {
                id: id.clone(),
                tier,
            }
        })
        .collect();
    models.sort_by_key(|m| m.tier);
    models
}

/// Ranks discovered models into tiers, keeping at most `per_tier` per tier.
#[must_use]
pub fn rank_models(
    account: &Account,
    raw: &[RawModel],
    rules: &[RankRule],
    per_tier: usize,
) -> Vec<RankedModel> {
    struct Candidate<'a> {
        model: &'a RawModel,
        tier: u8,
        preferred: usize,
        rule_index: usize,
        version: Vec<u32>,
        size: u64,
        preview: bool,
    }
    let mut candidates: Vec<Candidate<'_>> = Vec::new();
    for model in raw {
        let preferred = account
            .preferred_models
            .iter()
            .position(|p| p == &model.id)
            .unwrap_or(usize::MAX);
        if account.provider.free_models_only()
            && model.free != Some(true)
            && preferred == usize::MAX
        {
            continue;
        }
        let (rule_index, tier) = match match_rule(account.provider, &model.id, rules) {
            Some((_, r)) if r.exclude && preferred == usize::MAX => continue,
            Some((i, r)) => (i, r.tier.or(account.provider.unmatched_tier()).unwrap_or(3)),
            None if preferred != usize::MAX => {
                (rules.len(), account.provider.unmatched_tier().unwrap_or(3))
            }
            None => match account.provider.unmatched_tier() {
                Some(t) => (rules.len(), t),
                None => continue,
            },
        };
        let tier = account.tier_override.unwrap_or(tier);
        candidates.push(Candidate {
            model,
            tier,
            preferred,
            rule_index,
            version: model_version(&model.id),
            size: (model_size_b(&model.id).unwrap_or(0.0) * 1000.0) as u64,
            preview: is_preview(&model.id),
        });
    }
    candidates.sort_by(|a, b| {
        (a.tier, a.preferred, a.rule_index)
            .cmp(&(b.tier, b.preferred, b.rule_index))
            .then_with(|| b.version.cmp(&a.version))
            .then_with(|| b.size.cmp(&a.size))
            .then_with(|| a.preview.cmp(&b.preview))
            .then_with(|| b.model.context_length.cmp(&a.model.context_length))
            .then_with(|| a.model.id.cmp(&b.model.id))
    });
    let mut per: BTreeMap<u8, usize> = BTreeMap::new();
    let mut out = Vec::new();
    for c in candidates {
        let count = per.entry(c.tier).or_default();
        if *count >= per_tier.max(1) {
            continue;
        }
        *count += 1;
        out.push(RankedModel {
            id: c.model.id.clone(),
            tier: c.tier,
        });
    }
    out
}

fn match_rule<'a>(
    provider: ProviderKind,
    id: &str,
    rules: &'a [RankRule],
) -> Option<(usize, &'a RankRule)> {
    rules.iter().enumerate().find(|(_, r)| {
        r.provider.is_none_or(|p| p == provider)
            && glob_match(&r.pattern, id)
            && r.min_size_b
                .is_none_or(|min| model_size_b(id).is_some_and(|s| s >= min))
    })
}

/// Case-insensitive glob match supporting `*` and `?` over the whole string.
#[must_use]
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn tokens(id: &str) -> impl Iterator<Item = &str> {
    id.split(['-', '/', ':', '_']).filter(|t| !t.is_empty())
}

/// First dotted-numeric token, e.g. `gemini-3.8-flash` → `[3, 8]`.
#[must_use]
pub fn model_version(id: &str) -> Vec<u32> {
    tokens(id)
        .find(|t| {
            t.chars().all(|c| c.is_ascii_digit() || c == '.')
                && t.chars().next().is_some_and(|c| c.is_ascii_digit())
        })
        .map(|t| t.split('.').filter_map(|n| n.parse().ok()).collect())
        .unwrap_or_default()
}

/// Parameter count in billions advertised in the ID, e.g. `gpt-oss-120b` → 120.
#[must_use]
pub fn model_size_b(id: &str) -> Option<f64> {
    tokens(id).find_map(|t| {
        let lower = t.to_ascii_lowercase();
        let num = lower.strip_suffix('b')?;
        if num.is_empty() || !num.chars().next()?.is_ascii_digit() {
            return None;
        }
        num.parse::<f64>().ok()
    })
}

fn is_preview(id: &str) -> bool {
    tokens(id).any(|t| {
        let l = t.to_ascii_lowercase();
        l == "preview" || l == "exp" || l == "experimental" || l == "beta"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn account(provider: ProviderKind) -> Account {
        Account {
            id: provider.as_str().to_string(),
            provider,
            base_url: String::new(),
            api_key: None,
            api_key_env: None,
            pinned_models: Vec::new(),
            preferred_models: Vec::new(),
            tier_override: None,
            drop_params: Vec::new(),
            auto_detected: true,
        }
    }

    fn ids(models: &[RankedModel]) -> Vec<(&str, u8)> {
        models.iter().map(|m| (m.id.as_str(), m.tier)).collect()
    }

    #[test]
    fn globbing() {
        assert!(glob_match("gemini-*-flash", "Gemini-3.8-Flash"));
        assert!(!glob_match("gemini-*-flash", "gemini-3.5-flash-lite"));
        assert!(glob_match("*embed*", "gemini-embedding-001"));
        assert!(glob_match("qwen/*", "qwen/qwen3-32b"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "abbc"));
    }

    #[test]
    fn versions_and_sizes() {
        assert_eq!(model_version("gemini-3.8-flash"), vec![3, 8]);
        assert_eq!(model_version("gemma-4-31b-it"), vec![4]);
        assert!(model_version("openai/gpt-oss-120b").is_empty());
        assert_eq!(model_size_b("openai/gpt-oss-120b"), Some(120.0));
        assert_eq!(
            model_size_b("nvidia/nemotron-3-ultra-550b-a55b:free"),
            Some(550.0)
        );
        assert_eq!(model_size_b("gemini-3.8-flash"), None);
    }

    #[test]
    fn gemini_ranking_prefers_newest_flash_and_excludes_noise() {
        let raw: Vec<RawModel> = [
            "gemini-2.5-flash",
            "gemini-3.8-flash",
            "gemini-3.6-flash",
            "gemini-3.9-flash-preview-09-2026",
            "gemini-3.5-flash-lite",
            "gemini-3.1-flash-lite",
            "gemini-3-pro-preview",
            "gemini-embedding-001",
            "gemini-2.5-flash-preview-tts",
            "gemini-flash-latest",
            "gemma-3-27b-it",
            "gemma-4-31b-it",
            "gemini-2.5-flash-image",
            "imagen-4.0-generate-001",
            "aqa",
        ]
        .into_iter()
        .map(RawModel::new)
        .collect();
        let ranked = rank_models(&account(ProviderKind::Gemini), &raw, &builtin_rules(), 2);
        assert_eq!(
            ids(&ranked),
            vec![
                ("gemini-3.8-flash", 1),
                ("gemini-3.6-flash", 1),
                ("gemini-3.5-flash-lite", 2),
                ("gemini-3.1-flash-lite", 2),
                ("gemma-4-31b-it", 3),
                ("gemma-3-27b-it", 3),
            ]
        );
    }

    #[test]
    fn openrouter_only_free_and_size_tiers() {
        let json = json!({"data": [
            {"id": "nvidia/nemotron-3-ultra-550b-a55b:free", "context_length": 1000000},
            {"id": "nvidia/nemotron-3-super-120b-a12b:free", "context_length": 262144},
            {"id": "google/gemma-4-31b-it:free", "context_length": 262144},
            {"id": "openrouter/free", "pricing": {"prompt": "0", "completion": "0"}},
            {"id": "liquid/lfm-2.5-2.6b:free"},
            {"id": "anthropic/claude-x", "pricing": {"prompt": "0.000003", "completion": "0.000015"}},
            {"id": "nvidia/nemotron-3.5-content-safety:free"},
            {"id": "google/lyria-3-pro-preview", "pricing": {"prompt": "0", "completion": "0"}}
        ]});
        let raw = parse_models(ProviderKind::Openrouter, &json);
        let ranked = rank_models(
            &account(ProviderKind::Openrouter),
            &raw,
            &builtin_rules(),
            2,
        );
        assert_eq!(
            ids(&ranked),
            vec![
                ("nvidia/nemotron-3-ultra-550b-a55b:free", 1),
                ("nvidia/nemotron-3-super-120b-a12b:free", 1),
                ("google/gemma-4-31b-it:free", 2),
                ("openrouter/free", 3),
                ("liquid/lfm-2.5-2.6b:free", 3),
            ]
        );
    }

    #[test]
    fn groq_ranking() {
        let raw: Vec<RawModel> = [
            "openai/gpt-oss-20b",
            "openai/gpt-oss-120b",
            "whisper-large-v3",
            "meta-llama/llama-guard-4-12b",
            "qwen/qwen3-32b",
            "groq/compound",
            "allam-2-7b",
        ]
        .into_iter()
        .map(RawModel::new)
        .collect();
        let ranked = rank_models(&account(ProviderKind::Groq), &raw, &builtin_rules(), 2);
        assert_eq!(
            ids(&ranked),
            vec![
                ("openai/gpt-oss-120b", 1),
                ("openai/gpt-oss-20b", 2),
                ("qwen/qwen3-32b", 2),
                ("allam-2-7b", 3),
            ]
        );
    }

    #[test]
    fn user_rules_override_and_preferred_first() {
        let mut acct = account(ProviderKind::Ollama);
        acct.tier_override = Some(4);
        acct.preferred_models = vec!["qwen3:8b".into()];
        let raw: Vec<RawModel> = ["llama3:8b", "qwen3:8b", "nomic-embed-text"]
            .into_iter()
            .map(RawModel::new)
            .collect();
        let ranked = rank_models(&acct, &raw, &builtin_rules(), 2);
        assert_eq!(ids(&ranked), vec![("qwen3:8b", 4), ("llama3:8b", 4)]);

        let mut rules = vec![RankRule {
            provider: Some(ProviderKind::Groq),
            pattern: "allam*".into(),
            tier: Some(1),
            exclude: false,
            min_size_b: None,
        }];
        rules.extend(builtin_rules());
        let raw = vec![
            RawModel::new("allam-2-7b"),
            RawModel::new("openai/gpt-oss-120b"),
        ];
        let ranked = rank_models(&account(ProviderKind::Groq), &raw, &rules, 2);
        assert_eq!(
            ids(&ranked),
            vec![("allam-2-7b", 1), ("openai/gpt-oss-120b", 1)]
        );
    }

    #[test]
    fn parses_gemini_shapes_and_static_fallback() {
        let compat = json!({"object": "list", "data": [{"id": "models/gemini-3.8-flash", "object": "model"}]});
        assert_eq!(
            parse_models(ProviderKind::Gemini, &compat)[0].id,
            "gemini-3.8-flash"
        );
        let native = json!({"models": [
            {"name": "models/gemini-3.8-flash", "inputTokenLimit": 1048576, "supportedGenerationMethods": ["generateContent"]},
            {"name": "models/text-embedding-004", "supportedGenerationMethods": ["embedContent"]}
        ]});
        let parsed = parse_models(ProviderKind::Gemini, &native);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].context_length, Some(1_048_576));
        assert_eq!(
            ids(&static_models(&account(ProviderKind::Gemini))),
            vec![("gemini-flash-latest", 1), ("gemini-flash-lite-latest", 2)]
        );
    }

    #[test]
    fn pinned_keep_order_with_rule_tiers() {
        let mut acct = account(ProviderKind::Gemini);
        acct.pinned_models = vec![
            "gemini-3.5-flash-lite".into(),
            "gemini-3.8-flash".into(),
            "custom".into(),
        ];
        assert_eq!(
            ids(&rank_pinned(&acct, &builtin_rules())),
            vec![
                ("gemini-3.8-flash", 1),
                ("gemini-3.5-flash-lite", 2),
                ("custom", 3)
            ]
        );
    }
}
