//! 3-tier CommonMark AST document reconciliation and staged candidate span re-anchoring (WP-1.3, TB-7).
//!
//! Implements strict three-tier reconciliation hierarchy:
//! 1. Tier 1 (Canonical Key Match): Match by explicit `node_key` (`REQ-*`, `INV-*`, `DR-*`, `C-*`, `TB-*`, `TASK-*`).
//! 2. Tier 2 (Structural Heading Anchor Match): Match by deterministic, disambiguated hierarchical heading anchor (`ast_anchor`).
//! 3. Tier 3 (Content Hash Match): Match by normalized SHA-256 hash of title and text content.
//!
//! Active nodes are NEVER mutated in place during reconciliation (INV-2, LD-1, D-69, D-74).
//! Candidate re-anchoring coordinates (`candidate_reanchoring`) are staged as metadata, to be applied
//! strictly upon explicit supervisory staging approval.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use uuid::Uuid;

use crate::ingest::fallback::classify_chunk_fallback;
use crate::ingest::parser::ExtractedChunk;
use crate::storage::GraphNode;

/// The priority tier that produced a match during 3-tier document reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconciliationTier {
    /// Matched by explicit canonical identifier (`node_key`, e.g. `REQ-CORE-001`).
    Tier1CanonicalKey,
    /// Matched by deterministic structural heading anchor (`ast_anchor`).
    Tier2AstAnchor,
    /// Matched by normalized content SHA-256 hash.
    Tier3ContentHash,
}

/// Staged coordinate update for an existing active node whose byte offsets shifted in the revised document.
///
/// Implements D-69, D-74, and TB-7:
/// Active nodes are NEVER updated in place during reconciliation. Instead, candidate re-anchored
/// coordinates are staged in this struct (and formatted into candidate metadata) to be applied
/// strictly upon explicit human staging approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateSpanReanchor {
    /// Primary key of the active node being re-anchored.
    pub target_node_id: Uuid,
    /// Primary key alias for convenient access.
    pub node_id: Uuid,
    /// Canonical key of the target node, if any.
    pub node_key: Option<String>,
    /// AST anchor of the matched chunk in the new revision.
    pub ast_anchor: String,
    /// Document path of the source specification.
    pub doc_path: String,
    /// New document hash (Git blob SHA or content SHA-256).
    pub doc_hash: Option<String>,
    /// Previous 0-based start byte offset on the active node.
    pub old_byte_start: Option<i32>,
    /// Previous 0-based end byte offset on the active node.
    pub old_byte_end: Option<i32>,
    /// New 0-based start byte offset in the revised document.
    pub new_byte_start: i32,
    /// New 0-based end byte offset in the revised document.
    pub new_byte_end: i32,
    /// Priority tier that produced the match.
    pub match_tier: ReconciliationTier,
}

impl CandidateSpanReanchor {
    /// Returns true if the byte offsets differ from the active node's previous coordinates.
    #[must_use]
    pub fn is_span_shifted(&self) -> bool {
        self.old_byte_start != Some(self.new_byte_start)
            || self.old_byte_end != Some(self.new_byte_end)
    }

    /// Formats the candidate span re-anchoring metadata for storage in `attributes->'candidate_reanchoring'`.
    #[must_use]
    pub fn to_metadata_json(&self) -> serde_json::Value {
        serde_json::json!({
            "target_node_id": self.target_node_id,
            "node_key": self.node_key,
            "ast_anchor": self.ast_anchor,
            "doc_path": self.doc_path,
            "doc_hash": self.doc_hash,
            "old_byte_start": self.old_byte_start,
            "old_byte_end": self.old_byte_end,
            "new_byte_start": self.new_byte_start,
            "new_byte_end": self.new_byte_end,
            "match_tier": format!("{:?}", self.match_tier),
        })
    }
}

/// Candidate draft node to be created for newly added or modified content.
///
/// Implements D-16, D-62, and TB-7:
/// Newly added sections or sections with modified content generate candidate `DRAFT` nodes.
/// For modified sections, `replaces_node_id` records the ID of the active node being superseded upon approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewDraftNode {
    /// Extracted chunk from the revised document.
    pub chunk: ExtractedChunk,
    /// Canonical identifier, if extracted.
    pub node_key: Option<String>,
    /// Title or heading of the section.
    pub title: Option<String>,
    /// Sliced text content.
    pub content: String,
    /// Substrate node type (`REQUIREMENT`, `SPECIFICATION`, `TASK`, `UNCLASSIFIED`, etc.).
    pub node_type: String,
    /// Substrate governance policy (`HUMAN_REVIEW_REQUIRED`, `AUTONOMOUS_ELABORATION`, etc.).
    pub governance_policy: String,
    /// Document path of the source specification.
    pub doc_path: String,
    /// Git blob SHA or content SHA-256.
    pub doc_hash: Option<String>,
    /// 0-based start byte offset.
    pub byte_start: i32,
    /// 0-based end byte offset.
    pub byte_end: i32,
    /// Deterministic AST anchor.
    pub ast_anchor: String,
    /// Active node ID that this draft node replaces/supersedes upon approval, if any.
    pub replaces_node_id: Option<Uuid>,
    /// Candidate attributes including `ast_anchor` and `replaces_node_id`.
    pub attributes: serde_json::Value,
}

