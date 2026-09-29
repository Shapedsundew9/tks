//! CommonMark AST streaming parser and exact span extraction.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag};
use std::collections::HashMap;
use std::fmt;

use crate::ingest::matcher::scan_for_candidates;
use crate::ingest::span::{SpanError, slice_source_span};

/// Extracted structural Markdown chunk with 0-based byte offsets and provenance attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedChunk {
    /// 0-based start byte offset in the source document.
    pub byte_start: usize,
    /// 0-based end byte offset in the source document.
    pub byte_end: usize,
    /// Heading text of the chunk itself (for heading blocks) or enclosing section heading (for sub-blocks).
    pub heading: Option<String>,
    /// Globally unique, deterministic CommonMark AST anchor for this block.
    pub ast_anchor: String,
    /// Unique AST anchor of the immediate parent heading chunk (`DERIVED_FROM`), if any.
    pub parent_heading_chunk_id: Option<String>,
    /// RFC 2119 normative keywords extracted from this chunk.
    pub rfc2119_keywords: Vec<String>,
    /// Canonical substrate identifiers extracted from this chunk (`REQ-*`, `INV-*`, `DR-*`, `C-*`, `TB-*`, `TASK-*`).
    pub canonical_keys: Vec<String>,
    /// Flag indicating whether this chunk contains RFC 2119 keywords or canonical keys requiring semantic classification.
    pub is_candidate: bool,
    /// Exact sliced text content from the source span.
    pub content: Option<String>,
}

/// Errors that can occur during CommonMark AST decomposition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Byte slicing error or UTF-8 safety violation.
    Span(SpanError),
    /// Invalid document structure or unresolvable AST state.
    InvalidStructure(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Span(err) => write!(f, "span error during parsing: {err}"),
            Self::InvalidStructure(msg) => write!(f, "invalid structure: {msg}"),
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Span(err) => Some(err),
            _ => None,
        }
    }
}

impl From<SpanError> for ParseError {
    fn from(err: SpanError) -> Self {
        Self::Span(err)
    }
}

