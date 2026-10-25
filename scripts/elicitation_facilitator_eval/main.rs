//! Opt-in comparison of production facilitator adapters on synthetic contexts.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::Parser;
use serde::Serialize;
use tks::elicitation::{
    FACILITATOR_SYSTEM_PROMPT_V2, FacilitatorContext, FacilitatorProposal, FacilitatorProvider,
    INSPIRATION_SYSTEM_PROMPT_V3, OllamaFacilitatorProvider, TierfallFacilitatorProvider,
    lint_neutrality, parse_inspiration_ideas,
};

#[derive(Debug, Parser)]
struct Args {
    /// Required: permits external requests using synthetic, non-sensitive data only.
    #[arg(long)]
    live: bool,
    #[arg(long, default_value = "auto")]
    route: String,
    #[arg(long, env = "OLLAMA_HOST", default_value = "http://ollama:11434")]
    ollama_url: String,
    #[arg(long, env = "TKS_LLM_MODEL", default_value = "qwen3:8b")]
    ollama_model: String,
    #[arg(long, default_value = "1", value_parser = clap::value_parser!(u32).range(1..=10))]
    repetitions: u32,
    /// Total time allowed for each proposal, including fallback.
    #[arg(long, default_value = "180", value_parser = clap::value_parser!(u64).range(1..=600))]
    timeout_secs: u64,
    #[arg(long, default_value = "target/facilitator-comparison.json")]
    output: PathBuf,
}

