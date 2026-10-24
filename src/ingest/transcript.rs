//! Normative transcript and vision document parser (`tks-transcript/v1`).
//!
//! Provides byte-exact decomposition of conversational elicitation transcripts
//! and CommonMark vision documents into typed turns and [`SourceExcerpt`] records.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::LazyLock;

use crate::ingest::parser::{ParseError, parse_markdown};
use crate::ingest::span::slice_source_span;
use crate::ingest::{IngestError, SpanError};

static TURN_HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\*\*\[t(?P<num>\d+)\]\s+(?P<name>[^\(]+?)\s+\((?P<role>[^\)]+)\):\*\*\s*(?:_\((?P<ts>\d{2}:\d{2}:\d{2})\)_\s+)?",
    )
    .expect("valid turn header regex")
});

/// Allowed participant and speaker roles in `tks-transcript/v1`.
pub const ALLOWED_ROLES: &[&str] = &["stakeholder", "facilitator", "agent"];

/// Metadata declared in the transcript's opening YAML frontmatter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptFrontmatter {
    /// Schema format identifier; must be `tks-transcript/v1`.
    pub format: String,
    /// Session title or high-level problem statement.
    pub title: String,
    /// Unique identifier for this elicitation session within the project.
    pub session_id: String,
    /// Elicitation cycle number (1-based integer).
    pub cycle: u32,
    /// Date of conversation recording or synthesis (`YYYY-MM-DD`).
    pub date: String,
    /// Primary ISO language code (e.g. `en`).
    pub language: String,
    /// Provenance origin of transcript (`synthetic`, `manual`, `asr`).
    pub source: String,
    /// Registered participants and their respective roles.
    pub participants: Vec<Participant>,
}

/// A registered speaker participating in an elicitation conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Participant {
    /// Display name of the participant.
    pub name: String,
    /// Assigned conversational role (`stakeholder`, `facilitator`, or `agent`).
    pub role: String,
}

/// A parsed single conversational turn carrying byte-exact offsets into the source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedTurn {
    /// Turn identifier alias (`t1`, `t2`, ...).
    pub id: String,
    /// 1-based sequential turn number.
    pub number: u32,
    /// Participant display name.
    pub name: String,
    /// Participant role in this turn.
    pub role: String,
    /// Optional timestamp string (`HH:MM:SS`) if present in source.
    pub timestamp: Option<String>,
    /// 0-based byte offset where the entire turn block begins in the source.
    pub block_byte_start: usize,
    /// 0-based byte offset where the entire turn block ends in the source.
    pub block_byte_end: usize,
    /// 0-based byte offset where the utterance text begins in the source.
    pub utterance_byte_start: usize,
    /// 0-based byte offset where the utterance text ends in the source.
    pub utterance_byte_end: usize,
    /// Full text of the turn block (including speaker tag and utterance).
    pub block_text: String,
    /// Verbatim utterance text.
    pub utterance_text: String,
}

/// A fully parsed elicitation transcript comprising frontmatter and contiguous turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedTranscript {
    /// Parsed frontmatter metadata.
    pub frontmatter: TranscriptFrontmatter,
    /// Contiguous, 1-based turns.
    pub turns: Vec<ParsedTurn>,
}

/// A source excerpt candidate for the knowledge substrate (`SOURCE_EXCERPT`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceExcerpt {
    /// Deterministic excerpt identifier, e.g. `T01:t1` or `vision:c1`.
    pub excerpt_id: String,
    /// Relative document path within the repository or project.
    pub doc_path: String,
    /// Cryptographic SHA-256 hash of the parent document.
    pub doc_sha256: String,
    /// Ordinal alias (`t1`..`tN` for transcripts, `c1`..`cN` for vision documents).
    pub alias: String,
    /// Speaker name if extracted from a transcript turn.
    pub speaker: Option<String>,
    /// Speaker role if extracted from a transcript turn.
    pub role: Option<String>,
    /// Section heading if extracted from a structured vision document.
    pub heading: Option<String>,
    /// 0-based byte start in source document.
    pub byte_start: usize,
    /// 0-based byte end in source document.
    pub byte_end: usize,
    /// Verbatim sliced content matching `[byte_start..byte_end]` exactly.
    pub content: String,
}

