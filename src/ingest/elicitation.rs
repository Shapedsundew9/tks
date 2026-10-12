//! Intent Elicitation session protocol, grounding verification, and gap analysis (EB-4, D-124, D-125).
//!
//! Provides:
//! - [`ExtractionSessionState`]: Serialized session state stored in `ingestion_jobs.attributes`.
//! - [`verify_grounding_quote`]: Verbatim whitespace-normalised grounding quote verification.
//! - [`check_technology_lint`]: Premature-concretization mechanism term detection against a technology lexicon.
//! - [`validate_episodes`]: Contiguity and complete turn coverage verification.
//! - [`detect_gaps`]: Deterministic evaluation of the 8 canonical gap detectors.
//! - [`stage_goals_tx`]: Atomic staging of confirmed `GOAL` and `SOURCE_EXCERPT` nodes into `graph_nodes`.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use uuid::Uuid;

use crate::ingest::transcript::{ParsedTurn, Participant};

/// Canonical technology lexicon for premature concretization linting (§6.3, step 6).
pub const DEFAULT_TECHNOLOGY_LEXICON: &[&str] = &[
    "api",
    "apis",
    "app",
    "application",
    "aws",
    "backend",
    "cloud",
    "dashboard",
    "database",
    "db",
    "docker",
    "frontend",
    "graphql",
    "grpc",
    "http",
    "https",
    "json",
    "kafka",
    "kubernetes",
    "k8s",
    "microservice",
    "microservices",
    "mongodb",
    "mysql",
    "oauth",
    "plugin",
    "postgres",
    "postgresql",
    "react",
    "redis",
    "rest",
    "rpc",
    "saas",
    "sdk",
    "server",
    "slack",
    "sql",
    "sqlite",
    "table",
    "ui",
    "vue",
    "web",
    "webhook",
    "websocket",
];

/// Normalizes consecutive whitespace runs (spaces, tabs, newlines) into a single space.
#[must_use]
pub fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Verifies whether `quote` appears verbatim in `utterance` after collapsing whitespace.
#[must_use]
pub fn verify_grounding_quote(quote: &str, utterance: &str) -> bool {
    let q_norm = collapse_whitespace(quote);
    let u_norm = collapse_whitespace(utterance);
    if q_norm.is_empty() || u_norm.is_empty() {
        return false;
    }
    u_norm.contains(&q_norm)
}

/// Checks `statement` against the technology lexicon.
///
/// Returns warning strings for each mechanism noun present in the statement
/// that was not stated in any of the `grounding_quotes`.
#[must_use]
pub fn check_technology_lint(
    statement: &str,
    grounding_quotes: &[&str],
    custom_lexicon: Option<&[&str]>,
) -> Vec<String> {
    let lexicon = custom_lexicon.unwrap_or(DEFAULT_TECHNOLOGY_LEXICON);
    let mut warnings = Vec::new();

    let combined_quotes = grounding_quotes
        .iter()
        .map(|q| q.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");

    for &term in lexicon {
        let pattern = format!(r"(?i)\b{}\b", regex::escape(term));
        if let Ok(re) = Regex::new(&pattern)
            && re.is_match(statement)
            && !re.is_match(&combined_quotes)
        {
            warnings.push(format!(
                "Premature concretization: mechanism term '{term}' in statement was not stated in grounding quotes"
            ));
        }
    }

    warnings
}

/// Citation reference grounding a goal in a transcript turn or document section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundingCitation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    pub quote: String,
}

/// Directed link between two goal candidates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalLinkCandidate {
    #[serde(rename = "type")]
    pub link_type: String,
    pub target: String,
}

