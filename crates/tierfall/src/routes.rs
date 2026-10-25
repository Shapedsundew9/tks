//! Builds fallback chains (routes) from accounts, their ranked models and the config.

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;

use crate::config::{Account, ChainEntryConfig, Config};
use crate::discovery::AccountCatalog;
use crate::provider::ProviderKind;

/// One chain entry: a model on a specific account.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Target {
    pub account: String,
    pub model: String,
}

impl Target {
    #[must_use]
    pub fn new(account: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            account: account.into(),
            model: model.into(),
        }
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}", self.model, self.account)
    }
}

/// Inputs for chain building.
pub struct RouteContext<'a> {
    pub config: &'a Config,
    pub accounts: &'a [Account],
    pub catalogs: &'a BTreeMap<String, AccountCatalog>,
}

impl RouteContext<'_> {
    fn ranked(&self, account: &str) -> &[crate::discovery::RankedModel] {
        self.catalogs
            .get(account)
            .map_or(&[], |c| c.ranked.as_slice())
    }

    /// Accounts ordered by provider priority, then configured account order.
    fn accounts_by_priority(&self) -> Vec<&Account> {
        let prio = &self.config.router.provider_priority;
        let mut accts: Vec<&Account> = self.accounts.iter().collect();
        accts.sort_by_key(|a| {
            prio.iter()
                .position(|p| *p == a.provider)
                .unwrap_or(usize::MAX)
        });
        accts
    }

    /// Quality-first chain: for each tier, for each provider (priority order), the best model on
    /// every account of that provider, then the next-best on every account, and so on.
    #[must_use]
    pub fn quality_first(&self, filter: &dyn Fn(&Account, u8) -> bool) -> Vec<Target> {
        let max_tier = self
            .catalogs
            .values()
            .flat_map(|c| c.ranked.iter().map(|m| m.tier))
            .max()
            .unwrap_or(0);
        let mut providers: Vec<ProviderKind> = self.config.router.provider_priority.clone();
        for kind in ProviderKind::ALL {
            if !providers.contains(&kind) {
                providers.push(kind);
            }
        }
        let mut out = Vec::new();
        for tier in 1..=max_tier {
            for provider in &providers {
                let accts: Vec<&Account> = self
                    .accounts
                    .iter()
                    .filter(|a| a.provider == *provider && filter(a, tier))
                    .collect();
                let per_account: Vec<Vec<&str>> = accts
                    .iter()
                    .map(|a| {
                        self.ranked(&a.id)
                            .iter()
                            .filter(|m| m.tier == tier)
                            .map(|m| m.id.as_str())
                            .collect()
                    })
                    .collect();
                let depth = per_account.iter().map(Vec::len).max().unwrap_or(0);
                for rank in 0..depth {
                    for (acct, models) in accts.iter().zip(&per_account) {
                        if let Some(model) = models.get(rank) {
                            out.push(Target::new(&acct.id, *model));
                        }
                    }
                }
            }
        }
        dedup(out)
    }

    /// Built-in `fast`: Groq first, then Gemini Flash-Lite, then the rest of `auto`.
    #[must_use]
    pub fn fast(&self) -> Vec<Target> {
        let mut chain = self.quality_first(&|a, _| a.provider == ProviderKind::Groq);
        chain.extend(self.quality_first(&|a, t| a.provider == ProviderKind::Gemini && t == 2));
        chain.extend(self.quality_first(&|_, _| true));
        dedup(chain)
    }

    /// All accounts offering `model`, by provider priority then account order.
    #[must_use]
    pub fn concrete(&self, model: &str) -> Vec<Target> {
        self.accounts_by_priority()
            .into_iter()
            .filter(|a| {
                self.catalogs.get(&a.id).is_some_and(|c| c.offers(model))
                    || a.pinned_models.iter().any(|m| m == model)
            })
            .map(|a| Target::new(&a.id, model))
            .collect()
    }

    fn expand_entry(
        &self,
        entry: &ChainEntryConfig,
        builtin: &BTreeMap<String, Vec<Target>>,
        visiting: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) -> Vec<Target> {
        if let Some(route) = &entry.route {
            return self.expand_route(route, builtin, visiting, warnings);
        }
        let model = entry.model.as_deref().unwrap_or("*");
        if let Some(account) = &entry.account {
            if !self.accounts.iter().any(|a| &a.id == account) {
                warnings.push(format!(
                    "route entry references unavailable account '{account}'"
                ));
                return Vec::new();
            }
            if model == "*" {
                return self
                    .ranked(account)
                    .iter()
                    .map(|m| Target::new(account, &m.id))
                    .collect();
            }
            return vec![Target::new(account, model)];
        }
        if let Some(provider) = entry.provider {
            if model == "*" {
                return self.quality_first(&|a, _| a.provider == provider);
            }
            return self
                .accounts
                .iter()
                .filter(|a| a.provider == provider)
                .map(|a| Target::new(&a.id, model))
                .collect();
        }
        if model == "*" {
            return self.quality_first(&|_, _| true);
        }
        let chain = self.concrete(model);
        if chain.is_empty() {
            warnings.push(format!("no account offers model '{model}'"));
        }
        chain
    }

    fn expand_route(
        &self,
        name: &str,
        builtin: &BTreeMap<String, Vec<Target>>,
        visiting: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) -> Vec<Target> {
        let configured = self
            .config
            .routes
            .iter()
            .find(|r| r.name == name)
            .filter(|_| !visiting.iter().any(|v| v == name));
        match configured {
            Some(route) => {
                visiting.push(name.to_string());
                let mut chain = Vec::new();
                for entry in &route.chain {
                    chain.extend(self.expand_entry(entry, builtin, visiting, warnings));
                }
                visiting.pop();
                dedup(chain)
            }
            None => match builtin.get(name) {
                Some(chain) => chain.clone(),
                None => {
                    warnings.push(format!(
                        "route references unknown or recursive route '{name}'"
                    ));
                    Vec::new()
                }
            },
        }
    }

    /// Builds every route: `auto`, the built-in aliases (unless disabled), then configured routes
    /// (which override built-ins of the same name).
    #[must_use]
    pub fn build_table(&self) -> (BTreeMap<String, Vec<Target>>, Vec<String>) {
        let mut builtin = BTreeMap::new();
        builtin.insert("auto".to_string(), self.quality_first(&|_, _| true));
        if self.config.router.builtin_routes {
            builtin.insert("fast".to_string(), self.fast());
        }
        let mut warnings = Vec::new();
        let mut table = builtin.clone();
        for route in &self.config.routes {
            let mut visiting = Vec::new();
            let chain = self.expand_route(&route.name, &builtin, &mut visiting, &mut warnings);
            table.insert(route.name.clone(), chain);
        }
        (table, warnings)
    }
}

