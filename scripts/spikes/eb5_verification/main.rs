//! Spike EB-5 CLI Evaluation Binary: `eb5_spike_eval` (WP EB-5).
//!
//! Evaluates the Intent Verification mechanisms (Fresh-Context Agent Judge vs
//! Local ONNX NLI Cross-Encoder) across the 40+ adjudicated entailment cases.

mod evaluator;
mod model;

use clap::Parser;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "eb5_spike_eval",
    about = "Spike EB-5: Intent Verification Feasibility & NLI Cross-Encoder Comparison Harness"
)]
struct Args {
    /// Path to the fixtures directory containing transcripts and labels.
    #[arg(long, default_value = "tests/fixtures/elicitation")]
    fixtures: PathBuf,

    /// Path to output the empirical trial results report markdown.
    #[arg(long, default_value = "docs/vision/spike-eb5-results.md")]
    output: PathBuf,

    /// Enable live LLM invocations against Ollama / OpenAI endpoint if configured.
    #[arg(long)]
    live: bool,

    /// Suppress verbose console output.
    #[arg(short, long)]
    quiet: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    let fixtures_dir = if args.fixtures.is_relative() {
        manifest_dir.join(&args.fixtures)
    } else {
        args.fixtures
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
        println!("  TKS Spike EB-5: Intent Verification Feasibility & Comparison (WP EB-5)");
        println!("  Fresh-Context Agent Judge vs. Local ONNX NLI Cross-Encoder");
        println!(
            "================================================================================\n"
        );
        println!("Loading fixtures from: {}", fixtures_dir.display());
    }

    let summary = evaluator::run_evaluation(&fixtures_dir, args.live).await?;

    if !args.quiet {
        println!(
            "Evaluated {} test cases across {} domain fixtures.",
            summary.total_cases, summary.total_fixtures
        );
        println!(
            "Model A (Agent Judge): Agreement = {:.1}%, Phantom Catch = {:.1}%",
            summary.agent_metrics.agreement_rate, summary.agent_metrics.phantom_catch_rate
        );
        println!(
            "Model B (Local ONNX):   Agreement = {:.1}%, Phantom Catch = {:.1}%, Latency p50 = {:.2}ms",
            summary.onnx_metrics.agreement_rate,
            summary.onnx_metrics.phantom_catch_rate,
            summary.onnx_metrics.p50_latency_ms
        );
        println!(
            "Peak Resident Memory (RSS): {:.1} MB",
            summary.resident_memory_mb
        );
        println!("\nWriting evaluation report to: {}", output_path.display());
    }

    let report_content = evaluator::generate_markdown_report(&summary);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output_path, report_content)?;

    if !args.quiet {
        println!("Trial report generated successfully.\n");
    }

    Ok(())
}
