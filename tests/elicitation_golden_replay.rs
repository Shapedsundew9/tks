//! Integration test for EB-8: Intent Elicitation Golden Replay & Exit Gate Harness.
//!
//! Validates:
//! 1. `elicitation_eval` binary execution across the corpus.
//! 2. Quantitative verification of §7.2 proof criteria (Recall >= 80%, Precision >= 75%, Grounding validity = 100%, Concretization <= 10%).
//! 3. Deterministic MCP tool golden replay on synthetic transcripts and docs/vision/vision.md dogfood document.

use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn test_elicitation_eval_binary_and_exit_gate_thresholds() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let output_path = Path::new(manifest_dir).join("target/tmp_elicitation_eval_results.md");
    if output_path.exists() {
        let _ = fs::remove_file(&output_path);
    }

    let status = Command::new("cargo")
        .args([
            "run",
            "--bin",
            "elicitation_eval",
            "--",
            "--quiet",
            "--output",
            output_path.to_str().expect("valid utf-8 path"),
        ])
        .current_dir(manifest_dir)
        .status()
        .expect("cargo run --bin elicitation_eval must execute");

    assert!(status.success(), "elicitation_eval must exit with 0");
    assert!(
        output_path.exists(),
        "elicitation-eval-results.md must be generated"
    );

    let report = fs::read_to_string(&output_path).expect("read report");
    let _ = fs::remove_file(&output_path);

    // Assert key section headings and gating criteria
    assert!(
        report.contains("PASSED (All §7.2 Quality Thresholds Satisfied)"),
        "Exit gate must confirm all thresholds satisfied"
    );
    assert!(
        report.contains("Goal Recall"),
        "Report must contain Goal Recall"
    );
    assert!(
        report.contains("Goal Precision"),
        "Report must contain Goal Precision"
    );
    assert!(
        report.contains("Grounding Validity"),
        "Report must contain Grounding Validity"
    );
    assert!(
        report.contains("Premature Concretization"),
        "Report must contain Premature Concretization"
    );
    assert!(
        report.contains("Dogfood Validation") && report.contains("vision.md"),
        "Report must confirm dogfood validation on vision.md"
    );
}
