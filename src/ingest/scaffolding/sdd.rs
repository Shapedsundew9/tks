//! Zero-token Spec-Driven Development (SDD), BDD, and agent steering adapter (WP-4.2, TB-11, D-110).
//!
//! Parses BDD Feature files, GitHub Spec Kit (`.github/specs/`), AWS Kiro (`.kiro/`),
//! and repository steering directives (`AGENTS.md`, `CLAUDE.md`), synthesizing root
//! `REQUIREMENT` nodes satisfying Invariant INV-1 with upward `DERIVED_FROM` lineage.

use crate::ingest::IngestError;
use crate::ingest::parser::{ExtractedChunk, ExtractedEdge, sanitize_ast_block, slugify};
use crate::ingest::scaffolding::{ParsedScaffolding, ScaffoldingAdapter, canonical_key_slug};

/// Scaffolding adapter for BDD Feature files, Spec Kit, AWS Kiro, and AGENTS.md / CLAUDE.md.
#[derive(Debug, Clone, Default)]
pub struct SddAdapter;

impl SddAdapter {
    /// Creates a new `SddAdapter`.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    fn is_bdd(&self, path: &str, content: &str) -> bool {
        path.ends_with(".feature")
            || content
                .lines()
                .any(|l| l.trim_start().starts_with("Feature:"))
    }

    fn is_spec_kit(&self, path: &str) -> bool {
        path.contains(".github/specs/") || path.contains("/specs/") && path.ends_with(".md")
    }

    fn is_kiro(&self, path: &str) -> bool {
        path.contains(".kiro/") || path.contains("/kiro/")
    }

    fn is_agent_steering(&self, path: &str) -> bool {
        let file_name = path.rsplit('/').next().unwrap_or(path).to_ascii_uppercase();
        file_name == "AGENTS.MD" || file_name == "CLAUDE.MD"
    }

    fn parse_bdd(&self, path: &str, content: &str) -> Result<ParsedScaffolding, IngestError> {
        let mut chunks = Vec::new();
        let mut edges = Vec::new();
        let mut alias_counter = 1usize;

        let mut feature_title = "BDD Feature".to_string();
        let mut feature_desc = String::new();
        let mut in_feature_header = false;

        let mut current_scenario_title: Option<String> = None;
        let mut current_scenario_lines: Vec<String> = Vec::new();

        let mut scenarios: Vec<(String, String)> = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Feature:") {
                feature_title = trimmed.trim_start_matches("Feature:").trim().to_string();
                in_feature_header = true;
                continue;
            }

            if trimmed.starts_with("Scenario:") || trimmed.starts_with("Scenario Outline:") {
                in_feature_header = false;
                if let Some(st) = current_scenario_title.take() {
                    scenarios.push((st, current_scenario_lines.join("\n")));
                    current_scenario_lines.clear();
                }
                let st = if trimmed.starts_with("Scenario:") {
                    trimmed.trim_start_matches("Scenario:").trim().to_string()
                } else {
                    trimmed
                        .trim_start_matches("Scenario Outline:")
                        .trim()
                        .to_string()
                };
                current_scenario_title = Some(st);
                continue;
            }

            if in_feature_header {
                if !trimmed.is_empty() {
                    feature_desc.push_str(trimmed);
                    feature_desc.push('\n');
                }
            } else if current_scenario_title.is_some() {
                current_scenario_lines.push(line.to_string());
            }
        }

        if let Some(st) = current_scenario_title.take() {
            scenarios.push((st, current_scenario_lines.join("\n")));
        }

        // Synthesize Root REQUIREMENT node from BDD Feature
        let root_key = canonical_key_slug("REQ", &feature_title);
        let root_anchor = format!("{path}#feature");

        chunks.push(ExtractedChunk {
            byte_start: 0,
            byte_end: content.len().min(500),
            heading: Some(format!("Feature: {feature_title}")),
            ast_anchor: root_anchor.clone(),
            parent_heading_chunk_id: None,
            rfc2119_keywords: vec!["SHALL".to_string()],
            canonical_keys: vec![root_key],
            is_candidate: false,
            content: Some(sanitize_ast_block(if feature_desc.is_empty() {
                &feature_title
            } else {
                &feature_desc
            })),
            table_data: None,
            node_type: Some("REQUIREMENT".to_string()),
            atom_alias: Some(format!("c{alias_counter}")),
            trust_tier: "EXTERNALLY_IMPORTED".to_string(),
            modality: Some("SHALL".to_string()),
            confidence: Some(1.0),
        });
        alias_counter += 1;

        // Scenarios become SPECIFICATION nodes linked upward
        for (sc_title, sc_content) in scenarios {
            let sc_key = canonical_key_slug("SPEC", &sc_title);
            let sc_slug = slugify(&sc_title);
            let sc_anchor = format!("{path}#scenario-{sc_slug}");

            chunks.push(ExtractedChunk {
                byte_start: 0,
                byte_end: content.len(),
                heading: Some(format!("Scenario: {sc_title}")),
                ast_anchor: sc_anchor.clone(),
                parent_heading_chunk_id: Some(root_anchor.clone()),
                rfc2119_keywords: Vec::new(),
                canonical_keys: vec![sc_key],
                is_candidate: false,
                content: Some(sanitize_ast_block(&sc_content)),
                table_data: None,
                node_type: Some("SPECIFICATION".to_string()),
                atom_alias: Some(format!("c{alias_counter}")),
                trust_tier: "EXTERNALLY_IMPORTED".to_string(),
                modality: Some("MAY".to_string()),
                confidence: Some(1.0),
            });
            alias_counter += 1;

            edges.push(ExtractedEdge {
                from_anchor: sc_anchor,
                to_anchor: root_anchor.clone(),
                edge_type: "DERIVED_FROM".to_string(),
            });
        }

        Ok(ParsedScaffolding::new(chunks, edges))
    }

    fn parse_agent_steering(
        &self,
        path: &str,
        content: &str,
    ) -> Result<ParsedScaffolding, IngestError> {
        let mut chunks = Vec::new();
        let mut edges = Vec::new();
        let mut alias_counter = 1usize;

        let file_name = path.rsplit('/').next().unwrap_or(path);
        let root_key = if file_name.to_ascii_uppercase().contains("CLAUDE") {
            "REQ-CLAUDE-STEERING".to_string()
        } else {
            "REQ-AGENT-STEERING".to_string()
        };

        let root_anchor = format!("{path}#root");
        let root_heading = format!("Agent Steering Directives: {file_name}");

        // Synthesize Root REQUIREMENT
        chunks.push(ExtractedChunk {
            byte_start: 0,
            byte_end: content.len().min(300),
            heading: Some(root_heading),
            ast_anchor: root_anchor.clone(),
            parent_heading_chunk_id: None,
            rfc2119_keywords: vec!["SHALL".to_string()],
            canonical_keys: vec![root_key],
            is_candidate: false,
            content: Some(sanitize_ast_block(
                "Agent steering directives and governance guidelines root requirement.",
            )),
            table_data: None,
            node_type: Some("REQUIREMENT".to_string()),
            atom_alias: Some(format!("c{alias_counter}")),
            trust_tier: "EXTERNALLY_IMPORTED".to_string(),
            modality: Some("SHALL".to_string()),
            confidence: Some(1.0),
        });
        alias_counter += 1;

        // Parse major headings / sections into SPECIFICATION nodes
        let mut current_sec_title: Option<String> = None;
        let mut current_sec_lines: Vec<String> = Vec::new();

        for line in content.lines() {
            if line.starts_with('#') {
                if let Some(title) = current_sec_title.take() {
                    let sec_key = canonical_key_slug("SPEC-STEERING", &title);
                    let sec_slug = slugify(&title);
                    let sec_anchor = format!("{path}#{sec_slug}");

                    chunks.push(ExtractedChunk {
                        byte_start: 0,
                        byte_end: content.len(),
                        heading: Some(title),
                        ast_anchor: sec_anchor.clone(),
                        parent_heading_chunk_id: Some(root_anchor.clone()),
                        rfc2119_keywords: Vec::new(),
                        canonical_keys: vec![sec_key],
                        is_candidate: false,
                        content: Some(sanitize_ast_block(&current_sec_lines.join("\n"))),
                        table_data: None,
                        node_type: Some("SPECIFICATION".to_string()),
                        atom_alias: Some(format!("c{alias_counter}")),
                        trust_tier: "EXTERNALLY_IMPORTED".to_string(),
                        modality: Some("MAY".to_string()),
                        confidence: Some(1.0),
                    });
                    alias_counter += 1;

                    edges.push(ExtractedEdge {
                        from_anchor: sec_anchor,
                        to_anchor: root_anchor.clone(),
                        edge_type: "DERIVED_FROM".to_string(),
                    });
                    current_sec_lines.clear();
                }

                let heading = line.trim_start_matches('#').trim().to_string();
                if !heading.is_empty() {
                    current_sec_title = Some(heading);
                }
            } else if current_sec_title.is_some() {
                current_sec_lines.push(line.to_string());
            }
        }

        if let Some(title) = current_sec_title.take() {
            let sec_key = canonical_key_slug("SPEC-STEERING", &title);
            let sec_slug = slugify(&title);
            let sec_anchor = format!("{path}#{sec_slug}");

            chunks.push(ExtractedChunk {
                byte_start: 0,
                byte_end: content.len(),
                heading: Some(title),
                ast_anchor: sec_anchor.clone(),
                parent_heading_chunk_id: Some(root_anchor.clone()),
                rfc2119_keywords: Vec::new(),
                canonical_keys: vec![sec_key],
                is_candidate: false,
                content: Some(sanitize_ast_block(&current_sec_lines.join("\n"))),
                table_data: None,
                node_type: Some("SPECIFICATION".to_string()),
                atom_alias: Some(format!("c{alias_counter}")),
                trust_tier: "EXTERNALLY_IMPORTED".to_string(),
                modality: Some("MAY".to_string()),
                confidence: Some(1.0),
            });

            edges.push(ExtractedEdge {
                from_anchor: sec_anchor,
                to_anchor: root_anchor,
                edge_type: "DERIVED_FROM".to_string(),
            });
        }

        Ok(ParsedScaffolding::new(chunks, edges))
    }

    fn parse_spec_or_kiro(
        &self,
        path: &str,
        content: &str,
        prefix: &str,
    ) -> Result<ParsedScaffolding, IngestError> {
        let mut chunks = Vec::new();
        let mut edges = Vec::new();
        let mut alias_counter = 1usize;

        let file_name = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".md");
        let first_h1 = content
            .lines()
            .find(|l| l.starts_with("# "))
            .map(|l| l.trim_start_matches('#').trim().to_string())
            .unwrap_or_else(|| file_name.to_string());

        let root_key = canonical_key_slug(prefix, &first_h1);
        let root_anchor = format!("{path}#root");

        // Synthesize Root REQUIREMENT
        chunks.push(ExtractedChunk {
            byte_start: 0,
            byte_end: content.len().min(400),
            heading: Some(first_h1.clone()),
            ast_anchor: root_anchor.clone(),
            parent_heading_chunk_id: None,
            rfc2119_keywords: vec!["SHALL".to_string()],
            canonical_keys: vec![root_key],
            is_candidate: false,
            content: Some(sanitize_ast_block(&first_h1)),
            table_data: None,
            node_type: Some("REQUIREMENT".to_string()),
            atom_alias: Some(format!("c{alias_counter}")),
            trust_tier: "EXTERNALLY_IMPORTED".to_string(),
            modality: Some("SHALL".to_string()),
            confidence: Some(1.0),
        });
        alias_counter += 1;

        // Subsections become SPECIFICATION nodes linked upward
        let mut current_sec_title: Option<String> = None;
        let mut current_sec_lines: Vec<String> = Vec::new();

        for line in content.lines() {
            if line.starts_with("## ") || line.starts_with("### ") {
                if let Some(title) = current_sec_title.take() {
                    let sec_key = canonical_key_slug(&format!("{prefix}-SPEC"), &title);
                    let sec_slug = slugify(&title);
                    let sec_anchor = format!("{path}#{sec_slug}");

                    chunks.push(ExtractedChunk {
                        byte_start: 0,
                        byte_end: content.len(),
                        heading: Some(title),
                        ast_anchor: sec_anchor.clone(),
                        parent_heading_chunk_id: Some(root_anchor.clone()),
                        rfc2119_keywords: Vec::new(),
                        canonical_keys: vec![sec_key],
                        is_candidate: false,
                        content: Some(sanitize_ast_block(&current_sec_lines.join("\n"))),
                        table_data: None,
                        node_type: Some("SPECIFICATION".to_string()),
                        atom_alias: Some(format!("c{alias_counter}")),
                        trust_tier: "EXTERNALLY_IMPORTED".to_string(),
                        modality: Some("MAY".to_string()),
                        confidence: Some(1.0),
                    });
                    alias_counter += 1;

                    edges.push(ExtractedEdge {
                        from_anchor: sec_anchor,
                        to_anchor: root_anchor.clone(),
                        edge_type: "DERIVED_FROM".to_string(),
                    });
                    current_sec_lines.clear();
                }

                let heading = line.trim_start_matches('#').trim().to_string();
                if !heading.is_empty() {
                    current_sec_title = Some(heading);
                }
            } else if current_sec_title.is_some() {
                current_sec_lines.push(line.to_string());
            }
        }

        if let Some(title) = current_sec_title.take() {
            let sec_key = canonical_key_slug(&format!("{prefix}-SPEC"), &title);
            let sec_slug = slugify(&title);
            let sec_anchor = format!("{path}#{sec_slug}");

            chunks.push(ExtractedChunk {
                byte_start: 0,
                byte_end: content.len(),
                heading: Some(title),
                ast_anchor: sec_anchor.clone(),
                parent_heading_chunk_id: Some(root_anchor.clone()),
                rfc2119_keywords: Vec::new(),
                canonical_keys: vec![sec_key],
                is_candidate: false,
                content: Some(sanitize_ast_block(&current_sec_lines.join("\n"))),
                table_data: None,
                node_type: Some("SPECIFICATION".to_string()),
                atom_alias: Some(format!("c{alias_counter}")),
                trust_tier: "EXTERNALLY_IMPORTED".to_string(),
                modality: Some("MAY".to_string()),
                confidence: Some(1.0),
            });

            edges.push(ExtractedEdge {
                from_anchor: sec_anchor,
                to_anchor: root_anchor,
                edge_type: "DERIVED_FROM".to_string(),
            });
        }

        Ok(ParsedScaffolding::new(chunks, edges))
    }
}

impl ScaffoldingAdapter for SddAdapter {
    fn can_handle(&self, path: &str, content: &str) -> bool {
        self.is_bdd(path, content)
            || self.is_spec_kit(path)
            || self.is_kiro(path)
            || self.is_agent_steering(path)
    }

    fn parse(&self, path: &str, content: &str) -> Result<ParsedScaffolding, IngestError> {
        if self.is_bdd(path, content) {
            self.parse_bdd(path, content)
        } else if self.is_agent_steering(path) {
            self.parse_agent_steering(path, content)
        } else if self.is_kiro(path) {
            self.parse_spec_or_kiro(path, content, "KIRO")
        } else {
            self.parse_spec_or_kiro(path, content, "SPECKIT")
        }
    }
}
