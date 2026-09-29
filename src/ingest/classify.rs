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

/// Structured semantic classification tuple for an extracted AST chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassificationResult {
    /// Chunk identifier (e.g. ordinal alias "c1", "c2" or AST anchor).
    pub chunk_id: String,
    /// Entity node type (e.g. "REQUIREMENT", "SPECIFICATION", "TASK", "UNCLASSIFIED").
    pub node_type: String,
    /// Governance policy (e.g. "HUMAN_REVIEW_REQUIRED", "AUTONOMOUS_ELABORATION", "LOCKED").
    pub governance_policy: String,
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
/// (`chunk_id`, `node_type`, `governance_policy`) without echoing or repeating
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
   [{"chunk_id": "c1", "node_type": "REQUIREMENT|SPECIFICATION|TASK", "governance_policy": "HUMAN_REVIEW_REQUIRED|AUTONOMOUS_ELABORATION"}]
2. Do NOT echo, quote, or repeat any chunk text in your output.
3. Output NO explanation, conversational text, or surrounding markdown fences.

Allowed node_type values:
- "REQUIREMENT": Normative rules, constraints, architectural invariants, or RFC 2119 statements (MUST, SHALL, REQUIRED).
- "SPECIFICATION": Technical architecture, data structures, interface designs, schemas, component definitions.
- "TASK": Concrete implementation action items, work packages, or execution steps.

Allowed governance_policy values:
- "HUMAN_REVIEW_REQUIRED": Core requirements, architectural decisions, and safety-critical constraints.
- "AUTONOMOUS_ELABORATION": Execution subtasks and non-normative implementation actions.

Candidate Chunks:
"#,
    );

    for (idx, chunk) in candidate_chunks.iter().enumerate() {
        let alias = format!("c{}", idx + 1);
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

/// Parses compact classification JSON tuples from an LLM response string.
///
/// Handles raw JSON arrays, markdown code-fenced JSON, and extracts JSON arrays
/// embedded within surrounding prose. Validates that `node_type` and `governance_policy`
/// strictly conform to substrate ontology rules.
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
            .get("chunk_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ClassificationError::InvalidFormat(format!("item {i} missing 'chunk_id'"))
            })?
            .to_string();

        let raw_node_type = obj
            .get("node_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ClassificationError::InvalidFormat(format!("item {i} missing 'node_type'"))
            })?;

        let node_type = raw_node_type.trim().to_uppercase();
        if !VALID_NODE_TYPES.contains(&node_type.as_str()) {
            return Err(ClassificationError::InvalidNodeType(node_type));
        }

        let raw_gov_policy = obj
            .get("governance_policy")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ClassificationError::InvalidFormat(format!("item {i} missing 'governance_policy'"))
            })?;

        let governance_policy = raw_gov_policy.trim().to_uppercase();
        if !VALID_GOVERNANCE_POLICIES.contains(&governance_policy.as_str()) {
            return Err(ClassificationError::InvalidGovernancePolicy(
                governance_policy,
            ));
        }

        results.push(ClassificationResult {
            chunk_id,
            node_type,
            governance_policy,
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
        // Try matching by alias "c<N>" or "<N>"
        let matched_chunk = if let Some(stripped) = res.chunk_id.strip_prefix('c') {
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
        };

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
        };

        let chunks = vec![chunk1, chunk2];
        let classifications = vec![
            ClassificationResult {
                chunk_id: "c1".to_string(),
                node_type: "REQUIREMENT".to_string(),
                governance_policy: "HUMAN_REVIEW_REQUIRED".to_string(),
            },
            ClassificationResult {
                chunk_id: "c2".to_string(),
                node_type: "TASK".to_string(),
                governance_policy: "AUTONOMOUS_ELABORATION".to_string(),
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