/// Errors that can occur when parsing a transcript or vision document.
#[derive(Debug)]
pub enum TranscriptParseError {
    /// Missing or unclosed YAML frontmatter delimiter (`---`).
    MissingFrontmatter,
    /// Invalid frontmatter schema format version.
    InvalidFormatVersion { expected: String, got: String },
    /// Frontmatter YAML deserialization error.
    Yaml(serde_yaml::Error),
    /// Invalid participant role declared.
    InvalidRole { name: String, role: String },
    /// Non-turn paragraph found where a turn block was expected.
    MalformedTurnParagraph {
        paragraph_index: usize,
        byte_start: usize,
        content: String,
    },
    /// Turn numbers are non-contiguous or do not start at 1.
    NonContiguousTurn { expected: u32, got: u32 },
    /// Turn speaker is not registered in frontmatter participants.
    UnknownSpeaker { name: String, turn: u32 },
    /// Speaker role does not match frontmatter registration.
    RoleMismatch {
        name: String,
        expected: String,
        got: String,
        turn: u32,
    },
    /// Transcript must contain at least one turn.
    NoTurns,
    /// Byte span slicing failure.
    Span(SpanError),
}

impl fmt::Display for TranscriptParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFrontmatter => write!(f, "missing or unclosed YAML frontmatter ('---')"),
            Self::InvalidFormatVersion { expected, got } => {
                write!(
                    f,
                    "invalid transcript format version: expected '{expected}', got '{got}'"
                )
            }
            Self::Yaml(err) => write!(f, "YAML frontmatter parse error: {err}"),
            Self::InvalidRole { name, role } => {
                write!(
                    f,
                    "invalid role '{role}' for participant '{name}' (allowed: {ALLOWED_ROLES:?})"
                )
            }
            Self::MalformedTurnParagraph {
                paragraph_index,
                byte_start,
                content,
            } => {
                write!(
                    f,
                    "malformed turn block at paragraph {paragraph_index} (byte {byte_start}): expected '**[tN] Name (role):**', found {:?}",
                    content.chars().take(60).collect::<String>()
                )
            }
            Self::NonContiguousTurn { expected, got } => {
                write!(
                    f,
                    "non-contiguous turn numbering: expected t{expected}, got t{got}"
                )
            }
            Self::UnknownSpeaker { name, turn } => {
                write!(f, "turn t{turn} cites unregistered speaker '{name}'")
            }
            Self::RoleMismatch {
                name,
                expected,
                got,
                turn,
            } => {
                write!(
                    f,
                    "role mismatch in turn t{turn} for '{name}': frontmatter declared '{expected}', turn specifies '{got}'"
                )
            }
            Self::NoTurns => write!(f, "transcript must contain at least one turn (found 0)"),
            Self::Span(err) => write!(f, "byte span error: {err}"),
        }
    }
}

impl std::error::Error for TranscriptParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Yaml(err) => Some(err),
            Self::Span(err) => Some(err),
            _ => None,
        }
    }
}

impl From<SpanError> for TranscriptParseError {
    fn from(err: SpanError) -> Self {
        Self::Span(err)
    }
}

impl From<TranscriptParseError> for IngestError {
    fn from(err: TranscriptParseError) -> Self {
        Self::Parse(ParseError::InvalidStructure(err.to_string()))
    }
}

