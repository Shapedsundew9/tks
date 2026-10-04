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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Substrate node type (`REQUIREMENT`, `SPECIFICATION`, `TASK`, `DECISION`, `UNCLASSIFIED`).
    #[serde(default)]
    pub node_type: Option<String>,
    /// Sequential ordinal alias (`c1`, `c2`, ...) assigned during Stage 1 atomization.
    #[serde(default)]
    pub atom_alias: Option<String>,
    /// Content trust tier (`HUMAN_APPROVED`, `AGENT_DRAFTED`, `EXTERNALLY_IMPORTED`).
    #[serde(default = "default_trust_tier")]
    pub trust_tier: String,
    /// Normative modality (`SHALL`, `SHOULD`, `MAY`, `UNCLASSIFIED`).
    #[serde(default)]
    pub modality: Option<String>,
    /// Classification confidence score (0.0 to 1.0).
    #[serde(default)]
    pub confidence: Option<f64>,
}

fn default_trust_tier() -> String {
    "HUMAN_APPROVED".to_string()
}

impl PartialEq for ExtractedChunk {
    fn eq(&self, other: &Self) -> bool {
        self.byte_start == other.byte_start
            && self.byte_end == other.byte_end
            && self.heading == other.heading
            && self.ast_anchor == other.ast_anchor
            && self.parent_heading_chunk_id == other.parent_heading_chunk_id
            && self.rfc2119_keywords == other.rfc2119_keywords
            && self.canonical_keys == other.canonical_keys
            && self.is_candidate == other.is_candidate
            && self.content == other.content
            && self.table_data == other.table_data
            && self.node_type == other.node_type
            && self.atom_alias == other.atom_alias
            && self.trust_tier == other.trust_tier
            && self.modality == other.modality
            && match (self.confidence, other.confidence) {
                (Some(a), Some(b)) => a.to_bits() == b.to_bits(),
                (None, None) => true,
                _ => false,
            }
    }
}

impl Eq for ExtractedChunk {}

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

/// Sanitizes text content from CommonMark structural AST blocks to prevent
/// prompt injection payload escape and command execution sequences (Spike 9, R-11).
#[must_use]
pub fn sanitize_ast_block(content: &str) -> String {
    // Strip null bytes and non-printable control characters
    let filtered: String = content
        .chars()
        .filter(|&c| c == '\t' || c == '\n' || c == '\r' || !c.is_control())
        .collect();

    // Neutralize dangerous escape patterns (e.g. ANSI escape sequences)
    let s = filtered.replace("\x1b", "");

    // Strip delimiter tokens and prompt injection payloads (Spike 9, R-11)
    s.replace("<|im_start|>", "")
        .replace("<|im_end|>", "")
        .replace("<|fim_prefix|>", "")
        .replace("<|fim_middle|>", "")
        .replace("<|fim_suffix|>", "")
        .replace("<|endoftext|>", "")
}

/// Frontmatter metadata extracted from the start of a Markdown document.
#[derive(Debug, Clone, Default)]
pub struct FrontmatterMetadata {
    /// Explicit document / root type override (`REQUIREMENT`, `SPECIFICATION`, etc.).
    pub doc_type: Option<String>,
    /// Explicit content trust tier override (`HUMAN_APPROVED`, `AGENT_DRAFTED`, `EXTERNALLY_IMPORTED`).
    pub trust_tier: Option<String>,
    /// 0-based byte offset where frontmatter ends and document body begins.
    pub end_offset: usize,
}

