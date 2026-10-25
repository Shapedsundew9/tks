//! ASR lock manifest representation and asset integrity verification (EA-2.1, D-132, §6.3).

use serde::Deserialize;
use std::path::Path;

pub const ASR_LOCK_JSON: &str = include_str!("../../scripts/elicitation/asr-assets.lock");

/// Entry for an expected asset file.
#[derive(Debug, Clone, Deserialize)]
pub struct AsrFileEntry {
    pub name: String,
    pub sha256: String,
}

/// Upstream lock manifest for browser ASR assets.
#[derive(Debug, Clone, Deserialize)]
pub struct AsrLockManifest {
    pub version: String,
    pub licence: String,
    pub tarball_url: String,
    pub tarball_sha256: String,
    pub files: Vec<AsrFileEntry>,
}

impl AsrLockManifest {
    /// Loads the embedded default lock manifest.
    #[must_use]
    pub fn load_default() -> Self {
        serde_json::from_str(ASR_LOCK_JSON).expect("valid asr-assets.lock JSON")
    }

    /// Checks if a file name is explicitly permitted in the lockfile whitelist.
    #[must_use]
    pub fn is_file_allowed(&self, filename: &str) -> bool {
        self.files.iter().any(|f| f.name == filename)
    }

    /// Checks if the given directory contains all files defined in the lock manifest.
    #[must_use]
    pub fn is_dir_complete(&self, dir: &Path) -> bool {
        if !dir.is_dir() {
            return false;
        }
        for file in &self.files {
            if !dir.join(&file.name).is_file() {
                return false;
            }
        }
        true
    }
}
