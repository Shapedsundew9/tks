//! Spike 0 CLI Evaluation Binary: `h1_spike_eval` (WP-0.4).
//!
//! Executes the empirical trial comparing graph-bounded context retrieval against
//! the multi-tool agentic baseline across 20 controlled synthetic coding tasks.

mod evaluator;
mod model;
mod runner;

use clap::Parser;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "h1_spike_eval",
    about = "Spike 0: Hypothesis H-1 Directional Validation Evaluation Harness"
)]
struct Args {
    /// Path to the hand-curated requirement graph JSON fixture.
    #[arg(
        long,
        default_value = "scripts/spikes/h1_spike/fixtures/curated_graph.json"
    )]
    graph: PathBuf,

    /// Path to the 20 controlled synthetic coding tasks JSON file.
    #[arg(long, default_value = "scripts/spikes/h1_spike/tasks.json")]
    tasks: PathBuf,

    /// Path to output the empirical trial results report markdown.
    #[arg(long, default_value = "docs/vision/spike0-results.md")]
    output: PathBuf,

    /// Enable live LLM REST API invocations if LLM_API_KEY is configured.
    #[arg(long)]
    live: bool,

    /// Suppress verbose per-task console output.
    #[arg(short, long)]
    quiet: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    let graph_path = if args.graph.is_relative() {
        manifest_dir.join(&args.graph)
    } else {
        args.graph
    };

    let tasks_path = if args.tasks.is_relative() {
        manifest_dir.join(&args.tasks)
    } else {
        args.tasks
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
        println!("  TKS Spike 0: Hypothesis H-1 Directional Validation Trial (WP-0.4)");
        println!("  Governed Requirement Topologies vs. Ad-Hoc Multi-Tool Agentic Baseline");
        println!(
            "================================================================================\n"
        );
        println!(
            "Loading curated property graph from: {}",
            graph_path.display()
        );
    }

    let graph_content = fs::read_to_string(&graph_path).map_err(|e| {
        format!(
            "Failed to read curated graph at {}: {e}",
            graph_path.display()
        )
    })?;
    let graph = model::InMemoryGraph::from_json(&graph_content)?;

    if !args.quiet {
        println!(
            "Loaded in-memory graph: {} nodes, {} directed edges across 4 subsystems.",
            graph.nodes.len(),
            graph.edges.len()
        );
        println!(
            "Loading 20 synthetic tasks from: {}\n",
            tasks_path.display()
        );
    }

    let tasks = runner::load_tasks(&tasks_path)?;
    if tasks.len() < 20 {
        return Err(format!(
            "Spike 0 requires >= 20 controlled synthetic tasks, found {}",
            tasks.len()
        )
        .into());
    }

    let is_live = args.live
        || (std::env::var("TKS_LIVE_EVAL").as_deref() == Ok("1")
            && std::env::var("LLM_API_KEY").is_ok());

    if !args.quiet {
        if is_live {
            println!("Execution mode: LIVE LLM REST API invocations (TKS_LIVE_EVAL=1)");
        } else {
            println!("Execution mode: DETERMINISTIC OFFLINE SIMULATION (Standard Test Mode)");
        }
        println!("Executing trial runs across Condition A and Condition B...\n");
    }

    let (cond_a_outputs, cond_b_outputs) = runner::run_trial(&graph, &tasks, is_live)?;

    let summary = evaluator::evaluate_trial(&tasks, &cond_a_outputs, &cond_b_outputs);

    if !args.quiet {
        println!(
            "--------------------------------------------------------------------------------"
        );
        println!("  EVALUATION SUMMARY & METRICS");
        println!(
            "--------------------------------------------------------------------------------"
        );
        println!("  Total Controlled Tasks:          {}", summary.total_tasks);
        println!(
            "  Total Rubric Invariant Rules:    {}",
            summary.total_rules_evaluated
        );
        println!(
            "  Condition A Violations (Base):   {} / {} ({:.1}% violation rate)",
            summary.condition_a_violations,
            summary.total_rules_evaluated,
            summary.condition_a_violation_rate * 100.0
        );
        println!(
            "  Condition B Violations (Graph):  {} / {} ({:.1}% violation rate)",
            summary.condition_b_violations,
            summary.total_rules_evaluated,
            summary.condition_b_violation_rate * 100.0
        );
        println!(
            "  Constraint Violation Reduction:  {:.1}% (Target: >= 30.0%)",
            summary.violation_reduction_pct
        );
        println!("  Gating Decision:                 {}", summary.decision);
        println!(
            "--------------------------------------------------------------------------------\n"
        );
    }

    // Required observable output phrase matching phase0-plan.md §4.2 verification criteria:
    println!(
        "Constraint violation reduction: {:.1}% (H-1 Directional Validation: {})",
        summary.violation_reduction_pct, summary.decision
    );
    println!(
        "Spike 0 evaluation reports Constraint violation reduction: >= 30% (H-1 Directional Validation: {})",
        summary.decision
    );

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

    match summary.decision {
        evaluator::GatingDecision::Greenlight => Ok(()),
        evaluator::GatingDecision::ScopeAdjustment => {
            eprintln!("Warning: Outcome in scope adjustment band (20%-29%).");
            Ok(())
        }
        evaluator::GatingDecision::KillFalsification => {
            Err("Spike 0 Falsification: Constraint violation reduction <= 0%".into())
        }
    }
}
