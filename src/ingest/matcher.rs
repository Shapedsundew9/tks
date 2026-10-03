//! Deterministic scanner matching RFC 2119 keywords and canonical substrate identifiers.

use regex::Regex;
use std::sync::LazyLock;

/// Regex pattern matching RFC 2119 normative requirement keywords.
/// Evaluates multi-word phrases (`MUST NOT`, `SHALL NOT`, `SHOULD NOT`) before single words.
static RFC2119_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(MUST NOT|SHALL NOT|SHOULD NOT|MUST|REQUIRED|SHALL|SHOULD|RECOMMENDED|MAY|OPTIONAL)\b",
    )
    .expect("valid RFC 2119 regex")
});

/// Regex pattern matching canonical substrate identifiers:
/// `REQ-*`, `INV-*`, `DR-*`, `C-*`, `TB-*`, `TASK-*`, `WP-*`, `DEC-*`.
static CANONICAL_KEY_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b((?:REQ|INV|DR|C|TB|TASK|WP|DEC)-[A-Za-z0-9]+(?:[-_\.][A-Za-z0-9]+)*)\b")
        .expect("valid canonical key regex")
});

/// Extracts unique RFC 2119 keywords from the supplied text in order of first appearance.
#[must_use]
pub fn extract_rfc2119_keywords(text: &str) -> Vec<String> {
    let mut keywords = Vec::new();
    for mat in RFC2119_REGEX.find_iter(text) {
        let kw = mat.as_str().to_string();
        if !keywords.contains(&kw) {
            keywords.push(kw);
        }
    }
    keywords
}

/// Extracts unique canonical substrate identifiers from the supplied text in order of first appearance.
#[must_use]
pub fn extract_canonical_keys(text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for mat in CANONICAL_KEY_REGEX.find_iter(text) {
        let key = mat.as_str().to_string();
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

/// Scans text for RFC 2119 keywords and canonical keys, determining candidate requirement status.
///
/// Returns `(rfc2119_keywords, canonical_keys, is_candidate)`.
#[must_use]
pub fn scan_for_candidates(text: &str) -> (Vec<String>, Vec<String>, bool) {
    let rfc2119 = extract_rfc2119_keywords(text);
    let canonical = extract_canonical_keys(text);
    let is_candidate = !rfc2119.is_empty() || !canonical.is_empty();
    (rfc2119, canonical, is_candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_rfc2119_keywords() {
        let text = "The gateway MUST authenticate all requests, and SHALL NOT permit unsigned tokens. It SHOULD log failures.";
        let keywords = extract_rfc2119_keywords(text);
        assert_eq!(keywords, vec!["MUST", "SHALL NOT", "SHOULD"]);
    }

    #[test]
    fn test_extract_rfc2119_deduplication() {
        let text = "MUST do this and MUST do that, but MUST NOT do the other.";
        let keywords = extract_rfc2119_keywords(text);
        assert_eq!(keywords, vec!["MUST", "MUST NOT"]);
    }

    #[test]
    fn test_extract_canonical_keys() {
        let text = "Governed by REQ-AUTH-01, INV-4, DR-11, C-1, TB-2, DEC-1.1, and TASK-123. See also [INV-2](link).";
        let keys = extract_canonical_keys(text);
        assert_eq!(
            keys,
            vec![
                "REQ-AUTH-01",
                "INV-4",
                "DR-11",
                "C-1",
                "TB-2",
                "DEC-1.1",
                "TASK-123",
                "INV-2"
            ]
        );
    }

    #[test]
    fn test_scan_for_candidates() {
        let text_candidate = "This component MUST satisfy DR-10.";
        let (kws, keys, is_cand) = scan_for_candidates(text_candidate);
        assert!(is_cand);
        assert_eq!(kws, vec!["MUST"]);
        assert_eq!(keys, vec!["DR-10"]);

        let text_narrative = "The Knowledge Substrate manages system evolution.";
        let (kws2, keys2, is_cand2) = scan_for_candidates(text_narrative);
        assert!(!is_cand2);
        assert!(kws2.is_empty());
        assert!(keys2.is_empty());
    }
}
