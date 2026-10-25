//! The fallback router: resolves a requested model or route to a chain of (account, model)
//! targets, walks it while tracking quota state, and returns the first successful response.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_util::stream::{BoxStream, Stream, StreamExt};
use reqwest::header::HeaderMap;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::classify::{self, LimitScope, Outcome};
use crate::config::{Account, Config, Env, RankRule, resolve_accounts};
use crate::discovery::{
    self, AccountCatalog, CachedModels, DiscoverySource, ModelCache, RawModel, glob_match,
};
use crate::error::{Attempt, Error, Result, describe_error};
use crate::provider::ProviderKind;
use crate::routes::{RouteContext, Target};
use crate::sse::{SseEvent, SseParser};
use crate::state::{Availability, EffectiveLimit, StateStore};
use crate::types::{ChatChunk, ChatRequest, ChatResponse, ServedBy};

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);
const DISCOVERY_RETRY_SECS: i64 = 300;
const FLUSH_INTERVAL: Duration = Duration::from_secs(2);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn read<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The tierfall client. Cheap to clone; all clones share state.
#[derive(Clone)]
pub struct Tierfall {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Tierfall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tierfall")
            .field("accounts", &self.inner.accounts)
            .finish_non_exhaustive()
    }
}

struct Inner {
    config: Config,
    env: Env,
    config_path: Option<PathBuf>,
    accounts: Vec<Account>,
    rules: Vec<RankRule>,
    http: reqwest::Client,
    state: StateStore,
    cache_path: Option<PathBuf>,
    cache: Mutex<ModelCache>,
    catalogs: RwLock<BTreeMap<String, AccountCatalog>>,
    next_refresh: Mutex<BTreeMap<String, DateTime<Utc>>>,
    refreshing: Mutex<HashSet<String>>,
    routes: RwLock<BTreeMap<String, Vec<Target>>>,
    account_warnings: Vec<String>,
    route_warnings: RwLock<Vec<String>>,
}

/// A successful non-streaming completion with the raw upstream JSON body.
#[derive(Debug, Clone)]
pub struct RawCompletion {
    pub body: Bytes,
    pub served_by: ServedBy,
    /// Failed or skipped attempts that preceded the successful one.
    pub attempts: Vec<Attempt>,
}

/// Point-in-time view of accounts, models, quota state and routes.
#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_path: Option<PathBuf>,
    pub default_route: String,
    pub accounts: Vec<AccountStatus>,
    pub routes: BTreeMap<String, Vec<String>>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountStatus {
    pub id: String,
    pub provider: ProviderKind,
    pub base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    pub auto_detected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<String>,
    pub discovery: Option<DiscoverySource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovery_error: Option<String>,
    pub models: Vec<ModelStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<u8>,
    pub availability: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<DateTime<Utc>>,
    pub requests_today: u64,
    pub tokens_today: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpm: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpd: Option<u32>,
}

impl Tierfall {
    /// Zero-config entry point: loads the config file if one is found
    /// (`TIERFALL_CONFIG`, `./tierfall.toml`, `~/.config/tierfall/config.toml`) and
    /// auto-detects accounts from the process environment.
    pub async fn from_env() -> Result<Self> {
        Self::load(None).await
    }

    /// Loads an explicit config file (or discovers one when `None`) using the process environment.
    pub async fn load(config_path: Option<&Path>) -> Result<Self> {
        let env = Env::from_process();
        let (config, path) = Config::load(config_path, &env)?;
        Self::build(config, env, path).await
    }

    /// Loads the given config file using the process environment.
    pub async fn from_config_file(path: impl AsRef<Path>) -> Result<Self> {
        Self::load(Some(path.as_ref())).await
    }

    /// Uses `config` with the process environment.
    pub async fn new(config: Config) -> Result<Self> {
        Self::with_env(config, Env::from_process()).await
    }

    /// Uses `config` with an explicit environment snapshot (handy for tests).
    pub async fn with_env(config: Config, env: Env) -> Result<Self> {
        Self::build(config, env, None).await
    }

    async fn build(config: Config, env: Env, config_path: Option<PathBuf>) -> Result<Self> {
        config.validate()?;
        let (accounts, account_warnings) = resolve_accounts(&config, &env);
        if accounts.is_empty() {
            return Err(Error::Config(
                "no accounts available: set provider keys such as GEMINI_API_KEY, GROQ_API_KEY or \
                 OPENROUTER_API_KEY, or configure [[accounts]]"
                    .into(),
            ));
        }
        for w in &account_warnings {
            tracing::warn!(target: "tierfall", "{w}");
        }
        let (state_path, cache_path) = if config.router.persist_state {
            let dir = config.state_dir(&env);
            (Some(dir.join("state.json")), Some(dir.join("models.json")))
        } else {
            (None, None)
        };
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(config.router.connect_timeout_secs))
            .user_agent(concat!("tierfall/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let mut rules = config.rankings.clone();
        rules.extend(discovery::builtin_rules());
        let cache = cache_path
            .as_deref()
            .map(ModelCache::load)
            .unwrap_or_default();
        let inner = Arc::new(Inner {
            state: StateStore::open(state_path.clone()),
            config,
            env,
            config_path,
            accounts,
            rules,
            http,
            cache_path,
            cache: Mutex::new(cache),
            catalogs: RwLock::new(BTreeMap::new()),
            next_refresh: Mutex::new(BTreeMap::new()),
            refreshing: Mutex::new(HashSet::new()),
            routes: RwLock::new(BTreeMap::new()),
            account_warnings,
            route_warnings: RwLock::new(Vec::new()),
        });
        inner.discover_all(false).await;
        if state_path.is_some() {
            spawn_flusher(Arc::downgrade(&inner));
        }
        Ok(Self { inner })
    }

    /// Sends a non-streaming chat completion, falling back across the resolved chain.
    pub async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let raw = self.chat_raw(request).await?;
        let mut response: ChatResponse = serde_json::from_slice(&raw.body)?;
        response.served_by = Some(raw.served_by);
        Ok(response)
    }

