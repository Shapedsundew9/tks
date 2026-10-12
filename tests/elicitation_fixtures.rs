#![allow(dead_code, non_camel_case_types, clippy::upper_case_acronyms)]

use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct TranscriptFrontmatter {
    format: String,
    title: String,
    session_id: String,
    cycle: u32,
    date: String,
    language: String,
    source: String,
    participants: Vec<Participant>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Participant {
    name: String,
    role: String,
}

#[derive(Debug, Clone)]
struct Turn {
    id: String,
    number: u32,
    name: String,
    role: String,
    utterance: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LabelFile {
    schema_version: String,
    source: SourceInfo,
    adjudication: AdjudicationInfo,
    episodes: Vec<EpisodeLabel>,
    goals: Vec<GoalLabel>,
    traps: Vec<TrapLabel>,
    gaps: Vec<GapLabel>,
    entailment_cases: Vec<EntailmentCaseLabel>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceInfo {
    kind: SourceKind,
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SourceKind {
    Transcript,
    Document,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdjudicationInfo {
    status: AdjudicationStatus,
    labeled_by: String,
    adjudicated_by: Option<String>,
    notes: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
enum AdjudicationStatus {
    DRAFT,
    ADJUDICATED,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EpisodeLabel {
    id: String,
    turns: Vec<String>,
    topic: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalLabel {
    id: String,
    kind: GoalKind,
    statement: String,
    stakeholder: String,
    status: GoalStatus,
    episode: Option<String>,
    grounding: Vec<GroundingRef>,
    acceptable_paraphrases: Vec<String>,
    forbidden_concretizations: Vec<String>,
    links: Vec<GoalLink>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
enum GoalKind {
    ACHIEVE,
    MAINTAIN,
    AVOID,
    SOFTGOAL,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
enum GoalStatus {
    CONFIRMED,
    TENTATIVE,
    RETRACTED,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GroundingRef {
    Transcript { turn: String, quote: String },
    Document { section: String, quote: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalLink {
    #[serde(rename = "type")]
    link_type: LinkType,
    target: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
enum LinkType {
    REFINES,
    CONFLICTS_WITH,
    CONTRIBUTES_TO,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrapLabel {
    id: String,
    #[serde(rename = "type")]
    trap_type: TrapType,
    turns: Vec<String>,
    expected: TrapExpected,
    note: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Hash, Clone, Copy)]
enum TrapType {
    ANALOGY,
    UI_METAPHOR,
    RETRACTION,
    MULTI_TURN_COREFERENCE,
    VENTING_NO_GOAL,
    CROSS_TURN_CONTRADICTION,
    IMPLICIT_SOFTGOAL,
    EXPLICIT_ANTIGOAL,
    BOUNDARY_BY_NEGATION,
    MISSING_STAKEHOLDER,
    FACILITATOR_LEADING,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
enum TrapExpected {
    NO_GOAL,
    NO_MECHANISM_GOAL,
    ABSTRACT_GOAL,
    RETRACTED_NOT_STAGED,
    RESOLVED_REFERENT,
    CONFLICT_LINK,
    ANTIGOAL,
    GAP_REPORTED,
    FACILITATOR_NOT_SOURCE,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GapLabel {
    id: String,
    detector: GapDetector,
    refs: Vec<String>,
    description: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
enum GapDetector {
    SOFTGOAL_WITHOUT_CONTRIBUTOR,
    GOAL_WITHOUT_STAKEHOLDER,
    EPISODE_WITHOUT_GOAL,
    UNRESOLVED_CONFLICT,
    TENTATIVE_NEVER_CONFIRMED,
    ANTIGOAL_WITHOUT_RATIONALE,
    SILENT_STAKEHOLDER,
    NO_BOUNDARY_STATEMENT,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntailmentCaseLabel {
    id: String,
    premise_turns: Vec<String>,
    hypothesis: String,
    forward: EntailmentForward,
    reverse_overconstrained: bool,
    phantom: bool,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
enum EntailmentForward {
    ENTAILED,
    NEUTRAL,
    CONTRADICTED,
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_transcript(content: &str) -> (TranscriptFrontmatter, Vec<Turn>) {
    let mut parts = content.splitn(3, "---");
    let initial = parts.next().expect("initial segment");
    assert!(
        initial.trim().is_empty(),
        "Frontmatter must be at start of file"
    );

    let frontmatter_str = parts.next().expect("frontmatter content");
    let body = parts.next().expect("body content");

    let frontmatter: TranscriptFrontmatter =
        serde_yaml::from_str(frontmatter_str).expect("Valid YAML frontmatter");
    assert_eq!(
        frontmatter.format, "tks-transcript/v1",
        "format must be tks-transcript/v1"
    );

    let valid_roles = ["stakeholder", "facilitator", "agent"];
    for p in &frontmatter.participants {
        assert!(
            valid_roles.contains(&p.role.as_str()),
            "Invalid participant role: {}",
            p.role
        );
    }

    let turn_re = Regex::new(
        r"^\*\*\[t(?P<num>\d+)\]\s+(?P<name>[^\(]+?)\s+\((?P<role>[^\)]+)\):\*\*\s*(?:_\(\d{2}:\d{2}:\d{2}\)_\s+)?(?P<text>.*)$",
    )
    .expect("valid turn regex");

    let paragraphs: Vec<&str> = body
        .split("\n\n")
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();

    let mut turns = Vec::new();

    for (expected_num, para) in (1..).zip(paragraphs) {
        let caps = turn_re.captures(para).unwrap_or_else(|| {
            panic!(
                "Paragraph does not match expected turn header in session {}: {:?}",
                frontmatter.session_id, para
            )
        });

        let num: u32 = caps["num"].parse().expect("valid turn number integer");
        assert_eq!(
            num, expected_num,
            "Turn numbering must be contiguous in session {}: expected {}, got {}",
            frontmatter.session_id, expected_num, num
        );

        let name = caps["name"].trim().to_string();
        let role = caps["role"].trim().to_string();
        let utterance = caps["text"].trim().to_string();

        let participant = frontmatter
            .participants
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "Speaker '{}' not found in participants of session {}",
                    name, frontmatter.session_id
                )
            });
        assert_eq!(
            participant.role, role,
            "Speaker '{}' role mismatch in turn t{}: frontmatter says {}, turn says {}",
            name, num, participant.role, role
        );

        turns.push(Turn {
            id: format!("t{}", num),
            number: num,
            name,
            role,
            utterance,
        });
    }

    assert!(
        turns.len() >= 15 && turns.len() <= 40,
        "Turn count must be between 15 and 40 in session {}, found {}",
        frontmatter.session_id,
        turns.len()
    );

    (frontmatter, turns)
}

fn parse_vision_sections(content: &str) -> HashMap<String, String> {
    let mut sections = HashMap::new();
    let mut current_sec: Option<String> = None;
    let mut buf = Vec::new();

    for line in content.lines() {
        if line.starts_with('#') {
            if let Some(sec) = current_sec.take() {
                sections.insert(sec, buf.join("\n"));
                buf.clear();
            }
            let heading_text = line.trim_start_matches('#').trim().to_string();
            current_sec = Some(heading_text);
        } else {
            buf.push(line);
        }
    }
    if let Some(sec) = current_sec {
        sections.insert(sec, buf.join("\n"));
    }

    sections
}

#[test]
fn test_elicitation_corpus_and_labels_integrity() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let transcripts_dir = repo_root.join("tests/fixtures/elicitation/transcripts");
    let labels_dir = repo_root.join("tests/fixtures/elicitation/labels");

    let transcript_paths: Vec<PathBuf> = fs::read_dir(&transcripts_dir)
        .expect("transcripts dir exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .collect();

    assert!(
        transcript_paths.len() >= 10,
        "Corpus must contain at least 10 transcripts, found {}",
        transcript_paths.len()
    );

    let mut transcript_cache: HashMap<String, (TranscriptFrontmatter, Vec<Turn>)> = HashMap::new();

    for path in &transcript_paths {
        let content = fs::read_to_string(path).expect("read transcript file");
        let (frontmatter, turns) = parse_transcript(&content);
        let rel_path = path
            .strip_prefix(repo_root)
            .expect("strip repo root")
            .to_string_lossy()
            .to_string();
        transcript_cache.insert(rel_path, (frontmatter, turns));
    }

    let vision_path = repo_root.join("docs/vision/vision.md");
    let vision_content = fs::read_to_string(&vision_path).expect("read vision.md");
    let vision_sections = parse_vision_sections(&vision_content);

    let label_paths: Vec<PathBuf> = fs::read_dir(&labels_dir)
        .expect("labels dir exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(".labels.json"))
        })
        .collect();

    assert_eq!(
        label_paths.len(),
        11,
        "Corpus must contain exactly 11 label files (10 transcripts + vision.md), found {}",
        label_paths.len()
    );

    let mut draft_files = Vec::new();
    let mut all_traps: HashMap<TrapType, u32> = HashMap::new();
    let mut boundary_by_negation_count = 0;
    let mut no_boundary_statement_count = 0;
    let mut cross_turn_contradiction_count = 0;

    for path in &label_paths {
        let label_str = fs::read_to_string(path).expect("read label json");
        let label_file: LabelFile = serde_json::from_str(&label_str).unwrap_or_else(|err| {
            panic!(
                "Failed to deserialize label file {}: {}",
                path.display(),
                err
            )
        });

        assert_eq!(
            label_file.schema_version,
            "tks-elicitation-labels/v1",
            "schema_version mismatch in {}",
            path.display()
        );

        if label_file.adjudication.status == AdjudicationStatus::DRAFT {
            draft_files.push(path.file_name().unwrap().to_string_lossy().to_string());
        }

        let source_file_path = repo_root.join(&label_file.source.path);
        let source_bytes = fs::read(&source_file_path).unwrap_or_else(|err| {
            panic!(
                "Source file not found for label {}: {} ({})",
                path.display(),
                source_file_path.display(),
                err
            )
        });
        let mut hasher = Sha256::new();
        hasher.update(&source_bytes);
        let computed_sha256 = format!("{:x}", hasher.finalize());

        if label_file.source.kind == SourceKind::Document {
            if computed_sha256 != label_file.source.sha256 {
                panic!(
                    "source.sha256 mismatch for vision document {}! \
                    Expected (in labels): {}, Actual (file bytes): {}. \
                    Please re-label or re-pin vision.md SHA-256 after editing.",
                    source_file_path.display(),
                    label_file.source.sha256,
                    computed_sha256
                );
            }
        } else {
            assert_eq!(
                computed_sha256,
                label_file.source.sha256,
                "source.sha256 mismatch in {}: expected {}, computed {}",
                path.display(),
                label_file.source.sha256,
                computed_sha256
            );
        }

        match label_file.source.kind {
            SourceKind::Transcript => {
                let (frontmatter, turns) = transcript_cache
                    .get(&label_file.source.path)
                    .unwrap_or_else(|| {
                        panic!("Transcript cache missing path {}", label_file.source.path)
                    });

                let turn_map: HashMap<&str, &Turn> =
                    turns.iter().map(|t| (t.id.as_str(), t)).collect();

                let mut expected_first_turn_idx = 1;
                for ep in &label_file.episodes {
                    assert_eq!(
                        ep.turns.len(),
                        2,
                        "Episode {} in {} must have exactly [first, last] turns",
                        ep.id,
                        path.display()
                    );
                    let first_turn = &ep.turns[0];
                    let last_turn = &ep.turns[1];

                    let first_num: u32 = first_turn[1..]
                        .parse()
                        .expect("valid first turn index in episode");
                    let last_num: u32 = last_turn[1..]
                        .parse()
                        .expect("valid last turn index in episode");

                    assert!(
                        first_num <= last_num,
                        "Episode {} turn range invalid: {} > {}",
                        ep.id,
                        first_num,
                        last_num
                    );
                    assert_eq!(
                        first_num, expected_first_turn_idx,
                        "Episode {} must start immediately after previous episode at turn t{}, found t{}",
                        ep.id, expected_first_turn_idx, first_num
                    );
                    assert!(
                        last_num <= turns.len() as u32,
                        "Episode {} extends beyond transcript length {}",
                        ep.id,
                        turns.len()
                    );

                    expected_first_turn_idx = last_num + 1;
                }
                assert_eq!(
                    expected_first_turn_idx,
                    (turns.len() + 1) as u32,
                    "Episodes in {} do not cover all turns up to t{}",
                    path.display(),
                    turns.len()
                );

                let episode_ids: HashSet<&str> =
                    label_file.episodes.iter().map(|e| e.id.as_str()).collect();
                let goal_ids: HashSet<&str> =
                    label_file.goals.iter().map(|g| g.id.as_str()).collect();

                for goal in &label_file.goals {
                    if let Some(ref ep_id) = goal.episode {
                        assert!(
                            episode_ids.contains(ep_id.as_str()),
                            "Goal {} references unknown episode {} in {}",
                            goal.id,
                            ep_id,
                            path.display()
                        );
                    }

                    for link in &goal.links {
                        assert!(
                            goal_ids.contains(link.target.as_str()),
                            "Goal {} link target {} does not exist in {}",
                            goal.id,
                            link.target,
                            path.display()
                        );
                    }

                    for grounding in &goal.grounding {
                        match grounding {
                            GroundingRef::Transcript { turn, quote } => {
                                let turn_obj = turn_map.get(turn.as_str()).unwrap_or_else(|| {
                                    panic!(
                                        "Goal {} in {} cites non-existent turn {}",
                                        goal.id,
                                        path.display(),
                                        turn
                                    )
                                });
                                let quote_norm = collapse_whitespace(quote);
                                let utter_norm = collapse_whitespace(&turn_obj.utterance);
                                assert!(
                                    utter_norm.contains(&quote_norm),
                                    "Goal {} in {} quote not found in turn {}:\n  Quote: {:?}\n  Utterance: {:?}",
                                    goal.id,
                                    path.display(),
                                    turn,
                                    quote_norm,
                                    utter_norm
                                );
                            }
                            GroundingRef::Document { .. } => {
                                panic!("Document grounding not permitted in transcript labels");
                            }
                        }
                    }
                }

                assert!(
                    label_file.traps.len() >= 3,
                    "Transcript {} has fewer than 3 planted traps ({})",
                    path.display(),
                    label_file.traps.len()
                );
                for trap in &label_file.traps {
                    *all_traps.entry(trap.trap_type).or_insert(0) += 1;
                    if trap.trap_type == TrapType::BOUNDARY_BY_NEGATION {
                        boundary_by_negation_count += 1;
                    }
                    if trap.trap_type == TrapType::CROSS_TURN_CONTRADICTION {
                        cross_turn_contradiction_count += 1;
                    }
                    for t in &trap.turns {
                        assert!(
                            turn_map.contains_key(t.as_str()),
                            "Trap {} in {} cites non-existent turn {}",
                            trap.id,
                            path.display(),
                            t
                        );
                    }
                }

                for gap in &label_file.gaps {
                    if gap.detector == GapDetector::NO_BOUNDARY_STATEMENT {
                        no_boundary_statement_count += 1;
                    }
                    for ref_id in &gap.refs {
                        let is_goal = goal_ids.contains(ref_id.as_str());
                        let is_episode = episode_ids.contains(ref_id.as_str());
                        let is_turn = turn_map.contains_key(ref_id.as_str());
                        let is_participant =
                            frontmatter.participants.iter().any(|p| p.name == *ref_id);
                        assert!(
                            is_goal || is_episode || is_turn || is_participant,
                            "Gap {} in {} references unresolved id {}",
                            gap.id,
                            path.display(),
                            ref_id
                        );
                    }
                }

                assert!(
                    label_file.entailment_cases.len() >= 4,
                    "Transcript {} has fewer than 4 entailment cases ({})",
                    path.display(),
                    label_file.entailment_cases.len()
                );
                let mut has_phantom = false;
                let mut has_reverse_overconstrained = false;
                for ec in &label_file.entailment_cases {
                    if ec.phantom && ec.forward == EntailmentForward::NEUTRAL {
                        has_phantom = true;
                    }
                    if ec.reverse_overconstrained {
                        has_reverse_overconstrained = true;
                    }
                    for pt in &ec.premise_turns {
                        assert!(
                            turn_map.contains_key(pt.as_str()),
                            "Entailment case {} in {} references unknown turn {}",
                            ec.id,
                            path.display(),
                            pt
                        );
                    }
                }
                assert!(
                    has_phantom,
                    "Transcript {} lacks required phantom entailment case",
                    path.display()
                );
                assert!(
                    has_reverse_overconstrained,
                    "Transcript {} lacks required reverse_overconstrained entailment case",
                    path.display()
                );
            }
            SourceKind::Document => {
                assert!(
                    label_file.goals.len() >= 20 && label_file.goals.len() <= 40,
                    "vision.labels.json must contain 20-40 goals, found {}",
                    label_file.goals.len()
                );

                let softgoals = label_file
                    .goals
                    .iter()
                    .filter(|g| g.kind == GoalKind::SOFTGOAL)
                    .count();
                assert!(
                    softgoals >= 5,
                    "vision.labels.json must contain at least 5 SOFTGOAL goals, found {}",
                    softgoals
                );

                let antigoals = label_file
                    .goals
                    .iter()
                    .filter(|g| g.kind == GoalKind::AVOID)
                    .count();
                assert!(
                    antigoals >= 3,
                    "vision.labels.json must contain at least 3 AVOID anti-goals, found {}",
                    antigoals
                );

                for goal in &label_file.goals {
                    for grounding in &goal.grounding {
                        match grounding {
                            GroundingRef::Document { section, quote } => {
                                let sec_content = vision_sections.get(section).unwrap_or_else(|| {
                                    panic!(
                                        "Goal {} cites unknown section {:?} in vision.labels.json. Available sections: {:?}",
                                        goal.id,
                                        section,
                                        vision_sections.keys().collect::<Vec<_>>()
                                    )
                                });
                                let quote_norm = collapse_whitespace(quote);
                                let sec_norm = collapse_whitespace(sec_content);
                                assert!(
                                    sec_norm.contains(&quote_norm),
                                    "Goal {} in vision.labels.json quote not found in section {}:\n  Quote: {:?}\n  Section body: {:?}",
                                    goal.id,
                                    section,
                                    quote_norm,
                                    &sec_norm[..sec_norm.len().min(120)]
                                );
                            }
                            GroundingRef::Transcript { .. } => {
                                panic!("Transcript grounding not permitted in document labels");
                            }
                        }
                    }
                }
            }
        }
    }

    let all_trap_types = [
        TrapType::ANALOGY,
        TrapType::UI_METAPHOR,
        TrapType::RETRACTION,
        TrapType::MULTI_TURN_COREFERENCE,
        TrapType::VENTING_NO_GOAL,
        TrapType::CROSS_TURN_CONTRADICTION,
        TrapType::IMPLICIT_SOFTGOAL,
        TrapType::EXPLICIT_ANTIGOAL,
        TrapType::BOUNDARY_BY_NEGATION,
        TrapType::MISSING_STAKEHOLDER,
        TrapType::FACILITATOR_LEADING,
    ];

    for ttype in all_trap_types {
        let count = all_traps.get(&ttype).copied().unwrap_or(0);
        assert!(
            count >= 2,
            "Trap type {:?} occurs only {} times across corpus (expected >= 2)",
            ttype,
            count
        );
    }

    assert!(
        boundary_by_negation_count >= 2,
        "Expected >= 2 transcripts with BOUNDARY_BY_NEGATION, found {}",
        boundary_by_negation_count
    );
    assert!(
        no_boundary_statement_count >= 1,
        "Expected >= 1 transcript with NO_BOUNDARY_STATEMENT gap, found {}",
        no_boundary_statement_count
    );
    assert!(
        cross_turn_contradiction_count >= 2,
        "Expected >= 2 transcripts with CROSS_TURN_CONTRADICTION, found {}",
        cross_turn_contradiction_count
    );

    if !draft_files.is_empty() {
        eprintln!(
            "\n[EB-1 Human Adjudication Notice] The following {} label files are currently in DRAFT status awaiting human review and adjudication approval:",
            draft_files.len()
        );
        for f in &draft_files {
            eprintln!("  - {}", f);
        }
        eprintln!(
            "Review labels and set `adjudication.status` to 'ADJUDICATED' with `adjudicated_by`.\n"
        );
    }
}
