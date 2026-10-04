//! Semantic classification prompt builder and compact JSON response parser (WP-0.3).

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::ingest::parser::ExtractedChunk;

/// Permitted node types in the Knowledge Substrate ontology.
pub const VALID_NODE_TYPES: &[&str] = &[
    "REQUIREMENT",
    "SPECIFICATION",
    "TASK",
    "VERIFICATION",
    "DECISION",
    "UNCLASSIFIED",
];

/// Permitted governance policies in the Knowledge Substrate ontology.
pub const VALID_GOVERNANCE_POLICIES: &[&str] =
    &["HUMAN_REVIEW_REQUIRED", "AUTONOMOUS_ELABORATION", "LOCKED"];

/// Permitted normative modalities in the Knowledge Substrate ontology.
pub const VALID_MODALITIES: &[&str] = &["SHALL", "SHOULD", "MAY", "UNCLASSIFIED"];

/// Structured semantic classification tuple for an extracted AST chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassificationResult {
    /// Chunk identifier (e.g. ordinal alias "c1", "c2" or AST anchor).
    pub chunk_id: String,
    /// Entity node type (e.g. "REQUIREMENT", "SPECIFICATION", "TASK", "UNCLASSIFIED").
    pub node_type: String,
    /// Governance policy (e.g. "HUMAN_REVIEW_REQUIRED", "AUTONOMOUS_ELABORATION", "LOCKED").
    pub governance_policy: String,
    /// Normative modality (`SHALL`, `SHOULD`, `MAY`, `UNCLASSIFIED`).
    #[serde(default = "default_modality")]
    pub modality: String,
    /// Classification confidence score (0.0 to 1.0).
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    /// Model provenance (e.g. "qwen3:8b", "rfc2119_fallback").
    #[serde(default)]
    pub model_provenance: Option<String>,
}

fn default_modality() -> String {
    "UNCLASSIFIED".to_string()
}

fn default_confidence() -> f64 {
    1.0
}

/// Errors that can occur during classification prompt generation or response parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassificationError {
    /// JSON parsing or syntax error.
    Json(String),
    /// Structural error in the response format (e.g. not a JSON array).
    InvalidFormat(String),
    /// Invalid or unrecognized node type.
    InvalidNodeType(String),
    /// Invalid or unrecognized governance policy.
    InvalidGovernancePolicy(String),
    /// The response was empty or contained only whitespace.
    EmptyResponse,
}

impl fmt::Display for ClassificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(msg) => write!(f, "JSON parse error: {msg}"),
            Self::InvalidFormat(msg) => write!(f, "invalid response format: {msg}"),
            Self::InvalidNodeType(t) => write!(f, "unrecognized node_type '{t}'"),
            Self::InvalidGovernancePolicy(p) => write!(f, "unrecognized governance_policy '{p}'"),
            Self::EmptyResponse => write!(f, "classification response was empty"),
        }
    }
}

impl std::error::Error for ClassificationError {}

/// Builds a compact semantic classification prompt for candidate Markdown chunks.
///
/// Prompts an external LLM to classify candidate chunks into compact JSON tuples
/// (`alias`, `modality`, `confidence`) without echoing or repeating
/// chunk text, achieving minimal output token usage (<10% of document tokens).
#[must_use]
pub fn build_classification_prompt(chunks: &[ExtractedChunk]) -> String {
    let candidate_chunks: Vec<&ExtractedChunk> = if chunks.iter().any(|c| c.is_candidate) {
        chunks.iter().filter(|c| c.is_candidate).collect()
    } else {
        chunks.iter().collect()
    };

    let mut prompt = String::with_capacity(2048);

    prompt.push_str(
        r#"You are a technical specification classifier for The Knowledge Substrate (TKS).
Classify each candidate chunk below into a compact JSON array of classification tuples.

CRITICAL INSTRUCTIONS:
1. Output ONLY a valid JSON array matching the schema:
   [{"alias": "c1", "modality": "SHALL|SHOULD|MAY|UNCLASSIFIED", "confidence": 0.95}]
2. Do NOT echo, quote, or repeat any chunk text in your output.
3. Output NO explanation, conversational text, or surrounding markdown fences.

Allowed modality values:
- "SHALL": Normative rules, constraints, architectural invariants, or RFC 2119 statements (MUST, SHALL, REQUIRED).
- "SHOULD": Strong recommendation, best practice, or architectural guideline (RFC 2119 SHOULD, RECOMMENDED).
- "MAY": Optional specification, permissive guideline, alternative design choice (RFC 2119 MAY, OPTIONAL).
- "UNCLASSIFIED": Non-normative prose, narrative description, context, or unclassified candidate.

Allowed confidence values:
- Float from 0.0 to 1.0 reflecting classification certainty.

Candidate Chunks:
"#,
    );

    for (idx, chunk) in candidate_chunks.iter().enumerate() {
        let alias = chunk
            .atom_alias
            .clone()
            .unwrap_or_else(|| format!("c{}", idx + 1));
        prompt.push_str(&format!("\n[{alias}]"));

        if let Some(ref heading) = chunk.heading {
            prompt.push_str(&format!(" Section: \"{heading}\""));
        }

        if !chunk.canonical_keys.is_empty() {
            prompt.push_str(&format!(" Keys: {:?}", chunk.canonical_keys));
        }

        if !chunk.rfc2119_keywords.is_empty() {
            prompt.push_str(&format!(" Keywords: {:?}", chunk.rfc2119_keywords));
        }

        prompt.push('\n');

        if let Some(ref text) = chunk.content {
            prompt.push_str(text.trim());
        } else {
            prompt.push_str(&format!(
                "[Offset span: {}..{}]",
                chunk.byte_start, chunk.byte_end
            ));
        }
        prompt.push_str("\n\n");
    }

    prompt
}

