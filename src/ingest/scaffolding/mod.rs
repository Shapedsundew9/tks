//! Pluggable brownfield intent scaffolding adapters and registry (WP-4.2, TB-11, D-98, D-110).
//!
//! Provides zero-token mechanical parsers for OpenAPI/AsyncAPI specifications,
//! Architecture Decision Records (ADRs), Spec-Driven Development (SDD) files,
//! and agent steering files, synthesizing root `REQUIREMENT` nodes satisfying Invariant INV-1.

pub mod adr;
pub mod openapi;
pub mod sdd;

pub use adr::AdrAdapter;
pub use openapi::OpenApiAdapter;
pub use sdd::SddAdapter;

use serde::{Deserialize, Serialize};

use crate::ingest::IngestError;
use crate::ingest::parser::{DecompositionResult, ExtractedChunk, ExtractedEdge};

/// Output of a brownfield scaffolding adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedScaffolding {
    /// Extracted chunks including the synthesized root `REQUIREMENT` node.
    pub chunks: Vec<ExtractedChunk>,
    /// Upward structural hierarchy edges (`DERIVED_FROM`).
    pub edges: Vec<ExtractedEdge>,
}

impl ParsedScaffolding {
    /// Creates a new `ParsedScaffolding` container.
    #[must_use]
    pub fn new(chunks: Vec<ExtractedChunk>, edges: Vec<ExtractedEdge>) -> Self {
        Self { chunks, edges }
    }

    /// Returns the synthesized root requirement chunk, if present.
    #[must_use]
    pub fn root_requirement(&self) -> Option<&ExtractedChunk> {
        self.chunks.iter().find(|c| {
            c.node_type.as_deref() == Some("REQUIREMENT") && c.parent_heading_chunk_id.is_none()
        })
    }
}

impl From<ParsedScaffolding> for DecompositionResult {
    fn from(s: ParsedScaffolding) -> Self {
        DecompositionResult {
            chunks: s.chunks,
            edges: s.edges,
        }
    }
}

impl From<DecompositionResult> for ParsedScaffolding {
    fn from(d: DecompositionResult) -> Self {
        ParsedScaffolding {
            chunks: d.chunks,
            edges: d.edges,
        }
    }
}

/// Pluggable trait for brownfield format-specific mechanical parsing.
pub trait ScaffoldingAdapter: Send + Sync {
    /// Returns true if this adapter can handle the given document path or content.
    fn can_handle(&self, path: &str, content: &str) -> bool;

    /// Mechanically parses the document content into structural chunks and upward hierarchy edges.
    ///
    /// # Errors
    ///
    /// Returns [`IngestError`] if syntax parsing or structure extraction fails.
    fn parse(&self, path: &str, content: &str) -> Result<ParsedScaffolding, IngestError>;
}

/// Registry of registered scaffolding adapters.
pub struct ScaffoldingRegistry {
    adapters: Vec<Box<dyn ScaffoldingAdapter>>,
}

impl Default for ScaffoldingRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ScaffoldingRegistry {
    /// Creates a new registry pre-configured with standard brownfield adapters.
    #[must_use]
    pub fn new() -> Self {
        Self {
            adapters: vec![
                Box::new(OpenApiAdapter::new()),
                Box::new(AdrAdapter::new()),
                Box::new(SddAdapter::new()),
            ],
        }
    }

    /// Registers a new custom scaffolding adapter.
    pub fn register(&mut self, adapter: Box<dyn ScaffoldingAdapter>) {
        self.adapters.push(adapter);
    }

    /// Finds the first adapter capable of handling the specified path and content.
    #[must_use]
    pub fn find_adapter(&self, path: &str, content: &str) -> Option<&dyn ScaffoldingAdapter> {
        self.adapters
            .iter()
            .find(|a| a.can_handle(path, content))
            .map(|a| a.as_ref())
    }

    /// Attempts to parse the document using the matching adapter, if one is found.
    pub fn parse(
        &self,
        path: &str,
        content: &str,
    ) -> Option<Result<ParsedScaffolding, IngestError>> {
        self.find_adapter(path, content)
            .map(|adapter| adapter.parse(path, content))
    }
}

/// Convenience helper to parse a document via the default scaffolding adapter registry.
/// Returns `None` if no adapter handles the document.
pub fn parse_scaffolding(
    path: &str,
    content: &str,
) -> Option<Result<ParsedScaffolding, IngestError>> {
    let registry = ScaffoldingRegistry::new();
    registry.parse(path, content)
}

/// Helper function to format canonical human-readable keys (`PREFIX-SLUG`).
#[must_use]
pub fn canonical_key_slug(prefix: &str, name: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = true;
    for c in name.chars() {
        if c.is_alphanumeric() {
            slug.push(c.to_ascii_uppercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    if slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        prefix.to_string()
    } else if prefix.is_empty() {
        slug
    } else {
        format!("{prefix}-{slug}")
    }
}
