//! Zero-token OpenAPI 3.0/3.1 and AsyncAPI scaffolding adapter (WP-4.2, TB-11, D-110).
//!
//! Synthesizes root `REQUIREMENT` node from document metadata (`info.title`, `info.version`),
//! and extracts path operations and component schemas into `SPECIFICATION` nodes with
//! canonical keys (`API-GET-USERS`), maintaining unbroken upward `DERIVED_FROM` lineage.

use serde_json::Value;

use crate::ingest::IngestError;
use crate::ingest::parser::{ExtractedChunk, ExtractedEdge, sanitize_ast_block, slugify};
use crate::ingest::scaffolding::{ParsedScaffolding, ScaffoldingAdapter, canonical_key_slug};

/// Scaffolding adapter for OpenAPI 3.0/3.1 and AsyncAPI specifications.
#[derive(Debug, Clone, Default)]
pub struct OpenApiAdapter;

impl OpenApiAdapter {
    /// Creates a new `OpenApiAdapter`.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl ScaffoldingAdapter for OpenApiAdapter {
    fn can_handle(&self, path: &str, content: &str) -> bool {
        let path_lower = path.to_lowercase();
        let has_api_ext = path_lower.ends_with(".json")
            || path_lower.ends_with(".yaml")
            || path_lower.ends_with(".yml");

        if !has_api_ext {
            return false;
        }

        // Fast signature scan
        content.contains("openapi")
            || content.contains("swagger")
            || content.contains("asyncapi")
            || path_lower.contains("openapi")
            || path_lower.contains("swagger")
            || path_lower.contains("asyncapi")
    }

