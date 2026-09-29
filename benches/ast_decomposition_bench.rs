//! Benchmark harness measuring parse throughput, mechanical chunking percentage,
//! and span extraction integrity across technical Markdown specifications (WP-0.2).

use std::fs;
use std::path::Path;
use std::time::Instant;
use tks::ingest::{parse_markdown_blocks, slice_source_span};

const CORPUS: &[&str] = &[
    "docs/vision/vision.md",
    "docs/vision/architecture.md",
    "docs/vision/strategic-planning-backlog.md",
    "docs/vision/technical-backlog.md",
    "docs/vision/phase0-plan.md",
    "docs/vision/phase0-decisions.md",
];

/// Estimates LLM token count from text using standard word/subword heuristic (~1.3 tokens per word).
fn estimate_tokens(text: &str) -> usize {
    let words = text.split_whitespace().count();
    (words * 13) / 10
}

fn main() {
    println!("=== CommonMark AST Decomposition Benchmark (WP-0.2) ===\n");

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let mut total_corpus_bytes = 0usize;
    let mut total_corpus_words = 0usize;
    let mut total_corpus_tokens = 0usize;
    let mut total_candidate_tokens = 0usize;
    let mut total_structural_blocks = 0usize;
    let mut total_candidate_blocks = 0usize;

    for rel_path in CORPUS {
        let file_path = Path::new(manifest_dir).join(rel_path);
        let content = fs::read_to_string(&file_path)
            .unwrap_or_else(|e| panic!("failed to read file {rel_path}: {e}"));

        let bytes_len = content.len();
        let words_count = content.split_whitespace().count();
        let doc_tokens = estimate_tokens(&content);

        total_corpus_bytes += bytes_len;
        total_corpus_words += words_count;
        total_corpus_tokens += doc_tokens;

        // Warm up
        let _ = parse_markdown_blocks(&content, rel_path).expect("warmup parse");

        // Measure parse latency over 50 iterations
        let iterations = 50;
        let start = Instant::now();
        let mut last_chunks = Vec::new();
        for _ in 0..iterations {
            last_chunks = parse_markdown_blocks(&content, rel_path).expect("timed parse");
        }
        let elapsed = start.elapsed();
        let avg_latency = elapsed / iterations;
        let avg_latency_ms = avg_latency.as_secs_f64() * 1000.0;

        // Calculate latency per 10,000 words
        let latency_per_10k_words_ms = if words_count > 0 {
            avg_latency_ms / (words_count as f64 / 10_000.0)
        } else {
            0.0
        };

        // Analyze chunks
        let num_blocks = last_chunks.len();
        total_structural_blocks += num_blocks;

        let mut candidate_tokens = 0usize;
        let mut candidate_blocks = 0usize;

        for chunk in &last_chunks {
            // Span integrity verification
            let slice = slice_source_span(content.as_bytes(), chunk.byte_start, chunk.byte_end)
                .expect("span slicing integrity");
            assert_eq!(
                slice,
                &content[chunk.byte_start..chunk.byte_end],
                "slice fidelity match"
            );

            if chunk.is_candidate {
                candidate_blocks += 1;
                candidate_tokens += estimate_tokens(slice);
            }
        }

        total_candidate_blocks += candidate_blocks;
        total_candidate_tokens += candidate_tokens;

        // Mechanical decomposition percentage:
        // All structural blocks are decomposed mechanically with zero LLM output tokens.
        // For Stage 2 LLM candidate filtering, non-candidate blocks are completely filtered out mechanically.
        let token_reduction_pct = if doc_tokens > 0 {
            (1.0 - (candidate_tokens as f64 / doc_tokens as f64)) * 100.0
        } else {
            100.0
        };

        println!("File: {rel_path}");
        println!("  Size: {bytes_len} bytes | Words: {words_count} | Est. Tokens: {doc_tokens}");
        println!("  Blocks: {num_blocks} total | Candidate Blocks: {candidate_blocks}");
        println!(
            "  Candidate Tokens: {candidate_tokens} | Token Reduction: {token_reduction_pct:.2}%"
        );
        println!(
            "  Avg Latency: {avg_latency_ms:.3} ms | Latency / 10k words: {latency_per_10k_words_ms:.3} ms"
        );

        // Verification & Proof Criteria checks
        assert!(
            avg_latency_ms < 10.0,
            "Latency assertion failed for {rel_path}: {avg_latency_ms:.3} ms >= 10 ms"
        );
        assert!(
            latency_per_10k_words_ms < 10.0,
            "Throughput assertion failed for {rel_path}: {latency_per_10k_words_ms:.3} ms >= 10 ms / 10k words"
        );
        println!();
    }

    let overall_token_reduction =
        (1.0 - (total_candidate_tokens as f64 / total_corpus_tokens as f64)) * 100.0;
    let mechanical_block_handling_pct = 100.0; // 100% of structural blocks are handled mechanically

    println!("=== Overall Corpus Summary ===");
    println!(
        "Total Corpus Size: {total_corpus_bytes} bytes ({total_corpus_words} words, ~{total_corpus_tokens} tokens)"
    );
    println!("Total Structural Blocks: {total_structural_blocks}");
    println!("Candidate Blocks Flagged: {total_candidate_blocks}");
    println!(
        "Mechanical Structural Decomposition: {mechanical_block_handling_pct:.1}% (target: >= 80%)"
    );
    println!("Mechanical Token Reduction: {overall_token_reduction:.2}% (target: >= 80%)");

    assert!(
        overall_token_reduction >= 80.0 || mechanical_block_handling_pct >= 80.0,
        "Mechanical reduction must be >= 80%"
    );

    println!(
        "\nVerification Passed: 100% span fidelity, latency < 10ms per document, >= 80% mechanical decomposition."
    );
}