impl ParsedTranscript {
    /// Parses a UTF-8 string formatted according to `tks-transcript/v1`.
    ///
    /// Computes byte-exact start and end offsets for all turn blocks and their
    /// respective utterance bodies, asserting contiguous 1-based turn identifiers.
    ///
    /// # Errors
    /// Returns [`TranscriptParseError`] if frontmatter is invalid, turn headers
    /// are malformed, turn numbering has gaps, or speakers are unregistered.
    pub fn parse(content: &str) -> Result<Self, TranscriptParseError> {
        let (frontmatter, body_start_offset) = parse_frontmatter(content)?;

        let mut turns = Vec::new();
        let mut expected_turn_num = 1;

        let mut offset = body_start_offset;
        let content_bytes = content.as_bytes();
        let len = content_bytes.len();

        let mut paragraph_idx = 0;

        while offset < len {
            // Skip leading whitespace / newlines between paragraphs
            while offset < len
                && (content_bytes[offset] == b'\n'
                    || content_bytes[offset] == b'\r'
                    || content_bytes[offset] == b' ')
            {
                offset += 1;
            }
            if offset >= len {
                break;
            }

            paragraph_idx += 1;
            let block_byte_start = offset;

            // Find end of paragraph (double newline or end of file)
            let mut block_byte_end = offset;
            while block_byte_end < len {
                if content_bytes[block_byte_end] == b'\n' {
                    let next = block_byte_end + 1;
                    if next < len
                        && (content_bytes[next] == b'\n'
                            || (content_bytes[next] == b'\r'
                                && next + 1 < len
                                && content_bytes[next + 1] == b'\n'))
                    {
                        break;
                    }
                }
                block_byte_end += 1;
            }

            let block_slice =
                slice_source_span(content.as_bytes(), block_byte_start, block_byte_end)?;
            let block_trimmed = block_slice.trim_end();
            let effective_block_byte_end = block_byte_start + block_trimmed.len();

            let caps = TURN_HEADER_RE.captures(block_trimmed).ok_or_else(|| {
                TranscriptParseError::MalformedTurnParagraph {
                    paragraph_index: paragraph_idx,
                    byte_start: block_byte_start,
                    content: block_trimmed.to_string(),
                }
            })?;

            let whole_match = caps.get(0).expect("whole header match");
            let num: u32 =
                caps["num"]
                    .parse()
                    .map_err(|_| TranscriptParseError::MalformedTurnParagraph {
                        paragraph_index: paragraph_idx,
                        byte_start: block_byte_start,
                        content: block_trimmed.to_string(),
                    })?;

            if num != expected_turn_num {
                return Err(TranscriptParseError::NonContiguousTurn {
                    expected: expected_turn_num,
                    got: num,
                });
            }
            expected_turn_num += 1;

            let name = caps["name"].trim().to_string();
            let role = caps["role"].trim().to_string();
            let timestamp = caps.name("ts").map(|m| m.as_str().to_string());

            let participant = frontmatter
                .participants
                .iter()
                .find(|p| p.name == name)
                .ok_or_else(|| TranscriptParseError::UnknownSpeaker {
                    name: name.clone(),
                    turn: num,
                })?;

            if participant.role != role {
                return Err(TranscriptParseError::RoleMismatch {
                    name: name.clone(),
                    expected: participant.role.clone(),
                    got: role.clone(),
                    turn: num,
                });
            }

            let header_match_len = whole_match.end();
            let utterance_byte_start = block_byte_start + header_match_len;
            let utterance_byte_end = effective_block_byte_end;

            let raw_utterance =
                slice_source_span(content.as_bytes(), utterance_byte_start, utterance_byte_end)?;
            let trimmed_utterance = raw_utterance.trim();
            // Refine utterance byte offsets to trim surrounding whitespace
            let leading_ws = raw_utterance.len() - raw_utterance.trim_start().len();
            let trailing_ws = raw_utterance.len() - raw_utterance.trim_end().len();
            let actual_utterance_start = utterance_byte_start + leading_ws;
            let actual_utterance_end = utterance_byte_end.saturating_sub(trailing_ws);

            turns.push(ParsedTurn {
                id: format!("t{num}"),
                number: num,
                name,
                role,
                timestamp,
                block_byte_start,
                block_byte_end: effective_block_byte_end,
                utterance_byte_start: actual_utterance_start,
                utterance_byte_end: actual_utterance_end,
                block_text: block_trimmed.to_string(),
                utterance_text: trimmed_utterance.to_string(),
            });

            offset = block_byte_end;
        }

        if turns.is_empty() {
            return Err(TranscriptParseError::NoTurns);
        }

        Ok(Self { frontmatter, turns })
    }