fn normalize_modality(raw: &str) -> Result<String, ClassificationError> {
    let upper = raw.trim().to_uppercase();
    match upper.as_str() {
        "SHALL" | "MUST" | "REQUIRED" => Ok("SHALL".to_string()),
        "SHOULD" | "RECOMMENDED" => Ok("SHOULD".to_string()),
        "MAY" | "OPTIONAL" => Ok("MAY".to_string()),
        "UNCLASSIFIED" => Ok("UNCLASSIFIED".to_string()),
        other => Err(ClassificationError::InvalidFormat(format!(
            "invalid modality '{other}'"
        ))),
    }
}

/// Parses compact classification JSON tuples from an LLM response string.
///
/// Handles both compact modality tuples (`alias`, `modality`, `confidence`) and legacy
/// ontology tuples (`chunk_id`, `node_type`, `governance_policy`).
///
/// # Errors
///
/// Returns [`ClassificationError`] if the response is empty, contains invalid JSON,
/// does not represent an array, or contains unrecognized enum values.
pub fn parse_classification_tuples(
    response_json: &str,
) -> Result<Vec<ClassificationResult>, ClassificationError> {
    let trimmed = response_json.trim();
    if trimmed.is_empty() {
        return Err(ClassificationError::EmptyResponse);
    }

    // Strip markdown code fences if present (e.g. ```json ... ```)
    let json_str = extract_json_array(trimmed);

    let raw_tuples: Vec<serde_json::Value> =
        serde_json::from_str(json_str).map_err(|e| ClassificationError::Json(e.to_string()))?;

    let mut results = Vec::with_capacity(raw_tuples.len());

    for (i, val) in raw_tuples.into_iter().enumerate() {
        let obj = val.as_object().ok_or_else(|| {
            ClassificationError::InvalidFormat(format!("item {i} is not a JSON object"))
        })?;

        let chunk_id = obj
            .get("alias")
            .or_else(|| obj.get("chunk_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ClassificationError::InvalidFormat(format!(
                    "item {i} missing 'alias' or 'chunk_id'"
                ))
            })?
            .to_string();

        let raw_modality_opt = obj.get("modality").and_then(|v| v.as_str());
        let raw_node_type_opt = obj.get("node_type").and_then(|v| v.as_str());
        let raw_gov_opt = obj.get("governance_policy").and_then(|v| v.as_str());

        let (node_type, modality) = match (raw_node_type_opt, raw_modality_opt) {
            (Some(nt), Some(m)) => {
                let nt_upper = nt.trim().to_uppercase();
                if !VALID_NODE_TYPES.contains(&nt_upper.as_str()) {
                    return Err(ClassificationError::InvalidNodeType(nt_upper));
                }
                let m_upper = normalize_modality(m)?;
                (nt_upper, m_upper)
            }
            (Some(nt), None) => {
                let nt_upper = nt.trim().to_uppercase();
                if !VALID_NODE_TYPES.contains(&nt_upper.as_str()) {
                    return Err(ClassificationError::InvalidNodeType(nt_upper));
                }
                let m = match nt_upper.as_str() {
                    "REQUIREMENT" => "SHALL".to_string(),
                    "SPECIFICATION" => "MAY".to_string(),
                    "TASK" => "SHALL".to_string(),
                    _ => "UNCLASSIFIED".to_string(),
                };
                (nt_upper, m)
            }
            (None, Some(m)) => {
                let m_upper = normalize_modality(m)?;
                let nt = match m_upper.as_str() {
                    "SHALL" | "MUST" => "REQUIREMENT".to_string(),
                    "SHOULD" => "REQUIREMENT".to_string(),
                    "MAY" => "SPECIFICATION".to_string(),
                    _ => "UNCLASSIFIED".to_string(),
                };
                (nt, m_upper)
            }
            (None, None) => {
                return Err(ClassificationError::InvalidFormat(format!(
                    "item {i} missing both 'modality' and 'node_type'"
                )));
            }
        };

        let governance_policy = if let Some(gp) = raw_gov_opt {
            let gp_upper = gp.trim().to_uppercase();
            if !VALID_GOVERNANCE_POLICIES.contains(&gp_upper.as_str()) {
                return Err(ClassificationError::InvalidGovernancePolicy(gp_upper));
            }
            gp_upper
        } else {
            match node_type.as_str() {
                "TASK" | "SPECIFICATION" => "AUTONOMOUS_ELABORATION".to_string(),
                _ => "HUMAN_REVIEW_REQUIRED".to_string(),
            }
        };

        let confidence = obj
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);

        let model_provenance = obj
            .get("model_provenance")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        results.push(ClassificationResult {
            chunk_id,
            node_type,
            governance_policy,
            modality,
            confidence,
            model_provenance,
        });
    }

    Ok(results)
}

