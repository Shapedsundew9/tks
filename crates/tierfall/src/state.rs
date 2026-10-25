//! Per-entry quota state: cooldowns, daily exhaustion, usage counters, proactive limits,
//! and atomic JSON persistence.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::classify::{LimitScope, Outcome, next_daily_reset, quota_day};
use crate::provider::ProviderKind;

/// Proactive limits applying to one entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EffectiveLimit {
    pub rpm: Option<u32>,
    pub rpd: Option<u32>,
}

/// Whether an entry can be tried right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Ready,
    Cooldown(DateTime<Utc>),
    Exhausted(DateTime<Utc>),
    /// Proactively held back by a configured rpm/rpd limit.
    Limited(DateTime<Utc>),
    Gone,
    Disabled(String),
}

impl Availability {
    #[must_use]
    pub fn until(&self) -> Option<DateTime<Utc>> {
        match self {
            Availability::Cooldown(t) | Availability::Exhausted(t) | Availability::Limited(t) => {
                Some(*t)
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Availability::Ready => "ready",
            Availability::Cooldown(_) => "cooldown",
            Availability::Exhausted(_) => "exhausted",
            Availability::Limited(_) => "limited",
            Availability::Gone => "gone",
            Availability::Disabled(_) => "disabled",
        }
    }
}

/// Persisted state of one (account, model) entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EntryState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_until: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exhausted_until: Option<DateTime<Utc>>,
    #[serde(skip)]
    pub gone: bool,
    pub consecutive_failures: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_day: Option<String>,
    pub requests_today: u64,
    pub tokens_today: u64,
    #[serde(skip)]
    recent: VecDeque<DateTime<Utc>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct StateData {
    version: u32,
    entries: BTreeMap<String, EntryState>,
    #[serde(skip)]
    disabled_accounts: BTreeMap<String, String>,
}

/// Thread-safe state store with optional persistence.
#[derive(Debug)]
pub struct StateStore {
    data: Mutex<StateData>,
    path: Option<PathBuf>,
    dirty: AtomicBool,
}

fn key(account: &str, model: &str) -> String {
    format!("{account}|{model}")
}

const MAX_TRANSIENT_COOLDOWN_SECS: u64 = 15 * 60;

impl StateStore {
    /// Opens the store, loading `path` if it exists. Corrupt files are ignored with a warning.
    #[must_use]
    pub fn open(path: Option<PathBuf>) -> Self {
        let data = path
            .as_deref()
            .and_then(|p| match std::fs::read(p) {
                Ok(bytes) => match serde_json::from_slice::<StateData>(&bytes) {
                    Ok(d) => Some(d),
                    Err(e) => {
                        tracing::warn!(target: "tierfall", path = %p.display(), error = %e, "ignoring unreadable state file");
                        None
                    }
                },
                Err(_) => None,
            })
            .unwrap_or_default();
        Self {
            data: Mutex::new(data),
            path,
            dirty: AtomicBool::new(false),
        }
    }

