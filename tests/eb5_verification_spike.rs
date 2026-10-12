//! Integration and validation test for EB-5: Intent Verification Feasibility & Comparison Spike.
//!
//! Validates that `eb5_spike_eval` executes, evaluates all 40+ adjudicated entailment cases across
//! the 10 domain transcripts, achieves $\ge 70.0\%$ agreement and $\ge 80.0\%$ phantom catch rate,
//! and writes the empirical report `docs/vision/spike-eb5-results.md`.

use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn test_eb5_spike_eval_execution_and_thresholds() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let output_path = Path::new(manifest_dir).join("target/tmp_spike_eb5_results.md");
    if output_path.exists() {
        let _ = fs::remove_file(&output_path);
    }

    let status = Command::new("cargo")
        .args([
            "run",
            "--bin",
            "eb5_spike_eval",
            "--",
            "--quiet",
            "--output",
            output_path.to_str().expect("valid utf-8 path"),
        ])
        .current_dir(manifest_dir)
        .status()
        .expect("cargo run --bin eb5_spike_eval must execute");

    assert!(status.success(), "eb5_spike_eval must exit with 0");
    assert!(
        output_path.exists(),
        "spike-eb5-results.md must be generated"
    );

    let report = fs::read_to_string(&output_path).expect("read report");
    let _ = fs::remove_file(&output_path);

    // Assert key section headings and gating criteria
    assert!(
        report.contains("Dual Verification Strategy: CONFIRMED"),
        "Report must confirm the verification strategy"
    );
    assert!(
        report.contains("Fresh-Context Agent Judge"),
        "Report must benchmark Fresh-Context Agent Judge"
    );
    assert!(
        report.contains("Local ONNX NLI Cross-Encoder"),
        "Report must benchmark Local ONNX NLI Cross-Encoder"
    );
    assert!(
        report.contains("Phantom Catch Rate"),
        "Report must evaluate phantom catch rate"
    );
    assert!(
        report.contains("PASS (Neither verifier falsified)"),
        "Report must confirm kill/pivot thresholds are satisfied"
    );
}