    /// Like [`Tierfall::chat`] but returns the upstream JSON body untouched.
    pub async fn chat_raw(&self, request: ChatRequest) -> Result<RawCompletion> {
        self.complete_json(request_to_map(request)?).await
    }

    /// Non-streaming completion from an arbitrary OpenAI-style JSON body.
    pub async fn complete_json(&self, body: Map<String, Value>) -> Result<RawCompletion> {
        match self.run(body, false).await? {
            Committed::Body(raw) => Ok(raw),
            Committed::Stream(_) => Err(Error::Stream("unexpected stream".into())),
        }
    }

    /// Sends a streaming chat completion. Fallback happens until the first event arrives;
    /// after that the stream is committed to one provider.
    pub async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream> {
        self.stream_json(request_to_map(request)?).await
    }

    /// Streaming completion from an arbitrary OpenAI-style JSON body.
    pub async fn stream_json(&self, body: Map<String, Value>) -> Result<ChatStream> {
        match self.run(body, true).await? {
            Committed::Stream(s) => Ok(s),
            Committed::Body(_) => Err(Error::Stream("expected a stream".into())),
        }
    }

    /// Resolves a model/route name to its fallback chain without sending anything.
    pub fn resolve(&self, requested: &str) -> Result<(String, Vec<Target>)> {
        let inner = &self.inner;
        let name = if requested.is_empty() || requested == "default" {
            inner.config.router.default_route.as_str()
        } else {
            requested
        };
        if let Some(chain) = read(&inner.routes).get(name) {
            if chain.is_empty() {
                return Err(Error::Config(format!(
                    "route '{name}' has no available models (see `tierfall status`)"
                )));
            }
            return Ok((name.to_string(), chain.clone()));
        }
        if let Some((model, account)) = name.rsplit_once('@')
            && !model.is_empty()
            && inner.account(account).is_some()
        {
            return Ok((name.to_string(), vec![Target::new(account, model)]));
        }
        let catalogs = read(&inner.catalogs);
        let ctx = RouteContext {
            config: &inner.config,
            accounts: &inner.accounts,
            catalogs: &catalogs,
        };
        let chain = ctx.concrete(name);
        if chain.is_empty() {
            return Err(Error::UnknownModel(name.to_string()));
        }
        Ok((name.to_string(), chain))
    }