/// Converts heading text into a deterministic, URL-safe slug.
#[must_use]
pub fn slugify(text: &str) -> String {
    let mut slug = String::with_capacity(text.len());
    let mut last_was_dash = true;

    for ch in text.chars() {
        if ch.is_alphanumeric() {
            slug.extend(ch.to_lowercase());
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
        "section".to_string()
    } else {
        slug
    }
}

/// Heading stack item representing an active section in the H1–H4 nesting hierarchy.
#[derive(Debug, Clone)]
struct ActiveHeading {
    level: u8,
    title: String,
    ast_anchor: String,
    block_counter: usize,
}

fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Parses a CommonMark document into structural blocks, extracting byte spans, heading hierarchy,
/// unique deterministic AST anchors, RFC 2119 keywords, and canonical identifiers.
///
/// # Errors
///
/// Returns [`ParseError`] if byte slicing fails or UTF-8 character boundary safety is violated.
pub fn parse_markdown_blocks(
    source: &str,
    doc_path: &str,
) -> Result<Vec<ExtractedChunk>, ParseError> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);

    let parser = Parser::new_ext(source, options);
    let source_bytes = source.as_bytes();

    let mut chunks = Vec::new();
    let mut heading_stack: Vec<ActiveHeading> = Vec::new();
    let mut root_block_counter = 0usize;

    // Track heading slug occurrences per parent scope for disambiguation.
    // Key: (parent_scope_anchor, base_slug) -> occurrence count.
    let mut slug_scope_counts: HashMap<(String, String), usize> = HashMap::new();

    // Block tracking state
    let mut in_block = false;
    let mut block_depth = 0usize;
    let mut block_start = 0usize;
    let mut is_heading = false;
    let mut current_heading_level = HeadingLevel::H1;
    let mut heading_text_buf = String::new();

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if block_depth == 0 {
                    // Entering a new top-level structural block
                    in_block = true;
                    block_start = range.start;

                    if let Tag::Heading { level, .. } = tag {
                        is_heading = true;
                        current_heading_level = level;
                        heading_text_buf.clear();
                    } else {
                        is_heading = false;
                    }
                } else if is_heading {
                    // Nested tags inside heading (e.g. emphasis, code, links)
                }
                block_depth += 1;
            }
            Event::Text(text) | Event::Code(text) => {
                if is_heading {
                    heading_text_buf.push_str(&text);
                }
            }
            Event::End(_tag_end) => {
                block_depth = block_depth.saturating_sub(1);

                if block_depth == 0 && in_block {
                    in_block = false;
                    let block_end = range.end;

                    if block_start >= block_end || block_end > source_bytes.len() {
                        continue;
                    }

                    // Validate UTF-8 slice
                    let chunk_text = slice_source_span(source_bytes, block_start, block_end)?;
                    let (rfc2119_keywords, canonical_keys, is_candidate) =
                        scan_for_candidates(chunk_text);

                    if is_heading {
                        let level_u8 = heading_level_to_u8(current_heading_level);
                        let title = heading_text_buf.trim().to_string();
                        let base_slug = slugify(&title);

                        // Pop any active heading at or deeper than current heading level
                        while let Some(top) = heading_stack.last() {
                            if top.level >= level_u8 {
                                heading_stack.pop();
                            } else {
                                break;
                            }
                        }

                        // Determine parent heading chunk ID and parent scope
                        let (parent_id, parent_scope) = match heading_stack.last() {
                            Some(parent) => {
                                (Some(parent.ast_anchor.clone()), parent.ast_anchor.clone())
                            }
                            None => (None, String::new()),
                        };

                        // Disambiguate slug within parent scope
                        let count_entry = slug_scope_counts
                            .entry((parent_scope.clone(), base_slug.clone()))
                            .or_insert(0);
                        let occurrence = *count_entry;
                        *count_entry += 1;

                        let slug = if occurrence == 0 {
                            base_slug
                        } else {
                            format!("{base_slug}-{occurrence}")
                        };

                        // Generate deterministic AST anchor
                        let ast_anchor = if let Some(parent) = heading_stack.last() {
                            format!("{}/{}", parent.ast_anchor, slug)
                        } else {
                            format!("{doc_path}#{slug}")
                        };

                        let chunk = ExtractedChunk {
                            byte_start: block_start,
                            byte_end: block_end,
                            heading: Some(title.clone()),
                            ast_anchor: ast_anchor.clone(),
                            parent_heading_chunk_id: parent_id,
                            rfc2119_keywords,
                            canonical_keys,
                            is_candidate,
                            content: Some(chunk_text.to_string()),
                        };

                        chunks.push(chunk);

                        // Push this heading onto the active stack (capping tracking at H1-H4)
                        if level_u8 <= 4 {
                            heading_stack.push(ActiveHeading {
                                level: level_u8,
                                title,
                                ast_anchor,
                                block_counter: 0,
                            });
                        }
                    } else {
                        // Non-heading structural block
                        let (ast_anchor, parent_heading_chunk_id, enclosing_heading) =
                            if let Some(parent) = heading_stack.last_mut() {
                                let block_idx = parent.block_counter;
                                parent.block_counter += 1;
                                (
                                    format!("{}#block-{}", parent.ast_anchor, block_idx),
                                    Some(parent.ast_anchor.clone()),
                                    Some(parent.title.clone()),
                                )
                            } else {
                                let block_idx = root_block_counter;
                                root_block_counter += 1;
                                (format!("{doc_path}#block-{block_idx}"), None, None)
                            };

                        let chunk = ExtractedChunk {
                            byte_start: block_start,
                            byte_end: block_end,
                            heading: enclosing_heading,
                            ast_anchor,
                            parent_heading_chunk_id,
                            rfc2119_keywords,
                            canonical_keys,
                            is_candidate,
                            content: Some(chunk_text.to_string()),
                        };

                        chunks.push(chunk);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Executive Summary"), "executive-summary");
        assert_eq!(
            slugify("1. Section: Architecture & Design!"),
            "1-section-architecture-design"
        );
        assert_eq!(slugify("TB-2"), "tb-2");
        assert_eq!(slugify("   "), "section");
    }

    #[test]
    fn test_parse_basic_document() {
        let md = r#"# Document Title
Introductory paragraph.

## Section 1
First paragraph of section 1 with MUST keyword.

- item 1
- item 2

## Section 1
Duplicate section heading with REQ-CORE-001.
"#;
        let chunks = parse_markdown_blocks(md, "specs/test.md").unwrap();

        assert_eq!(chunks.len(), 7);

        // Chunk 0: H1
        assert_eq!(chunks[0].heading, Some("Document Title".to_string()));
        assert_eq!(chunks[0].ast_anchor, "specs/test.md#document-title");
        assert_eq!(chunks[0].parent_heading_chunk_id, None);

        // Chunk 1: Intro paragraph
        assert_eq!(chunks[1].ast_anchor, "specs/test.md#document-title#block-0");
        assert_eq!(
            chunks[1].parent_heading_chunk_id,
            Some("specs/test.md#document-title".to_string())
        );

        // Chunk 2: H2 (Section 1)
        assert_eq!(
            chunks[2].ast_anchor,
            "specs/test.md#document-title/section-1"
        );
        assert_eq!(
            chunks[2].parent_heading_chunk_id,
            Some("specs/test.md#document-title".to_string())
        );

        // Chunk 3: Paragraph in Section 1
        assert_eq!(
            chunks[3].ast_anchor,
            "specs/test.md#document-title/section-1#block-0"
        );
        assert!(chunks[3].is_candidate);
        assert_eq!(chunks[3].rfc2119_keywords, vec!["MUST"]);

        // Chunk 4: List in Section 1
        assert_eq!(
            chunks[4].ast_anchor,
            "specs/test.md#document-title/section-1#block-1"
        );

        // Chunk 5: Duplicate H2 (Section 1) -> disambiguated with -1
        assert_eq!(
            chunks[5].ast_anchor,
            "specs/test.md#document-title/section-1-1"
        );
        assert_eq!(
            chunks[5].parent_heading_chunk_id,
            Some("specs/test.md#document-title".to_string())
        );

        // Chunk 6: Paragraph in duplicate Section 1
        assert_eq!(
            chunks[6].ast_anchor,
            "specs/test.md#document-title/section-1-1#block-0"
        );
        assert_eq!(
            chunks[6].parent_heading_chunk_id,
            Some("specs/test.md#document-title/section-1-1".to_string())
        );
        assert!(chunks[6].is_candidate);
        assert_eq!(chunks[6].canonical_keys, vec!["REQ-CORE-001"]);
    }
}
