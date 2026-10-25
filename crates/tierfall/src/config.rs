//! TOML configuration, zero-config defaults and account resolution.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::provider::{ProviderKind, normalize_base_url};

/// Env var naming a config file.
pub const CONFIG_ENV: &str = "TIERFALL_CONFIG";
/// Env var overriding `router.account_order` (comma separated account IDs).
pub const ACCOUNT_ORDER_ENV: &str = "TIERFALL_ACCOUNT_ORDER";
/// Env var overriding `router.state_dir`.
pub const STATE_DIR_ENV: &str = "TIERFALL_STATE_DIR";
/// Env var selecting the Ollama model for the auto-detected Ollama account.
pub const OLLAMA_MODEL_ENV: &str = "TIERFALL_OLLAMA_MODEL";

/// Snapshot of environment variables. Empty values are treated as unset.
#[derive(Clone, Default)]
pub struct Env {
    vars: BTreeMap<String, String>,
}

impl Env {
    /// Captures the current process environment.
    #[must_use]
    pub fn from_process() -> Self {
        Self::from_pairs(std::env::vars())
    }

    /// Builds an environment from explicit pairs (useful for tests).
    pub fn from_pairs<K: Into<String>, V: Into<String>>(
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        Self {
            vars: pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    /// Returns a non-empty, trimmed value.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
    }

    /// True when the variable exists, even if empty.
    #[must_use]
    pub fn is_declared(&self, key: &str) -> bool {
        self.vars.contains_key(key)
    }

    fn names(&self) -> impl Iterator<Item = &str> {
        self.vars.keys().map(String::as_str)
    }
}

impl fmt::Debug for Env {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Env")
            .field("vars", &format!("<{} variables>", self.vars.len()))
            .finish()
    }
}

/// Top-level configuration. Every field is optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub server: ServerConfig,
    pub router: RouterConfig,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<AccountConfig>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<RouteConfig>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<LimitConfig>,
    /// Extra ranking rules, evaluated before the built-in ones (first match wins).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rankings: Vec<RankRule>,
}

/// Proxy server settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub bind: String,
    /// Env var holding the bearer token clients must send. Auth is off when it is unset.
    pub api_key_env: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:4141".to_string(),
            api_key_env: "TIERFALL_API_KEY".to_string(),
        }
    }
}

/// Routing behaviour.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RouterConfig {
    /// Route used when a request's `model` is empty or `default`.
    pub default_route: String,
    /// Wait in place for a per-minute limit that resets within this many seconds (0 = never wait).
    pub short_wait_secs: u64,
    pub request_timeout_secs: u64,
    pub connect_timeout_secs: u64,
    /// Base cooldown after a transient (5xx/timeout) failure; doubles per consecutive failure.
    pub transient_cooldown_secs: u64,
    /// Scan the environment for `GEMINI_API_KEY*`, `GROQ_API_KEY*`, `OPENROUTER_API_KEY*`.
    pub auto_detect: bool,
    /// Add a local Ollama account as the last-resort fallback.
    pub include_ollama: bool,
    /// Preferred account order; unlisted accounts follow (explicit first, then alphabetical).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub account_order: Vec<String>,
    /// Provider order inside each quality tier.
    pub provider_priority: Vec<ProviderKind>,
    /// Directory for `state.json` and `models.json`. Defaults to `~/.cache/tierfall`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_dir: Option<PathBuf>,
    pub persist_state: bool,
    /// Query each account's `/models` endpoint to pick models automatically.
    pub discovery: bool,
    pub discovery_ttl_hours: u64,
    /// Maximum discovered models per account per tier.
    pub models_per_tier: usize,
    /// Provide built-in aliases besides `auto` (currently `fast`). `auto` always exists.
    pub builtin_routes: bool,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            default_route: "auto".to_string(),
            short_wait_secs: 0,
            request_timeout_secs: 120,
            connect_timeout_secs: 10,
            transient_cooldown_secs: 30,
            auto_detect: true,
            include_ollama: true,
            account_order: Vec::new(),
            provider_priority: ProviderKind::ALL.to_vec(),
            state_dir: None,
            persist_state: true,
            discovery: true,
            discovery_ttl_hours: 24,
            models_per_tier: 2,
            builtin_routes: true,
        }
    }
}

