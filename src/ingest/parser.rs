//! CommonMark AST streaming parser and exact span extraction.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

use crate::ingest::IngestError;
use crate::ingest::matcher::scan_for_candidates;
use crate::ingest::span::{SpanError, slice_source_span};

/// Structured tabular data extracted from a CommonMark table block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TableData {
    /// Header column names in order of appearance.
    pub headers: Vec<String>,
    /// Table rows as maps from column header to cell text value.
    pub rows: Vec<HashMap<String, String>>,
}

/// Extracted structural Markdown chunk with 0-based byte offsets and provenance attributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Structured table data if this chunk represents a table or table row.
    #[serde(default)]
    pub table_data: Option<TableData>,
}

impl ExtractedChunk {
    /// Returns the primary canonical key if any was extracted.
    #[must_use]
    pub fn primary_node_key(&self) -> Option<&str> {
        self.canonical_keys.first().map(|s| s.as_str())
    }
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

    // Table tracking state
    let mut in_table = false;
    let mut in_table_head = false;
    let mut in_table_row = false;
    let mut in_table_cell = false;
    let mut current_cell_buf = String::new();
    let mut table_headers: Vec<String> = Vec::new();
    let mut current_row_cells: Vec<String> = Vec::new();
    let mut current_row_start = 0usize;
    let mut table_rows: Vec<HashMap<String, String>> = Vec::new();
    let mut table_row_chunks: Vec<ExtractedChunk> = Vec::new();

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