/// Removes duplicate targets, keeping the first occurrence.
#[must_use]
pub fn dedup(chain: Vec<Target>) -> Vec<Target> {
    let mut seen = HashSet::new();
    chain
        .into_iter()
        .filter(|t| seen.insert(t.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::{DiscoverySource, RankedModel};

    fn acct(id: &str, provider: ProviderKind) -> Account {
        Account {
            id: id.into(),
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

    fn catalog(models: &[(&str, u8)]) -> AccountCatalog {
        AccountCatalog {
            source: DiscoverySource::Live,
            fetched_at: None,
            available: models.iter().map(|(m, _)| (*m).to_string()).collect(),
            ranked: models
                .iter()
                .map(|(m, t)| RankedModel {
                    id: (*m).into(),
                    tier: *t,
                })
                .collect(),
            error: None,
        }
    }

    fn fixture() -> (Vec<Account>, BTreeMap<String, AccountCatalog>) {
        let accounts = vec![
            acct("gemini-ss", ProviderKind::Gemini),
            acct("gemini-am", ProviderKind::Gemini),
            acct("groq", ProviderKind::Groq),
            acct("openrouter", ProviderKind::Openrouter),
            acct("ollama", ProviderKind::Ollama),
        ];
        let gem = [
            ("g-flash-new", 1),
            ("g-flash-old", 1),
            ("g-lite", 2),
            ("gemma", 3),
        ];
        let mut catalogs = BTreeMap::new();
        catalogs.insert("gemini-ss".into(), catalog(&gem));
        catalogs.insert("gemini-am".into(), catalog(&gem));
        catalogs.insert("groq".into(), catalog(&[("oss-120b", 1), ("oss-20b", 2)]));
        catalogs.insert(
            "openrouter".into(),
            catalog(&[("big:free", 1), ("router", 3)]),
        );
        catalogs.insert("ollama".into(), catalog(&[("qwen3:8b", 4)]));
        (accounts, catalogs)
    }

    fn names(chain: &[Target]) -> Vec<String> {
        chain.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn auto_is_quality_first_across_accounts() {
        let (accounts, catalogs) = fixture();
        let config = Config::default();
        let ctx = RouteContext {
            config: &config,
            accounts: &accounts,
            catalogs: &catalogs,
        };
        let (table, warnings) = ctx.build_table();
        assert!(warnings.is_empty());
        assert_eq!(
            names(&table["auto"]),
            [
                "g-flash-new@gemini-ss",
                "g-flash-new@gemini-am",
                "g-flash-old@gemini-ss",
                "g-flash-old@gemini-am",
                "oss-120b@groq",
                "big:free@openrouter",
                "g-lite@gemini-ss",
                "g-lite@gemini-am",
                "oss-20b@groq",
                "gemma@gemini-ss",
                "gemma@gemini-am",
                "router@openrouter",
                "qwen3:8b@ollama",
            ]
        );
        let fast = names(&table["fast"]);
        assert_eq!(
            &fast[..4],
            [
                "oss-120b@groq",
                "oss-20b@groq",
                "g-lite@gemini-ss",
                "g-lite@gemini-am"
            ]
        );
        assert_eq!(fast.len(), table["auto"].len());
    }

    #[test]
    fn configured_routes_expand_all_entry_forms() {
        let (accounts, catalogs) = fixture();
        let config = Config::from_toml_str(
            r#"
            [[routes]]
            name = "smart"
            chain = [
              { provider = "gemini", model = "g-flash-new" },
              { account = "groq", model = "*" },
              { model = "big:free" },
              { route = "auto" },
            ]
            [[routes]]
            name = "auto"
            chain = [{ account = "ollama", model = "qwen3:8b" }, { route = "auto" }]
            "#,
        )
        .unwrap();
        let ctx = RouteContext {
            config: &config,
            accounts: &accounts,
            catalogs: &catalogs,
        };
        let (table, _) = ctx.build_table();
        let smart = names(&table["smart"]);
        assert_eq!(
            &smart[..5],
            [
                "g-flash-new@gemini-ss",
                "g-flash-new@gemini-am",
                "oss-120b@groq",
                "oss-20b@groq",
                "big:free@openrouter"
            ]
        );
        // `smart` splices the configured `auto`, which starts with ollama then the built-in auto.
        assert_eq!(smart[5], "qwen3:8b@ollama");
        assert_eq!(names(&table["auto"])[0], "qwen3:8b@ollama");
        assert_eq!(table["auto"].len(), 13);
    }

    #[test]
    fn concrete_model_routing() {
        let (accounts, catalogs) = fixture();
        let config = Config::default();
        let ctx = RouteContext {
            config: &config,
            accounts: &accounts,
            catalogs: &catalogs,
        };
        assert_eq!(
            names(&ctx.concrete("g-lite")),
            ["g-lite@gemini-ss", "g-lite@gemini-am"]
        );
        assert!(ctx.concrete("nope").is_empty());
    }
}
