//! Text normalisation for index keys.
//!
//! Keys are lower-cased and stripped of diacritics so that `zluty` finds
//! `žlutý` and `logos` finds `λόγος`-like accents (`λογος`).

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Is `c` an accent that can be typed without (Latin/Greek/Cyrillic
/// diacritics, Arabic vowel marks)? Marks that are part of the letter in
/// other scripts – Devanagari vowel signs, the Japanese dakuten (が = か + ゙),
/// Hangul jamo – must be kept, or unrelated words would share a key.
fn is_optional_mark(c: char) -> bool {
    is_combining_mark(c)
        && matches!(c,
            '\u{0300}'..='\u{036F}'   // combining diacritical marks
            | '\u{0483}'..='\u{0489}' // Cyrillic
            | '\u{064B}'..='\u{065F}' // Arabic harakat
            | '\u{0670}'              // Arabic superscript alef
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}')
}

/// Fold a word into its index key: NFD, drop optional accents, lowercase,
/// collapse whitespace, map a few special letters; recompose (NFC) so that
/// e.g. Hangul syllables and kana stay single characters.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for c in s.trim().nfd() {
        if is_optional_mark(c) {
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
    if out.is_ascii() { out } else { out.nfc().collect() }
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
    fn folds_other_scripts() {
        assert_eq!(fold("Привет"), "привет");
        assert_eq!(fold("йод"), "иод");
        assert_eq!(fold("كَلْب"), "كلب");
        // Letters whose marks are not optional stay distinct.
        assert_eq!(fold("कि"), "कि");
        assert_ne!(fold("कि"), fold("क"));
        assert_eq!(fold("がっこう"), "がっこう");
        assert_ne!(fold("が"), fold("か"));
        assert_eq!(fold("개"), "개");
        assert_eq!(fold("狗"), "狗");
        assert_eq!(fold("Português"), "portugues");
    }

    #[test]
    fn distance() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("pes", "pes"), 0);
    }
}
