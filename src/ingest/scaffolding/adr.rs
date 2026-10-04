//! Zero-token Architecture Decision Record (ADR) scaffolding adapter (WP-4.2, TB-11, D-110).
//!
//! Parses MADR and Nygard ADR formats, synthesizing a root `REQUIREMENT` node representing
//! the architectural decision collection, and mapping accepted ADRs to `DECISION` nodes
//! with canonical keys (e.g. `ADR-0012`) maintaining upward `DERIVED_FROM` lineage.

use regex::Regex;

use crate::ingest::IngestError;
use crate::ingest::parser::{ExtractedChunk, ExtractedEdge, sanitize_ast_block, slugify};
use crate::ingest::scaffolding::{ParsedScaffolding, ScaffoldingAdapter, canonical_key_slug};

/// Scaffolding adapter for MADR and Nygard ADR documents.
#[derive(Debug, Clone, Default)]
pub struct AdrAdapter;

impl AdrAdapter {
    /// Creates a new `AdrAdapter`.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl ScaffoldingAdapter for AdrAdapter {
    fn can_handle(&self, path: &str, content: &str) -> bool {
        let path_lower = path.to_lowercase();
        let is_md = path_lower.ends_with(".md");
        if !is_md {
            return false;
        }

        // Path indicator
        if path_lower.contains("/adr/")
            || path_lower.contains("/adrs/")
            || path_lower.starts_with("adr/")
            || path_lower.starts_with("docs/adr/")
            || path_lower.ends_with(".adr.md")
        {
            return true;
        }

        // File name matches number pattern e.g. 0001-*.md
        let file_name = path.rsplit('/').next().unwrap_or(path);
        let starts_with_digits = file_name.chars().take_while(|c| c.is_ascii_digit()).count() >= 3;
        if starts_with_digits {
            return true;
        }

        // MADR or Nygard template markers in content
        content.contains("Context and Problem Statement")
            || (content.contains("## Status")
                && content.contains("## Context")
                && content.contains("## Decision"))
            || (content.contains("* status:") || content.contains("status: accepted"))
    }

    fn parse(&self, path: &str, content: &str) -> Result<ParsedScaffolding, IngestError> {
        let mut chunks = Vec::new();
        let mut edges = Vec::new();

        // 1. Synthesize Root REQUIREMENT representing the ADR collection/domain
        let root_key = "REQ-ADR-COLLECTION".to_string();
        let root_anchor = format!("{path}#root");
        let root_heading = "Architecture Decision Records Collection".to_string();
        let root_desc = "Architectural governance root requirement synthesizing baseline ADR collection authority.";

        chunks.push(ExtractedChunk {
            byte_start: 0,
            byte_end: content.len().min(300),
            heading: Some(root_heading),
            ast_anchor: root_anchor.clone(),
            parent_heading_chunk_id: None,
            rfc2119_keywords: vec!["SHALL".to_string()],
            canonical_keys: vec![root_key],
            is_candidate: false,
            content: Some(sanitize_ast_block(root_desc)),
            table_data: None,
            node_type: Some("REQUIREMENT".to_string()),
            atom_alias: Some("c1".to_string()),
            trust_tier: "EXTERNALLY_IMPORTED".to_string(),
            modality: Some("SHALL".to_string()),
            confidence: Some(1.0),
        });

        // 2. Extract ADR Number and Canonical Key (e.g. ADR-0012)
        let file_name = path.rsplit('/').next().unwrap_or(path);
        let num_re = Regex::new(r"(?i)(?:adr[-_ ]?)?(\d{1,4})").ok();
        let adr_num_opt = num_re
            .as_ref()
            .and_then(|re| re.captures(file_name))
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str());

        // Also look at document H1
        let h1_title = content
            .lines()
            .find(|line| line.starts_with("# ") || line.starts_with("#\t"))
            .map(|line| line.trim_start_matches('#').trim().to_string())
            .unwrap_or_else(|| file_name.trim_end_matches(".md").to_string());

        let adr_canonical_key = if let Some(num_str) = adr_num_opt {
            let num: u32 = num_str.parse().unwrap_or(0);
            format!("ADR-{num:04}")
        } else if let Some(cap) = num_re
            .as_ref()
            .and_then(|re| re.captures(&h1_title))
            .and_then(|cap| cap.get(1))
        {
            let num: u32 = cap.as_str().parse().unwrap_or(0);
            format!("ADR-{num:04}")
        } else {
            canonical_key_slug("ADR", &h1_title)
        };

        // 3. Extract Status (e.g. Accepted, Proposed, Superseded)
        let is_accepted = {
            let lower = content.to_lowercase();
            lower.contains("accepted")
                || lower.contains("approved")
                || (!lower.contains("rejected")
                    && !lower.contains("superseded")
                    && !lower.contains("deprecated"))
        };

        // If accepted, map to a DECISION node
        let decision_node_type = if is_accepted {
            "DECISION".to_string()
        } else {
            "SPECIFICATION".to_string()
        };

        let decision_slug = slugify(&h1_title);
        let decision_anchor = format!("{path}#{decision_slug}");

        chunks.push(ExtractedChunk {
            byte_start: 0,
            byte_end: content.len(),
            heading: Some(h1_title),
            ast_anchor: decision_anchor.clone(),
            parent_heading_chunk_id: Some(root_anchor.clone()),
            rfc2119_keywords: Vec::new(),
            canonical_keys: vec![adr_canonical_key],
            is_candidate: false,
            content: Some(sanitize_ast_block(content)),
            table_data: None,
            node_type: Some(decision_node_type),
            atom_alias: Some("c2".to_string()),
            trust_tier: "EXTERNALLY_IMPORTED".to_string(),
            modality: Some("SHALL".to_string()),
            confidence: Some(1.0),
        });

        edges.push(ExtractedEdge {
            from_anchor: decision_anchor,
            to_anchor: root_anchor,
            edge_type: "DERIVED_FROM".to_string(),
        });

        Ok(ParsedScaffolding::new(chunks, edges))
    }
}