impl NewDraftNode {
    /// Returns true if this draft replaces an existing active node.
    #[must_use]
    pub fn is_replacement(&self) -> bool {
        self.replaces_node_id.is_some()
    }
}

/// Complete execution plan output by document reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationPlan {
    /// Existing active nodes whose spans shifted without content changes.
    pub reanchored_spans: Vec<CandidateSpanReanchor>,
    /// Candidate draft nodes for newly added or modified sections.
    pub new_draft_nodes: Vec<NewDraftNode>,
    /// Active node IDs that were deleted (missing from the revised document) or replaced.
    pub superseded_node_ids: Vec<Uuid>,
}

impl ReconciliationPlan {
    /// Returns the active node IDs that were deleted (not matched by any chunk in the revised document).
    #[must_use]
    pub fn deleted_node_ids(&self) -> Vec<Uuid> {
        let replaced_ids: HashSet<Uuid> = self
            .new_draft_nodes
            .iter()
            .filter_map(|d| d.replaces_node_id)
            .collect();
        self.superseded_node_ids
            .iter()
            .copied()
            .filter(|id| !replaced_ids.contains(id))
            .collect()
    }

    /// Returns the active node IDs that are replaced by candidate drafts.
    #[must_use]
    pub fn replaced_node_ids(&self) -> Vec<Uuid> {
        self.new_draft_nodes
            .iter()
            .filter_map(|d| d.replaces_node_id)
            .collect()
    }
}

/// Computes a normalized SHA-256 hash for document content and optional title.
///
/// Normalizes CRLF to LF and trims leading/trailing whitespace before hashing.
#[must_use]
pub fn compute_normalized_content_hash(title: Option<&str>, content: &str) -> String {
    let mut hasher = Sha256::new();
    if let Some(t) = title {
        let t_norm = t.trim();
        if !t_norm.is_empty() {
            hasher.update(t_norm.as_bytes());
            hasher.update(b"\n");
        }
    }
    let normalized = content.replace("\r\n", "\n");
    let trimmed = normalized.trim();
    hasher.update(trimmed.as_bytes());
    let result = hasher.finalize();
    format!("{result:x}")
}

/// Retrieves the `ast_anchor` stored in a `GraphNode`'s attributes JSONB, if present.
#[must_use]
pub fn get_node_ast_anchor(node: &GraphNode) -> Option<&str> {
    node.attributes.get("ast_anchor").and_then(|v| v.as_str())
}

/// Stages candidate re-anchoring coordinates into a candidate metadata JSONB array.
#[must_use]
pub fn stage_reanchoring_metadata(reanchored_spans: &[CandidateSpanReanchor]) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = reanchored_spans
        .iter()
        .map(|r| r.to_metadata_json())
        .collect();
    serde_json::Value::Array(entries)
}

/// Reconciles newly extracted blocks from a revised document against existing active nodes.
///
/// Follows the strict 3-tier priority matching hierarchy:
/// - Tier 1: Canonical Key Match (`node_key`)
/// - Tier 2: Structural Heading Anchor Match (`ast_anchor`)
/// - Tier 3: Normalized Content Hash Match (SHA-256)
///
/// Unchanged matching nodes generate [`CandidateSpanReanchor`] entries without mutating
/// active nodes in place. Modified sections generate candidate [`NewDraftNode`] entries
/// referencing `replaces_node_id`. Deleted sections append active node IDs to `superseded_node_ids`.
#[must_use]
pub fn reconcile_reingestion(
    new_chunks: &[ExtractedChunk],
    existing_active_nodes: &[GraphNode],
) -> ReconciliationPlan {
    reconcile_reingestion_with_options(new_chunks, existing_active_nodes, None, None)
}