/// Forward and reverse entailment verdict for a candidate goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationVerdict {
    pub goal_id: String,
    pub forward: String, // "ENTAILED", "NEUTRAL", "CONTRADICTED"
    #[serde(default)]
    pub reverse_overconstrained: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// An extracted candidate goal awaiting verification or staging.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalCandidate {
    pub id: String,
    pub kind: String, // "ACHIEVE", "MAINTAIN", "AVOID", "SOFTGOAL"
    pub statement: String,
    pub stakeholder: String,
    pub status: String, // "CONFIRMED", "TENTATIVE", "RETRACTED"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<String>,
    #[serde(default)]
    pub grounding: Vec<GroundingCitation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptable_paraphrases: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forbidden_concretizations: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<GoalLinkCandidate>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<VerificationVerdict>,
}

/// A contiguous topic episode segmenting conversational turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpisodeCandidate {
    pub id: String,
    pub turns: [String; 2], // [first_turn, last_turn]
    pub topic: String,
}

/// A detected gap in requirement or vision coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedGap {
    pub id: String,
    pub detector: String,
    pub refs: Vec<String>,
    pub description: String,
    pub suggested_question: String,
}

/// Complete in-flight elicitation extraction session state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionSessionState {
    pub session_id: Uuid,
    pub doc_path: String,
    pub document_hash: String,
    pub document_kind: String, // "transcript" or "document"
    pub cycle: u32,
    pub participants: Vec<Participant>,
    pub turns: Vec<ParsedTurn>,
    #[serde(default)]
    pub episodes: Vec<EpisodeCandidate>,
    #[serde(default)]
    pub goals: Vec<GoalCandidate>,
    #[serde(default)]
    pub verdicts: Vec<VerificationVerdict>,
    #[serde(default)]
    pub gaps: Vec<DetectedGap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_lexicon: Option<Vec<String>>,
}

static TURN_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^t(?P<num>\d+)$").expect("valid turn id regex"));

/// Validates that an array of episodes is non-empty, strictly contiguous, and covers all turns `t1..tN`.
///
/// # Errors
///
/// Returns an actionable error message describing gaps, overlaps, or boundary mismatches.
pub fn validate_episodes(episodes: &[EpisodeCandidate], total_turns: usize) -> Result<(), String> {
    if episodes.is_empty() {
        return Err("Episode submission cannot be empty".to_string());
    }
    if total_turns == 0 {
        return Err("Cannot validate episodes on a transcript with 0 turns".to_string());
    }

    let mut expected_start = 1u32;

    for ep in episodes {
        let start_num = parse_turn_number(&ep.turns[0])
            .ok_or_else(|| format!("Episode {} start turn '{}' is invalid", ep.id, ep.turns[0]))?;
        let end_num = parse_turn_number(&ep.turns[1])
            .ok_or_else(|| format!("Episode {} end turn '{}' is invalid", ep.id, ep.turns[1]))?;

        if start_num > end_num {
            return Err(format!(
                "Episode {} has inverted turns: t{start_num} > t{end_num}",
                ep.id
            ));
        }

        if start_num != expected_start {
            return Err(format!(
                "Episode {} non-contiguous: expected to start at t{expected_start}, but started at t{start_num}",
                ep.id
            ));
        }

        if end_num > total_turns as u32 {
            return Err(format!(
                "Episode {} exceeds transcript bounds: ends at t{end_num}, transcript has {total_turns} turns",
                ep.id
            ));
        }

        expected_start = end_num + 1;
    }

    if expected_start != (total_turns as u32 + 1) {
        return Err(format!(
            "Episodes do not cover all turns: ended at t{}, expected t{total_turns}",
            expected_start - 1
        ));
    }

    Ok(())
}

fn parse_turn_number(turn_id: &str) -> Option<u32> {
    TURN_ID_RE
        .captures(turn_id)
        .and_then(|c| c.name("num"))
        .and_then(|m| m.as_str().parse::<u32>().ok())
}