    async fn run(&self, body: Map<String, Value>, stream: bool) -> Result<Committed> {
        validate_body(&body)?;
        self.maybe_refresh();
        let requested = body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let (label, chain) = self.resolve(&requested)?;
        let inner = &self.inner;
        let router = &inner.config.router;
        let mut attempts: Vec<Attempt> = Vec::new();
        let mut calls: u32 = 0;
        let mut context_errors: u32 = 0;
        let mut context_error: Option<Error> = None;

        for target in &chain {
            let Some(account) = inner.account(&target.account) else {
                continue;
            };
            let model = target.model.as_str();
            let limit = inner.effective_limit(account, model);
            let availability =
                inner
                    .state
                    .try_acquire(&account.id, account.provider, model, limit, Utc::now());
            if availability != Availability::Ready {
                attempts.push(Attempt {
                    account: account.id.clone(),
                    model: model.to_string(),
                    outcome: format!("skipped:{}", availability.label()),
                    status: None,
                    until: availability.until(),
                });
                continue;
            }
            let payload = prepare_body(&body, account, model, stream);
            let mut waited = false;
            loop {
                calls += 1;
                let started = Instant::now();
                let result = inner.attempt(account, &payload, stream).await;
                let latency_ms = started.elapsed().as_millis() as u64;
                let served_by = ServedBy {
                    account: account.id.clone(),
                    provider: account.provider,
                    model: model.to_string(),
                    attempts: calls,
                };
                let now = Utc::now();
                let (outcome, status, error_body) = match result {
                    AttemptResult::Body {
                        body,
                        tokens,
                        headers,
                    } => {
                        tracing::info!(target: "tierfall", account = %account.id, model, outcome = "success", latency_ms, route = %label);
                        inner.on_success(account, model, tokens, &headers, now);
                        return Ok(Committed::Body(RawCompletion {
                            body,
                            served_by,
                            attempts,
                        }));
                    }
                    AttemptResult::Stream { state, headers } => {
                        tracing::info!(target: "tierfall", account = %account.id, model, outcome = "stream_committed", latency_ms, route = %label);
                        inner.on_success(account, model, 0, &headers, now);
                        let live = LiveStream {
                            st: state,
                            inner: Arc::clone(inner),
                            account: account.id.clone(),
                            model: model.to_string(),
                            tokens: 0,
                            finished: false,
                        };
                        return Ok(Committed::Stream(ChatStream {
                            served_by,
                            attempts,
                            events: live.into_events(),
                        }));
                    }
                    AttemptResult::Failed {
                        outcome,
                        status,
                        body,
                    } => (outcome, status, body),
                };
                tracing::info!(
                    target: "tierfall",
                    account = %account.id,
                    model,
                    outcome = outcome.class(),
                    status = status.unwrap_or(0),
                    latency_ms,
                    route = %label,
                    detail = %outcome_reason(&outcome, &error_body),
                );
                if let Outcome::RateLimited {
                    scope: LimitScope::Minute,
                    until,
                } = &outcome
                    && !waited
                    && router.short_wait_secs > 0
                    && *until - now <= chrono::Duration::seconds(router.short_wait_secs as i64)
                {
                    waited = true;
                    attempts.push(Attempt {
                        account: account.id.clone(),
                        model: model.to_string(),
                        outcome: "rate_limited:waited".into(),
                        status,
                        until: Some(*until),
                    });
                    tokio::time::sleep((*until - now).to_std().unwrap_or_default()).await;
                    continue;
                }
                inner.state.record(
                    &account.id,
                    model,
                    &outcome,
                    0,
                    router.transient_cooldown_secs,
                    now,
                );
                attempts.push(Attempt {
                    account: account.id.clone(),
                    model: model.to_string(),
                    outcome: outcome.class().to_string(),
                    status,
                    until: match &outcome {
                        Outcome::RateLimited { until, .. } => Some(*until),
                        _ => None,
                    },
                });
                match &outcome {
                    Outcome::ClientError { .. } => {
                        return Err(Error::Upstream {
                            status: status.unwrap_or(400),
                            body: error_body,
                            served_by,
                        });
                    }
                    Outcome::ContextTooLong { .. } => {
                        context_errors += 1;
                        context_error = Some(Error::Upstream {
                            status: status.unwrap_or(400),
                            body: error_body,
                            served_by,
                        });
                    }
                    Outcome::ModelGone { .. } => self.schedule_refresh(&account.id),
                    Outcome::AuthFailed { reason } => {
                        tracing::warn!(target: "tierfall", account = %account.id, %reason, "account disabled until restart");
                    }
                    _ => {}
                }
                break;
            }
        }
        if context_errors > 0
            && context_errors == calls
            && let Some(err) = context_error
        {
            return Err(err);
        }
        let retry_after = inner.earliest_retry(&chain);
        Err(Error::AllExhausted {
            route: label,
            retry_after,
            attempts,
        })
    }

    fn maybe_refresh(&self) {
        let now = Utc::now();
        let due: Vec<String> = lock(&self.inner.next_refresh)
            .iter()
            .filter(|(_, t)| **t <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            self.spawn_refresh(id, false);
        }
    }

    fn schedule_refresh(&self, account: &str) {
        self.spawn_refresh(account.to_string(), true);
    }