/// Parses optional YAML frontmatter (`--- ... ---`) at the beginning of a document.
#[must_use]
pub fn parse_yaml_frontmatter(source: &str) -> FrontmatterMetadata {
    if !source.starts_with("---") {
        return FrontmatterMetadata::default();
    }

    let after_dashes = &source[3..];
    let newline_idx = match after_dashes.find('\n') {
        Some(idx) if after_dashes[..idx].trim().is_empty() => 3 + idx + 1,
        _ => return FrontmatterMetadata::default(),
    };

    let body = &source[newline_idx..];
    let mut closing_pos = None;

    for (idx, _) in body.match_indices("---") {
        let is_line_start = idx == 0 || body.as_bytes()[idx - 1] == b'\n';
        let after = &body[idx + 3..];
        let next_nl = after.find('\n');
        let line_content = match next_nl {
            Some(n) => after[..n].trim(),
            None => after.trim(),
        };
        if is_line_start && line_content.is_empty() {
            let end_byte = newline_idx
                + idx
                + 3
                + match next_nl {
                    Some(n) => n + 1,
                    None => after.len(),
                };
            closing_pos = Some((idx, end_byte));
            break;
        }
    }

    let (body_len, end_offset) = match closing_pos {
        Some((bl, eo)) => (bl, eo),
        None => return FrontmatterMetadata::default(),
    };

    let fm_text = &body[..body_len];
    let mut doc_type = None;
    let mut trust_tier = None;

    for line in fm_text.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_lowercase();
            let val = v.trim().trim_matches('"').trim_matches('\'').trim();
            if key == "type" || key == "doc_type" || key == "node_type" {
                doc_type = Some(val.to_uppercase());
            } else if key == "trust_tier" || key == "trust" {
                trust_tier = Some(val.to_uppercase());
            }
        }
    }

    FrontmatterMetadata {
        doc_type,
        trust_tier,
        end_offset,
    }
}

/// Heading stack item representing an active section in the H1–H6 nesting hierarchy.
#[derive(Debug, Clone)]
struct ActiveHeading {
    level: u8,
    title: String,
    ast_anchor: String,
    node_type: String,
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
    parse_markdown_blocks_with_trust(source, doc_path, "HUMAN_APPROVED")
}