    /// Emits a list of [`SourceExcerpt`] records representing each conversational turn.
    ///
    /// Every excerpt is keyed by `format!("{}:{}", session_id, turn_id)` and
    /// points to the exact byte offsets of the turn's utterance.
    #[must_use]
    pub fn to_source_excerpts(&self, doc_path: &str, doc_sha256: &str) -> Vec<SourceExcerpt> {
        self.turns
            .iter()
            .map(|turn| SourceExcerpt {
                excerpt_id: format!("{}:{}", self.frontmatter.session_id, turn.id),
                doc_path: doc_path.to_string(),
                doc_sha256: doc_sha256.to_string(),
                alias: turn.id.clone(),
                speaker: Some(turn.name.clone()),
                role: Some(turn.role.clone()),
                heading: None,
                byte_start: turn.utterance_byte_start,
                byte_end: turn.utterance_byte_end,
                content: turn.utterance_text.clone(),
            })
            .collect()
    }
}

fn parse_frontmatter(
    content: &str,
) -> Result<(TranscriptFrontmatter, usize), TranscriptParseError> {
    let trimmed_start = content.trim_start();
    if !trimmed_start.starts_with("---") {
        return Err(TranscriptParseError::MissingFrontmatter);
    }

    let start_dashes_idx = content.find("---").expect("checked starts_with");
    let after_first_dashes = start_dashes_idx + 3;

    let closing_dashes_idx = content[after_first_dashes..]
        .find("\n---")
        .map(|idx| after_first_dashes + idx + 1)
        .ok_or(TranscriptParseError::MissingFrontmatter)?;

    let yaml_str = &content[after_first_dashes..closing_dashes_idx].trim();
    let frontmatter: TranscriptFrontmatter =
        serde_yaml::from_str(yaml_str).map_err(TranscriptParseError::Yaml)?;

    if frontmatter.format != "tks-transcript/v1" {
        return Err(TranscriptParseError::InvalidFormatVersion {
            expected: "tks-transcript/v1".to_string(),
            got: frontmatter.format,
        });
    }

    for p in &frontmatter.participants {
        if !ALLOWED_ROLES.contains(&p.role.as_str()) {
            return Err(TranscriptParseError::InvalidRole {
                name: p.name.clone(),
                role: p.role.clone(),
            });
        }
    }

    let end_of_frontmatter = closing_dashes_idx + 3;
    Ok((frontmatter, end_of_frontmatter))
}