    fn spawn_refresh(&self, account: String, force: bool) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if !lock(&self.inner.refreshing).insert(account.clone()) {
            return;
        }
        lock(&self.inner.next_refresh).remove(&account);
        let inner = Arc::clone(&self.inner);
        handle.spawn(async move {
            inner.refresh_account(&account, force).await;
            lock(&inner.refreshing).remove(&account);
        });
    }

    /// Re-fetches every account's model list (ignoring the cache TTL) and rebuilds routes.
    pub async fn refresh_models(&self) {
        self.inner.discover_all(true).await;
    }

    /// Writes quota state to disk now (it is also flushed periodically and on drop).
    pub fn flush_state(&self) -> Result<()> {
        self.inner.state.flush().map_err(Error::from)
    }

    /// Resolved accounts in fallback order.
    #[must_use]
    pub fn accounts(&self) -> &[Account] {
        &self.inner.accounts
    }

    #[must_use]
    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    #[must_use]
    pub fn config_path(&self) -> Option<&Path> {
        self.inner.config_path.as_deref()
    }

    /// Current route table.
    #[must_use]
    pub fn routes(&self) -> BTreeMap<String, Vec<Target>> {
        read(&self.inner.routes).clone()
    }

    /// Current model catalogs per account.
    #[must_use]
    pub fn catalogs(&self) -> BTreeMap<String, AccountCatalog> {
        read(&self.inner.catalogs).clone()
    }

    /// Configuration and routing warnings (never contain secrets).
    #[must_use]
    pub fn warnings(&self) -> Vec<String> {
        let mut w = self.inner.account_warnings.clone();
        w.extend(read(&self.inner.route_warnings).iter().cloned());
        w
    }

    /// The API key clients must present to the proxy, if `server.api_key_env` is set.
    #[must_use]
    pub fn server_api_key(&self) -> Option<String> {
        self.inner
            .env
            .get(&self.inner.config.server.api_key_env)
            .map(str::to_string)
    }

    /// OpenAI-style `GET /v1/models` payload: routes first, then routable concrete models.
    #[must_use]
    pub fn models(&self) -> Value {
        let inner = &self.inner;
        let routes = read(&inner.routes);
        let default = &inner.config.router.default_route;
        let mut data = Vec::new();
        let mut names: Vec<&String> = routes.keys().collect();
        names.sort_by_key(|n| (*n != default, (*n).clone()));
        for name in names {
            let chain: Vec<String> = routes[name].iter().map(ToString::to_string).collect();
            data.push(json!({
                "id": name,
                "object": "model",
                "created": 0,
                "owned_by": "tierfall",
                "tierfall": {"type": "route", "chain": chain},
            }));
        }
        let catalogs = read(&inner.catalogs);
        let mut order: Vec<(String, ProviderKind, Vec<String>)> = Vec::new();
        let ordered_targets = routes
            .get(default)
            .into_iter()
            .flatten()
            .map(|t| (t.account.clone(), t.model.clone()))
            .chain(inner.accounts.iter().flat_map(|a| {
                catalogs
                    .get(&a.id)
                    .into_iter()
                    .flat_map(|c| c.ranked.iter().map(|m| (a.id.clone(), m.id.clone())))
            }));
        for (account_id, model) in ordered_targets {
            let Some(account) = inner.account(&account_id) else {
                continue;
            };
            match order.iter_mut().find(|(m, _, _)| *m == model) {
                Some((_, _, accts)) => {
                    if !accts.contains(&account_id) {
                        accts.push(account_id);
                    }
                }
                None => order.push((model, account.provider, vec![account_id])),
            }
        }
        for (model, provider, accts) in order {
            if routes.contains_key(&model) {
                continue;
            }
            data.push(json!({
                "id": model,
                "object": "model",
                "created": 0,
                "owned_by": provider.as_str(),
                "tierfall": {"type": "model", "accounts": accts},
            }));
        }
        json!({"object": "list", "data": data})
    }

    /// Snapshot of accounts, availability and usage.
    #[must_use]
    pub fn status(&self) -> StatusReport {
        let inner = &self.inner;
        let now = Utc::now();
        let catalogs = read(&inner.catalogs);
        let routes = read(&inner.routes);
        let mut accounts = Vec::new();
        for account in &inner.accounts {
            let catalog = catalogs.get(&account.id);
            let mut models: Vec<(String, Option<u8>)> = catalog
                .map(|c| {
                    c.ranked
                        .iter()
                        .map(|m| (m.id.clone(), Some(m.tier)))
                        .collect()
                })
                .unwrap_or_default();
            for target in routes.values().flatten() {
                if target.account == account.id && !models.iter().any(|(m, _)| *m == target.model) {
                    models.push((target.model.clone(), None));
                }
            }
            let models = models
                .into_iter()
                .map(|(model, tier)| {
                    let limit = inner.effective_limit(account, &model);
                    let (availability, entry) =
                        inner
                            .state
                            .peek(&account.id, account.provider, &model, limit, now);
                    ModelStatus {
                        tier,
                        availability: availability.label(),
                        until: availability.until(),
                        requests_today: entry.requests_today,
                        tokens_today: entry.tokens_today,
                        last_error: entry.last_error,
                        rpm: limit.rpm,
                        rpd: limit.rpd,
                        model,
                    }
                })
                .collect();
            accounts.push(AccountStatus {
                id: account.id.clone(),
                provider: account.provider,
                base_url: account.base_url.clone(),
                api_key_env: account.api_key_env.clone(),
                auto_detected: account.auto_detected,
                disabled: inner.state.disabled_reason(&account.id),
                discovery: catalog.map(|c| c.source),
                fetched_at: catalog.and_then(|c| c.fetched_at),
                discovery_error: catalog.and_then(|c| c.error.clone()),
                models,
            });
        }
        let routes = routes
            .iter()
            .map(|(name, chain)| {
                (
                    name.clone(),
                    chain.iter().map(ToString::to_string).collect(),
                )
            })
            .collect();
        StatusReport {
            config_path: inner.config_path.clone(),
            state_path: inner.state.path().map(Path::to_path_buf),
            default_route: inner.config.router.default_route.clone(),
            accounts,
            routes,
            warnings: self.warnings(),
        }
    }
}

impl Inner {
    fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    fn effective_limit(&self, account: &Account, model: &str) -> EffectiveLimit {
        let mut limit = EffectiveLimit::default();
        for l in &self.config.limits {
            if l.account.as_ref().is_some_and(|a| *a != account.id)
                || l.provider.is_some_and(|p| p != account.provider)
                || l.model.as_ref().is_some_and(|m| !glob_match(m, model))
            {
                continue;
            }
            limit.rpm = min_opt(limit.rpm, l.rpm);
            limit.rpd = min_opt(limit.rpd, l.rpd);
        }
        limit
    }

