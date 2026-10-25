//! Deterministic mechanical neutrality linting for facilitator questions (EA-5, §6.6).

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use crate::ingest::elicitation::DEFAULT_TECHNOLOGY_LEXICON;

static PRESUPPOSITION_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        // e.g. "the system you're planning to build", "tool you want to build"
        Regex::new(r"(?i)\b(?:system|tool|app|application|platform|software)\s+(?:you(?:'re|\s+are)?\s+planning\s+to\s+build|you\s+want\s+to\s+build|you\s+plan\s+to\s+build|to\s+be\s+built)\b").unwrap(),
        // e.g. "the new tool", "the new system"
        Regex::new(r"(?i)\bthe\s+new\s+(?:tool|system|app|software|platform|solution)\b").unwrap(),
        // e.g. "system you want", "tool you need"
        Regex::new(r"(?i)\b(?:system|tool|platform|application|solution)\s+you\s+(?:want|need)\b").unwrap(),
        // e.g. "what features should the system have"
        Regex::new(r"(?i)\b(?:what\s+features|what\s+functionality)\s+(?:should|will)\s+the\s+(?:system|tool|app|software)\b").unwrap(),
    ]
});

static CLOSED_OPENERS: &[&str] = &[
    "is ",
    "are ",
    "do ",
    "does ",
    "did ",
    "can ",
    "could ",
    "would ",
    "should ",
    "will ",
    "have ",
    "has ",
    "was ",
    "were ",
    "is it ",
    "can we ",
    "should we ",
];

/// Severity/category of a neutrality finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeutralityFlagCategory {
    Presupposition,
    Technology,
    MultipleQuestions,
    ClosedQuestion,
}

/// A specific neutrality finding on a proposed question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeutralityFlag {
    pub category: NeutralityFlagCategory,
    pub message: String,
    pub terms: Vec<String>,
}

/// Deterministically lints a facilitator proposal for neutrality violations.
///
/// Flags:
/// 1. Solution-presupposing language (e.g. "the system you want to build").
/// 2. Technology mechanisms from the canonical technology lexicon.
/// 3. Multiple question marks (asking more than one question per turn).
/// 4. Closed yes/no question openers (warning only).
#[must_use]
pub fn lint_neutrality(text: &str) -> Vec<NeutralityFlag> {
    let mut flags = Vec::new();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return flags;
    }

    // 1. Solution presupposition check
    let mut matched_presuppositions = Vec::new();
    for pattern in PRESUPPOSITION_PATTERNS.iter() {
        for m in pattern.find_iter(trimmed) {
            matched_presuppositions.push(m.as_str().to_string());
        }
    }
    if !matched_presuppositions.is_empty() {
        flags.push(NeutralityFlag {
            category: NeutralityFlagCategory::Presupposition,
            message: "Presupposes a specific solution, tool, or system to be built".to_string(),
            terms: matched_presuppositions,
        });
    }

    // 2. Technology mechanism term check
    let words: Vec<&str> = trimmed
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .collect();

    let mut tech_terms = Vec::new();
    for word in words {
        let lower = word.to_lowercase();
        if DEFAULT_TECHNOLOGY_LEXICON.contains(&lower.as_str()) && !tech_terms.contains(&lower) {
            tech_terms.push(lower);
        }
    }
    if !tech_terms.is_empty() {
        flags.push(NeutralityFlag {
            category: NeutralityFlagCategory::Technology,
            message: "Contains technical or architectural mechanism terms".to_string(),
            terms: tech_terms,
        });
    }

    // 3. Multiple question marks check
    let question_mark_count = trimmed.chars().filter(|&c| c == '?').count();
    if question_mark_count > 1 {
        flags.push(NeutralityFlag {
            category: NeutralityFlagCategory::MultipleQuestions,
            message: format!(
                "Contains {question_mark_count} question marks (expected exactly one open question)"
            ),
            terms: vec![format!("{question_mark_count} questions")],
        });
    }

    // 4. Closed yes/no question opener check (warning)
    let lower_trimmed = trimmed.to_lowercase();
    for opener in CLOSED_OPENERS {
        if lower_trimmed.starts_with(opener) {
            flags.push(NeutralityFlag {
                category: NeutralityFlagCategory::ClosedQuestion,
                message: "Begins with a closed yes/no question opener".to_string(),
                terms: vec![opener.trim().to_string()],
            });
            break;
        }
    }

    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_neutral_question_passes() {
        let text =
            "What happens when an engineer needs to deploy a change across multiple environments?";
        let flags = lint_neutrality(text);
        assert!(
            flags.is_empty(),
            "expected neutral question to have no flags: {flags:?}"
        );
    }

    #[test]
    fn test_presupposition_flagged() {
        let text = "What is the primary goal of the system you're planning to build?";
        let flags = lint_neutrality(text);
        assert!(
            flags
                .iter()
                .any(|f| f.category == NeutralityFlagCategory::Presupposition)
        );
    }

    #[test]
    fn test_new_tool_presupposition() {
        let text = "How will the new tool help team members coordinate their work?";
        let flags = lint_neutrality(text);
        assert!(
            flags
                .iter()
                .any(|f| f.category == NeutralityFlagCategory::Presupposition)
        );
    }

    #[test]
    fn test_technology_terms_flagged() {
        let text = "What database schema or API endpoints are required for this workflow?";
        let flags = lint_neutrality(text);
        let tech = flags
            .iter()
            .find(|f| f.category == NeutralityFlagCategory::Technology);
        assert!(tech.is_some());
        let terms = &tech.unwrap().terms;
        assert!(terms.contains(&"database".to_string()));
        assert!(terms.contains(&"api".to_string()));
    }

    #[test]
    fn test_multiple_questions_flagged() {
        let text =
            "Who are the primary users? And what challenges do they experience most frequently?";
        let flags = lint_neutrality(text);
        assert!(
            flags
                .iter()
                .any(|f| f.category == NeutralityFlagCategory::MultipleQuestions)
        );
    }

    #[test]
    fn test_closed_opener_warning() {
        let text = "Is this issue currently affecting customers during peak hours?";
        let flags = lint_neutrality(text);
        assert!(
            flags
                .iter()
                .any(|f| f.category == NeutralityFlagCategory::ClosedQuestion)
        );
    }
}
