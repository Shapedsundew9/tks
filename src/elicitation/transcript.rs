//! Transcript rendering, export validation, and E-B interoperability (EA-4, §6.5).

use serde::{Deserialize, Serialize};

use super::session::{ElicitationSession, validate_session_slug};
use crate::ingest::transcript::ParsedTranscript;

/// Detailed validation error preventing a session from closing or exporting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseValidationError {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn: Option<u32>,
    pub message: String,
}

/// Escapes a string to be safely embedded inside double quotes in YAML.
fn escape_yaml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Renders an in-memory elicitation session as a canonical `tks-transcript/v1` Markdown document.
#[must_use]
pub fn render_transcript(session: &ElicitationSession) -> String {
    let mut out = String::new();
    let source = session.compute_source();

    out.push_str("---\n");
    out.push_str("format: tks-transcript/v1\n");
    out.push_str(&format!(
        "title: \"{}\"\n",
        escape_yaml_string(&session.title)
    ));
    out.push_str(&format!(
        "session_id: \"{}\"\n",
        escape_yaml_string(&session.session_id)
    ));
    out.push_str(&format!("cycle: {}\n", session.cycle));
    out.push_str(&format!(
        "date: \"{}\"\n",
        escape_yaml_string(&session.date)
    ));
    out.push_str(&format!(
        "language: \"{}\"\n",
        escape_yaml_string(&session.language)
    ));
    out.push_str(&format!("source: \"{source}\"\n"));
    out.push_str("participants:\n");
    for p in &session.participants {
        out.push_str(&format!(
            "  - {{ name: \"{}\", role: \"{}\" }}\n",
            escape_yaml_string(&p.name),
            escape_yaml_string(&p.role)
        ));
    }
    out.push_str("---\n\n");

    for (idx, turn) in session.turns.iter().enumerate() {
        let role_str = turn.role.to_string();
        let speaker_name = session
            .participants
            .iter()
            .find(|p| p.role == role_str)
            .map(|p| p.name.as_str())
            .unwrap_or(if turn.role == super::session::TurnRole::Stakeholder {
                "Stakeholder"
            } else {
                "Facilitator"
            });

        out.push_str(&format!(
            "**[t{}] {} ({}):** {}",
            turn.number, speaker_name, role_str, turn.text
        ));

        if idx + 1 < session.turns.len() {
            out.push_str("\n\n");
        } else {
            out.push('\n');
        }
    }

    out
}

/// Validates that a session satisfies all export rules (§6.5) before closing.
///
/// Checks in order:
/// 1. At least one turn.
/// 2. No uncleared uncertainty flags.
/// 3. Frontmatter self-checks (slug, cycle >= 1, ISO date, language, source).
/// 4. `ParsedTranscript::parse` succeeds on rendered text.
/// 5. Every rendered utterance matches stored turn text byte-for-byte.
pub fn validate_transcript_for_close(
    session: &ElicitationSession,
) -> Result<(), Vec<CloseValidationError>> {
    let mut errors = Vec::new();

    // 1. At least one turn
    if session.turns.is_empty() {
        errors.push(CloseValidationError {
            code: "no_turns".to_string(),
            turn: None,
            message: "session must contain at least one committed turn before closing".to_string(),
        });
    }

    // 2. Uncleared uncertainty flags
    for turn in &session.turns {
        if !turn.uncertain_flags.is_empty() {
            errors.push(CloseValidationError {
                code: "uncertain_flags".to_string(),
                turn: Some(turn.number),
                message: format!(
                    "turn t{} has {} uncleared uncertainty flag(s) that must be resolved or cleared",
                    turn.number,
                    turn.uncertain_flags.len()
                ),
            });
        }
    }

    // 3. Frontmatter self-checks
    if !validate_session_slug(&session.session_id) {
        errors.push(CloseValidationError {
            code: "invalid_session_id".to_string(),
            turn: None,
            message: format!("session_id '{}' is not a valid slug", session.session_id),
        });
    }
    if session.cycle < 1 {
        errors.push(CloseValidationError {
            code: "invalid_cycle".to_string(),
            turn: None,
            message: format!("cycle {} must be >= 1", session.cycle),
        });
    }
    if session.title.trim().is_empty() {
        errors.push(CloseValidationError {
            code: "empty_title".to_string(),
            turn: None,
            message: "session title cannot be empty".to_string(),
        });
    }
    if session.language != "en" {
        errors.push(CloseValidationError {
            code: "invalid_language".to_string(),
            turn: None,
            message: format!(
                "language '{}' is unsupported; expected 'en'",
                session.language
            ),
        });
    }
    let source = session.compute_source();
    if source != "manual" && source != "asr" {
        errors.push(CloseValidationError {
            code: "invalid_source".to_string(),
            turn: None,
            message: format!("source '{source}' is invalid; expected 'manual' or 'asr'"),
        });
    }
    // Date format YYYY-MM-DD
    if chrono::NaiveDate::parse_from_str(&session.date, "%Y-%m-%d").is_err() {
        errors.push(CloseValidationError {
            code: "invalid_date".to_string(),
            turn: None,
            message: format!("date '{}' is not formatted as YYYY-MM-DD", session.date),
        });
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    // 4. Render and parse check
    let rendered = render_transcript(session);
    let parsed = match ParsedTranscript::parse(&rendered) {
        Ok(p) => p,
        Err(e) => {
            errors.push(CloseValidationError {
                code: "parser_failure".to_string(),
                turn: None,
                message: format!("rendered transcript failed parser validation: {e}"),
            });
            return Err(errors);
        }
    };

    // 5. Utterance byte-exact match
    if parsed.turns.len() != session.turns.len() {
        errors.push(CloseValidationError {
            code: "turn_count_mismatch".to_string(),
            turn: None,
            message: format!(
                "rendered turn count {} does not match stored turns {}",
                parsed.turns.len(),
                session.turns.len()
            ),
        });
        return Err(errors);
    }

    for (stored, parsed_turn) in session.turns.iter().zip(parsed.turns.iter()) {
        if stored.text != parsed_turn.utterance_text {
            errors.push(CloseValidationError {
                code: "utterance_mismatch".to_string(),
                turn: Some(stored.number),
                message: format!(
                    "rendered utterance in turn t{} differs from stored text",
                    stored.number
                ),
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