/// An explicitly configured account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AccountConfig {
    pub id: String,
    pub provider: ProviderKind,
    /// Env var holding the API key (never put the key itself in config).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Pin these models (in this order) and skip discovery for the account.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    /// Models ranked first within their tier when discovered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_models: Vec<String>,
    /// Force every model of this account into one tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<u8>,
    /// Request parameters stripped before sending to this account.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drop_params: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// A named route (alias) with an explicit fallback chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteConfig {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub chain: Vec<ChainEntryConfig>,
}

/// One chain entry. Forms:
/// - `{ account = "groq", model = "openai/gpt-oss-120b" }` — one account
/// - `{ provider = "gemini", model = "gemini-x" }` — every account of a provider, in order
/// - `{ model = "gemini-x" }` — every account whose discovered models include it
/// - `{ account = "groq", model = "*" }` — every ranked model of the account
/// - `{ route = "auto" }` — splice in another route
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChainEntryConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
}

/// Optional proactive limits for matching (account|provider, model) entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LimitConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderKind>,
    /// Glob pattern; omitted means every model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpm: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpd: Option<u32>,
}

/// A model ranking rule. The first rule whose provider and glob pattern match decides.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RankRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderKind>,
    /// Case-insensitive glob (`*`, `?`) matched against the whole model ID.
    pub pattern: String,
    /// Quality tier (1 = best). Required unless `exclude` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<u8>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub exclude: bool,
    /// Only match models whose ID advertises at least this many billion parameters (`…-120b…`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_size_b: Option<f64>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(v: &bool) -> bool {
    !*v
}

impl Config {
    /// Parses and validates TOML.
    pub fn from_toml_str(text: &str) -> Result<Self> {
        let cfg: Config =
            toml::from_str(text).map_err(|e| Error::Config(format!("invalid TOML: {e}")))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reads and validates a TOML file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("cannot read {}: {e}", path.display())))?;
        Self::from_toml_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))
    }

    /// Finds the config file using the precedence: explicit path, `TIERFALL_CONFIG`,
    /// `./tierfall.toml`, `$XDG_CONFIG_HOME/tierfall/config.toml` (or `~/.config/...`).
    #[must_use]
    pub fn locate(explicit: Option<&Path>, env: &Env) -> Option<PathBuf> {
        if let Some(p) = explicit {
            return Some(p.to_path_buf());
        }
        if let Some(p) = env.get(CONFIG_ENV) {
            return Some(PathBuf::from(p));
        }
        let local = PathBuf::from("tierfall.toml");
        if local.is_file() {
            return Some(local);
        }
        let config_home = env
            .get("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env.get("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        let user = config_home.join("tierfall").join("config.toml");
        user.is_file().then_some(user)
    }

    /// Loads the config found by [`Config::locate`], or the zero-config default.
    pub fn load(explicit: Option<&Path>, env: &Env) -> Result<(Self, Option<PathBuf>)> {
        match Self::locate(explicit, env) {
            Some(path) => Ok((Self::from_file(&path)?, Some(path))),
            None => Ok((Self::default(), None)),
        }
    }

    /// Validates internal consistency.
    pub fn validate(&self) -> Result<()> {
        let mut ids = HashSet::new();
        for acct in &self.accounts {
            if acct.id.trim().is_empty() {
                return Err(Error::Config("account id must not be empty".into()));
            }
            if !ids.insert(acct.id.as_str()) {
                return Err(Error::Config(format!("duplicate account id '{}'", acct.id)));
            }
            if acct.id.contains('@') {
                return Err(Error::Config(format!(
                    "account id '{}' must not contain '@'",
                    acct.id
                )));
            }
            if acct.provider == ProviderKind::Openai && acct.base_url.is_none() {
                return Err(Error::Config(format!(
                    "account '{}': provider 'openai' requires base_url",
                    acct.id
                )));
            }
            if acct.provider.requires_key() && acct.api_key_env.is_none() {
                return Err(Error::Config(format!(
                    "account '{}': api_key_env is required for provider '{}'",
                    acct.id, acct.provider
                )));
            }
            if acct.tier == Some(0) {
                return Err(Error::Config(format!(
                    "account '{}': tier must be >= 1",
                    acct.id
                )));
            }
        }
        let mut names = HashSet::new();
        for route in &self.routes {
            if route.name.trim().is_empty() {
                return Err(Error::Config("route name must not be empty".into()));
            }
            if !names.insert(route.name.as_str()) {
                return Err(Error::Config(format!("duplicate route '{}'", route.name)));
            }
            if route.chain.is_empty() {
                return Err(Error::Config(format!(
                    "route '{}' has an empty chain",
                    route.name
                )));
            }
            for entry in &route.chain {
                match (&entry.route, &entry.model) {
                    (Some(_), None) if entry.account.is_none() && entry.provider.is_none() => {}
                    (None, Some(_)) if !(entry.account.is_some() && entry.provider.is_some()) => {}
                    _ => {
                        return Err(Error::Config(format!(
                            "route '{}': each chain entry needs either `route`, or `model` with at most one of `account`/`provider`",
                            route.name
                        )));
                    }
                }
            }
        }
        for rule in &self.rankings {
            if !rule.exclude && rule.tier.is_none_or(|t| t == 0) {
                return Err(Error::Config(format!(
                    "ranking rule '{}' needs tier >= 1 or exclude = true",
                    rule.pattern
                )));
            }
        }
        for limit in &self.limits {
            if limit.account.is_some() && limit.provider.is_some() {
                return Err(Error::Config(
                    "a limit may set `account` or `provider`, not both".into(),
                ));
            }
        }
        if self.router.provider_priority.is_empty() {
            return Err(Error::Config(
                "router.provider_priority must not be empty".into(),
            ));
        }
        Ok(())
    }

    /// Resolves the state directory (config, `TIERFALL_STATE_DIR`, XDG cache, `~/.cache`, temp).
    #[must_use]
    pub fn state_dir(&self, env: &Env) -> PathBuf {
        if let Some(dir) = &self.router.state_dir {
            return expand_tilde(dir, env);
        }
        if let Some(dir) = env.get(STATE_DIR_ENV) {
            return expand_tilde(Path::new(dir), env);
        }
        if let Some(dir) = env.get("XDG_CACHE_HOME") {
            return PathBuf::from(dir).join("tierfall");
        }
        if let Some(home) = env.get("HOME") {
            return PathBuf::from(home).join(".cache").join("tierfall");
        }
        std::env::temp_dir().join("tierfall")
    }
}