    fn on_success(
        &self,
        account: &Account,
        model: &str,
        tokens: u64,
        headers: &HeaderMap,
        now: DateTime<Utc>,
    ) {
        self.state.record(
            &account.id,
            model,
            &Outcome::Success,
            tokens,
            self.config.router.transient_cooldown_secs,
            now,
        );
        if let Some((scope, until)) = classify::learn_from_success(account.provider, headers, now) {
            tracing::info!(target: "tierfall", account = %account.id, model, ?scope, %until, "provider reports no remaining quota");
            self.state
                .apply_learned_limit(&account.id, model, scope, until);
        }
    }

    fn earliest_retry(&self, chain: &[Target]) -> Option<Duration> {
        let now = Utc::now();
        chain
            .iter()
            .filter_map(|t| {
                let account = self.account(&t.account)?;
                let limit = self.effective_limit(account, &t.model);
                let (availability, _) =
                    self.state
                        .peek(&account.id, account.provider, &t.model, limit, now);
                availability.until()
            })
            .min()
            .map(|t| (t - now).to_std().unwrap_or(Duration::ZERO))
    }

    async fn attempt(
        &self,
        account: &Account,
        payload: &Map<String, Value>,
        stream: bool,
    ) -> AttemptResult {
        let provider = account.provider;
        let url = format!("{}/chat/completions", account.base_url);
        let mut req = self.http.post(&url).json(payload);
        if let Some(key) = &account.api_key {
            req = req.bearer_auth(key);
        }
        for (name, value) in provider.extra_headers() {
            req = req.header(*name, *value);
        }
        let timeout = Duration::from_secs(self.config.router.request_timeout_secs);
        if !stream {
            req = req.timeout(timeout);
        }
        let resp = match tokio::time::timeout(timeout, req.send()).await {
            Err(_) => return AttemptResult::transient("timed out waiting for response", None),
            Ok(Err(e)) => {
                let reason = if e.is_timeout() {
                    "request timed out".to_string()
                } else {
                    format!("request failed: {}", describe_error(&e))
                };
                return AttemptResult::transient(reason, None);
            }
            Ok(Ok(resp)) => resp,
        };
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let is_json = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.contains("json"));
        if !stream || !(200..300).contains(&status) || is_json {
            let body = match resp.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    return AttemptResult::transient(
                        format!("reading response failed: {}", describe_error(&e)),
                        Some(status),
                    );
                }
            };
            let outcome = classify::classify(provider, status, &headers, &body, Utc::now());
            if outcome != Outcome::Success {
                return AttemptResult::Failed {
                    outcome,
                    status: Some(status),
                    body,
                };
            }
            if stream {
                return AttemptResult::transient("provider ignored stream=true", Some(status));
            }
            let tokens = serde_json::from_slice::<Value>(&body)
                .map(|v| usage_tokens(&v))
                .unwrap_or(0);
            return AttemptResult::Body {
                body,
                tokens,
                headers,
            };
        }
        let mut st = StreamState {
            body: resp.bytes_stream().boxed(),
            parser: SseParser::new(),
            pending: VecDeque::new(),
            ended: false,
            idle: timeout,
        };
        match tokio::time::timeout(timeout, st.fill()).await {
            Err(_) => {
                return AttemptResult::transient("timed out waiting for first event", Some(status));
            }
            Ok(Err(reason)) => return AttemptResult::transient(reason, Some(status)),
            Ok(Ok(())) => {}
        }
        let Some(first) = st.pending.front() else {
            return AttemptResult::transient("stream ended without data", Some(status));
        };
        if !first.is_done()
            && let Ok(v) = serde_json::from_str::<Value>(&first.data)
            && v.get("choices").is_none()
            && v.get("error").is_some()
        {
            let outcome = classify::classify_error_payload(provider, &v, &headers, Utc::now());
            return AttemptResult::Failed {
                outcome,
                status: Some(status),
                body: Bytes::from(first.data.clone()),
            };
        }
        AttemptResult::Stream { state: st, headers }
    }

    async fn discover_all(&self, force: bool) {
        let results = futures_util::future::join_all(
            self.accounts
                .iter()
                .map(|account| self.discover_account(account, force)),
        )
        .await;
        {
            let mut catalogs = write(&self.catalogs);
            let mut next = lock(&self.next_refresh);
            for (account, (catalog, next_at)) in self.accounts.iter().zip(results) {
                log_catalog(account, &catalog);
                catalogs.insert(account.id.clone(), catalog);
                match next_at {
                    Some(t) => next.insert(account.id.clone(), t),
                    None => next.remove(&account.id),
                };
            }
        }
        self.save_cache();
        self.rebuild_routes();
    }

    async fn refresh_account(&self, id: &str, force: bool) {
        let Some(account) = self.account(id) else {
            return;
        };
        let (catalog, next_at) = self.discover_account(account, force).await;
        log_catalog(account, &catalog);
        write(&self.catalogs).insert(account.id.clone(), catalog);
        if let Some(t) = next_at {
            lock(&self.next_refresh).insert(account.id.clone(), t);
        }
        self.save_cache();
        self.rebuild_routes();
    }

    async fn discover_account(
        &self,
        account: &Account,
        force: bool,
    ) -> (AccountCatalog, Option<DateTime<Utc>>) {
        let now = Utc::now();
        if !account.pinned_models.is_empty() {
            let ranked = discovery::rank_pinned(account, &self.rules);
            let catalog = AccountCatalog {
                source: DiscoverySource::Pinned,
                fetched_at: None,
                available: account.pinned_models.clone(),
                ranked,
                error: None,
            };
            return (catalog, None);
        }
        if !self.config.router.discovery {
            return (self.static_catalog(account, None), None);
        }
        let ttl = chrono::Duration::hours(self.config.router.discovery_ttl_hours as i64);
        let cached = lock(&self.cache)
            .accounts
            .get(&account.id)
            .filter(|c| c.base_url == account.base_url)
            .cloned();
        if !force
            && let Some(c) = &cached
            && now - c.fetched_at < ttl
        {
            let catalog = self.catalog_from(
                account,
                &c.models,
                DiscoverySource::Cache,
                Some(c.fetched_at),
                None,
            );
            return (catalog, Some(c.fetched_at + ttl));
        }
        match discovery::fetch_models(&self.http, account, DISCOVERY_TIMEOUT).await {
            Ok(models) => {
                lock(&self.cache).accounts.insert(
                    account.id.clone(),
                    CachedModels {
                        fetched_at: now,
                        base_url: account.base_url.clone(),
                        models: models.clone(),
                    },
                );
                let catalog =
                    self.catalog_from(account, &models, DiscoverySource::Live, Some(now), None);
                (catalog, Some(now + ttl))
            }
            Err(e) if e.auth => {
                let reason = format!("model discovery rejected the key: {}", e.message);
                self.state.disable_account(&account.id, &reason);
                let catalog = AccountCatalog {
                    source: DiscoverySource::Failed,
                    fetched_at: None,
                    available: Vec::new(),
                    ranked: Vec::new(),
                    error: Some(reason),
                };
                (catalog, None)
            }
            Err(e) => {
                let retry = Some(now + chrono::Duration::seconds(DISCOVERY_RETRY_SECS));
                if let Some(c) = cached {
                    let catalog = self.catalog_from(
                        account,
                        &c.models,
                        DiscoverySource::Cache,
                        Some(c.fetched_at),
                        Some(e.message),
                    );
                    return (catalog, retry);
                }
                if account.provider == ProviderKind::Ollama {
                    let catalog = AccountCatalog {
                        source: DiscoverySource::Failed,
                        fetched_at: None,
                        available: Vec::new(),
                        ranked: Vec::new(),
                        error: Some(e.message),
                    };
                    return (catalog, retry);
                }
                (self.static_catalog(account, Some(e.message)), retry)
            }
        }
    }

    fn catalog_from(
        &self,
        account: &Account,
        models: &[RawModel],
        source: DiscoverySource,
        fetched_at: Option<DateTime<Utc>>,
        error: Option<String>,
    ) -> AccountCatalog {
        let usable: Vec<RawModel> = models
            .iter()
            .filter(|m| !self.state.is_gone(&account.id, &m.id))
            .cloned()
            .collect();
        let mut ranked = discovery::rank_models(
            account,
            &usable,
            &self.rules,
            self.config.router.models_per_tier,
        );
        let mut error = error;
        if ranked.is_empty() {
            ranked = discovery::static_models(account);
            if !ranked.is_empty() && error.is_none() {
                error = Some(
                    "no listed model matched the ranking rules; using built-in defaults".into(),
                );
            }
        }
        AccountCatalog {
            source,
            fetched_at,
            available: usable.into_iter().map(|m| m.id).collect(),
            ranked,
            error,
        }
    }

    fn static_catalog(&self, account: &Account, error: Option<String>) -> AccountCatalog {
        let ranked = discovery::static_models(account);
        AccountCatalog {
            source: if ranked.is_empty() {
                DiscoverySource::Failed
            } else {
                DiscoverySource::Static
            },
            fetched_at: None,
            available: ranked.iter().map(|m| m.id.clone()).collect(),
            ranked,
            error,
        }
    }

    fn save_cache(&self) {
        let Some(path) = &self.cache_path else {
            return;
        };
        let snapshot = lock(&self.cache).clone();
        if let Err(e) = snapshot.save(path) {
            tracing::warn!(target: "tierfall", path = %path.display(), error = %e, "failed to write model cache");
        }
    }

    fn rebuild_routes(&self) {
        let catalogs = read(&self.catalogs);
        let ctx = RouteContext {
            config: &self.config,
            accounts: &self.accounts,
            catalogs: &catalogs,
        };
        let (table, warnings) = ctx.build_table();
        drop(catalogs);
        for w in &warnings {
            tracing::warn!(target: "tierfall", "{w}");
        }
        *write(&self.routes) = table;
        *write(&self.route_warnings) = warnings;
    }
}