                    if let Tag::Table(_) = tag {
                        in_table = true;
                        in_table_head = false;
                        in_table_row = false;
                        in_table_cell = false;
                        current_cell_buf.clear();
                        table_headers.clear();
                        table_rows.clear();
                        table_row_chunks.clear();
                    } else {
                        in_table = false;
                    }
                } else if in_table {
                    match tag {
                        Tag::TableHead => in_table_head = true,
                        Tag::TableRow => {
                            in_table_row = true;
                            current_row_cells.clear();
                            current_row_start = range.start;
                        }
                        Tag::TableCell => {
                            in_table_cell = true;
                            current_cell_buf.clear();
                        }
                        _ => {}
                    }
                }
                block_depth += 1;
            }
            Event::Text(text) | Event::Code(text) => {
                if is_heading {
                    heading_text_buf.push_str(&text);
                }
                if in_table_cell {
                    current_cell_buf.push_str(&text);
                }
            }
            Event::End(ref tag_end) => {
                if in_table {
                    match tag_end {
                        TagEnd::TableCell => {
                            in_table_cell = false;
                            let cell_text = current_cell_buf.trim().to_string();
                            if in_table_head {
                                table_headers.push(cell_text);
                            } else if in_table_row {
                                current_row_cells.push(cell_text);
                            }
                        }
                        TagEnd::TableHead => {
                            in_table_head = false;
                        }
                        TagEnd::TableRow => {
                            in_table_row = false;
                            let current_row_end = range.end;
                            let mut row_map = HashMap::new();
                            for (idx, header) in table_headers.iter().enumerate() {
                                let val = current_row_cells.get(idx).cloned().unwrap_or_default();
                                row_map.insert(header.clone(), val);
                            }
                            table_rows.push(row_map.clone());

                            if current_row_start < current_row_end
                                && current_row_end <= source_bytes.len()
                                && let Ok(row_text) = slice_source_span(
                                    source_bytes,
                                    current_row_start,
                                    current_row_end,
                                )
                            {
                                // Identify if this row is a specification/decision entity by checking if
                                // the primary identifier column (column named 'ID', 'Key', or the first column)
                                // contains an explicit canonical key (e.g. DR-1, C-1, INV-1, DEC-1.1).
                                let row_id_val = table_headers
                                    .iter()
                                    .enumerate()
                                    .find(|(_, h)| {
                                        let h_lower = h.to_lowercase();
                                        h_lower == "id"
                                            || h_lower.ends_with(" id")
                                            || h_lower == "key"
                                            || h_lower.ends_with(" key")
                                    })
                                    .and_then(|(idx, _)| current_row_cells.get(idx))
                                    .or_else(|| current_row_cells.first());

                                let entity_keys = row_id_val
                                    .map(|val| crate::ingest::matcher::extract_canonical_keys(val))
                                    .unwrap_or_default();

                                if !entity_keys.is_empty() {
                                    let (row_rfc, _all_keys, _row_cand) =
                                        scan_for_candidates(row_text);
                                    let (parent_id, parent_scope, enclosing_h) =
                                        match heading_stack.last() {
                                            Some(parent) => (
                                                Some(parent.ast_anchor.clone()),
                                                parent.ast_anchor.clone(),
                                                Some(parent.title.clone()),
                                            ),
                                            None => (None, doc_path.to_string(), None),
                                        };
                                    let primary_k = entity_keys
                                        .first()
                                        .cloned()
                                        .unwrap_or_else(|| format!("row-{}", table_rows.len()));
                                    let clean_k = slugify(&primary_k);
                                    let base_row_slug = format!("table-row-{clean_k}");
                                    let count_entry = slug_scope_counts
                                        .entry((parent_scope.clone(), base_row_slug.clone()))
                                        .or_insert(0);
                                    let occurrence = *count_entry;
                                    *count_entry += 1;

                                    let row_anchor = if occurrence == 0 {
                                        format!("{parent_scope}#{base_row_slug}")
                                    } else {
                                        format!("{parent_scope}#{base_row_slug}-{occurrence}")
                                    };

                                    table_row_chunks.push(ExtractedChunk {
                                        byte_start: current_row_start,
                                        byte_end: current_row_end,
                                        heading: enclosing_h,
                                        ast_anchor: row_anchor,
                                        parent_heading_chunk_id: parent_id,
                                        rfc2119_keywords: row_rfc,
                                        canonical_keys: entity_keys,
                                        is_candidate: true,
                                        content: Some(row_text.to_string()),
                                        table_data: Some(TableData {
                                            headers: table_headers.clone(),
                                            rows: vec![row_map],
                                        }),
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }

                block_depth = block_depth.saturating_sub(1);

                if block_depth == 0 && in_block {
                    in_block = false;
                    let block_end = range.end;

                    if block_start >= block_end || block_end > source_bytes.len() {
                        continue;
                    }

                    // Validate UTF-8 slice
                    let chunk_text = slice_source_span(source_bytes, block_start, block_end)?;
                    let (rfc2119_keywords, mut canonical_keys, mut is_candidate) =
                        scan_for_candidates(chunk_text);

                    let table_data = if in_table {
                        in_table = false;
                        Some(TableData {
                            headers: std::mem::take(&mut table_headers),
                            rows: std::mem::take(&mut table_rows),
                        })
                    } else {
                        None
                    };

                    let extracted_rows = if !table_row_chunks.is_empty() {
                        canonical_keys.clear();
                        is_candidate = false;
                        std::mem::take(&mut table_row_chunks)
                    } else {
                        Vec::new()
                    };

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
                            table_data: None,
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
                            table_data,
                        };

                        chunks.push(chunk);

                        for row_chunk in extracted_rows {
                            chunks.push(row_chunk);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(chunks)
}

/// Upward structural hierarchy edge (`DERIVED_FROM`) connecting a child chunk to its parent heading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedEdge {
    /// AST anchor of the child block or subsection.
    pub from_anchor: String,
    /// AST anchor of the immediate parent heading.
    pub to_anchor: String,
    /// Edge type (standardized to `"DERIVED_FROM"`).
    pub edge_type: String,
}

/// Complete result of mechanical CommonMark AST decomposition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecompositionResult {
    /// Extracted structural blocks and headings with exact byte offsets.
    pub chunks: Vec<ExtractedChunk>,
    /// Upward structural hierarchy edges (`DERIVED_FROM`).
    pub edges: Vec<ExtractedEdge>,
}

/// Parses a CommonMark document into structural chunks and upward hierarchy edges (`DERIVED_FROM`).
///
/// # Errors
///
/// Returns [`IngestError`] if byte slicing fails or character boundaries are violated.
pub fn parse_markdown(doc_path: &str, content: &str) -> Result<DecompositionResult, IngestError> {
    let chunks = parse_markdown_blocks(content, doc_path)?;
    let mut edges = Vec::new();

    for chunk in &chunks {
        if let Some(ref parent_anchor) = chunk.parent_heading_chunk_id {
            edges.push(ExtractedEdge {
                from_anchor: chunk.ast_anchor.clone(),
                to_anchor: parent_anchor.clone(),
                edge_type: "DERIVED_FROM".to_string(),
            });
        }
    }

    Ok(DecompositionResult { chunks, edges })
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

    #[test]
    fn test_parse_markdown_edges() {
        let md = r#"# Root Title
Introductory text.

## Section A
Paragraph in A.

### Subsection A.1
Paragraph in A.1.
"#;
        let res = parse_markdown("specs/doc.md", md).expect("valid parse");
        assert_eq!(res.chunks.len(), 6);
        // Chunks with parents:
        // Chunk 1 (Intro text) -> parent Chunk 0 (Root Title)
        // Chunk 2 (Section A) -> parent Chunk 0 (Root Title)
        // Chunk 3 (Paragraph in A) -> parent Chunk 2 (Section A)
        // Chunk 4 (Subsection A.1) -> parent Chunk 2 (Section A)
        // Chunk 5 (Paragraph in A.1) -> parent Chunk 4 (Subsection A.1)
        assert_eq!(res.edges.len(), 5);
        for edge in &res.edges {
            assert_eq!(edge.edge_type, "DERIVED_FROM");
            assert!(edge.from_anchor.starts_with("specs/doc.md#"));
            assert!(edge.to_anchor.starts_with("specs/doc.md#"));
        }

        // Verify specific hierarchy edges:
        assert_eq!(res.edges[0].from_anchor, "specs/doc.md#root-title#block-0");
        assert_eq!(res.edges[0].to_anchor, "specs/doc.md#root-title");

        assert_eq!(
            res.edges[1].from_anchor,
            "specs/doc.md#root-title/section-a"
        );
        assert_eq!(res.edges[1].to_anchor, "specs/doc.md#root-title");

        assert_eq!(
            res.edges[2].from_anchor,
            "specs/doc.md#root-title/section-a#block-0"
        );
        assert_eq!(res.edges[2].to_anchor, "specs/doc.md#root-title/section-a");

        assert_eq!(
            res.edges[3].from_anchor,
            "specs/doc.md#root-title/section-a/subsection-a-1"
        );
        assert_eq!(res.edges[3].to_anchor, "specs/doc.md#root-title/section-a");

        assert_eq!(
            res.edges[4].from_anchor,
            "specs/doc.md#root-title/section-a/subsection-a-1#block-0"
        );
        assert_eq!(
            res.edges[4].to_anchor,
            "specs/doc.md#root-title/section-a/subsection-a-1"
        );
    }

    #[test]
    fn test_parse_markdown_table_with_keys() {
        let md = r#"# Architecture Decisions

## Decision Ledger

| Decision ID | Work Package | Title | Category | Status |
| :--- | :--- | :--- | :--- | :--- |
| DEC-1.1 | WP-1.1 | In-Memory Index ODB Direct Writes | Technical Trade-off | Implemented |
| DEC-1.2 | WP-1.2 | Idempotent Root Empty Commit | Specification Gap | Implemented |
"#;
        let res = parse_markdown("specs/decisions.md", md).expect("valid parse");
        // Chunks:
        // 0: H1 Architecture Decisions
        // 1: H2 Decision Ledger
        // 2: Container Table block
        // 3: Table row DEC-1.1
        // 4: Table row DEC-1.2
        assert_eq!(res.chunks.len(), 5);

        // Check container table chunk
        let table_chunk = &res.chunks[2];
        assert!(table_chunk.table_data.is_some());
        let td = table_chunk.table_data.as_ref().unwrap();
        assert_eq!(
            td.headers,
            vec!["Decision ID", "Work Package", "Title", "Category", "Status"]
        );
        assert_eq!(td.rows.len(), 2);
        // Container table chunk clears canonical keys so it doesn't steal keys from rows
        assert!(table_chunk.canonical_keys.is_empty());

        // Check row chunk 1
        let row1 = &res.chunks[3];
        assert_eq!(row1.canonical_keys, vec!["DEC-1.1"]);
        assert!(row1.ast_anchor.contains("#table-row-dec-1-1"));
        assert!(row1.table_data.is_some());
        let row1_td = row1.table_data.as_ref().unwrap();
        assert_eq!(
            row1_td.rows[0].get("Decision ID").map(String::as_str),
            Some("DEC-1.1")
        );
        assert_eq!(
            row1_td.rows[0].get("Category").map(String::as_str),
            Some("Technical Trade-off")
        );

        // Check row chunk 2
        let row2 = &res.chunks[4];
        assert_eq!(row2.canonical_keys, vec!["DEC-1.2"]);
        assert!(row2.ast_anchor.contains("#table-row-dec-1-2"));
        assert!(row2.table_data.is_some());
        let row2_td = row2.table_data.as_ref().unwrap();
        assert_eq!(
            row2_td.rows[0].get("Decision ID").map(String::as_str),
            Some("DEC-1.2")
        );
        assert_eq!(
            row2_td.rows[0].get("Category").map(String::as_str),
            Some("Specification Gap")
        );
    }
}