fn expand_tilde(path: &Path, env: &Env) -> PathBuf {
    if let Ok(rest) = path.strip_prefix("~")
        && let Some(home) = env.get("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

/// A fully resolved account ready for use.
#[derive(Clone)]
pub struct Account {
    pub id: String,
    pub provider: ProviderKind,
    pub base_url: String,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub pinned_models: Vec<String>,
    pub preferred_models: Vec<String>,
    pub tier_override: Option<u8>,
    pub drop_params: Vec<String>,
    /// True when created by environment auto-detection.
    pub auto_detected: bool,
}

impl fmt::Debug for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Account")
            .field("id", &self.id)
            .field("provider", &self.provider)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("api_key_env", &self.api_key_env)
            .field("pinned_models", &self.pinned_models)
            .field("preferred_models", &self.preferred_models)
            .field("tier_override", &self.tier_override)
            .field("auto_detected", &self.auto_detected)
            .finish_non_exhaustive()
    }
}

impl Account {
    /// Converts back to an [`AccountConfig`] (used by `tierfall init`). Never includes the key.
    #[must_use]
    pub fn to_config(&self) -> AccountConfig {
        let default_base = self.provider.default_base_url();
        AccountConfig {
            id: self.id.clone(),
            provider: self.provider,
            api_key_env: self.api_key_env.clone(),
            base_url: (default_base != Some(self.base_url.as_str())).then(|| self.base_url.clone()),
            models: self.pinned_models.clone(),
            preferred_models: self.preferred_models.clone(),
            tier: self.tier_override,
            drop_params: self.drop_params.clone(),
            enabled: true,
        }
    }
}

