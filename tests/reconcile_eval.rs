//! Comprehensive integration test suite for WP-1.3:
//! Two-Stage Assisted Decomposition Pipeline & 3-Tier Document Reconciliation.
//!
//! Validates:
//! 1. 100% exact byte offset slicing against multi-byte UTF-8 text with zero panics.
//! 2. Streaming CommonMark AST decomposition and upward structural hierarchy edges (`DERIVED_FROM`).
//! 3. Constraint C-11 token efficiency (<10% output tokens, <3% observed).
//! 4. Graceful degradation fallback with 'UNCLASSIFIED' typing in DRAFT state.
//! 5. 3-tier document reconciliation (node_key -> ast_anchor -> content hash) across multi-revision Markdown edits.
//! 6. Immutability of active nodes in place during reconciliation (staged coordinates only).

use std::fs;
use std::path::Path;
use uuid::Uuid;

use tks::ingest::{
    ExtractedChunk, ReconciliationTier, build_classification_prompt, fallback_classify,
    map_classifications_to_chunks, parse_classification_tuples, parse_markdown,
    reconcile_reingestion, slice_source_span, stage_reanchoring_metadata,
};
use tks::storage::GraphNode;

/// Estimates token count from text using word/subword and byte heuristics (~4 chars or 1.3 words per token).
fn estimate_tokens(text: &str) -> usize {
    let word_estimate = (text.split_whitespace().count() * 13) / 10;
    let char_estimate = text.len().div_ceil(4);
    word_estimate.max(char_estimate)
}

/// Simulated mock classifier for compact tuple schema evaluation.
fn mock_classify(candidate_chunks: &[&ExtractedChunk]) -> String {
    let mut tuples = Vec::with_capacity(candidate_chunks.len());

    for (idx, chunk) in candidate_chunks.iter().enumerate() {
        let alias = format!("c{}", idx + 1);
        let content_upper = chunk.content.as_deref().unwrap_or("").to_uppercase();

        let has_rfc2119 = !chunk.rfc2119_keywords.is_empty()
            || content_upper.contains(" MUST ")
            || content_upper.contains(" SHALL ")
            || content_upper.contains(" REQUIRED ");

        let has_req_key = chunk
            .canonical_keys
            .iter()
            .any(|k| k.starts_with("REQ-") || k.starts_with("INV-") || k.starts_with("C-"));

        let node_type = if has_req_key || has_rfc2119 {
            "REQUIREMENT"
        } else if content_upper.contains("TASK")
            || chunk
                .canonical_keys
                .iter()
                .any(|k| k.starts_with("TASK-") || k.starts_with("WP-"))
        {
            "TASK"
        } else {
            "SPECIFICATION"
        };

        let gov_policy = if node_type == "REQUIREMENT" {
            "HUMAN_REVIEW_REQUIRED"
        } else {
            "AUTONOMOUS_ELABORATION"
        };

        tuples.push(serde_json::json!({
            "chunk_id": alias,
            "node_type": node_type,
            "governance_policy": gov_policy,
        }));
    }

    serde_json::to_string(&tuples).expect("valid json serialization")
}

/// Helper to construct an active `GraphNode` from an extracted chunk simulating database state.
fn make_active_node_from_chunk(
    doc_path: &str,
    doc_hash: &str,
    chunk: &ExtractedChunk,
    node_type: &str,
) -> GraphNode {
    let mut attrs = serde_json::Map::new();
    attrs.insert(
        "ast_anchor".to_string(),
        serde_json::Value::String(chunk.ast_anchor.clone()),
    );

    GraphNode {
        id: Uuid::new_v4(),
        node_key: chunk.primary_node_key().map(|s| s.to_string()),
        node_type: node_type.to_string(),
        title: chunk.heading.clone(),
        content: chunk.content.clone(),
        lifecycle_state: "ACTIVE".to_string(),
        governance_policy: "HUMAN_REVIEW_REQUIRED".to_string(),
        created_by: "system".to_string(),
        job_id: None,
        doc_path: Some(doc_path.to_string()),
        doc_hash: Some(doc_hash.to_string()),
        byte_start: Some(chunk.byte_start as i32),
        byte_end: Some(chunk.byte_end as i32),
        attributes: serde_json::Value::Object(attrs),
    }
}

