//! Elicitation configuration (D-EA-6, D-EA-11, D-EA-12).

use std::path::PathBuf;
use std::time::Duration;

/// Provider used to generate facilitator proposals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FacilitatorProviderKind {
    #[default]
    None,
    Ollama,
    Tierfall,
    Cli,
}

impl FacilitatorProviderKind {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Ollama => "ollama",
            Self::Tierfall => "tierfall",
            Self::Cli => "cli",
        }
    }
}

impl std::str::FromStr for FacilitatorProviderKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "ollama" => Ok(Self::Ollama),
            "tierfall" => Ok(Self::Tierfall),
            "cli" | "agy" | "antigravity-cli" => Ok(Self::Cli),
            other => Err(format!(
                "invalid facilitator provider '{other}'; expected 'none', 'ollama', 'tierfall', or 'cli'"
            )),
        }
    }
}

/// Runtime configuration for elicitation draft sessions and engine capabilities.
#[derive(Debug, Clone)]
pub struct ElicitationConfig {
    pub facilitator_provider: FacilitatorProviderKind,
    pub ollama_url: String,
    pub facilitator_model: String,
    pub facilitator_route: String,
    pub cli_tool: String,
    pub cli_model: Option<String>,
    pub asr_assets_dir: Option<PathBuf>,
    pub idle_expiry: Duration,
}

impl Default for ElicitationConfig {
    fn default() -> Self {
        Self {
            facilitator_provider: FacilitatorProviderKind::None,
            ollama_url: crate::elicitation::facilitator::resolve_ollama_url(""),
            facilitator_model: "qwen3:8b".to_string(),
            facilitator_route: "auto".to_string(),
            cli_tool: "agy".to_string(),
            cli_model: None,
            asr_assets_dir: None,
            idle_expiry: Duration::from_secs(24 * 3600),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_facilitator_provider_kind_parsing() {
        assert_eq!(
            FacilitatorProviderKind::from_str("none").unwrap(),
            FacilitatorProviderKind::None
        );
        assert_eq!(
            FacilitatorProviderKind::from_str("ollama").unwrap(),
            FacilitatorProviderKind::Ollama
        );
        assert_eq!(
            FacilitatorProviderKind::from_str("tierfall").unwrap(),
            FacilitatorProviderKind::Tierfall
        );
        assert_eq!(
            FacilitatorProviderKind::from_str("cli").unwrap(),
            FacilitatorProviderKind::Cli
        );
        assert_eq!(
            FacilitatorProviderKind::from_str("agy").unwrap(),
            FacilitatorProviderKind::Cli
        );
        assert_eq!(
            FacilitatorProviderKind::from_str("antigravity-cli").unwrap(),
            FacilitatorProviderKind::Cli
        );

        let err = FacilitatorProviderKind::from_str("unknown").unwrap_err();
        assert!(err.contains("'none', 'ollama', 'tierfall', or 'cli'"));
    }

    #[test]
    fn test_facilitator_provider_kind_as_str() {
        assert_eq!(FacilitatorProviderKind::None.as_str(), "none");
        assert_eq!(FacilitatorProviderKind::Ollama.as_str(), "ollama");
        assert_eq!(FacilitatorProviderKind::Tierfall.as_str(), "tierfall");
        assert_eq!(FacilitatorProviderKind::Cli.as_str(), "cli");
    }
}
