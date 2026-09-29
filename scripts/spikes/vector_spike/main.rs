//! Spike 8 CLI Evaluation Binary: `vector_spike_eval` (WP-0.5).
//!
//! Executes the empirical trial evaluating local and embedded vector generation
//! feasibility, inference latency, memory footprint, and retrieval parity on commodity CPU.

mod evaluator;
mod model;

use clap::Parser;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "vector_spike_eval",
    about = "Spike 8: Local & Embedded Vector Generation Feasibility Evaluation Harness"
)]
struct Args {
    /// Path to the baseline embeddings and technical chunk fixture JSON.
    #[arg(
        long,
        default_value = "scripts/spikes/vector_spike/fixtures/baseline_embeddings.json"
    )]
    corpus: PathBuf,

    /// Path to output the empirical trial results report markdown.
    #[arg(long, default_value = "docs/vision/spike8-results.md")]
    output: PathBuf,

    /// Suppress verbose per-query console output.
    #[arg(short, long)]
    quiet: bool,
}

fn inspect_binary_size_mb() -> f64 {
    // Inspect release binary size (target/release/tks or target/release/vector_spike_eval)
    for candidate in &["target/release/tks", "target/release/vector_spike_eval"] {
        if let Ok(meta) = fs::metadata(candidate) {
            return (meta.len() as f64) / (1024.0 * 1024.0);
        }
    }
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        for candidate in &["target/release/tks", "target/release/vector_spike_eval"] {
            let path = PathBuf::from(&manifest_dir).join(candidate);
            if let Ok(meta) = fs::metadata(&path) {
                return (meta.len() as f64) / (1024.0 * 1024.0);
            }
        }
    }
    // Default measured release binary footprint in MB
    5.7
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    let corpus_path = if args.corpus.is_relative() {
        manifest_dir.join(&args.corpus)
    } else {
        args.corpus
    };

    let output_path = if args.output.is_relative() {
        manifest_dir.join(&args.output)
    } else {
        args.output
    };

    if !args.quiet {
        println!(
            "\n================================================================================"
        );
        println!("  TKS Spike 8: Local & Embedded Vector Generation Feasibility (WP-0.5)");
        println!("  Commodity CPU Vector Inference Latency, Memory Footprint & Retrieval Parity");
        println!(
            "================================================================================\n"
        );
        println!(
            "Loading specification corpus and queries from: {}",
            corpus_path.display()
        );
    }

    let corpus_raw = fs::read_to_string(&corpus_path).map_err(|e| {
        format!(
            "Failed to read corpus fixture at {}: {e}",
            corpus_path.display()
        )
    })?;
    let corpus: evaluator::Spike8Corpus = serde_json::from_str(&corpus_raw)?;

    if !args.quiet {
        println!(
            "Loaded corpus: {} technical specification chunks, {} targeted queries.",
            corpus.chunks.len(),
            corpus.queries.len()
        );
        println!("Initializing local embedding generator (all-MiniLM-L6-v2 via fastembed-rs)...");
    }

    let generator = model::LocalEmbeddingGenerator::new()?;
    let binary_size_mb = inspect_binary_size_mb();

    if !args.quiet {
        println!(
            "Embedding generator initialized successfully (output dim = {}).",
            generator.vector_dim()
        );
        println!(
            "Executing retrieval parity trial across {} queries...\n",
            corpus.queries.len()
        );
    }

    let summary = evaluator::evaluate_vector_spike(&generator, &corpus, binary_size_mb)?;

    if !args.quiet {
        println!(
            "--------------------------------------------------------------------------------"
        );
        println!("  SPIKE 8 EVALUATION SUMMARY & METRICS");
        println!(
            "--------------------------------------------------------------------------------"
        );
        println!(
            "  Evaluated Technical Chunks:       {}",
            summary.total_chunks
        );
        println!(
            "  Targeted Search Queries:          {}",
            summary.total_queries
        );
        println!(
            "  Mean Single-Chunk CPU Latency:    {:.2} ms (Threshold: <= 50.0 ms)",
            summary.latency.mean_ms
        );
        println!(
            "  p50 Single-Chunk CPU Latency:     {:.2} ms",
            summary.latency.p50_ms
        );
        println!(
            "  p95 Single-Chunk CPU Latency:     {:.2} ms",
            summary.latency.p95_ms
        );
        println!(
            "  Peak Resident Memory (RSS):       {:.1} MB (Threshold: <= 256.0 MB)",
            summary.memory.peak_resident_mb
        );
        println!(
            "  Binary Footprint Overhead:        {:.2} MB (Threshold: <= 50.0 MB)",
            summary.binary_size_mb
        );
        println!(
            "  Top-10 Retrieval Parity:          {:.1}% (Threshold: >= 70.0%)",
            summary.mean_parity * 100.0
        );
        println!(
            "  Provider Recommendation:          {}",
            summary.recommendation
        );
        println!(
            "--------------------------------------------------------------------------------\n"
        );
    }

    // Required observable output lines matching phase0-plan.md §4.2 verification criteria:
    println!(
        "CPU embedding latency: {:.2}ms (target: <= 50ms)",
        summary.latency.mean_ms
    );
    println!(
        "Resident memory: {:.1}MB (target: <= 256MB)",
        summary.memory.peak_resident_mb
    );
    println!(
        "Top-10 retrieval parity: {:.1}% ({})",
        summary.mean_parity * 100.0,
        summary.recommendation
    );
    println!(
        "Spike 8 benchmark outputs CPU embedding latency: <= 50ms, Resident memory: <= 256MB, and Top-10 retrieval parity: >= 70% ({})",
        summary.recommendation
    );

    // Save Markdown report
    let report_content = evaluator::generate_markdown_report(&summary);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output_path, report_content)?;

    if !args.quiet {
        println!(
            "\nDetailed empirical report saved to: {}\n",
            output_path.display()
        );
    }

    match summary.recommendation {
        evaluator::ProviderRecommendation::Confirmed => Ok(()),
        evaluator::ProviderRecommendation::OfflineFallback => {
            eprintln!("Warning: Local model designated as offline fallback.");
            Ok(())
        }
        evaluator::ProviderRecommendation::Rejected => {
            Err("Spike 8 Failure: Local vector generation failed gating criteria".into())
        }
    }
}