#[derive(Serialize)]
struct Sample {
    scenario: String,
    mode: &'static str,
    backend: &'static str,
    requested: String,
    repetition: u32,
    duration_ms: u64,
    proposal: Option<FacilitatorProposal>,
    neutrality_flags: Vec<tks::elicitation::NeutralityFlag>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct Summary {
    backend: &'static str,
    samples: usize,
    successes: usize,
    failures: usize,
    local_fallbacks: usize,
    p50_ms: Option<u64>,
    p95_ms: Option<u64>,
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    synthetic_only: bool,
    percentile_method: &'static str,
    review_rubric: [&'static str; 5],
    summaries: Vec<Summary>,
    samples: Vec<Sample>,
}

fn scenarios() -> Vec<(&'static str, FacilitatorContext)> {
    let established = vec![
        ("stakeholder".to_string(), "A community tool library lends tools to registered members. A volunteer checks each tool out and back in.".to_string()),
        ("stakeholder".to_string(), "Each member may borrow at most three tools at once for seven days. Out of scope: payments and delivery.".to_string()),
    ];
    let mut cases = Vec::new();
    for (name, inspiration, focus, turns) in [
        ("opening", false, None, Vec::new()),
        ("established", false, None, established.clone()),
        (
            "focused",
            false,
            Some("A tool is returned damaged"),
            established.clone(),
        ),
        ("inspiration", true, None, established.clone()),
        ("excluded-ideas", true, Some("Overdue tools"), established),
    ] {
        cases.push((
            name,
            FacilitatorContext {
                system_prompt: if inspiration {
                    INSPIRATION_SYSTEM_PROMPT_V3
                } else {
                    FACILITATOR_SYSTEM_PROMPT_V2
                }
                .to_string(),
                session_title: "Synthetic community tool library".to_string(),
                is_inspiration: inspiration,
                focus_topic: focus.map(ToString::to_string),
                turns,
                explored_items: if name == "excluded-ideas" {
                    vec!["Reminder cadence for overdue loans".to_string()]
                } else {
                    Vec::new()
                },
                ..Default::default()
            },
        ));
    }
    cases
}

fn percentile(sorted: &[u64], percent: usize) -> Option<u64> {
    if sorted.is_empty() {
        None
    } else {
        Some(sorted[(sorted.len() * percent).div_ceil(100) - 1])
    }
}

fn summarize(samples: &[Sample], backend: &'static str) -> Summary {
    let selected: Vec<_> = samples.iter().filter(|s| s.backend == backend).collect();
    let mut timings: Vec<_> = selected
        .iter()
        .filter(|s| s.proposal.is_some())
        .map(|s| s.duration_ms)
        .collect();
    timings.sort_unstable();
    Summary {
        backend,
        samples: selected.len(),
        successes: timings.len(),
        failures: selected.len() - timings.len(),
        local_fallbacks: selected
            .iter()
            .filter(|s| {
                backend == "tierfall"
                    && s.proposal
                        .as_ref()
                        .is_some_and(|p| p.generation.provider == "ollama")
            })
            .count(),
        p50_ms: percentile(&timings, 50),
        p95_ms: percentile(&timings, 95),
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    if !args.live {
        return Err(
            "No requests sent: pass --live to compare providers on synthetic scenarios".into(),
        );
    }
    let tierfall = match tierfall::Tierfall::from_env().await {
        Ok(client) => {
            client.resolve(&args.route)?;
            Ok(TierfallFacilitatorProvider {
                client,
                route: args.route.clone(),
            })
        }
        Err(_) => Err("Tierfall initialization failed; check tierfall status".to_string()),
    };
    let ollama = OllamaFacilitatorProvider::new(args.ollama_url, args.ollama_model.clone());
    let mut samples = Vec::new();
    for repetition in 1..=args.repetitions {
        for (scenario, context) in scenarios() {
            // Alternate backend order to reduce systematic warm-up/order bias.
            let backends = if repetition % 2 == 1 {
                ["tierfall", "ollama"]
            } else {
                ["ollama", "tierfall"]
            };
            for backend in backends {
                let provider: Result<&dyn FacilitatorProvider, String> = if backend == "tierfall" {
                    tierfall
                        .as_ref()
                        .map(|p| p as &dyn FacilitatorProvider)
                        .map_err(Clone::clone)
                } else {
                    Ok(&ollama)
                };
                let start = Instant::now();
                let result = match provider {
                    Ok(provider) => match tokio::time::timeout(
                        Duration::from_secs(args.timeout_secs),
                        provider.propose(context.clone()),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => Err(format!("Proposal exceeded {} seconds", args.timeout_secs)),
                    },
                    Err(error) => Err(error),
                };
                let (proposal, error) = match result {
                    Ok(proposal)
                        if context.is_inspiration
                            && parse_inspiration_ideas(&proposal.text).is_empty() =>
                    {
                        (
                            Some(proposal),
                            Some("Inspiration response contained no well-formed ideas".to_string()),
                        )
                    }
                    Ok(proposal) => (Some(proposal), None),
                    Err(error) => (None, Some(error)),
                };
                let flags = if context.is_inspiration {
                    Vec::new()
                } else {
                    proposal
                        .as_ref()
                        .map(|p| lint_neutrality(&p.text))
                        .unwrap_or_default()
                };
                eprintln!(
                    "{backend}/{scenario} repetition {repetition}: {} ({} ms)",
                    if error.is_some() {
                        "failed"
                    } else {
                        "completed"
                    },
                    start.elapsed().as_millis()
                );
                samples.push(Sample {
                    scenario: scenario.to_string(),
                    mode: if context.is_inspiration {
                        "inspiration"
                    } else {
                        "socratic"
                    },
                    backend,
                    requested: if backend == "tierfall" {
                        args.route.clone()
                    } else {
                        args.ollama_model.clone()
                    },
                    repetition,
                    duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
                    proposal,
                    neutrality_flags: flags,
                    error,
                });
            }
        }
    }
    let summaries = vec![
        summarize(&samples, "tierfall"),
        summarize(&samples, "ollama"),
    ];
    let failed = summaries.iter().any(|s| s.failures > 0);
    let report = Report {
        schema_version: 1,
        synthetic_only: true,
        percentile_method: "nearest rank; successful samples only; includes fallback time; excludes initialization",
        review_rubric: [
            "Grounding: uses only the supplied synthetic domain context",
            "Neutrality: avoids presuppositions and leading questions",
            "Novelty: does not repeat established or excluded ideas",
            "Scope: respects explicit exclusions",
            "Mode: one bisection question, or one to three inspiration statements",
        ],
        summaries,
        samples,
    };
    if let Some(parent) = args.output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&args.output, serde_json::to_vec_pretty(&report)?)?;
    for summary in &report.summaries {
        println!(
            "{}: {}/{} succeeded, p50={:?}ms p95={:?}ms, local fallbacks={}",
            summary.backend,
            summary.successes,
            summary.samples,
            summary.p50_ms,
            summary.p95_ms,
            summary.local_fallbacks
        );
    }
    println!("Report: {}", args.output.display());
    if failed {
        return Err("Comparison had failed samples; partial results saved in report".into());
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(Args::parse()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_and_empty_samples() {
        assert_eq!(percentile(&[], 50), None);
        assert_eq!(percentile(&[10, 20, 30, 40, 50], 50), Some(30));
        assert_eq!(percentile(&[10, 20, 30, 40, 50], 95), Some(50));
        assert_eq!(summarize(&[], "tierfall").successes, 0);
    }

    #[tokio::test]
    async fn opt_in_required_before_initialization() {
        let args = Args::try_parse_from(["eval"]).unwrap();
        assert!(
            run(args)
                .await
                .unwrap_err()
                .to_string()
                .contains("No requests sent")
        );
        assert!(Args::try_parse_from(["eval", "--repetitions", "0"]).is_err());
        assert!(Args::try_parse_from(["eval", "--timeout-secs", "0"]).is_err());
    }

    #[test]
    fn synthetic_cases_cover_both_modes_and_exclusions() {
        let cases = scenarios();
        assert_eq!(cases.len(), 5);
        assert!(cases.iter().any(|(_, c)| c.is_inspiration));
        assert!(cases.iter().any(|(_, c)| !c.explored_items.is_empty()));
        assert!(cases.iter().any(|(_, c)| c.turns.is_empty()));
    }

    #[test]
    fn aggregation_keeps_failures_usage_and_local_fallback_honest() {
        let mut samples = Vec::new();
        for duration in [10, 20, 900] {
            let success = duration != 900;
            samples.push(Sample {
                scenario: "synthetic".to_string(),
                mode: "socratic",
                backend: "tierfall",
                requested: "auto".to_string(),
                repetition: 1,
                duration_ms: duration,
                proposal: success.then(|| FacilitatorProposal {
                    text: "A or B?".to_string(),
                    generation: tks::elicitation::GenerationMetadata {
                        provider: "ollama".to_string(),
                        account: Some("local".to_string()),
                        model: "local-model".to_string(),
                        attempts: 2,
                        duration_ms: duration,
                        usage: None,
                    },
                }),
                neutrality_flags: Vec::new(),
                error: (!success).then(|| "exhausted".to_string()),
            });
        }
        let summary = summarize(&samples, "tierfall");
        assert_eq!(
            (summary.successes, summary.failures, summary.local_fallbacks),
            (2, 1, 2)
        );
        assert_eq!((summary.p50_ms, summary.p95_ms), (Some(10), Some(20)));
        let json = serde_json::to_value(&samples).unwrap();
        assert!(json[0]["proposal"]["generation"]["usage"].is_null());
        assert_eq!(json[2]["error"], "exhausted");
        assert!(json[2]["proposal"].is_null());
    }
}
