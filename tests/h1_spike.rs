//! Integration test suite for Spike 0 / WP-0.4 (Hypothesis H-1 Directional Validation).
//!
//! Validates in-memory property graph topology, depth-bounded envelope assembly,
//! synthetic task rubrics, evaluation calculation, and CLI execution.

use std::fs;
use std::path::Path;
use std::process::Command;

#[path = "../scripts/spikes/h1_spike/evaluator.rs"]
mod evaluator;
#[path = "../scripts/spikes/h1_spike/model.rs"]
mod model;
#[path = "../scripts/spikes/h1_spike/runner.rs"]
mod runner;

use evaluator::{GatingDecision, evaluate_trial};
use model::{InMemoryGraph, assemble_in_memory_envelope};
use runner::load_tasks;

#[test]
fn test_curated_graph_topology_integrity() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let graph_path =
        Path::new(manifest_dir).join("scripts/spikes/h1_spike/fixtures/curated_graph.json");
    let content = fs::read_to_string(&graph_path).expect("curated_graph.json must exist");
    let graph = InMemoryGraph::from_json(&content).expect("valid graph JSON");

    // Spec WP-0.4 requires ~50-100 nodes
    assert!(
        graph.nodes.len() >= 50 && graph.nodes.len() <= 100,
        "Graph node count must be between 50 and 100 (found {})",
        graph.nodes.len()
    );

    // Assert all 4 subsystems are represented
    let subsystems: std::collections::HashSet<_> =
        graph.nodes.values().map(|n| n.subsystem.as_str()).collect();
    assert!(subsystems.contains("auth"), "Must contain auth subsystem");
    assert!(
        subsystems.contains("storage"),
        "Must contain storage subsystem"
    );
    assert!(
        subsystems.contains("gateway"),
        "Must contain gateway subsystem"
    );
    assert!(
        subsystems.contains("ledger"),
        "Must contain ledger subsystem"
    );

    // Assert cross-cutting invariant nodes are present
    assert!(
        graph.get_node("INV-TRACE-01").is_some(),
        "INV-TRACE-01 must exist"
    );
    assert!(
        graph.get_node("INV-AUTH-02").is_some(),
        "INV-AUTH-02 must exist"
    );
    assert!(
        graph.get_node("INV-ERR-03").is_some(),
        "INV-ERR-03 must exist"
    );
    assert!(
        graph.get_node("INV-STATE-04").is_some(),
        "INV-STATE-04 must exist"
    );
    assert!(
        graph.get_node("INV-AUDIT-05").is_some(),
        "INV-AUDIT-05 must exist"
    );
    assert!(
        graph.get_node("INV-CONCUR-06").is_some(),
        "INV-CONCUR-06 must exist"
    );
    assert!(
        graph.get_node("INV-SPAN-07").is_some(),
        "INV-SPAN-07 must exist"
    );
    assert!(
        graph.get_node("INV-SEC-08").is_some(),
        "INV-SEC-08 must exist"
    );

    // Assert all edge endpoints reference existing nodes
    for edge in &graph.edges {
        assert!(
            graph.nodes.contains_key(&edge.from_node_id),
            "Edge {} references missing from_node_id {}",
            edge.edge_id,
            edge.from_node_id
        );
        assert!(
            graph.nodes.contains_key(&edge.to_node_id),
            "Edge {} references missing to_node_id {}",
            edge.edge_id,
            edge.to_node_id
        );
    }
}

#[test]
fn test_synthetic_tasks_suite_integrity() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let tasks_path = Path::new(manifest_dir).join("scripts/spikes/h1_spike/tasks.json");
    let tasks = load_tasks(&tasks_path).expect("tasks.json must load cleanly");

    // Spec WP-0.4 requires >= 20 tasks
    assert!(
        tasks.len() >= 20,
        "Must have >= 20 synthetic tasks (found {})",
        tasks.len()
    );

    let graph_path =
        Path::new(manifest_dir).join("scripts/spikes/h1_spike/fixtures/curated_graph.json");
    let graph_content = fs::read_to_string(&graph_path).expect("curated_graph.json must exist");
    let graph = InMemoryGraph::from_json(&graph_content).expect("valid graph JSON");

    for task in &tasks {
        assert!(
            graph.nodes.contains_key(&task.target_node_id),
            "Task {} references missing target_node_id {}",
            task.task_id,
            task.target_node_id
        );

        assert!(
            !task.rubric.is_empty(),
            "Task {} must have rubric rules",
            task.task_id
        );
        for rule in &task.rubric {
            assert!(
                graph.get_node(&rule.invariant_key).is_some(),
                "Rubric rule {} references unknown invariant {}",
                rule.id,
                rule.invariant_key
            );
        }
    }
}

