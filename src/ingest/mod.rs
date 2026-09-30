//! Document ingestion, AST decomposition, and span extraction modules.

pub mod classify;
pub mod fallback;
pub mod matcher;
pub mod parser;
pub mod reconcile;
pub mod span;

pub use classify::*;
pub use fallback::*;
pub use matcher::*;
pub use parser::*;
pub use reconcile::*;
pub use span::*;

use std::fmt;

/// Top-level error enum for document ingestion, parsing, and reconciliation.
#[derive(Debug)]
pub enum IngestError {
    /// CommonMark AST parsing or structure error.
    Parse(parser::ParseError),
    /// Byte span offset or UTF-8 safety violation.
    Span(span::SpanError),
    /// Semantic classification prompt or response parsing error.
    Classification(classify::ClassificationError),
    /// Document reconciliation error.
    Reconciliation(String),
}

impl fmt::Display for IngestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(err) => write!(f, "ingestion parse error: {err}"),
            Self::Span(err) => write!(f, "ingestion span error: {err}"),
            Self::Classification(err) => write!(f, "ingestion classification error: {err}"),
            Self::Reconciliation(msg) => write!(f, "ingestion reconciliation error: {msg}"),
        }
    }
}

impl std::error::Error for IngestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse(err) => Some(err),
            Self::Span(err) => Some(err),
            Self::Classification(err) => Some(err),
            Self::Reconciliation(_) => None,
        }
    }
}

impl From<parser::ParseError> for IngestError {
    fn from(err: parser::ParseError) -> Self {
        Self::Parse(err)
    }
}

impl From<span::SpanError> for IngestError {
    fn from(err: span::SpanError) -> Self {
        Self::Span(err)
    }
}

impl From<classify::ClassificationError> for IngestError {
    fn from(err: classify::ClassificationError) -> Self {
        Self::Classification(err)
    }
}
