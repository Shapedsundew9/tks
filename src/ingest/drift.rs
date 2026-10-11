//! Code-side contract drift detection engine (WP-4.5, TB-12.3).
//!
//! Compares files and public interfaces touched in a git pull request diff against
//! active requirement specifications and verification manifests, flagging un-attributed
//! modifications to public interfaces or contracts before merge.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Structured anomaly detected during code-side contract drift analysis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractDriftAnomaly {
    /// File path where un-attributed contract drift was detected.
    pub file: String,
    /// Standardized anomaly type identifier (e.g. `UNATTRIBUTED_CONTRACT_MODIFICATION`).
    pub anomaly_type: String,
    /// Human-readable explanation and remediation guidance.
    pub message: String,
}

/// Comprehensive outcome of a contract drift inspection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractDriftResult {
    /// Overall evaluation status: `CLEAN` or `DRIFT_DETECTED`.
    pub status: String,
    /// All file paths touched in the diff.
    pub touched_files: Vec<String>,
    /// Subset of touched files classified as contracts, schemas, or public interfaces.
    pub contract_files: Vec<String>,
    /// Un-attributed contract modifications or interface drifts detected.
    pub anomalies: Vec<ContractDriftAnomaly>,
}

/// Parses a unified git diff and extracts distinct touched file paths.
#[must_use]
pub fn parse_diff_touched_files(diff: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let path_b = parts[3].trim_start_matches("b/");
                if path_b != "/dev/null" && !files.contains(&path_b.to_string()) {
                    files.push(path_b.to_string());
                }
            }
        } else if let Some(stripped) = line.strip_prefix("+++ b/") {
            let path = stripped.trim();
            if path != "/dev/null" && !files.contains(&path.to_string()) {
                files.push(path.to_string());
            }
        } else if let Some(stripped) = line.strip_prefix("--- a/") {
            let path = stripped.trim();
            if path != "/dev/null" && !files.contains(&path.to_string()) {
                files.push(path.to_string());
            }
        }
    }
    files
}

/// Determines whether a repository file path represents a contract, schema, or public interface.
#[must_use]
pub fn is_contract_file(path: &str) -> bool {
    let p = path.to_lowercase();
    p.starts_with("specs/")
        || p.starts_with("docs/specs/")
        || p.starts_with(".github/specs/")
        || p.starts_with(".kiro/")
        || p.starts_with("contracts/")
        || p.starts_with("proto/")
        || p.starts_with("api/")
        || p.starts_with("openapi/")
        || p.ends_with(".openapi.yaml")
        || p.ends_with(".openapi.yml")
        || p.ends_with(".openapi.json")
        || p.ends_with("cargo.toml")
        || p.starts_with("src/gateway/routes/")
        || p.starts_with("src/gateway/mcp/")
        || p == "src/lib.rs"
        || p == "src/main.rs"
}

/// Inspects diff lines for modifications to exported public Rust signatures or contract declarations.
#[must_use]
pub fn diff_modifies_public_interface(diff: &str) -> bool {
    for line in diff.lines() {
        if (line.starts_with('+') || line.starts_with('-'))
            && !line.starts_with("+++")
            && !line.starts_with("---")
        {
            let trimmed = line[1..].trim_start();
            if trimmed.starts_with("pub fn ")
                || trimmed.starts_with("pub async fn ")
                || trimmed.starts_with("pub struct ")
                || trimmed.starts_with("pub enum ")
                || trimmed.starts_with("pub trait ")
                || trimmed.starts_with("pub type ")
                || trimmed.starts_with("pub const ")
                || trimmed.starts_with("paths:")
                || trimmed.starts_with("components:")
                || trimmed.starts_with("[dependencies")
            {
                return true;
            }
        }
    }
    false
}

/// Compares files touched in `diff` against paths recorded in active requirement specifications,
/// verification manifests, and task attributes, flagging un-attributed contract modifications.
///
/// # Errors
///
/// Returns `tokio_postgres::Error` if database query fails.
pub async fn check_contract_drift(
    client: &(impl tokio_postgres::GenericClient + ?Sized),
    diff: &str,
    target_task_ids: &[Uuid],
) -> Result<ContractDriftResult, tokio_postgres::Error> {
    let touched_files = parse_diff_touched_files(diff);
    let mut contract_files = Vec::new();
    let mut anomalies = Vec::new();

    for file in &touched_files {
        let is_contract = is_contract_file(file) || diff_modifies_public_interface(diff);
        if !is_contract {
            continue;
        }

        contract_files.push(file.clone());

        // Check if file is attributed to an active requirement or task
        let attributed_rows = client
            .query(
                "SELECT id FROM graph_nodes \
                 WHERE lifecycle_state = 'ACTIVE' \
                   AND ( \
                       doc_path = $1 \
                       OR (attributes->>'doc_path') = $1 \
                       OR (attributes->'contract_paths') @> to_jsonb($1::text) \
                       OR (attributes->'touched_files') @> to_jsonb($1::text) \
                       OR (attributes->'file_paths') @> to_jsonb($1::text) \
                       OR (attributes->'target_files') @> to_jsonb($1::text) \
                       OR (attributes->'manifest_files') @> to_jsonb($1::text) \
                   );",
                &[&file],
            )
            .await?;

        let mut is_attributed = !attributed_rows.is_empty();

        // Also check if any target task explicitly attributes this file
        if !is_attributed && !target_task_ids.is_empty() {
            let task_rows = client
                .query(
                    "SELECT id FROM graph_nodes \
                     WHERE id = ANY($1::uuid[]) \
                       AND ( \
                           doc_path = $2 \
                           OR (attributes->>'file') = $2 \
                           OR (attributes->'touched_files') @> to_jsonb($2::text) \
                           OR (attributes->'file_paths') @> to_jsonb($2::text) \
                       );",
                    &[&target_task_ids, &file],
                )
                .await?;
            if !task_rows.is_empty() {
                is_attributed = true;
            }
        }

        if !is_attributed {
            anomalies.push(ContractDriftAnomaly {
                file: file.clone(),
                anomaly_type: "UNATTRIBUTED_CONTRACT_MODIFICATION".to_string(),
                message: format!(
                    "Touched public contract or interface '{file}' is not attributed to any active requirement specification or verification task"
                ),
            });
        }
    }

    let status = if anomalies.is_empty() {
        "CLEAN".to_string()
    } else {
        "DRIFT_DETECTED".to_string()
    };

    Ok(ContractDriftResult {
        status,
        touched_files,
        contract_files,
        anomalies,
    })
}
