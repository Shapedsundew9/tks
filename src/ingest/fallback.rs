//! Graceful degradation fallback assigning default requirement types (WP-0.3, D-37, D-62).

use crate::ingest::classify::ClassificationResult;
use crate::ingest::parser::ExtractedChunk;

/// Fallback classification assigning default types when LLM API credentials
/// are unconfigured or when external API requests fail/timeout.
///
/// Implements Decisions D-37 and D-62:
/// - Chunks matching RFC 2119 keywords default to `node_type = "REQUIREMENT"`,
///   `governance_policy = "HUMAN_REVIEW_REQUIRED"`.
/// - Other candidate chunks default to `node_type = "UNCLASSIFIED"`,
///   `governance_policy = "HUMAN_REVIEW_REQUIRED"`.
///
/// In `DRAFT` lifecycle state, `'UNCLASSIFIED'` is explicitly permitted by the
/// conditional check constraint `chk_node_type` on `graph_nodes` without database error.
#[must_use]
pub fn fallback_classify(chunks: &[ExtractedChunk]) -> Vec<ClassificationResult> {
    let candidate_chunks: Vec<&ExtractedChunk> = if chunks.iter().any(|c| c.is_candidate) {
        chunks.iter().filter(|c| c.is_candidate).collect()
    } else {
        chunks.iter().collect()
    };

    candidate_chunks
        .iter()
        .enumerate()
        .map(|(idx, chunk)| {
            let (node_type, governance_policy) = classify_chunk_fallback(chunk);
            ClassificationResult {
                chunk_id: format!("c{}", idx + 1),
                node_type,
                governance_policy,
            }
        })
        .collect()
}

/// Evaluates default node type and governance policy for an individual chunk.
#[must_use]
pub fn classify_chunk_fallback(chunk: &ExtractedChunk) -> (String, String) {
    // 1. Check primary canonical key for explicit Decision prefixes
    if let Some(key) = chunk.primary_node_key()
        && (key.starts_with("DEC-") || key.starts_with("D-"))
    {
        return ("DECISION".to_string(), "HUMAN_REVIEW_REQUIRED".to_string());
    }

    // 2. Check if any extracted canonical key indicates DECISION
    for key in &chunk.canonical_keys {
        if key.starts_with("DEC-") || key.starts_with("D-") {
            return ("DECISION".to_string(), "HUMAN_REVIEW_REQUIRED".to_string());
        }
    }

    // 3. Check heading text for Decision markers
    if let Some(ref heading) = chunk.heading {
        let trimmed_h = heading.trim();
        if trimmed_h.starts_with("Decision ")
            || trimmed_h.starts_with("DEC-")
            || trimmed_h.starts_with("D-")
            || trimmed_h.contains("Decision:")
        {
            return ("DECISION".to_string(), "HUMAN_REVIEW_REQUIRED".to_string());
        }
    }

    // 4. Check for RFC 2119 keywords
    if !chunk.rfc2119_keywords.is_empty() {
        (
            "REQUIREMENT".to_string(),
            "HUMAN_REVIEW_REQUIRED".to_string(),
        )
    } else {
        (
            "UNCLASSIFIED".to_string(),
            "HUMAN_REVIEW_REQUIRED".to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_classify_rfc2119_keywords() {
        let chunk = ExtractedChunk {
            byte_start: 0,
            byte_end: 25,
            heading: Some("Security Policy".to_string()),
            ast_anchor: "sec.md#auth#block-0".to_string(),
            parent_heading_chunk_id: None,
            rfc2119_keywords: vec!["MUST".to_string()],
            canonical_keys: vec![],
            is_candidate: true,
            content: Some("Requests MUST be signed.".to_string()),
            table_data: None,
        };

        let results = fallback_classify(&[chunk]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, "c1");
        assert_eq!(results[0].node_type, "REQUIREMENT");
        assert_eq!(results[0].governance_policy, "HUMAN_REVIEW_REQUIRED");
    }

    #[test]
    fn test_fallback_classify_unclassified() {
        let chunk = ExtractedChunk {
            byte_start: 0,
            byte_end: 30,
            heading: None,
            ast_anchor: "sec.md#block-1".to_string(),
            parent_heading_chunk_id: None,
            rfc2119_keywords: vec![],
            canonical_keys: vec!["TB-5".to_string()],
            is_candidate: true,
            content: Some("Development identity seed.".to_string()),
            table_data: None,
        };

        let results = fallback_classify(&[chunk]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, "c1");
        assert_eq!(results[0].node_type, "UNCLASSIFIED");
        assert_eq!(results[0].governance_policy, "HUMAN_REVIEW_REQUIRED");
    }

    #[test]
    fn test_fallback_classify_decision_and_constraints() {
        let dec_chunk = ExtractedChunk {
            byte_start: 0,
            byte_end: 50,
            heading: Some("DEC-1.1: Direct Writes".to_string()),
            ast_anchor: "dec.md#dec-1-1".to_string(),
            parent_heading_chunk_id: None,
            rfc2119_keywords: vec![],
            canonical_keys: vec!["DEC-1.1".to_string()],
            is_candidate: true,
            content: Some("Decision on direct writes.".to_string()),
            table_data: None,
        };

        let d_chunk = ExtractedChunk {
            byte_start: 51,
            byte_end: 100,
            heading: Some("D-1: Single PostgreSQL Engine".to_string()),
            ast_anchor: "arch.md#d-1".to_string(),
            parent_heading_chunk_id: None,
            rfc2119_keywords: vec![],
            canonical_keys: vec!["D-1".to_string()],
            is_candidate: true,
            content: Some("Decision on single engine.".to_string()),
            table_data: None,
        };

        let c_chunk = ExtractedChunk {
            byte_start: 101,
            byte_end: 150,
            heading: Some("Derived Constraints".to_string()),
            ast_anchor: "arch.md#constraints#c-1".to_string(),
            parent_heading_chunk_id: None,
            rfc2119_keywords: vec!["MUST".to_string()],
            canonical_keys: vec!["C-1".to_string()],
            is_candidate: true,
            content: Some(
                "Pure Rust package repository: all substrate code MUST be authored in Rust."
                    .to_string(),
            ),
            table_data: None,
        };

        let results = fallback_classify(&[dec_chunk, d_chunk, c_chunk]);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].node_type, "DECISION");
        assert_eq!(results[0].governance_policy, "HUMAN_REVIEW_REQUIRED");
        assert_eq!(results[1].node_type, "DECISION");
        assert_eq!(results[1].governance_policy, "HUMAN_REVIEW_REQUIRED");
        assert_eq!(results[2].node_type, "REQUIREMENT");
        assert_eq!(results[2].governance_policy, "HUMAN_REVIEW_REQUIRED");
    }
}