/// Resolves configured and auto-detected accounts in their final order.
/// Returns the accounts plus human-readable warnings (no secrets).
#[must_use]
pub fn resolve_accounts(cfg: &Config, env: &Env) -> (Vec<Account>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut accounts: Vec<Account> = Vec::new();
    let mut used_envs: HashSet<String> = HashSet::new();

    for ac in &cfg.accounts {
        if let Some(name) = &ac.api_key_env {
            used_envs.insert(name.clone());
        }
        if !ac.enabled {
            continue;
        }
        let api_key = ac
            .api_key_env
            .as_deref()
            .and_then(|name| env.get(name))
            .map(str::to_string);
        if ac.provider.requires_key() && api_key.is_none() {
            warnings.push(format!(
                "account '{}' skipped: env var {} is unset or empty",
                ac.id,
                ac.api_key_env.as_deref().unwrap_or("?")
            ));
            continue;
        }
        let base_url = match (&ac.base_url, ac.provider) {
            (Some(url), _) => normalize_base_url(url),
            (None, ProviderKind::Ollama) => ollama_base_url(env),
            (None, kind) => kind.default_base_url().unwrap_or_default().to_string(),
        };
        accounts.push(Account {
            id: ac.id.clone(),
            provider: ac.provider,
            base_url,
            api_key,
            api_key_env: ac.api_key_env.clone(),
            pinned_models: ac.models.clone(),
            preferred_models: ac.preferred_models.clone(),
            tier_override: ac.tier,
            drop_params: ac.drop_params.clone(),
            auto_detected: false,
        });
    }

    let mut taken: HashSet<String> = cfg.accounts.iter().map(|a| a.id.clone()).collect();
    if cfg.router.auto_detect {
        let mut detected = Vec::new();
        for name in env.names() {
            if used_envs.contains(name) {
                continue;
            }
            let Some((kind, suffix)) = match_key_env(name) else {
                continue;
            };
            let id = match suffix {
                Some(s) => format!("{}-{}", kind.as_str(), s.to_ascii_lowercase()),
                None => kind.as_str().to_string(),
            };
            let Some(key) = env.get(name) else {
                warnings.push(format!("env var {name} is declared but empty; skipping"));
                continue;
            };
            if !taken.insert(id.clone()) {
                continue;
            }
            detected.push(Account {
                id,
                provider: kind,
                base_url: kind.default_base_url().unwrap_or_default().to_string(),
                api_key: Some(key.to_string()),
                api_key_env: Some(name.to_string()),
                pinned_models: Vec::new(),
                preferred_models: Vec::new(),
                tier_override: None,
                drop_params: Vec::new(),
                auto_detected: true,
            });
        }
        detected.sort_by(|a, b| a.id.cmp(&b.id));
        accounts.extend(detected);
    }

    if cfg.router.include_ollama
        && !accounts.iter().any(|a| a.provider == ProviderKind::Ollama)
        && taken.insert("ollama".to_string())
    {
        let model = env
            .get(OLLAMA_MODEL_ENV)
            .or_else(|| env.get("TKS_LLM_MODEL"))
            .unwrap_or("qwen3:8b")
            .to_string();
        accounts.push(Account {
            id: "ollama".to_string(),
            provider: ProviderKind::Ollama,
            base_url: ollama_base_url(env),
            api_key: None,
            api_key_env: None,
            pinned_models: Vec::new(),
            preferred_models: vec![model],
            tier_override: Some(4),
            drop_params: Vec::new(),
            auto_detected: true,
        });
    }

    let order: Vec<String> = match env.get(ACCOUNT_ORDER_ENV) {
        Some(list) => list
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        None => cfg.router.account_order.clone(),
    };
    for name in &order {
        if !accounts.iter().any(|a| &a.id == name) {
            warnings.push(format!("account_order mentions unknown account '{name}'"));
        }
    }
    // Stable sort keeps explicit-then-alphabetical order for unlisted accounts.
    accounts.sort_by_key(|a| order.iter().position(|o| o == &a.id).unwrap_or(usize::MAX));
    (accounts, warnings)
}

fn match_key_env(name: &str) -> Option<(ProviderKind, Option<&str>)> {
    for kind in ProviderKind::ALL {
        let Some(prefix) = kind.key_env_prefix() else {
            continue;
        };
        if name == prefix {
            return Some((kind, None));
        }
        if let Some(rest) = name.strip_prefix(prefix)
            && let Some(suffix) = rest.strip_prefix('_')
            && !suffix.is_empty()
            && suffix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Some((kind, Some(suffix)));
        }
    }
    None
}