    fn parse(&self, path: &str, content: &str) -> Result<ParsedScaffolding, IngestError> {
        let value: Value = if content.trim_start().starts_with('{') || path.ends_with(".json") {
            serde_json::from_str(content).or_else(|_| serde_yaml::from_str(content))?
        } else {
            serde_yaml::from_str(content)?
        };

        let mut chunks = Vec::new();
        let mut edges = Vec::new();
        let mut alias_counter = 1usize;

        let is_asyncapi = value.get("asyncapi").is_some();

        // 1. Synthesize Root REQUIREMENT node from `info` container
        let info = value.get("info").and_then(|v| v.as_object());
        let raw_title = info
            .and_then(|i| i.get("title"))
            .and_then(|t| t.as_str())
            .unwrap_or("API Specification");
        let version = info
            .and_then(|i| i.get("version"))
            .and_then(|v| v.as_str())
            .unwrap_or("1.0.0");
        let description = info
            .and_then(|i| i.get("description"))
            .and_then(|d| d.as_str())
            .unwrap_or("");

        let root_key = canonical_key_slug("API", raw_title);
        let root_anchor = format!("{path}#root");
        let root_heading = format!("{raw_title} (v{version})");

        let root_content = if description.is_empty() {
            format!("{root_heading} specification root requirement.")
        } else {
            description.to_string()
        };

        chunks.push(ExtractedChunk {
            byte_start: 0,
            byte_end: content.len().min(500),
            heading: Some(root_heading),
            ast_anchor: root_anchor.clone(),
            parent_heading_chunk_id: None,
            rfc2119_keywords: vec!["SHALL".to_string()],
            canonical_keys: vec![root_key.clone()],
            is_candidate: false,
            content: Some(sanitize_ast_block(&root_content)),
            table_data: None,
            node_type: Some("REQUIREMENT".to_string()),
            atom_alias: Some(format!("c{alias_counter}")),
            trust_tier: "EXTERNALLY_IMPORTED".to_string(),
            modality: Some("SHALL".to_string()),
            confidence: Some(1.0),
        });
        alias_counter += 1;

        // 2. Extract Path Operations (OpenAPI)
        if let Some(paths) = value.get("paths").and_then(|p| p.as_object()) {
            for (path_name, path_item) in paths {
                if let Some(path_obj) = path_item.as_object() {
                    for (method, op_val) in path_obj {
                        let method_upper = method.to_ascii_uppercase();
                        if !matches!(
                            method_upper.as_str(),
                            "GET"
                                | "POST"
                                | "PUT"
                                | "DELETE"
                                | "PATCH"
                                | "OPTIONS"
                                | "HEAD"
                                | "TRACE"
                        ) {
                            continue;
                        }

                        let op_canonical_key = canonical_key_slug(
                            &format!("API-{method_upper}"),
                            path_name.trim_start_matches('/'),
                        );

                        let op_summary = op_val
                            .get("summary")
                            .and_then(|s| s.as_str())
                            .or_else(|| op_val.get("description").and_then(|d| d.as_str()))
                            .unwrap_or("");

                        let op_heading = format!("{method_upper} {path_name}");
                        let op_slug =
                            format!("{}-{}", method.to_ascii_lowercase(), slugify(path_name));
                        let op_anchor = format!("{path}#{op_slug}");

                        let mut body_text = format!("Endpoint: {op_heading}\n");
                        if !op_summary.is_empty() {
                            body_text.push_str(&format!("Summary: {op_summary}\n"));
                        }
                        if let Some(op_id) = op_val.get("operationId").and_then(|id| id.as_str()) {
                            body_text.push_str(&format!("OperationId: {op_id}\n"));
                        }

                        chunks.push(ExtractedChunk {
                            byte_start: 0,
                            byte_end: content.len(),
                            heading: Some(op_heading),
                            ast_anchor: op_anchor.clone(),
                            parent_heading_chunk_id: Some(root_anchor.clone()),
                            rfc2119_keywords: Vec::new(),
                            canonical_keys: vec![op_canonical_key],
                            is_candidate: false,
                            content: Some(sanitize_ast_block(&body_text)),
                            table_data: None,
                            node_type: Some("SPECIFICATION".to_string()),
                            atom_alias: Some(format!("c{alias_counter}")),
                            trust_tier: "EXTERNALLY_IMPORTED".to_string(),
                            modality: Some("MAY".to_string()),
                            confidence: Some(1.0),
                        });
                        alias_counter += 1;

                        edges.push(ExtractedEdge {
                            from_anchor: op_anchor,
                            to_anchor: root_anchor.clone(),
                            edge_type: "DERIVED_FROM".to_string(),
                        });
                    }
                }
            }
        }

        // 3. Extract Component Schemas (OpenAPI)
        if let Some(components) = value.get("components").and_then(|c| c.as_object())
            && let Some(schemas) = components.get("schemas").and_then(|s| s.as_object())
        {
            for (schema_name, schema_val) in schemas {
                let schema_canonical_key = canonical_key_slug("SCHEMA", schema_name);
                let schema_slug = slugify(schema_name);
                let schema_anchor = format!("{path}#schema-{schema_slug}");
                let schema_heading = format!("Schema: {schema_name}");

                let desc = schema_val
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("");
                let schema_type = schema_val
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("object");

                let content_text = if desc.is_empty() {
                    format!("Physical schema contract: {schema_name} (type: {schema_type})")
                } else {
                    format!("{desc} (type: {schema_type})")
                };

                chunks.push(ExtractedChunk {
                    byte_start: 0,
                    byte_end: content.len(),
                    heading: Some(schema_heading),
                    ast_anchor: schema_anchor.clone(),
                    parent_heading_chunk_id: Some(root_anchor.clone()),
                    rfc2119_keywords: Vec::new(),
                    canonical_keys: vec![schema_canonical_key],
                    is_candidate: false,
                    content: Some(sanitize_ast_block(&content_text)),
                    table_data: None,
                    node_type: Some("SPECIFICATION".to_string()),
                    atom_alias: Some(format!("c{alias_counter}")),
                    trust_tier: "EXTERNALLY_IMPORTED".to_string(),
                    modality: Some("MAY".to_string()),
                    confidence: Some(1.0),
                });
                alias_counter += 1;

                edges.push(ExtractedEdge {
                    from_anchor: schema_anchor,
                    to_anchor: root_anchor.clone(),
                    edge_type: "DERIVED_FROM".to_string(),
                });
            }
        }

        // 4. Extract Channels (AsyncAPI)
        if is_asyncapi && let Some(channels) = value.get("channels").and_then(|c| c.as_object()) {
            for (channel_name, channel_item) in channels {
                let channel_canonical_key =
                    canonical_key_slug("ASYNCAPI-CHANNEL", channel_name.trim_start_matches('/'));
                let channel_slug = slugify(channel_name);
                let channel_anchor = format!("{path}#channel-{channel_slug}");
                let channel_heading = format!("Channel: {channel_name}");

                let desc = channel_item
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("");
                let body_text = format!("AsyncAPI Channel: {channel_name}\nDescription: {desc}");

                chunks.push(ExtractedChunk {
                    byte_start: 0,
                    byte_end: content.len(),
                    heading: Some(channel_heading),
                    ast_anchor: channel_anchor.clone(),
                    parent_heading_chunk_id: Some(root_anchor.clone()),
                    rfc2119_keywords: Vec::new(),
                    canonical_keys: vec![channel_canonical_key],
                    is_candidate: false,
                    content: Some(sanitize_ast_block(&body_text)),
                    table_data: None,
                    node_type: Some("SPECIFICATION".to_string()),
                    atom_alias: Some(format!("c{alias_counter}")),
                    trust_tier: "EXTERNALLY_IMPORTED".to_string(),
                    modality: Some("MAY".to_string()),
                    confidence: Some(1.0),
                });
                alias_counter += 1;

                edges.push(ExtractedEdge {
                    from_anchor: channel_anchor,
                    to_anchor: root_anchor.clone(),
                    edge_type: "DERIVED_FROM".to_string(),
                });
            }
        }

        Ok(ParsedScaffolding::new(chunks, edges))
    }
}
