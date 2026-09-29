//! UTF-8 safe zero-copy byte slicing for source document spans.

use std::fmt;

/// Errors that can occur when slicing a byte span from a source buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanError {
    /// Start offset is strictly greater than end offset.
    InvalidRange { start: usize, end: usize },
    /// Requested byte span exceeds the length of the source buffer.
    OutOfBounds {
        start: usize,
        end: usize,
        len: usize,
    },
    /// The sliced byte slice does not form a valid UTF-8 string boundary.
    InvalidUtf8 {
        start: usize,
        end: usize,
        source: std::str::Utf8Error,
    },
}

impl fmt::Display for SpanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange { start, end } => {
                write!(f, "invalid byte span range: start ({start}) > end ({end})")
            }
            Self::OutOfBounds { start, end, len } => {
                write!(
                    f,
                    "byte span [{start}..{end}] is out of bounds for buffer length {len}"
                )
            }
            Self::InvalidUtf8 { start, end, source } => {
                write!(
                    f,
                    "byte span [{start}..{end}] does not align to valid UTF-8 character boundaries: {source}"
                )
            }
        }
    }
}

impl std::error::Error for SpanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidUtf8 { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Slices a UTF-8 string reference from raw source bytes using 0-based byte offsets.
///
/// This function verifies range validity and character boundary safety, returning a
/// [`SpanError`] rather than panicking if indices do not align with UTF-8 character boundaries.
///
/// # Errors
///
/// Returns [`SpanError::InvalidRange`] if `start > end`.
/// Returns [`SpanError::OutOfBounds`] if `end > source_bytes.len()`.
/// Returns [`SpanError::InvalidUtf8`] if the byte slice is not valid UTF-8.
pub fn slice_source_span(source_bytes: &[u8], start: usize, end: usize) -> Result<&str, SpanError> {
    if start > end {
        return Err(SpanError::InvalidRange { start, end });
    }
    if end > source_bytes.len() {
        return Err(SpanError::OutOfBounds {
            start,
            end,
            len: source_bytes.len(),
        });
    }

    let slice = &source_bytes[start..end];
    std::str::from_utf8(slice).map_err(|source| SpanError::InvalidUtf8 { start, end, source })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_ascii_span() {
        let text = "Hello, world!";
        let slice = slice_source_span(text.as_bytes(), 0, 5).unwrap();
        assert_eq!(slice, "Hello");
    }

    #[test]
    fn test_empty_span() {
        let text = "Hello";
        let slice = slice_source_span(text.as_bytes(), 2, 2).unwrap();
        assert_eq!(slice, "");
    }

    #[test]
    fn test_invalid_range() {
        let text = "Hello";
        let err = slice_source_span(text.as_bytes(), 4, 2).unwrap_err();
        assert_eq!(err, SpanError::InvalidRange { start: 4, end: 2 });
    }

    #[test]
    fn test_out_of_bounds() {
        let text = "Hello";
        let err = slice_source_span(text.as_bytes(), 0, 10).unwrap_err();
        assert_eq!(
            err,
            SpanError::OutOfBounds {
                start: 0,
                end: 10,
                len: 5
            }
        );
    }

    #[test]
    fn test_multibyte_characters_safe_slicing() {
        // Multi-byte test string containing:
        // - em-dash: '—' (3 bytes: E2 80 94)
        // - curly quotes: '“' (3 bytes: E2 80 9C), '”' (3 bytes: E2 80 9D)
        // - mathematical symbols: '≥' (3 bytes: E2 89 A5), '≤' (3 bytes: E2 89 A4), '→' (3 bytes: E2 86 92)
        let sample = "Latency ≤ 50ms, binary ≥ 10MB → greenlight — “approved”";
        let bytes = sample.as_bytes();

        // 1. Slice exact substring containing math symbols
        // Find position of '≤'
        let le_pos = sample.find('≤').unwrap();
        let ge_pos = sample.find('≥').unwrap();
        let arrow_pos = sample.find('→').unwrap();
        let emdash_pos = sample.find('—').unwrap();
        let quote_pos = sample.find('“').unwrap();

        // Each of these characters is 3 UTF-8 bytes
        assert_eq!('≤'.len_utf8(), 3);
        assert_eq!('≥'.len_utf8(), 3);
        assert_eq!('→'.len_utf8(), 3);
        assert_eq!('—'.len_utf8(), 3);
        assert_eq!('“'.len_utf8(), 3);

        // Whole symbol slicing succeeds
        let le_str = slice_source_span(bytes, le_pos, le_pos + 3).unwrap();
        assert_eq!(le_str, "≤");

        let ge_str = slice_source_span(bytes, ge_pos, ge_pos + 3).unwrap();
        assert_eq!(ge_str, "≥");

        let arrow_str = slice_source_span(bytes, arrow_pos, arrow_pos + 3).unwrap();
        assert_eq!(arrow_str, "→");

        let emdash_str = slice_source_span(bytes, emdash_pos, emdash_pos + 3).unwrap();
        assert_eq!(emdash_str, "—");

        let quote_str = slice_source_span(bytes, quote_pos, quote_pos + 3).unwrap();
        assert_eq!(quote_str, "“");

        // 2. Slicing in the middle of a multi-byte character must fail gracefully without panicking
        let split_res = slice_source_span(bytes, le_pos, le_pos + 1);
        assert!(split_res.is_err());
        match split_res.unwrap_err() {
            SpanError::InvalidUtf8 { start, end, .. } => {
                assert_eq!(start, le_pos);
                assert_eq!(end, le_pos + 1);
            }
            other => panic!("expected InvalidUtf8, got {other:?}"),
        }

        let split_mid = slice_source_span(bytes, arrow_pos + 1, arrow_pos + 3);
        assert!(split_mid.is_err());
    }
}
