//! Integration test validating CommonMark AST decomposition and exact span extraction spike (WP-0.2).

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use tks::ingest::{parse_markdown_blocks, slice_source_span};

const CORPUS_FILES: &[&str] = &[
    "docs/vision/vision.md",
    "docs/vision/architecture.md",
    "docs/vision/strategic-planning-backlog.md",
    "docs/vision/technical-backlog.md",
    "docs/vision/phase0-plan.md",
    "docs/vision/phase0-decisions.md",
];

#[test]
fn test_ast_spike_corpus_files() {
    for rel_path in CORPUS_FILES {
        let file_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel_path);
        let content = fs::read_to_string(&file_path)
            .unwrap_or_else(|e| panic!("failed to read file {rel_path}: {e}"));

        let chunks = parse_markdown_blocks(&content, rel_path)
            .unwrap_or_else(|e| panic!("failed to parse {rel_path}: {e}"));

        assert!(
            !chunks.is_empty(),
            "expected non-empty chunks for {rel_path}"
        );

        let mut seen_anchors = HashSet::new();
        let mut heading_anchors = HashSet::new();

        // 1. Check all heading chunks and build heading anchor lookup
        for chunk in &chunks {
            if chunk.ast_anchor.contains('#') && !chunk.ast_anchor.contains("#block-") {
                heading_anchors.insert(chunk.ast_anchor.clone());
            }
        }

        for (idx, chunk) in chunks.iter().enumerate() {
            // Assertion 1: Zero character boundary slicing panics and 100% round-trip fidelity
            let slice = slice_source_span(content.as_bytes(), chunk.byte_start, chunk.byte_end)
                .unwrap_or_else(|e| {
                    panic!(
                        "span slicing failed in {rel_path} for chunk {idx} [{start}..{end}]: {e}",
                        start = chunk.byte_start,
                        end = chunk.byte_end
                    )
                });

            // Exact match against &content[byte_start..byte_end]
            let expected_slice = &content[chunk.byte_start..chunk.byte_end];
            assert_eq!(
                slice, expected_slice,
                "slice mismatch in {rel_path} at chunk {idx}"
            );

            // Assertion 2: All extracted ast_anchor values are globally unique within the document
            assert!(
                seen_anchors.insert(chunk.ast_anchor.clone()),
                "duplicate ast_anchor detected in {rel_path}: {anchor}",
                anchor = chunk.ast_anchor
            );

            // Assertion 3: Structural child chunks correctly reference their immediate parent heading chunk
            if let Some(ref parent_id) = chunk.parent_heading_chunk_id {
                assert!(
                    heading_anchors.contains(parent_id),
                    "chunk {idx} ({anchor}) in {rel_path} references non-existent parent heading {parent_id}",
                    anchor = chunk.ast_anchor
                );
            }
        }
    }
}

#[test]
fn test_multibyte_utf8_span_safety_and_roundtrip() {
    let synthetic = r#"# Heading with Math ≥ and ≤ Symbols

Text with em-dash — and curly “quotes” alongside arrows → pointing to requirements.

## Child Section: REQ-01 & C-2

- Bullet item with $E=mc^2$ and symbols: ≥, ≤, ≠, ≈
- Another bullet with RFC 2119 keyword: MUST adhere to INV-1.

> Blockquote with curly ‘single quotes’ and German umlauts: ä, ö, ü, ß.
"#;
    let chunks = parse_markdown_blocks(synthetic, "specs/multibyte.md")
        .expect("successful parse of multi-byte document");

    assert!(!chunks.is_empty());
    let mut seen_anchors = HashSet::new();

    for (idx, chunk) in chunks.iter().enumerate() {
        // Must never panic on char boundaries
        let slice = slice_source_span(synthetic.as_bytes(), chunk.byte_start, chunk.byte_end)
            .unwrap_or_else(|e| panic!("failed slicing multi-byte chunk {idx}: {e}"));

        assert_eq!(slice, &synthetic[chunk.byte_start..chunk.byte_end]);
        assert!(seen_anchors.insert(chunk.ast_anchor.clone()));
    }
}