/// Parses a CommonMark document into structural blocks with an explicit default trust tier.
///
/// # Errors
///
/// Returns [`ParseError`] if byte slicing fails or UTF-8 character boundary safety is violated.
pub fn parse_markdown_blocks_with_trust(
    source: &str,
    doc_path: &str,
    default_trust_tier: &str,
) -> Result<Vec<ExtractedChunk>, ParseError> {
    let frontmatter = parse_yaml_frontmatter(source);
    let doc_trust_tier = frontmatter
        .trust_tier
        .as_deref()
        .unwrap_or(default_trust_tier)
        .to_string();

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
    let mut ordinal_alias_counter = 1usize;

    // Track heading slug occurrences per parent scope for disambiguation.
    let mut slug_scope_counts: HashMap<(String, String), usize> = HashMap::new();

    // Block tracking state
    let mut in_block = false;
    let mut block_depth = 0usize;
    let mut block_start = 0usize;
    let mut is_heading = false;
    let mut in_code_block = false;
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
        if range.end <= frontmatter.end_offset {
            continue;
        }

        match event {
            Event::Start(tag) => {
                if block_depth == 0 {
                    in_block = true;
                    block_start = range.start;

                    match &tag {
                        Tag::Heading { level, .. } => {
                            is_heading = true;
                            current_heading_level = *level;
                            heading_text_buf.clear();
                        }
                        Tag::CodeBlock(_) => {
                            is_heading = false;
                            in_code_block = true;
                        }
                        Tag::Table(_) => {
                            is_heading = false;
                            in_table = true;
                            in_table_head = false;
                            in_table_row = false;
                            in_table_cell = false;
                            current_cell_buf.clear();
                            table_headers.clear();
                            table_rows.clear();
                            table_row_chunks.clear();
                        }
                        _ => {
                            is_heading = false;
                            in_table = false;
                            in_code_block = false;
                        }
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

                                    let alias = format!("c{ordinal_alias_counter}");
                                    ordinal_alias_counter += 1;

                                    let row_node_type = if entity_keys
                                        .iter()
                                        .any(|k| k.starts_with("DEC-") || k.starts_with("D-"))
                                    {
                                        "DECISION".to_string()
                                    } else {
                                        "SPECIFICATION".to_string()
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
                                        node_type: Some(row_node_type),
                                        atom_alias: Some(alias),
                                        trust_tier: doc_trust_tier.clone(),
                                        modality: Some("MAY".to_string()),
                                        confidence: Some(0.90),
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }

                if let TagEnd::CodeBlock = tag_end {
                    in_code_block = false;
                }

                block_depth = block_depth.saturating_sub(1);

                if block_depth == 0 && in_block {
                    in_block = false;
                    let block_end = range.end;

                    if block_start >= block_end || block_end > source_bytes.len() {
                        continue;
                    }

                    let chunk_text = slice_source_span(source_bytes, block_start, block_end)?;
                    let (rfc2119_keywords, mut canonical_keys, is_candidate) =
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
                        std::mem::take(&mut table_row_chunks)
                    } else {
                        Vec::new()
                    };

                    if is_heading {
                        let level_u8 = heading_level_to_u8(current_heading_level);
                        let title = heading_text_buf.trim().to_string();
                        let base_slug = slugify(&title);

                        while let Some(top) = heading_stack.last() {
                            if top.level >= level_u8 {
                                heading_stack.pop();
                            } else {
                                break;
                            }
                        }

                        let (parent_id, parent_scope) = match heading_stack.last() {
                            Some(parent) => {
                                (Some(parent.ast_anchor.clone()), parent.ast_anchor.clone())
                            }
                            None => (None, String::new()),
                        };

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

                        let ast_anchor = if let Some(parent) = heading_stack.last() {
                            format!("{}/{}", parent.ast_anchor, slug)
                        } else {
                            format!("{doc_path}#{slug}")
                        };

                        // Structural Heading Classification (D-94, WP-4.1)
                        let heading_node_type = if let Some(ref ft) = frontmatter.doc_type {
                            ft.clone()
                        } else {
                            let mut decision = false;
                            for key in &canonical_keys {
                                if key.starts_with("DEC-") || key.starts_with("D-") {
                                    decision = true;
                                    break;
                                }
                            }
                            if decision
                                || title.starts_with("Decision ")
                                || title.starts_with("DEC-")
                                || title.starts_with("D-")
                                || title.contains("Decision:")
                            {
                                "DECISION".to_string()
                            } else if level_u8 <= 2 {
                                "REQUIREMENT".to_string()
                            } else {
                                "SPECIFICATION".to_string()
                            }
                        };

                        let heading_modality = if heading_node_type == "REQUIREMENT" {
                            Some("SHALL".to_string())
                        } else if heading_node_type == "SPECIFICATION" {
                            Some("MAY".to_string())
                        } else {
                            Some("UNCLASSIFIED".to_string())
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
                            content: Some(sanitize_ast_block(chunk_text)),
                            table_data: None,
                            node_type: Some(heading_node_type.clone()),
                            atom_alias: None,
                            trust_tier: doc_trust_tier.clone(),
                            modality: heading_modality,
                            confidence: Some(1.0),
                        };

                        chunks.push(chunk);

                        if level_u8 <= 6 {
                            heading_stack.push(ActiveHeading {
                                level: level_u8,
                                title,
                                ast_anchor,
                                node_type: heading_node_type,
                                block_counter: 0,
                            });
                        }
                    } else if table_data.is_some() {
                        let (parent_anchor, parent_heading_chunk_id, enclosing_heading) =
                            if let Some(parent) = heading_stack.last_mut() {
                                let _block_idx = parent.block_counter;
                                parent.block_counter += 1;
                                (
                                    parent.ast_anchor.clone(),
                                    Some(parent.ast_anchor.clone()),
                                    Some(parent.title.clone()),
                                )
                            } else {
                                let block_idx = root_block_counter;
                                root_block_counter += 1;
                                (format!("{doc_path}#block-{block_idx}"), None, None)
                            };

                        let block_idx = if heading_stack.is_empty() {
                            root_block_counter - 1
                        } else {
                            heading_stack.last().unwrap().block_counter - 1
                        };

                        let ast_anchor = if heading_stack.is_empty() {
                            format!("{doc_path}#block-{block_idx}")
                        } else {
                            format!("{parent_anchor}#block-{block_idx}")
                        };

                        let alias = format!("c{ordinal_alias_counter}");
                        ordinal_alias_counter += 1;

                        chunks.push(ExtractedChunk {
                            byte_start: block_start,
                            byte_end: block_end,
                            heading: enclosing_heading,
                            ast_anchor,
                            parent_heading_chunk_id,
                            rfc2119_keywords,
                            canonical_keys,
                            is_candidate: false,
                            content: Some(sanitize_ast_block(chunk_text)),
                            table_data,
                            node_type: Some("SPECIFICATION".to_string()),
                            atom_alias: Some(alias),
                            trust_tier: doc_trust_tier.clone(),
                            modality: Some("UNCLASSIFIED".to_string()),
                            confidence: Some(1.0),
                        });

                        for row_chunk in extracted_rows {
                            chunks.push(row_chunk);
                        }
                    } else {
                        // Non-heading structural blocks: paragraphs, lists, blockquotes, code
                        let (parent_heading_chunk_id, enclosing_heading, enclosing_type) =
                            if let Some(parent) = heading_stack.last_mut() {
                                (
                                    Some(parent.ast_anchor.clone()),
                                    Some(parent.title.clone()),
                                    Some(parent.node_type.clone()),
                                )
                            } else {
                                (None, None, None)
                            };

                        use unicode_segmentation::UnicodeSegmentation;

                        let mut sentences = Vec::new();

                        if in_code_block {
                            let trimmed = chunk_text.trim();
                            if !trimmed.is_empty() {
                                let leading_ws = chunk_text.len() - chunk_text.trim_start().len();
                                let s_start = block_start + leading_ws;
                                let s_end = s_start + trimmed.len();
                                sentences.push((s_start, s_end, trimmed));
                            }
                        } else {
                            for (offset, sentence_str) in chunk_text.split_sentence_bound_indices()
                            {
                                let trimmed = sentence_str.trim();
                                if trimmed.is_empty() {
                                    continue;
                                }
                                let leading_ws =
                                    sentence_str.len() - sentence_str.trim_start().len();
                                let s_start = block_start + offset + leading_ws;
                                let s_end = s_start + trimmed.len();
                                sentences.push((s_start, s_end, trimmed));
                            }
                        }

                        if sentences.is_empty() {
                            continue;
                        }

                        let block_idx = if let Some(parent) = heading_stack.last_mut() {
                            let b = parent.block_counter;
                            parent.block_counter += 1;
                            b
                        } else {
                            let b = root_block_counter;
                            root_block_counter += 1;
                            b
                        };

                        let multiple_sentences = sentences.len() > 1;

                        for (s_idx, (s_start, s_end, sentence_text)) in
                            sentences.into_iter().enumerate()
                        {
                            let (rfc2119_keywords, canonical_keys, is_cand) =
                                scan_for_candidates(sentence_text);

                            let clause_anchor = if heading_stack.is_empty() {
                                if multiple_sentences {
                                    format!("{doc_path}#block-{}#c{}", block_idx, s_idx + 1)
                                } else {
                                    format!("{doc_path}#block-{block_idx}")
                                }
                            } else {
                                let parent_ast = &heading_stack.last().unwrap().ast_anchor;
                                if multiple_sentences {
                                    format!("{parent_ast}#block-{}#c{}", block_idx, s_idx + 1)
                                } else {
                                    format!("{parent_ast}#block-{block_idx}")
                                }
                            };

                            let alias = format!("c{ordinal_alias_counter}");
                            ordinal_alias_counter += 1;

                            let clause_node_type = if canonical_keys
                                .iter()
                                .any(|k| k.starts_with("DEC-") || k.starts_with("D-"))
                            {
                                "DECISION".to_string()
                            } else if canonical_keys.iter().any(|k| k.starts_with("TASK-")) {
                                "TASK".to_string()
                            } else if !rfc2119_keywords.is_empty()
                                || canonical_keys.iter().any(|k| {
                                    k.starts_with("REQ-")
                                        || k.starts_with("INV-")
                                        || k.starts_with("C-")
                                })
                            {
                                "REQUIREMENT".to_string()
                            } else if let Some(ref enc) = enclosing_type {
                                if enc == "SPECIFICATION" {
                                    "SPECIFICATION".to_string()
                                } else {
                                    "UNCLASSIFIED".to_string()
                                }
                            } else {
                                "UNCLASSIFIED".to_string()
                            };

                            let (modality, confidence) = if clause_node_type == "REQUIREMENT" {
                                if rfc2119_keywords.iter().any(|k| k == "SHOULD") {
                                    ("SHOULD".to_string(), 0.85)
                                } else {
                                    ("SHALL".to_string(), 0.95)
                                }
                            } else if clause_node_type == "SPECIFICATION" {
                                ("MAY".to_string(), 0.80)
                            } else {
                                ("UNCLASSIFIED".to_string(), 0.50)
                            };

                            let sanitized_content = sanitize_ast_block(sentence_text);

                            chunks.push(ExtractedChunk {
                                byte_start: s_start,
                                byte_end: s_end,
                                heading: enclosing_heading.clone(),
                                ast_anchor: clause_anchor,
                                parent_heading_chunk_id: parent_heading_chunk_id.clone(),
                                rfc2119_keywords,
                                canonical_keys,
                                is_candidate: is_cand,
                                content: Some(sanitized_content),
                                table_data: None,
                                node_type: Some(clause_node_type),
                                atom_alias: Some(alias),
                                trust_tier: doc_trust_tier.clone(),
                                modality: Some(modality),
                                confidence: Some(confidence),
                            });
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
    parse_markdown_with_trust(doc_path, content, "HUMAN_APPROVED")
}

/// Parses a CommonMark document into structural chunks and upward hierarchy edges (`DERIVED_FROM`)
/// with an explicit content trust tier.
///
/// # Errors
///
/// Returns [`IngestError`] if byte slicing fails or character boundaries are violated.
pub fn parse_markdown_with_trust(
    doc_path: &str,
    content: &str,
    default_trust_tier: &str,
) -> Result<DecompositionResult, IngestError> {
    let chunks = parse_markdown_blocks_with_trust(content, doc_path, default_trust_tier)?;
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

        assert_eq!(chunks.len(), 8);

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

        // Chunk 4: List item 1 in Section 1
        assert_eq!(
            chunks[4].ast_anchor,
            "specs/test.md#document-title/section-1#block-1#c1"
        );

        // Chunk 5: List item 2 in Section 1
        assert_eq!(
            chunks[5].ast_anchor,
            "specs/test.md#document-title/section-1#block-1#c2"
        );

        // Chunk 6: Duplicate H2 (Section 1) -> disambiguated with -1
        assert_eq!(
            chunks[6].ast_anchor,
            "specs/test.md#document-title/section-1-1"
        );
        assert_eq!(
            chunks[6].parent_heading_chunk_id,
            Some("specs/test.md#document-title".to_string())
        );

        // Chunk 7: Paragraph in duplicate Section 1
        assert_eq!(
            chunks[7].ast_anchor,
            "specs/test.md#document-title/section-1-1#block-0"
        );
        assert_eq!(
            chunks[7].parent_heading_chunk_id,
            Some("specs/test.md#document-title/section-1-1".to_string())
        );
        assert!(chunks[7].is_candidate);
        assert_eq!(chunks[7].canonical_keys, vec!["REQ-CORE-001"]);
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
