//! Text normalisation for index keys.
//!
//! Keys are lower-cased and stripped of diacritics so that `zluty` finds
//! `žlutý` and `logos` finds `λόγος`-like accents (`λογος`).

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Fold a word into its index key: NFD, drop combining marks, lowercase,
/// collapse whitespace, map a few special letters.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for c in s.trim().nfd() {
        if is_combining_mark(c) {
            continue;
        }
        if c.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
            continue;
        }
        last_space = false;
        match c {
            'ß' => out.push_str("ss"),
            'ς' => out.push('σ'),
            'ł' | 'Ł' => out.push('l'),
            'ø' | 'Ø' => out.push('o'),
            'æ' | 'Æ' => out.push_str("ae"),
            'œ' | 'Œ' => out.push_str("oe"),
            '’' | 'ʼ' => out.push('\''),
            _ => out.extend(c.to_lowercase()),
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Canonical (NFC, trimmed) form of user input or a headword.
pub fn nfc(s: &str) -> String {
    s.trim().nfc().collect()
}

/// Levenshtein distance over chars (used to rank fuzzy hits).
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_czech() {
        assert_eq!(fold("Žlutý  kůň"), "zluty kun");
        assert_eq!(fold("ŘEŘICHA"), "rericha");
    }

    #[test]
    fn folds_german_and_greek() {
        assert_eq!(fold("Größe"), "grosse");
        assert_eq!(fold("λόγος"), "λογοσ");
        assert_eq!(fold("ἄνθρωπος"), "ανθρωποσ");
    }

    #[test]
    fn distance() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("pes", "pes"), 0);
    }
}