/// Decomposes a free-form vision Markdown document (such as `docs/vision/vision.md`)
/// into byte-exact [`SourceExcerpt`] nodes using CommonMark Stage 1 atomization.
///
/// Each sentence chunk is assigned a sequential alias (`c1`, `c2`, ...) and
/// records its enclosing section heading and exact byte span.
///
/// # Errors
/// Returns [`IngestError`] if AST decomposition or span slicing fails.
pub fn parse_vision_doc_excerpts(
    content: &str,
    doc_path: &str,
    doc_sha256: &str,
) -> Result<Vec<SourceExcerpt>, IngestError> {
    let decomp = parse_markdown(doc_path, content)?;

    let mut excerpts = Vec::new();

    for (alias_counter, chunk) in (1..).zip(decomp.chunks) {
        let span_slice = slice_source_span(content.as_bytes(), chunk.byte_start, chunk.byte_end)?;
        let alias = format!("c{alias_counter}");
        let excerpt_id = format!("{doc_path}:{alias}");

        excerpts.push(SourceExcerpt {
            excerpt_id,
            doc_path: doc_path.to_string(),
            doc_sha256: doc_sha256.to_string(),
            alias,
            speaker: None,
            role: None,
            heading: chunk.heading,
            byte_start: chunk.byte_start,
            byte_end: chunk.byte_end,
            content: span_slice.to_string(),
        });
    }

    Ok(excerpts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn test_parse_valid_synthetic_corpus() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let transcripts_dir = repo_root.join("tests/fixtures/elicitation/transcripts");

        let entries = fs::read_dir(transcripts_dir).expect("read transcripts dir");
        let mut count = 0;

        for entry in entries {
            let path = entry.expect("valid entry").path();
            if path.extension().is_some_and(|ext| ext == "md") {
                let content = fs::read_to_string(&path).expect("read file");
                let parsed = ParsedTranscript::parse(&content)
                    .unwrap_or_else(|err| panic!("Failed to parse {}: {}", path.display(), err));

                assert_eq!(parsed.frontmatter.format, "tks-transcript/v1");
                assert!(!parsed.turns.is_empty());

                // Byte-exact slicing assertions:
                for turn in &parsed.turns {
                    let sliced_block = &content[turn.block_byte_start..turn.block_byte_end];
                    assert_eq!(
                        sliced_block,
                        turn.block_text,
                        "Block span mismatch in turn {} of {}",
                        turn.id,
                        path.display()
                    );

                    let sliced_utterance =
                        &content[turn.utterance_byte_start..turn.utterance_byte_end];
                    assert_eq!(
                        sliced_utterance,
                        turn.utterance_text,
                        "Utterance span mismatch in turn {} of {}",
                        turn.id,
                        path.display()
                    );
                }

                // Check SourceExcerpt conversion
                let excerpts = parsed.to_source_excerpts("doc.md", "mock-sha");
                assert_eq!(excerpts.len(), parsed.turns.len());
                for (exc, turn) in excerpts.iter().zip(&parsed.turns) {
                    assert_eq!(exc.alias, turn.id);
                    assert_eq!(exc.content, turn.utterance_text);
                    assert_eq!(&content[exc.byte_start..exc.byte_end], exc.content);
                }

                count += 1;
            }
        }

        assert_eq!(count, 10, "Expected all 10 synthetic transcripts to parse");
    }

    #[test]
    fn test_vision_document_excerpts_byte_exact() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let vision_path = repo_root.join("docs/vision/vision.md");
        let content = fs::read_to_string(&vision_path).expect("read vision.md");

        let excerpts = parse_vision_doc_excerpts(&content, "docs/vision/vision.md", "mock_sha")
            .expect("parse vision doc excerpts");

        assert!(
            excerpts.len() >= 50,
            "Expected vision document to produce >= 50 excerpts, got {}",
            excerpts.len()
        );

        // Verify byte-exact slicing for every excerpt
        for exc in &excerpts {
            let sliced = &content[exc.byte_start..exc.byte_end];
            assert_eq!(
                sliced, exc.content,
                "Excerpt {} sliced span mismatch in vision.md",
                exc.alias
            );
        }
    }

    #[test]
    fn test_rejects_missing_frontmatter() {
        let input = "**[t1] Priya (stakeholder):** Hello world.";
        let err = ParsedTranscript::parse(input).unwrap_err();
        assert!(matches!(err, TranscriptParseError::MissingFrontmatter));
    }

    #[test]
    fn test_rejects_invalid_format_version() {
        let input = r#"---
format: tks-transcript/v2
title: "Test"
session_id: "T99"
cycle: 1
date: "2026-10-12"
language: "en"
source: "synthetic"
participants:
  - { name: "Priya", role: "stakeholder" }
---

**[t1] Priya (stakeholder):** Hello.
"#;
        let err = ParsedTranscript::parse(input).unwrap_err();
        assert!(matches!(
            err,
            TranscriptParseError::InvalidFormatVersion { .. }
        ));
    }

    #[test]
    fn test_rejects_non_contiguous_turn_gap() {
        let input = r#"---
format: tks-transcript/v1
title: "Test"
session_id: "T99"
cycle: 1
date: "2026-10-12"
language: "en"
source: "synthetic"
participants:
  - { name: "Priya", role: "stakeholder" }
---

**[t1] Priya (stakeholder):** First turn.

**[t3] Priya (stakeholder):** Gap in turns!
"#;
        let err = ParsedTranscript::parse(input).unwrap_err();
        assert!(matches!(
            err,
            TranscriptParseError::NonContiguousTurn {
                expected: 2,
                got: 3
            }
        ));
    }

    #[test]
    fn test_rejects_unregistered_speaker() {
        let input = r#"---
format: tks-transcript/v1
title: "Test"
session_id: "T99"
cycle: 1
date: "2026-10-12"
language: "en"
source: "synthetic"
participants:
  - { name: "Priya", role: "stakeholder" }
---

**[t1] Bob (facilitator):** Unregistered speaker.
"#;
        let err = ParsedTranscript::parse(input).unwrap_err();
        assert!(matches!(err, TranscriptParseError::UnknownSpeaker { .. }));
    }

    #[test]
    fn test_rejects_speaker_role_mismatch() {
        let input = r#"---
format: tks-transcript/v1
title: "Test"
session_id: "T99"
cycle: 1
date: "2026-10-12"
language: "en"
source: "synthetic"
participants:
  - { name: "Priya", role: "stakeholder" }
---

**[t1] Priya (facilitator):** Wrong role.
"#;
        let err = ParsedTranscript::parse(input).unwrap_err();
        assert!(matches!(err, TranscriptParseError::RoleMismatch { .. }));
    }

    #[test]
    fn test_rejects_zero_turn_transcript() {
        let input = r#"---
format: tks-transcript/v1
title: "Test"
session_id: "T99"
cycle: 1
date: "2026-10-12"
language: "en"
source: "manual"
participants:
  - { name: "Priya", role: "stakeholder" }
---
"#;
        let err = ParsedTranscript::parse(input).unwrap_err();
        assert!(matches!(err, TranscriptParseError::NoTurns));
    }

    #[test]
    fn test_accepts_single_turn_transcript() {
        let input = r#"---
format: tks-transcript/v1
title: "Single Turn Test"
session_id: "T99"
cycle: 1
date: "2026-10-12"
language: "en"
source: "manual"
participants:
  - { name: "Priya", role: "stakeholder" }
---

**[t1] Priya (stakeholder):** Hello world this is the only turn.
"#;
        let parsed =
            ParsedTranscript::parse(input).expect("single-turn transcript should be valid");
        assert_eq!(parsed.turns.len(), 1);
        assert_eq!(
            parsed.turns[0].utterance_text,
            "Hello world this is the only turn."
        );
    }

    #[test]
    fn test_accepts_100_turn_transcript() {
        let mut input = String::from(
            r#"---
format: tks-transcript/v1
title: "100 Turn Test"
session_id: "T100"
cycle: 1
date: "2026-10-24"
language: "en"
source: "manual"
participants:
  - { name: "Priya", role: "stakeholder" }
  - { name: "Facilitator", role: "facilitator" }
---

"#,
        );
        for i in 1..=100 {
            let (name, role) = if i % 2 == 1 {
                ("Priya", "stakeholder")
            } else {
                ("Facilitator", "facilitator")
            };
            input.push_str(&format!(
                "**[t{i}] {name} ({role}):** This is turn number {i}.\n\n"
            ));
        }

        let parsed = ParsedTranscript::parse(&input).expect("100-turn transcript should be valid");
        assert_eq!(parsed.turns.len(), 100);
        assert_eq!(parsed.turns[99].number, 100);
    }
}
