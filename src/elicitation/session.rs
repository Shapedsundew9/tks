//! In-memory elicitation session and turn models (D-130..D-134, D-EA-7..D-EA-15).

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use uuid::Uuid;

static SLUG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$").expect("valid slug regex"));

/// Role of a dialogue participant in an elicitation session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnRole {
    Stakeholder,
    Facilitator,
}

impl std::fmt::Display for TurnRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stakeholder => write!(f, "stakeholder"),
            Self::Facilitator => write!(f, "facilitator"),
        }
    }
}

/// Input modality through which a turn was entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputMode {
    Typed,
    Dictated,
}

impl std::fmt::Display for InputMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Typed => write!(f, "typed"),
            Self::Dictated => write!(f, "dictated"),
        }
    }
}

/// Lifecycle state of a live draft session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Open,
    Closed,
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => write!(f, "open"),
            Self::Closed => write!(f, "closed"),
        }
    }
}

/// An uncertainty flag highlighting a span within turn text (D-EA-15).
/// Never exported in transcripts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UncertainFlag {
    pub start: usize,
    pub end: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Registered participant in an elicitation session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElicitationParticipant {
    pub name: String,
    pub role: String,
}

/// A conversational turn in an elicitation session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElicitationTurn {
    pub number: u32,
    pub role: TurnRole,
    pub text: String,
    pub input_mode: InputMode,
    #[serde(default)]
    pub uncertain_flags: Vec<UncertainFlag>,
    pub committed_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<DateTime<Utc>>,
}

/// A question imported from an E-B gap analysis report for facilitator context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedGapQuestion {
    pub gap_id: String,
    pub detector: String,
    pub suggested_question: String,
}

/// Live draft elicitation session state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElicitationSession {
    pub elicitation_id: Uuid,
    pub state: SessionState,
    pub title: String,
    pub session_id: String,
    pub cycle: u32,
    pub date: String,
    pub language: String,
    pub participants: Vec<ElicitationParticipant>,
    pub turns: Vec<ElicitationTurn>,
    #[serde(default)]
    pub imported_gap_questions: Vec<ImportedGapQuestion>,
    #[serde(default)]
    pub explored_items: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub source: String,
}

impl ElicitationSession {
    /// Computes the transcript-level provenance (D-EA-7):
    /// `asr` if any committed turn was dictated, otherwise `manual`.
    #[must_use]
    pub fn compute_source(&self) -> &'static str {
        if self
            .turns
            .iter()
            .any(|t| t.input_mode == InputMode::Dictated)
        {
            "asr"
        } else {
            "manual"
        }
    }
}

/// Validates a participant display name (D-EA-9, §6.2).
///
/// Participant names must be non-empty, at most 64 characters, distinct, and
/// contain none of `(`, `)`, `[`, `]`, `*`, or newlines.
pub fn validate_participant_name(name: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() {
        return Err("participant name cannot be empty");
    }
    if name.chars().count() > 64 {
        return Err("participant name exceeds 64 characters");
    }
    for ch in name.chars() {
        if matches!(ch, '(' | ')' | '[' | ']' | '*' | '\r' | '\n') {
            return Err(
                "participant name contains invalid characters: '(', ')', '[', ']', '*', or newline",
            );
        }
    }
    Ok(())
}

/// Validates a `session_id` slug (`^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`).
#[must_use]
pub fn validate_session_slug(slug: &str) -> bool {
    SLUG_RE.is_match(slug)
}

/// Normalises turn text at submission time (§6.2):
/// - CR/LF and tabs become single spaces.
/// - C0/C1 control characters are stripped.
/// - Runs of whitespace collapse to single spaces.
/// - Leading and trailing whitespace is trimmed.
/// - Empty text after normalisation is rejected.
/// - Text exceeding 16 KiB (16,384 bytes) is rejected.
pub fn normalize_turn_text(input: &str) -> Result<String, &'static str> {
    if input.len() > 16 * 1024 {
        return Err("turn text exceeds maximum length of 16 KiB");
    }

    let mut buf = String::with_capacity(input.len());
    let mut last_was_space = true; // trims leading whitespace naturally

    for ch in input.chars() {
        let is_space_like = ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r';
        if is_space_like {
            if !last_was_space {
                buf.push(' ');
                last_was_space = true;
            }
        } else if !ch.is_control() {
            buf.push(ch);
            last_was_space = false;
        }
    }

    let trimmed = buf.trim_end();
    if trimmed.is_empty() {
        return Err("turn text cannot be empty after normalisation");
    }

    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_participant_name() {
        assert!(validate_participant_name("Priya").is_ok());
        assert!(validate_participant_name("Dr. Alex Carter").is_ok());
        assert!(validate_participant_name("").is_err());
        assert!(validate_participant_name("   ").is_err());
        assert!(validate_participant_name("Priya (Lead)").is_err());
        assert!(validate_participant_name("Bob [Dev]").is_err());
        assert!(validate_participant_name("Alice *").is_err());
        assert!(validate_participant_name("Alice\nBob").is_err());
        assert!(validate_participant_name(&"a".repeat(65)).is_err());
        assert!(validate_participant_name(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn test_validate_session_slug() {
        assert!(validate_session_slug("T01-capacity"));
        assert!(validate_session_slug("alpha_beta_1"));
        assert!(validate_session_slug("a"));
        assert!(!validate_session_slug("-invalid-start"));
        assert!(!validate_session_slug("_invalid-start"));
        assert!(!validate_session_slug(""));
        assert!(!validate_session_slug("has space"));
        assert!(!validate_session_slug("has/slash"));
        assert!(validate_session_slug(&"a".repeat(64)));
        assert!(!validate_session_slug(&"a".repeat(65)));
    }

    #[test]
    fn test_normalize_turn_text() {
        let raw = "  Hello   world!\r\n\tThis is   a \x00test.\n\n  ";
        let norm = normalize_turn_text(raw).expect("valid normalized text");
        assert_eq!(norm, "Hello world! This is a test.");

        assert!(normalize_turn_text("   \n\t  ").is_err());
        let huge = "a".repeat(16 * 1024 + 1);
        assert!(normalize_turn_text(&huge).is_err());
    }
}