/// Evaluates the 8 canonical gap detectors against current session state.
#[must_use]
pub fn detect_gaps(session: &ExtractionSessionState) -> Vec<DetectedGap> {
    let mut gaps = Vec::new();
    let mut gap_counter = 1usize;

    // 1. SOFTGOAL_WITHOUT_CONTRIBUTOR
    let softgoals: Vec<&GoalCandidate> = session
        .goals
        .iter()
        .filter(|g| g.kind == "SOFTGOAL" && g.status != "RETRACTED")
        .collect();

    let contributor_targets: HashSet<&str> = session
        .goals
        .iter()
        .filter(|g| g.status != "RETRACTED")
        .filter_map(|g| g.links.as_ref())
        .flat_map(|links| links.iter())
        .filter(|l| l.link_type == "CONTRIBUTES_TO")
        .map(|l| l.target.as_str())
        .collect();

    for sg in softgoals {
        if !contributor_targets.contains(sg.id.as_str()) {
            gaps.push(DetectedGap {
                id: format!("GP-{}", gap_counter),
                detector: "SOFTGOAL_WITHOUT_CONTRIBUTOR".to_string(),
                refs: vec![sg.id.clone()],
                description: format!(
                    "Softgoal '{}' ({}) has zero incoming CONTRIBUTES_TO links",
                    sg.id, sg.statement
                ),
                suggested_question: format!(
                    "What specific functional behaviors or practices contribute towards '{}'?",
                    sg.statement
                ),
            });
            gap_counter += 1;
        }
    }

    // 2. GOAL_WITHOUT_STAKEHOLDER
    let valid_participant_names: HashSet<&str> = session
        .participants
        .iter()
        .map(|p| p.name.as_str())
        .collect();

    for g in session.goals.iter().filter(|g| g.status != "RETRACTED") {
        let is_empty = g.stakeholder.trim().is_empty() || g.stakeholder.trim() == "unknown";
        let is_unregistered = !valid_participant_names.is_empty()
            && !valid_participant_names.contains(g.stakeholder.as_str());

        if is_empty || is_unregistered {
            gaps.push(DetectedGap {
                id: format!("GP-{}", gap_counter),
                detector: "GOAL_WITHOUT_STAKEHOLDER".to_string(),
                refs: vec![g.id.clone()],
                description: format!(
                    "Goal '{}' has missing or unverified stakeholder: '{}'",
                    g.id, g.stakeholder
                ),
                suggested_question: format!(
                    "Who is the primary owner or beneficiary of the goal '{}'?",
                    g.statement
                ),
            });
            gap_counter += 1;
        }
    }

    // 3. EPISODE_WITHOUT_GOAL
    let episode_goal_counts: HashMap<&str, usize> = session
        .goals
        .iter()
        .filter(|g| g.status != "RETRACTED")
        .filter_map(|g| g.episode.as_deref())
        .fold(HashMap::new(), |mut acc, ep_id| {
            *acc.entry(ep_id).or_insert(0) += 1;
            acc
        });

    for ep in &session.episodes {
        if episode_goal_counts
            .get(ep.id.as_str())
            .copied()
            .unwrap_or(0)
            == 0
        {
            gaps.push(DetectedGap {
                id: format!("GP-{}", gap_counter),
                detector: "EPISODE_WITHOUT_GOAL".to_string(),
                refs: vec![ep.id.clone()],
                description: format!(
                    "Episode '{}' ({}) produced 0 confirmed or tentative goals",
                    ep.id, ep.topic
                ),
                suggested_question: format!(
                    "Did our discussion around '{}' imply any specific objectives or boundaries?",
                    ep.topic
                ),
            });
            gap_counter += 1;
        }
    }

    // 4. UNRESOLVED_CONFLICT
    for g in session.goals.iter().filter(|g| g.status != "RETRACTED") {
        if let Some(links) = &g.links {
            for l in links.iter().filter(|l| l.link_type == "CONFLICTS_WITH") {
                // If conflict target exists and g.id < l.target to avoid duplicate pairs
                if g.id < l.target {
                    gaps.push(DetectedGap {
                        id: format!("GP-{}", gap_counter),
                        detector: "UNRESOLVED_CONFLICT".to_string(),
                        refs: vec![g.id.clone(), l.target.clone()],
                        description: format!(
                            "Conflict between '{}' and '{}' has no explicit priority or resolution decision",
                            g.id, l.target
                        ),
                        suggested_question: format!(
                            "When trade-offs arise between '{}' and target '{}', which takes precedence?",
                            g.statement, l.target
                        ),
                    });
                    gap_counter += 1;
                }
            }
        }
    }

    // 5. TENTATIVE_NEVER_CONFIRMED
    for g in session.goals.iter().filter(|g| g.status == "TENTATIVE") {
        gaps.push(DetectedGap {
            id: format!("GP-{}", gap_counter),
            detector: "TENTATIVE_NEVER_CONFIRMED".to_string(),
            refs: vec![g.id.clone()],
            description: format!(
                "Goal '{}' remains marked as TENTATIVE without stakeholder confirmation",
                g.id
            ),
            suggested_question: format!(
                "Would you like to confirm '{}' as a definite requirement?",
                g.statement
            ),
        });
        gap_counter += 1;
    }

    // 6. ANTIGOAL_WITHOUT_RATIONALE
    for g in session
        .goals
        .iter()
        .filter(|g| g.kind == "AVOID" && g.status != "RETRACTED")
    {
        let no_rationale = g
            .rationale
            .as_ref()
            .map(|r| r.trim().is_empty())
            .unwrap_or(true);
        if no_rationale {
            gaps.push(DetectedGap {
                id: format!("GP-{}", gap_counter),
                detector: "ANTIGOAL_WITHOUT_RATIONALE".to_string(),
                refs: vec![g.id.clone()],
                description: format!(
                    "Anti-goal '{}' ('{}') specifies no explicit rationale",
                    g.id, g.statement
                ),
                suggested_question: format!(
                    "What specific harm or failure mode are we trying to prevent by avoiding '{}'?",
                    g.statement
                ),
            });
            gap_counter += 1;
        }
    }

    // 7. SILENT_STAKEHOLDER
    for p in session
        .participants
        .iter()
        .filter(|p| p.role == "stakeholder")
    {
        let turns_spoken = session.turns.iter().filter(|t| t.name == p.name).count();
        let goals_attributed = session
            .goals
            .iter()
            .filter(|g| g.stakeholder == p.name && g.status != "RETRACTED")
            .count();

        if turns_spoken == 0 || goals_attributed == 0 {
            gaps.push(DetectedGap {
                id: format!("GP-{}", gap_counter),
                detector: "SILENT_STAKEHOLDER".to_string(),
                refs: vec![p.name.clone()],
                description: format!(
                    "Registered stakeholder '{}' has {} turns spoken and {} goals attributed",
                    p.name, turns_spoken, goals_attributed
                ),
                suggested_question: format!(
                    "What are {}'s primary concerns and perspectives on this domain?",
                    p.name
                ),
            });
            gap_counter += 1;
        }
    }

    // 8. NO_BOUNDARY_STATEMENT
    let antigoal_count = session
        .goals
        .iter()
        .filter(|g| g.kind == "AVOID" && g.status != "RETRACTED")
        .count();

    if antigoal_count == 0 {
        gaps.push(DetectedGap {
            id: format!("GP-{}", gap_counter),
            detector: "NO_BOUNDARY_STATEMENT".to_string(),
            refs: vec![],
            description: "No anti-goals or boundary scoping statements have been extracted"
                .to_string(),
            suggested_question:
                "What is explicitly out of scope, or what should this system NOT do?".to_string(),
        });
    }

    gaps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_grounding_quote_exact_and_perturbed() {
        let utterance = "Honestly, I spend half my Monday chasing people on Slack for updates.";

        // 1. Exact match
        assert!(verify_grounding_quote("half my Monday", utterance));

        // 2. Whitespace-collapsed match
        assert!(verify_grounding_quote("half   my\nMonday", utterance));

        // 3. Altered word rejected
        assert!(!verify_grounding_quote("half my Tuesday", utterance));

        // 4. Two turns spliced / phantom phrase rejected
        assert!(!verify_grounding_quote(
            "half my Monday chasing people on Teams",
            utterance
        ));

        // 5. Empty quote rejected
        assert!(!verify_grounding_quote("", utterance));
        assert!(!verify_grounding_quote("   ", utterance));
    }

    #[test]
    fn test_technology_lint_premature_concretization() {
        let statement =
            "Project managers need a real-time dashboard and a Slack integration to track tasks.";
        let quotes = &["I spend half my Monday chasing people on Slack."];

        let warnings = check_technology_lint(statement, quotes, None);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("dashboard"));
        assert!(!warnings[0].contains("slack")); // Slack was stated in the quote
    }

    #[test]
    fn test_validate_episodes_contiguity() {
        let episodes = vec![
            EpisodeCandidate {
                id: "E1".to_string(),
                turns: ["t1".to_string(), "t4".to_string()],
                topic: "Intro".to_string(),
            },
            EpisodeCandidate {
                id: "E2".to_string(),
                turns: ["t5".to_string(), "t8".to_string()],
                topic: "Pain points".to_string(),
            },
        ];

        // Valid for 8 turns
        assert!(validate_episodes(&episodes, 8).is_ok());

        // Fails if total turns was 10 (incomplete coverage)
        assert!(validate_episodes(&episodes, 10).is_err());

        // Fails if gap exists (e.g. E2 starts at t6)
        let gapped = vec![
            EpisodeCandidate {
                id: "E1".to_string(),
                turns: ["t1".to_string(), "t4".to_string()],
                topic: "Intro".to_string(),
            },
            EpisodeCandidate {
                id: "E2".to_string(),
                turns: ["t6".to_string(), "t8".to_string()],
                topic: "Pain points".to_string(),
            },
        ];
        assert!(validate_episodes(&gapped, 8).is_err());
    }

    #[test]
    fn test_detect_gaps_softgoal_and_silent_stakeholder() {
        let state = ExtractionSessionState {
            session_id: Uuid::new_v4(),
            doc_path: "test.md".to_string(),
            document_hash: "hash".to_string(),
            document_kind: "transcript".to_string(),
            cycle: 1,
            participants: vec![
                Participant {
                    name: "Alice".to_string(),
                    role: "stakeholder".to_string(),
                },
                Participant {
                    name: "Bob".to_string(),
                    role: "stakeholder".to_string(),
                },
            ],
            turns: vec![ParsedTurn {
                id: "t1".to_string(),
                number: 1,
                name: "Alice".to_string(),
                role: "stakeholder".to_string(),
                timestamp: None,
                block_byte_start: 0,
                block_byte_end: 50,
                utterance_byte_start: 20,
                utterance_byte_end: 50,
                block_text: "**[t1] Alice (stakeholder):** Hello".to_string(),
                utterance_text: "Hello".to_string(),
            }],
            episodes: vec![EpisodeCandidate {
                id: "E1".to_string(),
                turns: ["t1".to_string(), "t1".to_string()],
                topic: "Intro".to_string(),
            }],
            goals: vec![GoalCandidate {
                id: "G1".to_string(),
                kind: "SOFTGOAL".to_string(),
                statement: "Minimal friction".to_string(),
                stakeholder: "Alice".to_string(),
                status: "CONFIRMED".to_string(),
                rationale: None,
                episode: Some("E1".to_string()),
                grounding: vec![GroundingCitation {
                    turn: Some("t1".to_string()),
                    section: None,
                    quote: "Hello".to_string(),
                }],
                acceptable_paraphrases: None,
                forbidden_concretizations: None,
                links: None,
                warnings: vec![],
                verification: None,
            }],
            verdicts: vec![],
            gaps: vec![],
            custom_lexicon: None,
        };

        let gaps = detect_gaps(&state);
        let detectors: Vec<&str> = gaps.iter().map(|g| g.detector.as_str()).collect();

        assert!(detectors.contains(&"SOFTGOAL_WITHOUT_CONTRIBUTOR"));
        assert!(detectors.contains(&"SILENT_STAKEHOLDER")); // Bob is silent
        assert!(detectors.contains(&"NO_BOUNDARY_STATEMENT")); // No AVOID goal
    }
}