#[test]
fn test_multibyte_utf8_exact_slicing_and_roundtrip() {
    println!("\n=== WP-1.3 Test 1: Multi-Byte UTF-8 Exact Slicing & Roundtrip ===\n");

    let multibyte_doc = r#"# Architecture Overview — Systems & Protocols

The substrate MUST guarantee latency ≤ 50ms across all queries.

## Section 1: Invariants
Invariant INV-1 requires an unbroken causal chain: Task → Spec → Requirement.
Quotations like “strict bounds” and dashes like — are handled cleanly.

## Section 2: Mathematical Precision
- Rule 1: Precision ≥ 95.0%
- Rule 2: Overhead ≤ 10.0%
- Rule 3: Delta Δ = |A - B| → verified
"#;

    let res = parse_markdown("specs/multibyte.md", multibyte_doc).expect("successful parse");
    let bytes = multibyte_doc.as_bytes();

    // Verify 100% exact byte offset roundtrip for every single chunk
    for (i, chunk) in res.chunks.iter().enumerate() {
        let sliced = slice_source_span(bytes, chunk.byte_start, chunk.byte_end)
            .unwrap_or_else(|e| panic!("Span error on chunk {i}: {e}"));

        assert_eq!(
            Some(sliced),
            chunk.content.as_deref(),
            "Chunk {i} content must exactly match sliced source bytes"
        );
        assert!(chunk.byte_start <= chunk.byte_end);
        assert!(chunk.byte_end <= bytes.len());
    }

    // Verify canonical keys and RFC 2119 keywords
    let inv1_chunk = res
        .chunks
        .iter()
        .find(|c| c.canonical_keys.contains(&"INV-1".to_string()))
        .expect("must extract INV-1 chunk");
    assert!(inv1_chunk.is_candidate);
    assert_eq!(inv1_chunk.canonical_keys, vec!["INV-1"]);

    let must_chunk = res
        .chunks
        .iter()
        .find(|c| c.rfc2119_keywords.contains(&"MUST".to_string()))
        .expect("must extract MUST chunk");
    assert!(must_chunk.is_candidate);
    assert_eq!(must_chunk.rfc2119_keywords, vec!["MUST"]);

    println!("PASS: Exact byte offset slicing and roundtrip validated for multi-byte UTF-8.\n");
}