/// Reconciles newly extracted blocks with optional document path and document hash parameters.
#[must_use]
pub fn reconcile_reingestion_with_options(
    new_chunks: &[ExtractedChunk],
    existing_active_nodes: &[GraphNode],
    doc_path_override: Option<&str>,
    doc_hash: Option<&str>,
) -> ReconciliationPlan {
    let mut matched_chunk_indices = HashSet::new();
    let mut matched_node_ids = HashSet::new();
    let mut tier_matches: Vec<(usize, usize, ReconciliationTier)> = Vec::new();

    // Precompute full (title + content) hashes for active nodes
    let node_hashes: Vec<String> = existing_active_nodes
        .iter()
        .map(|n| {
            compute_normalized_content_hash(n.title.as_deref(), n.content.as_deref().unwrap_or(""))
        })
        .collect();

    // Precompute content-only hashes for active nodes
    let node_content_only_hashes: Vec<String> = existing_active_nodes
        .iter()
        .map(|n| compute_normalized_content_hash(None, n.content.as_deref().unwrap_or("")))
        .collect();

    // Precompute full (heading + content) hashes for chunks
    let chunk_hashes: Vec<String> = new_chunks
        .iter()
        .map(|c| {
            compute_normalized_content_hash(
                c.heading.as_deref(),
                c.content.as_deref().unwrap_or(""),
            )
        })
        .collect();

    // Precompute content-only hashes for chunks
    let chunk_content_only_hashes: Vec<String> = new_chunks
        .iter()
        .map(|c| compute_normalized_content_hash(None, c.content.as_deref().unwrap_or("")))
        .collect();

    let content_matches = |c_idx: usize, n_idx: usize| -> bool {
        chunk_hashes[c_idx] == node_hashes[n_idx]
            || chunk_content_only_hashes[c_idx] == node_content_only_hashes[n_idx]
    };

    // Tier 1: Canonical Key Match (node_key)
    struct KeyCandidate {
        chunk_idx: usize,
        node_idx: usize,
        score: u8,
    }
    let mut key_candidates = Vec::new();

    for (c_idx, chunk) in new_chunks.iter().enumerate() {
        if chunk.canonical_keys.is_empty() {
            continue;
        }
        for (n_idx, node) in existing_active_nodes.iter().enumerate() {
            if let Some(ref node_key) = node.node_key
                && chunk.canonical_keys.contains(node_key)
            {
                let is_primary = chunk.primary_node_key() == Some(node_key.as_str());
                let same_content = content_matches(c_idx, n_idx);
                let same_anchor = get_node_ast_anchor(node) == Some(&chunk.ast_anchor);

                let score = if is_primary && same_content {
                    4
                } else if same_content {
                    3
                } else if is_primary && same_anchor {
                    2
                } else if is_primary {
                    1
                } else {
                    0
                };
                key_candidates.push(KeyCandidate {
                    chunk_idx: c_idx,
                    node_idx: n_idx,
                    score,
                });
            }
        }
    }

    key_candidates.sort_by_key(|a| std::cmp::Reverse(a.score));
    for cand in key_candidates {
        if !matched_chunk_indices.contains(&cand.chunk_idx)
            && !matched_node_ids.contains(&existing_active_nodes[cand.node_idx].id)
        {
            matched_chunk_indices.insert(cand.chunk_idx);
            matched_node_ids.insert(existing_active_nodes[cand.node_idx].id);
            tier_matches.push((
                cand.chunk_idx,
                cand.node_idx,
                ReconciliationTier::Tier1CanonicalKey,
            ));
        }
    }

    // Tier 2a: Structural Heading Anchor Match (ast_anchor) with identical content
    for (c_idx, chunk) in new_chunks.iter().enumerate() {
        if matched_chunk_indices.contains(&c_idx) {
            continue;
        }
        for (n_idx, node) in existing_active_nodes.iter().enumerate() {
            if matched_node_ids.contains(&node.id) {
                continue;
            }
            if get_node_ast_anchor(node) == Some(&chunk.ast_anchor) && content_matches(c_idx, n_idx)
            {
                matched_chunk_indices.insert(c_idx);
                matched_node_ids.insert(node.id);
                tier_matches.push((c_idx, n_idx, ReconciliationTier::Tier2AstAnchor));
                break;
            }
        }
    }

    // Tier 3: Content Hash Match (preserves identity when blocks shift ordinal positions)
    // Pass 3a: full (title + content) hash match
    for (c_idx, _chunk) in new_chunks.iter().enumerate() {
        if matched_chunk_indices.contains(&c_idx) {
            continue;
        }
        for (n_idx, node) in existing_active_nodes.iter().enumerate() {
            if matched_node_ids.contains(&node.id) {
                continue;
            }
            if chunk_hashes[c_idx] == node_hashes[n_idx] {
                matched_chunk_indices.insert(c_idx);
                matched_node_ids.insert(node.id);
                tier_matches.push((c_idx, n_idx, ReconciliationTier::Tier3ContentHash));
                break;
            }
        }
    }
    // Pass 3b: content-only hash match
    for (c_idx, _chunk) in new_chunks.iter().enumerate() {
        if matched_chunk_indices.contains(&c_idx) {
            continue;
        }
        for (n_idx, node) in existing_active_nodes.iter().enumerate() {
            if matched_node_ids.contains(&node.id) {
                continue;
            }
            if chunk_content_only_hashes[c_idx] == node_content_only_hashes[n_idx] {
                matched_chunk_indices.insert(c_idx);
                matched_node_ids.insert(node.id);
                tier_matches.push((c_idx, n_idx, ReconciliationTier::Tier3ContentHash));
                break;
            }
        }
    }

    // Tier 2b: Remaining Structural Heading Anchor Match (ast_anchor) for modified content
    for (c_idx, chunk) in new_chunks.iter().enumerate() {
        if matched_chunk_indices.contains(&c_idx) {
            continue;
        }
        for (n_idx, node) in existing_active_nodes.iter().enumerate() {
            if matched_node_ids.contains(&node.id) {
                continue;
            }
            if get_node_ast_anchor(node) == Some(&chunk.ast_anchor) {
                matched_chunk_indices.insert(c_idx);
                matched_node_ids.insert(node.id);
                tier_matches.push((c_idx, n_idx, ReconciliationTier::Tier2AstAnchor));
                break;
            }
        }
    }

    let mut reanchored_spans = Vec::new();
    let mut new_draft_nodes = Vec::new();
    let mut superseded_node_ids = Vec::new();

    for (c_idx, n_idx, tier) in tier_matches {
        let chunk = &new_chunks[c_idx];
        let node = &existing_active_nodes[n_idx];
        let is_content_identical = content_matches(c_idx, n_idx);

        let doc_path = doc_path_override
            .map(|s| s.to_string())
            .or_else(|| node.doc_path.clone())
            .unwrap_or_else(|| chunk.ast_anchor.split('#').next().unwrap_or("").to_string());

        if is_content_identical {
            reanchored_spans.push(CandidateSpanReanchor {
                target_node_id: node.id,
                node_id: node.id,
                node_key: node
                    .node_key
                    .clone()
                    .or_else(|| chunk.primary_node_key().map(|s| s.to_string())),
                ast_anchor: chunk.ast_anchor.clone(),
                doc_path,
                doc_hash: doc_hash
                    .map(|s| s.to_string())
                    .or_else(|| node.doc_hash.clone()),
                old_byte_start: node.byte_start,
                old_byte_end: node.byte_end,
                new_byte_start: chunk.byte_start as i32,
                new_byte_end: chunk.byte_end as i32,
                match_tier: tier,
            });
        } else {
            let (fallback_type, fallback_gov) = classify_chunk_fallback(chunk);
            let mut attrs = serde_json::Map::new();
            attrs.insert(
                "ast_anchor".to_string(),
                serde_json::Value::String(chunk.ast_anchor.clone()),
            );
            attrs.insert(
                "replaces_node_id".to_string(),
                serde_json::Value::String(node.id.to_string()),
            );

            let node_type = if node.node_type != "UNCLASSIFIED" {
                node.node_type.clone()
            } else {
                fallback_type
            };

            let governance_policy = if !node.governance_policy.is_empty() {
                node.governance_policy.clone()
            } else {
                fallback_gov
            };

            let draft = NewDraftNode {
                chunk: chunk.clone(),
                node_key: chunk
                    .primary_node_key()
                    .map(|s| s.to_string())
                    .or_else(|| node.node_key.clone()),
                title: chunk.heading.clone().or_else(|| node.title.clone()),
                content: chunk.content.clone().unwrap_or_default(),
                node_type,
                governance_policy,
                doc_path,
                doc_hash: doc_hash
                    .map(|s| s.to_string())
                    .or_else(|| node.doc_hash.clone()),
                byte_start: chunk.byte_start as i32,
                byte_end: chunk.byte_end as i32,
                ast_anchor: chunk.ast_anchor.clone(),
                replaces_node_id: Some(node.id),
                attributes: serde_json::Value::Object(attrs),
            };
            new_draft_nodes.push(draft);
            superseded_node_ids.push(node.id);
        }
    }

    // Extract table metadata across all chunks in this document to enrich corresponding decision/requirement nodes
    let mut key_to_table_metadata: std::collections::HashMap<
        String,
        serde_json::Map<String, serde_json::Value>,
    > = std::collections::HashMap::new();

    for chunk in new_chunks {
        if let Some(ref td) = chunk.table_data {
            for row in &td.rows {
                for val in row.values() {
                    for key in crate::ingest::matcher::extract_canonical_keys(val) {
                        let mut meta = serde_json::Map::new();
                        for (col_name, col_val) in row {
                            let norm_col = col_name.to_lowercase().replace(' ', "_");
                            meta.insert(norm_col, serde_json::Value::String(col_val.clone()));
                        }
                        key_to_table_metadata.insert(key, meta);
                    }
                }
            }
        }
    }

    // Enrich replaced drafts with table metadata if matching
    for draft in &mut new_draft_nodes {
        if let Some(ref key) = draft.node_key
            && let Some(meta) = key_to_table_metadata.get(key)
            && let serde_json::Value::Object(ref mut map) = draft.attributes
        {
            for (k, v) in meta {
                if !map.contains_key(k) {
                    map.insert(k.clone(), v.clone());
                }
            }
        }
    }

    // Track assigned canonical keys to ensure node_key uniqueness across active nodes and drafts (DEC-1.17, D-58)
    let mut assigned_node_keys: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for span in &reanchored_spans {
        if let Some(ref k) = span.node_key {
            assigned_node_keys.insert(k.clone());
        }
    }
    for draft in &new_draft_nodes {
        if let Some(ref k) = draft.node_key {
            assigned_node_keys.insert(k.clone());
        }
    }

    // Remaining unmatched chunks: newly added sections
    // Prioritize heading chunks before table row chunks and other blocks
    let mut unmatched_indices: Vec<usize> = (0..new_chunks.len())
        .filter(|idx| !matched_chunk_indices.contains(idx))
        .collect();

    unmatched_indices.sort_by_key(|&idx| {
        let c = &new_chunks[idx];
        if c.heading.is_some()
            && !c.ast_anchor.contains("#block-")
            && !c.ast_anchor.contains("#table-row-")
        {
            0
        } else if c.ast_anchor.contains("#table-row-") {
            1
        } else {
            2
        }
    });

    for c_idx in unmatched_indices {
        let chunk = &new_chunks[c_idx];
        let (node_type, governance_policy) = classify_chunk_fallback(chunk);
        let mut attrs = serde_json::Map::new();
        attrs.insert(
            "ast_anchor".to_string(),
            serde_json::Value::String(chunk.ast_anchor.clone()),
        );

        // If chunk has table_data, attach it to attributes
        if let Some(ref td) = chunk.table_data {
            attrs.insert("is_table".to_string(), serde_json::json!(true));
            attrs.insert("table_headers".to_string(), serde_json::json!(td.headers));
            attrs.insert("table_rows".to_string(), serde_json::json!(td.rows));
        }

        let doc_path = doc_path_override
            .map(|s| s.to_string())
            .unwrap_or_else(|| chunk.ast_anchor.split('#').next().unwrap_or("").to_string());

        let node_key = if let Some(key) = chunk.primary_node_key() {
            if !assigned_node_keys.contains(key) {
                assigned_node_keys.insert(key.to_string());
                Some(key.to_string())
            } else {
                None
            }
        } else {
            None
        };

        // If this node has a node_key, enrich attributes with table metadata if available
        if let Some(ref key) = node_key
            && let Some(meta) = key_to_table_metadata.get(key)
        {
            for (k, v) in meta {
                if !attrs.contains_key(k) {
                    attrs.insert(k.clone(), v.clone());
                }
            }
        }

        // Title resolution: for table row chunks, derive an informative title from row columns if available
        let title = if chunk.ast_anchor.contains("#table-row-") {
            if let Some(ref key) = node_key {
                let candidate_title = chunk
                    .table_data
                    .as_ref()
                    .and_then(|td| td.rows.first())
                    .and_then(|row| {
                        row.get("Constraint")
                            .or_else(|| row.get("Statement"))
                            .or_else(|| row.get("Title"))
                            .or_else(|| row.get("Name"))
                    })
                    .map(|text| {
                        let trimmed = text.trim();
                        if trimmed.len() > 80 {
                            format!("{key}: {}...", &trimmed[..77])
                        } else {
                            format!("{key}: {trimmed}")
                        }
                    });
                candidate_title
                    .or_else(|| chunk.heading.as_ref().map(|h| format!("{h}: {key}")))
                    .or(chunk.heading.clone())
            } else {
                chunk.heading.clone()
            }
        } else {
            chunk.heading.clone()
        };

        let draft = NewDraftNode {
            chunk: chunk.clone(),
            node_key,
            title,
            content: chunk.content.clone().unwrap_or_default(),
            node_type,
            governance_policy,
            doc_path,
            doc_hash: doc_hash.map(|s| s.to_string()),
            byte_start: chunk.byte_start as i32,
            byte_end: chunk.byte_end as i32,
            ast_anchor: chunk.ast_anchor.clone(),
            replaces_node_id: None,
            attributes: serde_json::Value::Object(attrs),
        };
        new_draft_nodes.push(draft);
    }

    // Remaining unmatched active nodes: deleted sections missing from revised document
    for node in existing_active_nodes {
        if !matched_node_ids.contains(&node.id) {
            superseded_node_ids.push(node.id);
        }
    }

    ReconciliationPlan {
        reanchored_spans,
        new_draft_nodes,
        superseded_node_ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_node(
        node_key: Option<&str>,
        title: Option<&str>,
        content: Option<&str>,
        ast_anchor: &str,
        byte_start: i32,
        byte_end: i32,
    ) -> GraphNode {
        let mut attrs = serde_json::Map::new();
        attrs.insert(
            "ast_anchor".to_string(),
            serde_json::Value::String(ast_anchor.to_string()),
        );

        GraphNode {
            id: Uuid::new_v4(),
            node_key: node_key.map(|s| s.to_string()),
            node_type: "REQUIREMENT".to_string(),
            title: title.map(|s| s.to_string()),
            content: content.map(|s| s.to_string()),
            lifecycle_state: "ACTIVE".to_string(),
            governance_policy: "HUMAN_REVIEW_REQUIRED".to_string(),
            created_by: "system".to_string(),
            job_id: None,
            doc_path: Some("specs/test.md".to_string()),
            doc_hash: Some("hash1".to_string()),
            byte_start: Some(byte_start),
            byte_end: Some(byte_end),
            attributes: serde_json::Value::Object(attrs),
        }
    }

    fn make_test_chunk(
        ast_anchor: &str,
        canonical_keys: Vec<&str>,
        heading: Option<&str>,
        content: &str,
        byte_start: usize,
        byte_end: usize,
    ) -> ExtractedChunk {
        ExtractedChunk {
            byte_start,
            byte_end,
            heading: heading.map(|s| s.to_string()),
            ast_anchor: ast_anchor.to_string(),
            parent_heading_chunk_id: None,
            rfc2119_keywords: vec![],
            canonical_keys: canonical_keys.into_iter().map(|s| s.to_string()).collect(),
            is_candidate: true,
            content: Some(content.to_string()),
            table_data: None,
        }
    }

    #[test]
    fn test_tier1_canonical_key_match_reanchor() {
        let node = make_test_node(
            Some("REQ-001"),
            Some("Auth Req"),
            Some("Must sign tokens."),
            "specs/test.md#auth#block-0",
            10,
            50,
        );

        // Chunk shifted by +20 bytes, identical content
        let chunk = make_test_chunk(
            "specs/test.md#auth-v2#block-0",
            vec!["REQ-001"],
            Some("Auth Req"),
            "Must sign tokens.",
            30,
            70,
        );

        let plan = reconcile_reingestion(&[chunk], std::slice::from_ref(&node));
        assert_eq!(plan.reanchored_spans.len(), 1);
        assert_eq!(plan.new_draft_nodes.len(), 0);
        assert_eq!(plan.superseded_node_ids.len(), 0);

        let reanchor = &plan.reanchored_spans[0];
        assert_eq!(reanchor.target_node_id, node.id);
        assert_eq!(reanchor.match_tier, ReconciliationTier::Tier1CanonicalKey);
        assert_eq!(reanchor.old_byte_start, Some(10));
        assert_eq!(reanchor.old_byte_end, Some(50));
        assert_eq!(reanchor.new_byte_start, 30);
        assert_eq!(reanchor.new_byte_end, 70);
        assert!(reanchor.is_span_shifted());

        // Verify active node was NOT mutated in place
        assert_eq!(node.byte_start, Some(10));
        assert_eq!(node.byte_end, Some(50));
    }

    #[test]
    fn test_tier2_ast_anchor_match_reanchor() {
        let node = make_test_node(
            None,
            Some("Section 2"),
            Some("Paragraph content."),
            "specs/test.md#section-2#block-0",
            100,
            150,
        );

        // Chunk with no key, identical anchor and content, shifted by +50 bytes
        let chunk = make_test_chunk(
            "specs/test.md#section-2#block-0",
            vec![],
            Some("Section 2"),
            "Paragraph content.",
            150,
            200,
        );

        let plan = reconcile_reingestion(&[chunk], std::slice::from_ref(&node));
        assert_eq!(plan.reanchored_spans.len(), 1);
        assert_eq!(
            plan.reanchored_spans[0].match_tier,
            ReconciliationTier::Tier2AstAnchor
        );
        assert_eq!(plan.reanchored_spans[0].target_node_id, node.id);
        assert_eq!(plan.reanchored_spans[0].new_byte_start, 150);
        assert_eq!(plan.reanchored_spans[0].new_byte_end, 200);
    }

    #[test]
    fn test_tier3_content_hash_match_reanchor() {
        let node = make_test_node(
            None,
            Some("Moved Section"),
            Some("Exact unchanged text block."),
            "specs/test.md#old-section#block-0",
            200,
            250,
        );

        // Anchor changed because section moved, no key, but text content is identical
        let chunk = make_test_chunk(
            "specs/test.md#new-location#block-3",
            vec![],
            Some("Moved Section"),
            "Exact unchanged text block.",
            400,
            450,
        );

        let plan = reconcile_reingestion(&[chunk], std::slice::from_ref(&node));
        assert_eq!(plan.reanchored_spans.len(), 1);
        assert_eq!(
            plan.reanchored_spans[0].match_tier,
            ReconciliationTier::Tier3ContentHash
        );
        assert_eq!(plan.reanchored_spans[0].target_node_id, node.id);
        assert_eq!(plan.reanchored_spans[0].new_byte_start, 400);
        assert_eq!(plan.reanchored_spans[0].new_byte_end, 450);
    }

    #[test]
    fn test_content_modified_stages_replacement_draft() {
        let node = make_test_node(
            Some("REQ-002"),
            Some("Crypto"),
            Some("Old encryption requirement."),
            "specs/test.md#crypto#block-0",
            50,
            100,
        );

        // Content changed from "Old encryption requirement." to "Updated post-quantum encryption requirement."
        let chunk = make_test_chunk(
            "specs/test.md#crypto#block-0",
            vec!["REQ-002"],
            Some("Crypto"),
            "Updated post-quantum encryption requirement.",
            50,
            120,
        );

        let plan = reconcile_reingestion(&[chunk], std::slice::from_ref(&node));
        assert_eq!(plan.reanchored_spans.len(), 0);
        assert_eq!(plan.new_draft_nodes.len(), 1);
        assert_eq!(plan.superseded_node_ids.len(), 1);
        assert_eq!(plan.superseded_node_ids[0], node.id);

        let draft = &plan.new_draft_nodes[0];
        assert_eq!(draft.replaces_node_id, Some(node.id));
        assert!(draft.is_replacement());
        assert_eq!(draft.node_key, Some("REQ-002".to_string()));
        assert_eq!(
            draft.content,
            "Updated post-quantum encryption requirement."
        );
    }

    #[test]
    fn test_deleted_and_new_sections() {
        let node_kept = make_test_node(
            Some("REQ-KEPT"),
            Some("Kept"),
            Some("I remain."),
            "specs/test.md#kept#block-0",
            0,
            20,
        );
        let node_deleted = make_test_node(
            Some("REQ-DEL"),
            Some("Deleted"),
            Some("I am removed."),
            "specs/test.md#del#block-0",
            30,
            60,
        );

        let chunk_kept = make_test_chunk(
            "specs/test.md#kept#block-0",
            vec!["REQ-KEPT"],
            Some("Kept"),
            "I remain.",
            0,
            20,
        );
        let chunk_brand_new = make_test_chunk(
            "specs/test.md#brand-new#block-0",
            vec!["REQ-BRAND-NEW"],
            Some("New Feature"),
            "Completely new functionality.",
            70,
            110,
        );

        let plan = reconcile_reingestion(
            &[chunk_kept, chunk_brand_new],
            &[node_kept.clone(), node_deleted.clone()],
        );

        assert_eq!(plan.reanchored_spans.len(), 1);
        assert_eq!(plan.reanchored_spans[0].target_node_id, node_kept.id);

        assert_eq!(plan.new_draft_nodes.len(), 1);
        assert_eq!(plan.new_draft_nodes[0].replaces_node_id, None);
        assert!(!plan.new_draft_nodes[0].is_replacement());
        assert_eq!(
            plan.new_draft_nodes[0].node_key,
            Some("REQ-BRAND-NEW".to_string())
        );

        assert_eq!(plan.superseded_node_ids, vec![node_deleted.id]);
        assert_eq!(plan.deleted_node_ids(), vec![node_deleted.id]);
    }

    #[test]
    fn test_table_metadata_enrichment_and_heading_priority() {
        use crate::ingest::parser::TableData;
        use std::collections::HashMap;

        // Table row metadata for DEC-1.1 and C-1
        let mut row_dec = HashMap::new();
        row_dec.insert("Decision ID".to_string(), "DEC-1.1".to_string());
        row_dec.insert("Category".to_string(), "Technical Trade-off".to_string());
        row_dec.insert("Status".to_string(), "Implemented".to_string());

        let mut row_c = HashMap::new();
        row_c.insert("ID".to_string(), "C-1".to_string());
        row_c.insert("Constraint".to_string(), "Pure Rust package".to_string());
        row_c.insert("Source".to_string(), "GEMINI.md".to_string());

        let table_td = TableData {
            headers: vec![
                "Decision ID".to_string(),
                "Category".to_string(),
                "Status".to_string(),
            ],
            rows: vec![row_dec.clone()],
        };

        let row_c_td = TableData {
            headers: vec![
                "ID".to_string(),
                "Constraint".to_string(),
                "Source".to_string(),
            ],
            rows: vec![row_c.clone()],
        };

        // Table chunk (container)
        let mut table_chunk = make_test_chunk(
            "specs/dec.md#ledger#block-0",
            vec![],
            Some("Decision Ledger"),
            "| DEC-1.1 | Technical Trade-off | Implemented |",
            0,
            100,
        );
        table_chunk.table_data = Some(table_td);

        // Table row chunk for DEC-1.1
        let mut table_row_dec_chunk = make_test_chunk(
            "specs/dec.md#ledger#table-row-dec-1-1",
            vec!["DEC-1.1"],
            Some("Decision Ledger"),
            "| DEC-1.1 | Technical Trade-off | Implemented |",
            10,
            50,
        );
        table_row_dec_chunk.table_data = Some(TableData {
            headers: vec![
                "Decision ID".to_string(),
                "Category".to_string(),
                "Status".to_string(),
            ],
            rows: vec![row_dec],
        });

        // Table row chunk for C-1 (only defined in table, no heading)
        let mut table_row_c_chunk = make_test_chunk(
            "specs/dec.md#constraints#table-row-c-1",
            vec!["C-1"],
            Some("Derived Constraints"),
            "| C-1 | Pure Rust package | GEMINI.md |",
            60,
            90,
        );
        table_row_c_chunk.table_data = Some(row_c_td);

        // Heading chunk for DEC-1.1
        let heading_dec_chunk = make_test_chunk(
            "specs/dec.md#dec-1-1",
            vec!["DEC-1.1"],
            Some("DEC-1.1: Direct Writes"),
            "### DEC-1.1: Direct Writes",
            150,
            200,
        );

        let plan = reconcile_reingestion(
            &[
                table_chunk,
                table_row_dec_chunk,
                table_row_c_chunk,
                heading_dec_chunk,
            ],
            &[],
        );

        assert_eq!(plan.new_draft_nodes.len(), 4);

        // 1. Heading chunk should have claimed node_key = Some("DEC-1.1")
        let dec_draft = plan
            .new_draft_nodes
            .iter()
            .find(|d| d.node_key == Some("DEC-1.1".to_string()))
            .expect("DEC-1.1 draft exists");
        assert_eq!(dec_draft.node_type, "DECISION");
        assert_eq!(dec_draft.governance_policy, "HUMAN_REVIEW_REQUIRED");
        assert_eq!(
            dec_draft
                .attributes
                .get("category")
                .and_then(|v| v.as_str()),
            Some("Technical Trade-off")
        );
        assert_eq!(
            dec_draft.attributes.get("status").and_then(|v| v.as_str()),
            Some("Implemented")
        );

        // 2. Table row chunk for DEC-1.1 should have yielded its key (node_key = None)
        let row_dec_draft = plan
            .new_draft_nodes
            .iter()
            .find(|d| d.ast_anchor.contains("#table-row-dec-1-1"))
            .expect("row draft exists");
        assert_eq!(row_dec_draft.node_key, None);

        // 3. Table row chunk for C-1 should have claimed node_key = Some("C-1")
        let c_draft = plan
            .new_draft_nodes
            .iter()
            .find(|d| d.node_key == Some("C-1".to_string()))
            .expect("C-1 draft exists");
        assert_eq!(c_draft.node_type, "UNCLASSIFIED");
        assert_eq!(c_draft.governance_policy, "HUMAN_REVIEW_REQUIRED");
        assert_eq!(
            c_draft.attributes.get("source").and_then(|v| v.as_str()),
            Some("GEMINI.md")
        );
        assert_eq!(c_draft.title.as_deref(), Some("C-1: Pure Rust package"));
    }
}
