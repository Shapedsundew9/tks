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
        };

        let results = fallback_classify(&[chunk]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, "c1");
        assert_eq!(results[0].node_type, "UNCLASSIFIED");
        assert_eq!(results[0].governance_policy, "HUMAN_REVIEW_REQUIRED");
    }
}