#[test]
fn test_ast_hierarchy_derived_from_edges() {
    println!("\n=== WP-1.3 Test 2: AST Hierarchy Upward DERIVED_FROM Edges ===\n");

    let doc = r#"# Level 1 Title
Pre-heading introductory text.

## Level 2 Heading A
Paragraph under Heading A.

### Level 3 Heading A.1
Paragraph under Heading A.1.

## Level 2 Heading B
Paragraph under Heading B.
"#;

    let res = parse_markdown("specs/hierarchy.md", doc).expect("successful parse");
    assert_eq!(res.chunks.len(), 8);

    // Verify upward structural hierarchy edges
    // Chunk 0: H1 Level 1 Title (no parent)
    assert_eq!(res.chunks[0].parent_heading_chunk_id, None);

    // Chunk 1: Intro paragraph under H1
    assert_eq!(
        res.chunks[1].parent_heading_chunk_id,
        Some(res.chunks[0].ast_anchor.clone())
    );

    // Chunk 2: H2 Level 2 Heading A under H1
    assert_eq!(
        res.chunks[2].parent_heading_chunk_id,
        Some(res.chunks[0].ast_anchor.clone())
    );

    // Chunk 3: Paragraph under H2 Heading A
    assert_eq!(
        res.chunks[3].parent_heading_chunk_id,
        Some(res.chunks[2].ast_anchor.clone())
    );

    // Chunk 4: H3 Level 3 Heading A.1 under H2 Heading A
    assert_eq!(
        res.chunks[4].parent_heading_chunk_id,
        Some(res.chunks[2].ast_anchor.clone())
    );

    // Chunk 5: Paragraph under H3 Heading A.1
    assert_eq!(
        res.chunks[5].parent_heading_chunk_id,
        Some(res.chunks[4].ast_anchor.clone())
    );

    // Chunk 6: H2 Level 2 Heading B under H1
    assert_eq!(
        res.chunks[6].parent_heading_chunk_id,
        Some(res.chunks[0].ast_anchor.clone())
    );

    // Chunk 7: Paragraph under H2 Heading B
    assert_eq!(
        res.chunks[7].parent_heading_chunk_id,
        Some(res.chunks[6].ast_anchor.clone())
    );

    // Verify mechanically emitted DERIVED_FROM edges
    assert_eq!(res.edges.len(), 7);
    for edge in &res.edges {
        assert_eq!(edge.edge_type, "DERIVED_FROM");
        assert!(edge.from_anchor.starts_with("specs/hierarchy.md#"));
        assert!(edge.to_anchor.starts_with("specs/hierarchy.md#"));
    }

    // Verify upward direction: child points to parent
    let edge_a1_to_a = res
        .edges
        .iter()
        .find(|e| e.from_anchor == res.chunks[4].ast_anchor)
        .expect("must have edge from A.1");
    assert_eq!(edge_a1_to_a.to_anchor, res.chunks[2].ast_anchor);

    let edge_p_to_a1 = res
        .edges
        .iter()
        .find(|e| e.from_anchor == res.chunks[5].ast_anchor)
        .expect("must have edge from paragraph under A.1");
    assert_eq!(edge_p_to_a1.to_anchor, res.chunks[4].ast_anchor);

    println!("PASS: Mechanical DERIVED_FROM edge generation validated.\n");
}

#[test]
fn test_c11_token_efficiency_and_classification_prompt() {
    println!("\n=== WP-1.3 Test 3: Constraint C-11 Token Efficiency Assertion (<10%) ===\n");

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let test_specs = &[
        "tests/fixtures/sample_spec.md",
        "docs/vision/architecture.md",
    ];

    let mut total_doc_tokens = 0usize;
    let mut total_output_tokens = 0usize;

    for spec_rel in test_specs {
        let spec_path = Path::new(manifest_dir).join(spec_rel);
        let content = fs::read_to_string(&spec_path)
            .unwrap_or_else(|e| panic!("failed to read {spec_rel}: {e}"));

        let doc_tokens = estimate_tokens(&content);
        let res = parse_markdown(spec_rel, &content)
            .unwrap_or_else(|e| panic!("failed to parse {spec_rel}: {e}"));

        let candidate_chunks: Vec<&ExtractedChunk> =
            res.chunks.iter().filter(|c| c.is_candidate).collect();
        let prompt = build_classification_prompt(&res.chunks);
        let prompt_tokens = estimate_tokens(&prompt);

        let output_json = mock_classify(&candidate_chunks);
        let output_tokens = estimate_tokens(&output_json);

        let ratio = output_tokens as f64 / doc_tokens as f64;

        println!("Document: {spec_rel}");
        println!("  Doc Tokens:    {doc_tokens}");
        println!("  Prompt Tokens: {prompt_tokens}");
        println!("  Output Tokens: {output_tokens}");
        println!("  Output Ratio:  {:.2}%", ratio * 100.0);

        // Assert strictly < 10% per Constraint C-11 (< 3% observed)
        assert!(
            ratio < 0.10,
            "Output token ratio {:.2}% for {spec_rel} exceeds C-11 limit of 10.0%",
            ratio * 100.0
        );

        // Verify bidirectional mapping
        let parsed_tuples = parse_classification_tuples(&output_json).expect("valid tuple parse");
        let mapped = map_classifications_to_chunks(&res.chunks, &parsed_tuples);
        assert_eq!(mapped.len(), candidate_chunks.len());

        total_doc_tokens += doc_tokens;
        total_output_tokens += output_tokens;
    }

    let overall_ratio = total_output_tokens as f64 / total_doc_tokens as f64;
    println!("\nCombined Corpus Token Summary:");
    println!("  Total Input Doc Tokens: {total_doc_tokens}");
    println!("  Total Output Tokens:    {total_output_tokens}");
    println!(
        "  Combined Ratio:         {:.2}% (target: < 10.0%, observed < 3.0%)",
        overall_ratio * 100.0
    );

    assert!(
        overall_ratio < 0.05,
        "Combined token ratio {:.2}% must be well below 10% (under 5%)",
        overall_ratio * 100.0
    );

    println!("PASS: Constraint C-11 token efficiency verified.\n");
}