fn log_catalog(account: &Account, catalog: &AccountCatalog) {
    let ranked: Vec<String> = catalog
        .ranked
        .iter()
        .map(|m| format!("{}(t{})", m.id, m.tier))
        .collect();
    match &catalog.error {
        Some(error) => {
            tracing::warn!(target: "tierfall", account = %account.id, source = ?catalog.source, %error, models = ?ranked, "model discovery");
        }
        None => {
            tracing::debug!(target: "tierfall", account = %account.id, source = ?catalog.source, models = ?ranked, "model discovery");
        }
    }
}

fn spawn_flusher(weak: Weak<Inner>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        let mut tick = tokio::time::interval(FLUSH_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tick.tick().await;
        loop {
            tick.tick().await;
            let Some(inner) = weak.upgrade() else {
                break;
            };
            if let Err(e) = inner.state.flush() {
                tracing::warn!(target: "tierfall", error = %e, "failed to persist state");
            }
        }
    });
}

fn min_opt(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

fn usage_tokens(v: &Value) -> u64 {
    v.get("usage").filter(|u| u.is_object()).map_or(0, |u| {
        u.get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| {
                u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0)
                    + u.get("completion_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
            })
    })
}

fn outcome_reason(outcome: &Outcome, body: &[u8]) -> String {
    match outcome {
        Outcome::Transient { reason }
        | Outcome::AuthFailed { reason }
        | Outcome::ModelGone { reason }
        | Outcome::ContextTooLong { reason }
        | Outcome::ClientError { reason } => reason.clone(),
        Outcome::RateLimited { until, .. } => {
            format!("until {until}; {}", classify::error_message(body))
        }
        Outcome::Success => String::new(),
    }
}