#[test]
fn test_assemble_in_memory_envelope_depth_and_constraints() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let graph_path =
        Path::new(manifest_dir).join("scripts/spikes/h1_spike/fixtures/curated_graph.json");
    let content = fs::read_to_string(&graph_path).expect("curated_graph.json must exist");
    let graph = InMemoryGraph::from_json(&content).expect("valid graph JSON");

    // Test assembling envelope for node-task-gate-01 at depth 2
    let envelope = assemble_in_memory_envelope(&graph, "node-task-gate-01", 2);

    assert_eq!(envelope.target_node.id, "node-task-gate-01");
    assert_eq!(envelope.target_node.node_key, "TASK-GATE-01");

    // Ancestor requirements must include parent requirement and epic
    assert!(
        !envelope.ancestor_requirements.is_empty(),
        "Must extract ancestors"
    );
    let anc_keys: Vec<_> = envelope
        .ancestor_requirements
        .iter()
        .map(|n| n.node_key.as_str())
        .collect();
    assert!(
        anc_keys.contains(&"REQ-GATE-01"),
        "Must contain parent REQ-GATE-01"
    );
    assert!(
        anc_keys.contains(&"EPIC-GATEWAY"),
        "Must contain ancestor EPIC-GATEWAY at depth 2"
    );

    // Sibling constraints must contain CONSTRAINED_BY rules
    assert!(
        !envelope.sibling_constraints.is_empty(),
        "Must extract sibling constraints"
    );
    let con_keys: Vec<_> = envelope
        .sibling_constraints
        .iter()
        .map(|n| n.node_key.as_str())
        .collect();
    assert!(
        con_keys.contains(&"INV-TRACE-01"),
        "Must contain bound INV-TRACE-01"
    );
    assert!(
        con_keys.contains(&"INV-ERR-03"),
        "Must contain bound INV-ERR-03"
    );

    assert_eq!(
        envelope.total_nodes,
        1 + envelope.ancestor_requirements.len() + envelope.sibling_constraints.len()
    );

    let md = envelope.format_markdown();
    assert!(md.contains("TOPOLOGICAL CONTEXT ENVELOPE"));
    assert!(md.contains("TASK-GATE-01"));
    assert!(md.contains("INV-TRACE-01"));
}

#[test]
fn test_trial_evaluation_and_cal_h1_threshold() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let graph_path =
        Path::new(manifest_dir).join("scripts/spikes/h1_spike/fixtures/curated_graph.json");
    let graph = InMemoryGraph::from_json(&fs::read_to_string(&graph_path).unwrap()).unwrap();

    let tasks_path = Path::new(manifest_dir).join("scripts/spikes/h1_spike/tasks.json");
    let tasks = load_tasks(&tasks_path).unwrap();

    let (cond_a_codes, cond_b_codes) = runner::run_trial(&graph, &tasks, false).unwrap();
    let summary = evaluate_trial(&tasks, &cond_a_codes, &cond_b_codes);

    assert_eq!(summary.total_tasks, 20);
    assert!(summary.condition_a_violations > summary.condition_b_violations);

    // CAL-H1 directional validation requires >= 30.0% reduction
    assert!(
        summary.violation_reduction_pct >= 30.0,
        "Constraint violation reduction {:.1}% must be >= 30.0% (CAL-H1 threshold)",
        summary.violation_reduction_pct
    );
    assert_eq!(summary.decision, GatingDecision::Greenlight);

    // Generate markdown report and assert required headings
    let report = evaluator::generate_markdown_report(&summary);
    assert!(report.contains("Spike 0 Empirical Trial Report"));
    assert!(report.contains("GREENLIGHT FOR PHASE 1 CONSTRUCTION"));
}

#[test]
fn test_cli_binary_execution() {
    let bin_path = env!("CARGO_BIN_EXE_h1_spike_eval");
    let output = Command::new(bin_path)
        .arg("--quiet")
        .output()
        .expect("Failed to execute h1_spike_eval binary");

    assert!(
        output.status.success(),
        "Binary exited with non-zero status"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("H-1 Directional Validation: GREENLIGHT"),
        "Stdout missing greenlight confirmation: {}",
        stdout
    );
    assert!(
        stdout.contains(
            "Spike 0 evaluation reports Constraint violation reduction: >= 30% (H-1 Directional Validation: GREENLIGHT)"
        ),
        "Stdout missing exact required phrase: {}",
        stdout
    );
}