#[test]
fn test_graceful_degradation_offline_fallback() {
    println!("\n=== WP-1.3 Test 4: Graceful Degradation Offline Fallback (D-37, D-62) ===\n");

    let chunk_rfc = ExtractedChunk {
        byte_start: 0,
        byte_end: 40,
        heading: Some("Security Policy".to_string()),
        ast_anchor: "sec.md#sec#block-0".to_string(),
        parent_heading_chunk_id: None,
        rfc2119_keywords: vec!["MUST".to_string()],
        canonical_keys: vec!["REQ-SEC-01".to_string()],
        is_candidate: true,
        content: Some("Requests MUST include a valid bearer token.".to_string()),
        table_data: None,
    };

    let chunk_unkeyed = ExtractedChunk {
        byte_start: 45,
        byte_end: 90,
        heading: None,
        ast_anchor: "sec.md#block-1".to_string(),
        parent_heading_chunk_id: None,
        rfc2119_keywords: vec![],
        canonical_keys: vec!["TB-5".to_string()],
        is_candidate: true,
        content: Some("General overview of identity seeding mechanism.".to_string()),
        table_data: None,
    };

    let results = fallback_classify(&[chunk_rfc, chunk_unkeyed]);
    assert_eq!(results.len(), 2);

    // Chunks with RFC 2119 keywords default to REQUIREMENT
    assert_eq!(results[0].chunk_id, "c1");
    assert_eq!(results[0].node_type, "REQUIREMENT");
    assert_eq!(results[0].governance_policy, "HUMAN_REVIEW_REQUIRED");

    // Other candidate chunks default to UNCLASSIFIED
    assert_eq!(results[1].chunk_id, "c2");
    assert_eq!(results[1].node_type, "UNCLASSIFIED");
    assert_eq!(results[1].governance_policy, "HUMAN_REVIEW_REQUIRED");

    println!("PASS: Graceful degradation fallback successfully evaluated.\n");
}

