//! Dictionary data model shared by the builder, the on-disk packs and the app.

use serde::{Deserialize, Serialize};

/// Metadata describing one dictionary pack.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DictMeta {
    /// Stable identifier, e.g. `cs-en`.
    pub id: String,
    /// Human readable name, e.g. `Czech → English`.
    pub name: String,
    /// ISO 639 code of the headword language (`cs`, `en`, `de`, `grc`).
    pub source_lang: String,
    /// ISO 639 code of the translation language.
    pub target_lang: String,
    /// Where the data came from, e.g. `Wiktionary (kaikki.org), GNU/FDL slovník`.
    pub sources: String,
    /// License / attribution text that must accompany the data.
    pub license: String,
    /// Build date (YYYY-MM-DD).
    pub built: String,
    /// Number of entries in the pack.
    pub entries: u32,
}

/// One dictionary entry: a headword with a single part of speech.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub headword: String,
    /// Part of speech, short English form (`noun`, `verb`, `adj`, ...).
    pub pos: String,
    /// Grammatical information, e.g. `masculine inanimate`, `perfective`.
    pub grammar: Vec<String>,
    /// IPA pronunciations.
    pub ipa: Vec<String>,
    /// URLs of pronunciation recordings (mp3 preferred).
    pub audio: Vec<String>,
    /// Hyphenation / syllabification, e.g. `žlu‧tý`.
    pub hyphenation: Option<String>,
    /// A few notable forms for display (`comparative žlutější`).
    pub forms: Vec<Form>,
    pub senses: Vec<Sense>,
    pub etymology: Option<String>,
    /// Short label of the data source (`Wiktionary`, `GNU/FDL`).
    pub source: String,
    /// Inflected forms used only for lookup (not displayed). Not stored in
    /// packs – the builder turns them into index keys.
    #[serde(skip)]
    pub index_forms: Vec<String>,
    /// True when every sense is just "inflection of X" (builder may drop it
    /// if X's inflection table already covers the word). Not stored.
    #[serde(skip)]
    pub form_only: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Form {
    pub form: String,
    pub tags: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Sense {
    /// Translations or (for X→English Wiktionary data) English glosses.
    pub translations: Vec<String>,
    /// Qualifier / domain / sense description, shown in italics.
    pub note: Option<String>,
    pub examples: Vec<Example>,
    pub synonyms: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Example {
    pub text: String,
    pub translation: Option<String>,
}

impl Entry {
    pub fn new(headword: impl Into<String>, pos: impl Into<String>, source: impl Into<String>) -> Self {
        Entry {
            headword: headword.into(),
            pos: pos.into(),
            source: source.into(),
            ..Default::default()
        }
    }
}

/// Human friendly part-of-speech label.
pub fn pos_label(pos: &str) -> &str {
    match pos {
        "noun" => "noun",
        "verb" => "verb",
        "adj" => "adjective",
        "adv" => "adverb",
        "pron" => "pronoun",
        "num" => "numeral",
        "prep" => "preposition",
        "conj" => "conjunction",
        "intj" => "interjection",
        "particle" => "particle",
        "name" => "proper noun",
        "phrase" => "phrase",
        "det" => "determiner",
        "abbrev" => "abbreviation",
        other => other,
    }
}