fn ollama_base_url(env: &Env) -> String {
    let host = env.get("OLLAMA_HOST").unwrap_or("http://ollama:11434");
    let base = normalize_base_url(host);
    if base.ends_with("/v1") {
        base
    } else {
        format!("{base}/v1")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Env {
        Env::from_pairs(pairs.iter().map(|(k, v)| (*k, *v)))
    }

    #[test]
    fn zero_config_detects_accounts_and_orders_them() {
        let e = env(&[
            ("GEMINI_API_KEY_SS", "k1"),
            ("GEMINI_API_KEY_AM", "k2"),
            ("GEMINI_API_KEY_JB", "k3"),
            ("GEMINI_API_KEY", ""),
            ("GROQ_API_KEY", "g"),
            ("OPENROUTER_API_KEY", "o"),
            ("TIERFALL_ACCOUNT_ORDER", "gemini-ss, gemini-am,gemini-jb"),
            ("OLLAMA_HOST", "http://localhost:11434"),
            ("TKS_LLM_MODEL", "qwen3:8b"),
        ]);
        let (accts, warnings) = resolve_accounts(&Config::default(), &e);
        let ids: Vec<_> = accts.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "gemini-ss",
                "gemini-am",
                "gemini-jb",
                "groq",
                "openrouter",
                "ollama"
            ]
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("GEMINI_API_KEY is declared but empty"))
        );
        let ollama = accts.iter().find(|a| a.id == "ollama").unwrap();
        assert_eq!(ollama.base_url, "http://localhost:11434/v1");
        assert_eq!(ollama.preferred_models, ["qwen3:8b"]);
        assert_eq!(accts[0].api_key.as_deref(), Some("k1"));
        assert!(!format!("{:?}", accts[0]).contains("k1"));
    }

    #[test]
    fn alphabetical_without_order_and_explicit_accounts_win() {
        let cfg = Config::from_toml_str(
            r#"
            [router]
            include_ollama = false
            [[accounts]]
            id = "main"
            provider = "gemini"
            api_key_env = "GEMINI_API_KEY_SS"
            models = ["gemini-x"]
            "#,
        )
        .unwrap();
        let e = env(&[
            ("GEMINI_API_KEY_SS", "k1"),
            ("GEMINI_API_KEY_JB", "k3"),
            ("GEMINI_API_KEY_AM", "k2"),
        ]);
        let (accts, _) = resolve_accounts(&cfg, &e);
        let ids: Vec<_> = accts.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["main", "gemini-am", "gemini-jb"]);
        assert_eq!(accts[0].pinned_models, ["gemini-x"]);
    }

    #[test]
    fn missing_key_skips_explicit_account() {
        let cfg = Config::from_toml_str(
            r#"
            [router]
            auto_detect = false
            include_ollama = false
            [[accounts]]
            id = "x"
            provider = "groq"
            api_key_env = "NOPE"
            "#,
        )
        .unwrap();
        let (accts, warnings) = resolve_accounts(&cfg, &env(&[]));
        assert!(accts.is_empty());
        assert!(warnings[0].contains("NOPE"));
    }

    #[test]
    fn validation_errors() {
        let bad = [
            "[[accounts]]\nid='a'\nprovider='openai'\napi_key_env='K'\n",
            "[[accounts]]\nid='a'\nprovider='groq'\n",
            "[[routes]]\nname='r'\nchain=[]\n",
            "[[routes]]\nname='r'\nchain=[{account='a', provider='groq', model='m'}]\n",
            "[[routes]]\nname='r'\nchain=[{route='auto', model='m'}]\n",
            "[[rankings]]\npattern='x*'\n",
            "[router]\nbogus = 1\n",
        ];
        for text in bad {
            assert!(
                Config::from_toml_str(text).is_err(),
                "should reject: {text}"
            );
        }
    }

    #[test]
    fn full_config_parses_and_round_trips() {
        let text = r#"
            [server]
            bind = "0.0.0.0:9000"
            [router]
            short_wait_secs = 3
            account_order = ["b", "a"]
            provider_priority = ["groq", "gemini"]
            [[accounts]]
            id = "a"
            provider = "gemini"
            api_key_env = "A"
            [[accounts]]
            id = "b"
            provider = "openai"
            api_key_env = "B"
            base_url = "https://example.com/v1"
            tier = 1
            [[routes]]
            name = "smart"
            chain = [
              { provider = "gemini", model = "gemini-x" },
              { account = "b", model = "*" },
              { route = "auto" },
            ]
            [[limits]]
            provider = "gemini"
            model = "gemini-x*"
            rpm = 5
            rpd = 20
            [[rankings]]
            provider = "groq"
            pattern = "*kimi*"
            tier = 1
        "#;
        let cfg = Config::from_toml_str(text).unwrap();
        assert_eq!(cfg.server.bind, "0.0.0.0:9000");
        assert_eq!(cfg.router.short_wait_secs, 3);
        assert_eq!(cfg.routes[0].chain.len(), 3);
        let back = toml::to_string_pretty(&cfg).unwrap();
        assert_eq!(Config::from_toml_str(&back).unwrap(), cfg);
    }

    #[test]
    fn state_dir_resolution() {
        let cfg = Config::default();
        assert_eq!(
            cfg.state_dir(&env(&[("HOME", "/h")])),
            PathBuf::from("/h/.cache/tierfall")
        );
        assert_eq!(
            cfg.state_dir(&env(&[("HOME", "/h"), ("TIERFALL_STATE_DIR", "~/s")])),
            PathBuf::from("/h/s")
        );
    }
}