fn request_to_map(request: ChatRequest) -> Result<Map<String, Value>> {
    match serde_json::to_value(request)? {
        Value::Object(map) => Ok(map),
        _ => Err(Error::InvalidRequest(
            "request must be a JSON object".into(),
        )),
    }
}

fn validate_body(body: &Map<String, Value>) -> Result<()> {
    match body.get("model") {
        None | Some(Value::Null | Value::String(_)) => {}
        Some(_) => return Err(Error::InvalidRequest("`model` must be a string".into())),
    }
    match body.get("messages") {
        Some(Value::Array(m)) if !m.is_empty() => Ok(()),
        _ => Err(Error::InvalidRequest(
            "`messages` must be a non-empty array".into(),
        )),
    }
}

fn prepare_body(
    base: &Map<String, Value>,
    account: &Account,
    model: &str,
    stream: bool,
) -> Map<String, Value> {
    let mut body = base.clone();
    body.insert("model".into(), Value::String(model.to_string()));
    if stream {
        body.insert("stream".into(), Value::Bool(true));
    } else {
        body.remove("stream");
        body.remove("stream_options");
    }
    for param in &account.drop_params {
        body.remove(param);
    }
    body
}

enum Committed {
    Body(RawCompletion),
    Stream(ChatStream),
}

enum AttemptResult {
    Body {
        body: Bytes,
        tokens: u64,
        headers: HeaderMap,
    },
    Stream {
        state: StreamState,
        headers: HeaderMap,
    },
    Failed {
        outcome: Outcome,
        status: Option<u16>,
        body: Bytes,
    },
}

impl AttemptResult {
    fn transient(reason: impl Into<String>, status: Option<u16>) -> Self {
        AttemptResult::Failed {
            outcome: Outcome::Transient {
                reason: reason.into(),
            },
            status,
            body: Bytes::new(),
        }
    }
}

struct StreamState {
    body: BoxStream<'static, reqwest::Result<Bytes>>,
    parser: SseParser,
    pending: VecDeque<SseEvent>,
    ended: bool,
    idle: Duration,
}

impl StreamState {
    /// Reads until at least one event is pending or the body ends.
    async fn fill(&mut self) -> std::result::Result<(), String> {
        while self.pending.is_empty() && !self.ended {
            match tokio::time::timeout(self.idle, self.body.next()).await {
                Err(_) => return Err("stream idle timeout".into()),
                Ok(Some(Ok(bytes))) => self.pending.extend(self.parser.push(&bytes)),
                Ok(Some(Err(e))) => {
                    return Err(format!("stream read failed: {}", describe_error(&e)));
                }
                Ok(None) => {
                    self.ended = true;
                    self.pending.extend(self.parser.finish());
                }
            }
        }
        Ok(())
    }
}

struct LiveStream {
    st: StreamState,
    inner: Arc<Inner>,
    account: String,
    model: String,
    tokens: u64,
    finished: bool,
}

