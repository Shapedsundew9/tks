//! Elicitation configuration (D-EA-6, D-EA-11, D-EA-12).

use std::path::PathBuf;
use std::time::Duration;

/// Provider used to generate facilitator proposals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FacilitatorProviderKind {
    #[default]
    None,
    Ollama,
    Tierfall,
}

impl FacilitatorProviderKind {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Ollama => "ollama",
            Self::Tierfall => "tierfall",
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
            other => Err(format!(
                "invalid facilitator provider '{other}'; expected 'none', 'ollama', or 'tierfall'"
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
            asr_assets_dir: None,
            idle_expiry: Duration::from_secs(24 * 3600),
        }
    }
}