    /// In-memory only store.
    #[must_use]
    pub fn in_memory() -> Self {
        Self::open(None)
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StateData> {
        self.data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn touch(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Checks availability and, when ready, reserves a request slot (counts toward rpm/rpd).
    pub fn try_acquire(
        &self,
        account: &str,
        provider: ProviderKind,
        model: &str,
        limit: EffectiveLimit,
        now: DateTime<Utc>,
    ) -> Availability {
        let mut data = self.lock();
        if let Some(reason) = data.disabled_accounts.get(account) {
            return Availability::Disabled(reason.clone());
        }
        let entry = data.entries.entry(key(account, model)).or_default();
        roll_day(entry, provider, now);
        let availability = evaluate(entry, provider, limit, now);
        if availability == Availability::Ready {
            entry.recent.push_back(now);
            entry.requests_today += 1;
            self.touch();
        }
        availability
    }

    /// Reports availability without reserving anything.
    #[must_use]
    pub fn peek(
        &self,
        account: &str,
        provider: ProviderKind,
        model: &str,
        limit: EffectiveLimit,
        now: DateTime<Utc>,
    ) -> (Availability, EntryState) {
        let mut data = self.lock();
        if let Some(reason) = data.disabled_accounts.get(account).cloned() {
            let entry = data
                .entries
                .get(&key(account, model))
                .cloned()
                .unwrap_or_default();
            return (Availability::Disabled(reason), entry);
        }
        let entry = data.entries.entry(key(account, model)).or_default();
        roll_day(entry, provider, now);
        (evaluate(entry, provider, limit, now), entry.clone())
    }

    /// Records the outcome of an attempt.
    pub fn record(
        &self,
        account: &str,
        model: &str,
        outcome: &Outcome,
        tokens: u64,
        transient_base_secs: u64,
        now: DateTime<Utc>,
    ) {
        let mut data = self.lock();
        if let Outcome::AuthFailed { reason } = outcome {
            data.disabled_accounts
                .insert(account.to_string(), reason.clone());
        }
        let entry = data.entries.entry(key(account, model)).or_default();
        match outcome {
            Outcome::Success => {
                entry.consecutive_failures = 0;
                entry.cooldown_until = None;
                entry.last_error = None;
                entry.tokens_today += tokens;
            }
            Outcome::RateLimited {
                scope: LimitScope::Minute,
                until,
            } => {
                entry.cooldown_until = Some(*until);
                entry.last_error = Some(outcome.class().to_string());
            }
            Outcome::RateLimited {
                scope: LimitScope::Day,
                until,
            } => {
                entry.exhausted_until = Some(*until);
                entry.last_error = Some(outcome.class().to_string());
            }
            Outcome::Transient { reason } => {
                entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
                let exp = entry.consecutive_failures.saturating_sub(1).min(10);
                let secs = transient_base_secs
                    .saturating_mul(1 << exp)
                    .min(MAX_TRANSIENT_COOLDOWN_SECS);
                entry.cooldown_until = Some(now + chrono::Duration::seconds(secs as i64));
                entry.last_error = Some(format!("transient: {reason}"));
            }
            Outcome::ModelGone { reason } => {
                entry.gone = true;
                entry.last_error = Some(format!("model_gone: {reason}"));
            }
            Outcome::AuthFailed { reason }
            | Outcome::ContextTooLong { reason }
            | Outcome::ClientError { reason } => {
                entry.last_error = Some(format!("{}: {reason}", outcome.class()));
            }
        }
        self.touch();
    }

    /// Adds tokens to today's usage counter (used when a stream reports usage at its end).
    pub fn add_tokens(&self, account: &str, model: &str, tokens: u64) {
        if tokens == 0 {
            return;
        }
        let mut data = self.lock();
        let entry = data.entries.entry(key(account, model)).or_default();
        entry.tokens_today += tokens;
        self.touch();
    }

    /// Applies a limit learned from successful-response headers.
    pub fn apply_learned_limit(
        &self,
        account: &str,
        model: &str,
        scope: LimitScope,
        until: DateTime<Utc>,
    ) {
        let mut data = self.lock();
        let entry = data.entries.entry(key(account, model)).or_default();
        match scope {
            LimitScope::Minute => entry.cooldown_until = Some(until),
            LimitScope::Day => entry.exhausted_until = Some(until),
        }
        self.touch();
    }

    /// Disables every entry of an account until restart.
    pub fn disable_account(&self, account: &str, reason: &str) {
        self.lock()
            .disabled_accounts
            .insert(account.to_string(), reason.to_string());
    }

    #[must_use]
    pub fn disabled_reason(&self, account: &str) -> Option<String> {
        self.lock().disabled_accounts.get(account).cloned()
    }

    /// True when the model was reported as no longer existing on this account.
    #[must_use]
    pub fn is_gone(&self, account: &str, model: &str) -> bool {
        self.lock()
            .entries
            .get(&key(account, model))
            .is_some_and(|e| e.gone)
    }

    /// Clears `gone` markers for an account (after its model list is refreshed).
    pub fn clear_gone(&self, account: &str) {
        let prefix = format!("{account}|");
        let mut data = self.lock();
        for (k, e) in &mut data.entries {
            if k.starts_with(&prefix) {
                e.gone = false;
            }
        }
    }

    /// Writes the state file if anything changed since the last flush.
    pub fn flush(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if !self.dirty.swap(false, Ordering::Relaxed) {
            return Ok(());
        }
        let now = Utc::now();
        let json = {
            let mut data = self.lock();
            data.version = 1;
            for e in data.entries.values_mut() {
                if e.cooldown_until.is_some_and(|t| t <= now) {
                    e.cooldown_until = None;
                }
                if e.exhausted_until.is_some_and(|t| t <= now) {
                    e.exhausted_until = None;
                }
            }
            serde_json::to_vec_pretty(&*data).map_err(std::io::Error::other)?
        };
        let result = write_atomic(path, &json);
        if result.is_err() {
            self.touch();
        }
        result
    }
}

impl Drop for StateStore {
    fn drop(&mut self) {
        if let Err(e) = self.flush() {
            tracing::warn!(target: "tierfall", error = %e, "failed to persist state on shutdown");
        }
    }
}

fn roll_day(entry: &mut EntryState, provider: ProviderKind, now: DateTime<Utc>) {
    let today = quota_day(provider, now);
    if entry.usage_day.as_deref() != Some(today.as_str()) {
        entry.usage_day = Some(today);
        entry.requests_today = 0;
        entry.tokens_today = 0;
    }
}

fn evaluate(
    entry: &mut EntryState,
    provider: ProviderKind,
    limit: EffectiveLimit,
    now: DateTime<Utc>,
) -> Availability {
    if entry.gone {
        return Availability::Gone;
    }
    if let Some(t) = entry.exhausted_until {
        if t > now {
            return Availability::Exhausted(t);
        }
        entry.exhausted_until = None;
    }
    if let Some(t) = entry.cooldown_until {
        if t > now {
            return Availability::Cooldown(t);
        }
        entry.cooldown_until = None;
    }
    let window_start = now - chrono::Duration::seconds(60);
    while entry.recent.front().is_some_and(|t| *t <= window_start) {
        entry.recent.pop_front();
    }
    if let Some(rpd) = limit.rpd
        && entry.requests_today >= u64::from(rpd)
    {
        return Availability::Limited(next_daily_reset(provider, now));
    }
    if let Some(rpm) = limit.rpm
        && entry.recent.len() >= rpm as usize
        && let Some(oldest) = entry.recent.front()
    {
        return Availability::Limited(*oldest + chrono::Duration::seconds(60));
    }
    Availability::Ready
}

/// Writes `bytes` to `path` via a temp file and rename.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 25, 16, 0, 0).unwrap()
    }

    const G: ProviderKind = ProviderKind::Gemini;

    #[test]
    fn cooldown_and_exhaustion() {
        let s = StateStore::in_memory();
        let lim = EffectiveLimit::default();
        assert_eq!(s.try_acquire("a", G, "m", lim, t0()), Availability::Ready);
        let until = t0() + chrono::Duration::seconds(30);
        s.record(
            "a",
            "m",
            &Outcome::RateLimited {
                scope: LimitScope::Minute,
                until,
            },
            0,
            30,
            t0(),
        );
        assert_eq!(
            s.try_acquire("a", G, "m", lim, t0()),
            Availability::Cooldown(until)
        );
        assert_eq!(s.try_acquire("a", G, "m", lim, until), Availability::Ready);

        let reset = next_daily_reset(G, t0());
        s.record(
            "a",
            "m",
            &Outcome::RateLimited {
                scope: LimitScope::Day,
                until: reset,
            },
            0,
            30,
            t0(),
        );
        assert_eq!(
            s.try_acquire("a", G, "m", lim, t0()),
            Availability::Exhausted(reset)
        );
        assert_eq!(
            s.try_acquire("a", G, "other", lim, t0()),
            Availability::Ready
        );
    }

    #[test]
    fn transient_backoff_and_auth_disable() {
        let s = StateStore::in_memory();
        let lim = EffectiveLimit::default();
        let tr = Outcome::Transient { reason: "x".into() };
        s.record("a", "m", &tr, 0, 10, t0());
        assert_eq!(
            s.try_acquire("a", G, "m", lim, t0()),
            Availability::Cooldown(t0() + chrono::Duration::seconds(10))
        );
        s.record("a", "m", &tr, 0, 10, t0());
        assert_eq!(
            s.try_acquire("a", G, "m", lim, t0()),
            Availability::Cooldown(t0() + chrono::Duration::seconds(20))
        );
        s.record(
            "a",
            "m",
            &Outcome::AuthFailed {
                reason: "bad key".into(),
            },
            0,
            10,
            t0(),
        );
        assert!(matches!(
            s.try_acquire("a", G, "zzz", lim, t0()),
            Availability::Disabled(_)
        ));
    }

    #[test]
    fn proactive_rpm_and_rpd() {
        let s = StateStore::in_memory();
        let lim = EffectiveLimit {
            rpm: Some(2),
            rpd: Some(3),
        };
        assert_eq!(s.try_acquire("a", G, "m", lim, t0()), Availability::Ready);
        assert_eq!(s.try_acquire("a", G, "m", lim, t0()), Availability::Ready);
        assert_eq!(
            s.try_acquire("a", G, "m", lim, t0()),
            Availability::Limited(t0() + chrono::Duration::seconds(60))
        );
        let later = t0() + chrono::Duration::seconds(61);
        assert_eq!(s.try_acquire("a", G, "m", lim, later), Availability::Ready);
        assert_eq!(
            s.try_acquire("a", G, "m", lim, later),
            Availability::Limited(next_daily_reset(G, later))
        );
        // New quota day resets the daily counter.
        let tomorrow = next_daily_reset(G, later) + chrono::Duration::seconds(1);
        assert_eq!(
            s.try_acquire("a", G, "m", lim, tomorrow),
            Availability::Ready
        );
    }

    #[test]
    fn persistence_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let far = Utc::now() + chrono::Duration::hours(5);
        {
            let s = StateStore::open(Some(path.clone()));
            s.try_acquire("a", G, "m", EffectiveLimit::default(), Utc::now());
            s.record(
                "a",
                "m",
                &Outcome::RateLimited {
                    scope: LimitScope::Day,
                    until: far,
                },
                0,
                30,
                Utc::now(),
            );
            s.record(
                "a",
                "m",
                &Outcome::AuthFailed { reason: "x".into() },
                0,
                30,
                Utc::now(),
            );
            s.flush().unwrap();
        }
        let s = StateStore::open(Some(path));
        let (avail, entry) = s.peek("a", G, "m", EffectiveLimit::default(), Utc::now());
        // Exhaustion persists; account disablement does not.
        assert_eq!(avail, Availability::Exhausted(far));
        assert_eq!(entry.requests_today, 1);
    }
}