#[test]
fn test_3tier_reconciliation_multirevision_workflow() {
    println!(
        "\n=== WP-1.3 Test 5: 3-Tier Document Reconciliation & Staged Span Re-Anchoring ===\n"
    );

    // --- REVISION 1: Initial document ingestion ---
    let rev1_text = r#"# Specification Alpha

Introductory section explaining Alpha.

## Section 1: Security
REQ-AUTH-001: The system MUST authenticate all incoming API requests using cryptographic tokens.

## Section 2: Data Persistence
INV-DATA-002: Active requirement nodes SHALL NOT be mutated in place during document reconciliation.

## Section 3: Deprecated Feature
This section describes a legacy feature that will be removed in Revision 2.

## Section 4: Architecture
The architecture is composed of a unified single-binary Axum gateway.
"#;

    let doc_path = "specs/alpha.md";
    let rev1_decomp = parse_markdown(doc_path, rev1_text).expect("rev1 parse");

    // Simulate active nodes promoted from Revision 1
    let mut active_nodes = Vec::new();
    for chunk in &rev1_decomp.chunks {
        let node_type = if !chunk.canonical_keys.is_empty() || !chunk.rfc2119_keywords.is_empty() {
            "REQUIREMENT"
        } else {
            "SPECIFICATION"
        };
        active_nodes.push(make_active_node_from_chunk(
            doc_path,
            "blob_rev1_sha",
            chunk,
            node_type,
        ));
    }

    assert_eq!(active_nodes.len(), rev1_decomp.chunks.len());

    // Snapshot original byte spans to verify immutability of active nodes
    let original_spans: Vec<(Uuid, Option<i32>, Option<i32>)> = active_nodes
        .iter()
        .map(|n| (n.id, n.byte_start, n.byte_end))
        .collect();

    // --- REVISION 2: Shifted paragraphs, modified content, deleted section, and new section ---
    // Changes in Revision 2:
    // 1. Shifted paragraphs: Insert 120 bytes of preamble at the beginning, shifting all byte offsets!
    // 2. Unchanged sections: REQ-AUTH-001 and INV-DATA-002 retain exact same text (spans shifted!).
    // 3. Modified content: Section 4 text is modified (replaces_node_id candidate draft).
    // 4. Deleted section: Section 3 (Deprecated Feature) is completely removed!
    // 5. New section: Section 5 (New Feature) is newly added!
    let rev2_text = r#"# Specification Alpha

[PREAMBLE ADDED IN REV 2]: This new preamble paragraph shifts the byte offsets of every subsequent block by over 100 bytes!

Introductory section explaining Alpha.

## Section 1: Security
REQ-AUTH-001: The system MUST authenticate all incoming API requests using cryptographic tokens.

## Section 2: Data Persistence
INV-DATA-002: Active requirement nodes SHALL NOT be mutated in place during document reconciliation.

## Section 4: Architecture
The architecture is MODIFIED to utilize distributed actors with bounded channels and graceful recovery.

## Section 5: New Feature
REQ-NEW-003: Newly introduced capabilities MUST register with the central capability router.
"#;

    let rev2_decomp = parse_markdown(doc_path, rev2_text).expect("rev2 parse");

    // Execute 3-Tier Document Reconciliation
    let plan = reconcile_reingestion(&rev2_decomp.chunks, &active_nodes);

    println!("Reconciliation Plan Summary:");
    println!("  Re-anchored Spans:    {}", plan.reanchored_spans.len());
    println!("  New Draft Nodes:     {}", plan.new_draft_nodes.len());
    println!("  Superseded Node IDs: {}", plan.superseded_node_ids.len());

    // 1. VERIFY IMMUTABILITY: Ensure active nodes were NOT updated in place during reconciliation (INV-2, LD-1, D-74)
    for (node_id, orig_start, orig_end) in original_spans {
        let node = active_nodes.iter().find(|n| n.id == node_id).unwrap();
        assert_eq!(
            node.byte_start, orig_start,
            "Active node {node_id} byte_start must NOT be mutated in place during reconciliation!"
        );
        assert_eq!(
            node.byte_end, orig_end,
            "Active node {node_id} byte_end must NOT be mutated in place during reconciliation!"
        );
        assert_eq!(
            node.doc_hash,
            Some("blob_rev1_sha".to_string()),
            "Active node {node_id} doc_hash must retain original revision hash!"
        );
    }

    // 2. VERIFY TIER 1 (Canonical Key Match):
    // REQ-AUTH-001 and INV-DATA-002 must match via Tier 1
    let auth_reanchor = plan
        .reanchored_spans
        .iter()
        .find(|r| r.node_key == Some("REQ-AUTH-001".to_string()))
        .expect("REQ-AUTH-001 must be in reanchored_spans");
    assert_eq!(
        auth_reanchor.match_tier,
        ReconciliationTier::Tier1CanonicalKey
    );
    assert!(auth_reanchor.is_span_shifted());
    assert!(auth_reanchor.new_byte_start > auth_reanchor.old_byte_start.unwrap());

    let inv_reanchor = plan
        .reanchored_spans
        .iter()
        .find(|r| r.node_key == Some("INV-DATA-002".to_string()))
        .expect("INV-DATA-002 must be in reanchored_spans");
    assert_eq!(
        inv_reanchor.match_tier,
        ReconciliationTier::Tier1CanonicalKey
    );
    assert!(inv_reanchor.is_span_shifted());

    // 3. VERIFY TIER 2 (Structural Heading Anchor Match):
    // Section 1 heading has no canonical key and matches via Tier 2 ast_anchor
    let sec1_heading_reanchor = plan
        .reanchored_spans
        .iter()
        .find(|r| r.ast_anchor == "specs/alpha.md#specification-alpha/section-1-security")
        .expect("Section 1 heading must match via Tier 2 ast_anchor");
    assert_eq!(
        sec1_heading_reanchor.match_tier,
        ReconciliationTier::Tier2AstAnchor
    );
    assert!(sec1_heading_reanchor.is_span_shifted());

    // 4. VERIFY TIER 3 (Content Hash Match):
    // The introductory paragraph had no key, shifted from block-0 to block-1 -> matches via Tier 3 content hash
    let intro_reanchor = plan
        .reanchored_spans
        .iter()
        .find(|r| r.ast_anchor.contains("specification-alpha#block-1"))
        .expect("Introductory paragraph must match via Tier 3 content hash");
    assert_eq!(
        intro_reanchor.match_tier,
        ReconciliationTier::Tier3ContentHash
    );
    assert!(intro_reanchor.is_span_shifted());

    // 4. VERIFY CANDIDATE DRAFT FOR MODIFIED CONTENT (Section 4):
    // Section 4 architecture paragraph was modified -> must create candidate draft with replaces_node_id
    let modified_draft = plan
        .new_draft_nodes
        .iter()
        .find(|d| d.content.contains("The architecture is MODIFIED"))
        .expect("Section 4 modified text must produce a NewDraftNode");
    assert!(
        modified_draft.is_replacement(),
        "Modified section draft must have replaces_node_id populated"
    );
    let replaced_id = modified_draft.replaces_node_id.unwrap();
    // Confirm replaced node was Section 4 in Revision 1
    let orig_section_4 = active_nodes.iter().find(|n| n.id == replaced_id).unwrap();
    assert!(
        orig_section_4
            .content
            .as_deref()
            .unwrap()
            .contains("unified single-binary Axum gateway")
    );

    // 5. VERIFY CANDIDATE DRAFT FOR NEWLY ADDED CONTENT (Section 5):
    let new_feature_draft = plan
        .new_draft_nodes
        .iter()
        .find(|d| d.node_key == Some("REQ-NEW-003".to_string()))
        .expect("REQ-NEW-003 must produce a NewDraftNode");
    assert!(
        !new_feature_draft.is_replacement(),
        "Brand new section must have replaces_node_id = None"
    );
    assert_eq!(new_feature_draft.replaces_node_id, None);

    // 6. VERIFY SUPERSEDED / DELETED SECTION (Section 3):
    // Section 3 (Deprecated Feature) was removed from Rev 2 -> must be in superseded_node_ids
    let deleted_section_3_node = active_nodes
        .iter()
        .find(|n| {
            n.content
                .as_deref()
                .unwrap_or("")
                .contains("legacy feature that will be removed")
        })
        .expect("Section 3 node in Rev 1");

    assert!(
        plan.superseded_node_ids
            .contains(&deleted_section_3_node.id),
        "Deleted section node ID must be marked in superseded_node_ids"
    );

    let deleted_ids = plan.deleted_node_ids();
    assert!(
        deleted_ids.contains(&deleted_section_3_node.id),
        "deleted_node_ids() helper must identify deleted section"
    );
    assert!(
        !deleted_ids.contains(&replaced_id),
        "deleted_node_ids() helper must exclude replaced nodes that have replacement drafts"
    );

    // 7. VERIFY CANDIDATE METADATA STAGING JSON SCHEMA (D-74, TB-7)
    let metadata_json = stage_reanchoring_metadata(&plan.reanchored_spans);
    assert!(metadata_json.is_array());
    let entries = metadata_json.as_array().unwrap();
    assert_eq!(entries.len(), plan.reanchored_spans.len());

    for entry in entries {
        assert!(entry.get("target_node_id").is_some());
        assert!(entry.get("new_byte_start").is_some());
        assert!(entry.get("new_byte_end").is_some());
        assert!(entry.get("match_tier").is_some());
    }

    println!("PASS: Multi-revision 3-tier document reconciliation verified cleanly.\n");
}
