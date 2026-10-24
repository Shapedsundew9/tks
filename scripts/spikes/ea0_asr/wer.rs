//! EA-0: reproducible WER / correction-effort scoring.
//!
//! Normalisation follows the spirit of the Open ASR Leaderboard's English normaliser, reduced
//! to rules that are easy to audit: lower-case, punctuation removed (apostrophes kept inside
//! words), hyphens split, filler words dropped, a few spelling standardisations, and integers
//! below 10 000 spelled out. Both reference and hypothesis go through the same function.

const FILLERS: &[&str] = &["hmm", "mm", "mhm", "mmm", "uh", "um", "er", "ah"];

fn standardise(word: &str) -> Option<&'static str> {
    Some(match word {
        "mr" => "mister",
        "mrs" => "missus",
        "dr" => "doctor",
        "ok" => "okay",
        "gonna" => "going to",
        "wanna" => "want to",
        _ => return None,
    })
}

fn number_words(n: u32) -> String {
    const ONES: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 10] = [
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    match n {
        0..=19 => ONES[n as usize].to_string(),
        20..=99 => {
            let t = TENS[(n / 10) as usize];
            if n.is_multiple_of(10) {
                t.to_string()
            } else {
                format!("{t} {}", ONES[(n % 10) as usize])
            }
        }
        100..=999 => {
            let rest = n % 100;
            let head = format!("{} hundred", ONES[(n / 100) as usize]);
            if rest == 0 {
                head
            } else {
                format!("{head} {}", number_words(rest))
            }
        }
        _ => {
            let rest = n % 1000;
            let head = format!("{} thousand", number_words(n / 1000));
            if rest == 0 {
                head
            } else {
                format!("{head} {}", number_words(rest))
            }
        }
    }
}

/// Normalises text into comparable words.
pub fn normalize(text: &str) -> Vec<String> {
    let cleaned: String = text
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '\'' {
                c
            } else {
                ' '
            }
        })
        .collect();
    let mut out = Vec::new();
    for raw in cleaned.split_whitespace() {
        let word = raw.trim_matches('\'');
        if word.is_empty() || FILLERS.contains(&word) {
            continue;
        }
        if let Some(std) = standardise(word) {
            out.extend(std.split(' ').map(str::to_string));
        } else if let Ok(n) = word.parse::<u32>()
            && n < 10_000
        {
            out.extend(number_words(n).split(' ').map(str::to_string));
        } else {
            out.push(word.to_string());
        }
    }
    out
}

/// Word-level edit counts between a reference and a hypothesis.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct Edits {
    pub substitutions: usize,
    pub deletions: usize,
    pub insertions: usize,
    pub reference_words: usize,
}

impl Edits {
    pub fn total(&self) -> usize {
        self.substitutions + self.deletions + self.insertions
    }

    pub fn add(&mut self, other: &Edits) {
        self.substitutions += other.substitutions;
        self.deletions += other.deletions;
        self.insertions += other.insertions;
        self.reference_words += other.reference_words;
    }

    /// Corpus-level WER (sum of edits / sum of reference words).
    pub fn wer(&self) -> f64 {
        if self.reference_words == 0 {
            return if self.insertions == 0 { 0.0 } else { 1.0 };
        }
        self.total() as f64 / self.reference_words as f64
    }
}

/// Levenshtein alignment over words, returning S/D/I counts.
pub fn score(reference: &str, hypothesis: &str) -> Edits {
    let r = normalize(reference);
    let h = normalize(hypothesis);
    // dp[i][j] = (cost, subs, dels, ins)
    let mut dp = vec![vec![(0usize, 0usize, 0usize, 0usize); h.len() + 1]; r.len() + 1];
    for (i, row) in dp.iter_mut().enumerate().skip(1) {
        row[0] = (i, 0, i, 0);
    }
    for (j, cell) in dp[0].iter_mut().enumerate().skip(1) {
        *cell = (j, 0, 0, j);
    }
    for i in 1..=r.len() {
        for j in 1..=h.len() {
            let diag = dp[i - 1][j - 1];
            let same = r[i - 1] == h[j - 1];
            let sub = (
                diag.0 + usize::from(!same),
                diag.1 + usize::from(!same),
                diag.2,
                diag.3,
            );
            let up = dp[i - 1][j];
            let del = (up.0 + 1, up.1, up.2 + 1, up.3);
            let left = dp[i][j - 1];
            let ins = (left.0 + 1, left.1, left.2, left.3 + 1);
            dp[i][j] = [sub, del, ins]
                .into_iter()
                .min_by_key(|c| c.0)
                .unwrap_or(sub);
        }
    }
    let (_, substitutions, deletions, insertions) = dp[r.len()][h.len()];
    Edits {
        substitutions,
        deletions,
        insertions,
        reference_words: r.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_case_punctuation_numbers_and_fillers() {
        assert_eq!(
            normalize("Mr. Quilter, um, has 21 well-known cats!"),
            [
                "mister", "quilter", "has", "twenty", "one", "well", "known", "cats"
            ]
        );
    }

    #[test]
    fn counts_edits() {
        let e = score("alpha beta gamma delta", "alpha gamma delta epsilon");
        assert_eq!((e.substitutions, e.deletions, e.insertions), (0, 1, 1));
        assert_eq!(e.reference_words, 4);
        let e = score("alpha beta gamma", "alpha zeta gamma");
        assert_eq!((e.substitutions, e.deletions, e.insertions), (1, 0, 0));
    }
}