/// Extracts the JSON array string from potentially fenced or noisy LLM output.
fn extract_json_array(text: &str) -> &str {
    let mut s = text.trim();

    // Check for markdown code fences
    if s.starts_with("```") {
        if let Some(newline_idx) = s.find('\n') {
            s = &s[newline_idx + 1..];
        }
        if let Some(end_fence) = s.rfind("```") {
            s = s[..end_fence].trim();
        }
    }

    // If there is still outer text, locate the outermost [ and ]
    if let (Some(start), Some(end)) = (s.find('['), s.rfind(']'))
        && start <= end
    {
        return &s[start..=end];
    }

    s
}

/// Maps parsed classification results back to their originating extracted AST chunks.
///
/// Supports matching by ordinal alias (`c1`, `c2`, ...) and by exact `ast_anchor`.
#[must_use]
pub fn map_classifications_to_chunks<'a>(
    chunks: &'a [ExtractedChunk],
    classifications: &[ClassificationResult],
) -> Vec<(&'a ExtractedChunk, ClassificationResult)> {
    let candidate_chunks: Vec<&ExtractedChunk> = if chunks.iter().any(|c| c.is_candidate) {
        chunks.iter().filter(|c| c.is_candidate).collect()
    } else {
        chunks.iter().collect()
    };

    let mut mapped = Vec::with_capacity(classifications.len());

    for res in classifications {
        // Try matching by atom_alias first
        let matched_chunk = candidate_chunks
            .iter()
            .copied()
            .find(|c| c.atom_alias.as_deref() == Some(&res.chunk_id))
            .or_else(|| {
                // Try matching by alias "c<N>" or "<N>"
                if let Some(stripped) = res.chunk_id.strip_prefix('c') {
                    stripped.parse::<usize>().ok().and_then(|idx| {
                        if idx > 0 && idx <= candidate_chunks.len() {
                            Some(candidate_chunks[idx - 1])
                        } else {
                            None
                        }
                    })
                } else if let Ok(idx) = res.chunk_id.parse::<usize>() {
                    if idx > 0 && idx <= candidate_chunks.len() {
                        Some(candidate_chunks[idx - 1])
                    } else {
                        None
                    }
                } else {
                    // Try matching by exact ast_anchor
                    candidate_chunks
                        .iter()
                        .copied()
                        .find(|c| c.ast_anchor == res.chunk_id)
                }
            });

        if let Some(chunk) = matched_chunk {
            mapped.push((chunk, res.clone()));
        }
    }

    mapped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_classification_prompt() {
        let chunk = ExtractedChunk {
            byte_start: 0,
            byte_end: 45,
            heading: Some("Architecture Invariants".to_string()),
            ast_anchor: "doc.md#invariants#block-0".to_string(),
            parent_heading_chunk_id: None,
            rfc2119_keywords: vec!["MUST".to_string()],
            canonical_keys: vec!["INV-1".to_string()],
            is_candidate: true,
            content: Some("The substrate MUST enforce 0-based byte offsets.".to_string()),
            table_data: None,
            node_type: None,
            atom_alias: None,
            trust_tier: "HUMAN_APPROVED".to_string(),
            modality: None,
            confidence: None,
        };

        let prompt = build_classification_prompt(&[chunk]);
        assert!(prompt.contains("[c1]"));
        assert!(prompt.contains("Section: \"Architecture Invariants\""));
        assert!(prompt.contains("INV-1"));
        assert!(prompt.contains("MUST"));
        assert!(prompt.contains("The substrate MUST enforce 0-based byte offsets."));
        assert!(prompt.contains("CRITICAL INSTRUCTIONS:"));
    }

    #[test]
    fn test_parse_classification_tuples_clean_json() {
        let json = r#"[
            {"chunk_id": "c1", "node_type": "REQUIREMENT", "governance_policy": "HUMAN_REVIEW_REQUIRED"},
            {"chunk_id": "c2", "node_type": "TASK", "governance_policy": "AUTONOMOUS_ELABORATION"}
        ]"#;

        let parsed = parse_classification_tuples(json).expect("valid parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].chunk_id, "c1");
        assert_eq!(parsed[0].node_type, "REQUIREMENT");
        assert_eq!(parsed[0].governance_policy, "HUMAN_REVIEW_REQUIRED");
        assert_eq!(parsed[1].chunk_id, "c2");
        assert_eq!(parsed[1].node_type, "TASK");
        assert_eq!(parsed[1].governance_policy, "AUTONOMOUS_ELABORATION");
    }

    #[test]
    fn test_parse_classification_tuples_fenced_markdown() {
        let json = "```json\n[\n  {\"chunk_id\": \"c1\", \"node_type\": \"SPECIFICATION\", \"governance_policy\": \"HUMAN_REVIEW_REQUIRED\"}\n]\n```";

        let parsed = parse_classification_tuples(json).expect("valid parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].chunk_id, "c1");
        assert_eq!(parsed[0].node_type, "SPECIFICATION");
    }

    #[test]
    fn test_parse_classification_tuples_with_surrounding_prose() {
        let json = "Here is the classification output:\n\n[\n  {\"chunk_id\": \"c1\", \"node_type\": \"REQUIREMENT\", \"governance_policy\": \"HUMAN_REVIEW_REQUIRED\"}\n]\n\nHope this helps!";

        let parsed = parse_classification_tuples(json).expect("valid parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].chunk_id, "c1");
        assert_eq!(parsed[0].node_type, "REQUIREMENT");
    }

    #[test]
    fn test_parse_classification_tuples_invalid_node_type() {
        let json = r#"[{"chunk_id": "c1", "node_type": "UNKNOWN_FOO", "governance_policy": "HUMAN_REVIEW_REQUIRED"}]"#;
        let err = parse_classification_tuples(json).expect_err("should fail");
        assert_eq!(
            err,
            ClassificationError::InvalidNodeType("UNKNOWN_FOO".to_string())
        );
    }

    #[test]
    fn test_parse_classification_tuples_empty() {
        let err = parse_classification_tuples("   ").expect_err("should fail");
        assert_eq!(err, ClassificationError::EmptyResponse);
    }

    #[test]
    fn test_map_classifications_to_chunks() {
        let chunk1 = ExtractedChunk {
            byte_start: 0,
            byte_end: 10,
            heading: None,
            ast_anchor: "doc.md#block-0".to_string(),
            parent_heading_chunk_id: None,
            rfc2119_keywords: vec![],
            canonical_keys: vec![],
            is_candidate: true,
            content: Some("chunk 1".to_string()),
            table_data: None,
            node_type: None,
            atom_alias: None,
            trust_tier: "HUMAN_APPROVED".to_string(),
            modality: None,
            confidence: None,
        };
        let chunk2 = ExtractedChunk {
            byte_start: 11,
            byte_end: 20,
            heading: None,
            ast_anchor: "doc.md#block-1".to_string(),
            parent_heading_chunk_id: None,
            rfc2119_keywords: vec![],
            canonical_keys: vec![],
            is_candidate: true,
            content: Some("chunk 2".to_string()),
            table_data: None,
            node_type: None,
            atom_alias: None,
            trust_tier: "HUMAN_APPROVED".to_string(),
            modality: None,
            confidence: None,
        };

        let chunks = vec![chunk1, chunk2];
        let classifications = vec![
            ClassificationResult {
                chunk_id: "c1".to_string(),
                node_type: "REQUIREMENT".to_string(),
                governance_policy: "HUMAN_REVIEW_REQUIRED".to_string(),
                modality: "REQUIREMENT".to_string(),
                confidence: 1.0,
                model_provenance: Some("test".to_string()),
            },
            ClassificationResult {
                chunk_id: "c2".to_string(),
                node_type: "TASK".to_string(),
                governance_policy: "AUTONOMOUS_ELABORATION".to_string(),
                modality: "TASK".to_string(),
                confidence: 1.0,
                model_provenance: Some("test".to_string()),
            },
        ];

        let mapped = map_classifications_to_chunks(&chunks, &classifications);
        assert_eq!(mapped.len(), 2);
        assert_eq!(mapped[0].0.ast_anchor, "doc.md#block-0");
        assert_eq!(mapped[0].1.node_type, "REQUIREMENT");
        assert_eq!(mapped[1].0.ast_anchor, "doc.md#block-1");
        assert_eq!(mapped[1].1.node_type, "TASK");
    }
}