impl LiveStream {
    fn observe(&mut self, event: &SseEvent) {
        if event.is_done() {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(&event.data) else {
            return;
        };
        self.tokens = self.tokens.max(usage_tokens(&v));
        if v.get("choices").is_none() && v.get("error").is_some() {
            let reason = classify::error_message(event.data.as_bytes());
            tracing::warn!(target: "tierfall", account = %self.account, model = %self.model, %reason, "error inside committed stream");
            self.inner.state.record(
                &self.account,
                &self.model,
                &Outcome::Transient {
                    reason: format!("mid-stream error: {reason}"),
                },
                0,
                self.inner.config.router.transient_cooldown_secs,
                Utc::now(),
            );
        }
    }

    fn finish(&mut self) {
        if !self.finished {
            self.finished = true;
            self.inner
                .state
                .add_tokens(&self.account, &self.model, self.tokens);
        }
    }

    fn into_events(self) -> BoxStream<'static, Result<SseEvent>> {
        futures_util::stream::unfold(self, |mut live| async move {
            if live.finished {
                return None;
            }
            match live.st.fill().await {
                Err(reason) => {
                    live.inner.state.record(
                        &live.account,
                        &live.model,
                        &Outcome::Transient {
                            reason: reason.clone(),
                        },
                        0,
                        live.inner.config.router.transient_cooldown_secs,
                        Utc::now(),
                    );
                    live.finish();
                    Some((Err(Error::Stream(reason)), live))
                }
                Ok(()) => match live.st.pending.pop_front() {
                    Some(event) => {
                        live.observe(&event);
                        Some((Ok(event), live))
                    }
                    None => {
                        live.finish();
                        None
                    }
                },
            }
        })
        .boxed()
    }
}

/// A committed streaming completion. Yields parsed [`ChatChunk`]s; the `[DONE]` sentinel is
/// consumed silently. Use [`ChatStream::into_events`] for raw SSE pass-through.
pub struct ChatStream {
    served_by: ServedBy,
    attempts: Vec<Attempt>,
    events: BoxStream<'static, Result<SseEvent>>,
}

impl std::fmt::Debug for ChatStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatStream")
            .field("served_by", &self.served_by)
            .field("attempts", &self.attempts)
            .finish_non_exhaustive()
    }
}

impl ChatStream {
    #[must_use]
    pub fn served_by(&self) -> &ServedBy {
        &self.served_by
    }

    /// Failed or skipped attempts that preceded the committed one.
    #[must_use]
    pub fn attempts(&self) -> &[Attempt] {
        &self.attempts
    }

    /// Splits into metadata and the raw SSE event stream (including `[DONE]` if sent upstream).
    pub fn into_events(self) -> (ServedBy, Vec<Attempt>, BoxStream<'static, Result<SseEvent>>) {
        (self.served_by, self.attempts, self.events)
    }

    /// Collects the remaining stream into the concatenated text of the first choice.
    pub async fn collect_text(mut self) -> Result<String> {
        let mut text = String::new();
        while let Some(chunk) = self.next().await {
            if let Some(t) = chunk?.text() {
                text.push_str(t);
            }
        }
        Ok(text)
    }
}

impl Stream for ChatStream {
    type Item = Result<ChatChunk>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            let event = match this.events.poll_next_unpin(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                Poll::Ready(Some(Ok(event))) => event,
            };
            if event.is_done() || event.data.trim().is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(&event.data) {
                Ok(v) => v,
                Err(e) => return Poll::Ready(Some(Err(e.into()))),
            };
            if value.get("choices").is_none() && value.get("error").is_some() {
                return Poll::Ready(Some(Err(Error::Stream(classify::error_message(
                    event.data.as_bytes(),
                )))));
            }
            return Poll::Ready(Some(serde_json::from_value(value).map_err(Error::from)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_preparation() {
        let mut base = Map::new();
        base.insert("model".into(), json!("auto"));
        base.insert("stream".into(), json!(true));
        base.insert("stream_options".into(), json!({"include_usage": true}));
        base.insert("seed".into(), json!(1));
        let account = Account {
            id: "a".into(),
            provider: ProviderKind::Groq,
            base_url: String::new(),
            api_key: None,
            api_key_env: None,
            pinned_models: Vec::new(),
            preferred_models: Vec::new(),
            tier_override: None,
            drop_params: vec!["seed".into()],
            auto_detected: false,
        };
        let body = prepare_body(&base, &account, "m", false);
        assert_eq!(body.get("model"), Some(&json!("m")));
        assert!(!body.contains_key("stream") && !body.contains_key("stream_options"));
        assert!(!body.contains_key("seed"));
        let body = prepare_body(&base, &account, "m", true);
        assert_eq!(body.get("stream"), Some(&json!(true)));
        assert!(body.contains_key("stream_options"));
    }

    #[test]
    fn validation_and_usage() {
        let mut body = Map::new();
        assert!(validate_body(&body).is_err());
        body.insert(
            "messages".into(),
            json!([{"role": "user", "content": "hi"}]),
        );
        assert!(validate_body(&body).is_ok());
        body.insert("model".into(), json!(3));
        assert!(validate_body(&body).is_err());
        assert_eq!(usage_tokens(&json!({"usage": {"total_tokens": 7}})), 7);
        assert_eq!(
            usage_tokens(&json!({"usage": {"prompt_tokens": 2, "completion_tokens": 3}})),
            5
        );
        assert_eq!(usage_tokens(&json!({"usage": null})), 0);
        assert_eq!(min_opt(Some(3), Some(2)), Some(2));
        assert_eq!(min_opt(None, Some(2)), Some(2));
    }
}
