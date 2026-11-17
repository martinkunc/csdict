//! Conversion of Wiktionary data extracted by wiktextract (kaikki.org JSONL)
//! into [`Entry`] values. Used both by the offline builder and by the app's
//! online lookup (kaikki.org serves the same JSON per word).

use serde_json::Value;

use crate::model::{Entry, Example, Sense};

pub const SOURCE: &str = "Wiktionary";

/// kaikki.org language names used in URLs, by ISO code.
pub fn language_name(code: &str) -> Option<&'static str> {
    Some(match code {
        "cs" => "Czech",
        "en" => "English",
        "de" => "German",
        "grc" => "Ancient Greek",
        "el" => "Greek",
        "sk" => "Slovak",
        "pl" => "Polish",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        "la" => "Latin",
        "ru" => "Russian",
        "pt" => "Portuguese",
        "ja" => "Japanese",
        "ar" => "Arabic",
        "zh" => "Chinese",
        "hi" => "Hindi",
        "ko" => "Korean",
        _ => return None,
    })
}

/// File name stem of the kaikki.org full dump of a language, e.g.
/// `kaikki.org-dictionary-AncientGreek` (spaces are dropped).
pub fn dump_stem(code: &str) -> Option<String> {
    Some(format!("kaikki.org-dictionary-{}", language_name(code)?.replace(' ', "")))
}

/// URL of the kaikki.org per-word JSONL page for `word` in language `code`.
pub fn word_url(code: &str, word: &str) -> Option<String> {
    let lang = language_name(code)?;
    let mut chars = word.chars();
    let c1: String = chars.next()?.to_string();
    let c2: String = word.chars().take(2).collect();
    let enc = |s: &str| urlencoding::encode(s).into_owned();
    Some(format!(
        "https://kaikki.org/dictionary/{}/meaning/{}/{}/{}.jsonl",
        enc(lang),
        enc(&c1),
        enc(&c2),
        enc(word)
    ))
}

const SKIP_TAGS: &[&str] = &[
    "form-of", "alt-of", "no-gloss", "table-tags", "inflection-template", "class",
    "error-unknown-tag", "morpheme", "canonical",
];

fn str_list<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a str> {
    v.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

fn obj_list<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v.get(key).and_then(Value::as_array).into_iter().flatten()
}

fn push_unique(v: &mut Vec<String>, s: &str, max: usize) {
    let s = s.trim();
    if !s.is_empty() && v.len() < max && !v.iter().any(|x| x == s) {
        v.push(s.to_owned());
    }
}

/// Pronunciation data (IPA, audio) and hyphenation of a kaikki entry.
fn fill_sounds(v: &Value, e: &mut Entry) {
    for s in obj_list(v, "sounds") {
        if let Some(ipa) = s.get("ipa").and_then(Value::as_str) {
            let tags: Vec<&str> = str_list(s, "tags").collect();
            let ipa = if tags.is_empty() { ipa.to_owned() } else { format!("{ipa} ({})", tags.join(", ").replace('-', " ")) };
            push_unique(&mut e.ipa, &ipa, 3);
        }
        let url = s
            .get("mp3_url")
            .or_else(|| s.get("ogg_url"))
            .and_then(Value::as_str);
        if let Some(url) = url {
            push_unique(&mut e.audio, url, 3);
        }
    }
    // Readings that can be typed on a Latin keyboard become lookup keys:
    // Mandarin pinyin (`gǒu (gou³)` → `gǒu`).
    for s in obj_list(v, "sounds") {
        let tags: Vec<&str> = str_list(s, "tags").collect();
        if tags.contains(&"Pinyin") && tags.contains(&"Standard")
            && let Some(p) = s.get("zh_pron").and_then(Value::as_str)
        {
            let p = p.split(" (").next().unwrap_or(p);
            push_unique(&mut e.index_forms, p, 400);
        }
    }
    if let Some(h) = obj_list(v, "hyphenations").next() {
        let parts: Vec<&str> = str_list(h, "parts").collect();
        if parts.len() > 1 {
            e.hyphenation = Some(parts.join("‧"));
        }
    }
}

/// The head line, e.g. `pes m anim (diminutive pejsek)` → `m anim (diminutive pejsek)`.
fn head_grammar(v: &Value, word: &str) -> Option<String> {
    let exp = obj_list(v, "head_templates")
        .next()?
        .get("expansion")?
        .as_str()?
        .trim();
    let rest = exp.strip_prefix(word).unwrap_or(exp).trim();
    (!rest.is_empty()).then(|| rest.to_owned())
}

fn etymology(v: &Value) -> Option<String> {
    let t = v.get("etymology_text")?.as_str()?.trim();
    let t = if t.starts_with("Etymology tree") {
        t.rsplit('\n').next().unwrap_or(t)
    } else {
        t
    };
    (!t.is_empty()).then(|| t.to_owned())
}

fn sense_note(s: &Value) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for t in str_list(s, "raw_tags").chain(str_list(s, "tags")) {
        if !SKIP_TAGS.contains(&t) && !parts.iter().any(|p| p == t) {
            parts.push(t.replace('-', " "));
        }
    }
    for t in str_list(s, "topics") {
        if parts.len() < 6 && !parts.iter().any(|p| p == t) {
            parts.push(t.replace('-', " "));
        }
    }
    if let Some(q) = s.get("qualifier").and_then(Value::as_str) {
        parts.push(q.to_owned());
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// Convert one kaikki entry of a non-English (or English) word into an
/// [`Entry`] whose sense translations are the English glosses.
pub fn entry_from_kaikki(v: &Value) -> Option<Entry> {
    let word = v.get("word")?.as_str()?;
    let pos = v.get("pos").and_then(Value::as_str).unwrap_or("");
    let mut e = Entry::new(word, pos, SOURCE);
    if let Some(g) = head_grammar(v, word) {
        e.grammar.push(g);
    }
    fill_sounds(v, &mut e);
    e.etymology = etymology(v);

    for f in obj_list(v, "forms") {
        // Japanese kana reading of the headword (ruby: [[kanji, kana], …]).
        let ruby: String = obj_list(f, "ruby")
            .filter_map(|r| r.get(1).and_then(Value::as_str))
            .collect();
        if !ruby.is_empty() && ruby != word {
            push_unique(&mut e.index_forms, &ruby, 400);
        }
        let tags: Vec<&str> = str_list(f, "tags").collect();
        if tags.iter().any(|t| (SKIP_TAGS.contains(t) && *t != "canonical") || *t == "counter") {
            continue;
        }
        if let Some(form) = f.get("form").and_then(Value::as_str)
            && form != word
            && form != "-"
            && form != "—"
        {
            push_unique(&mut e.index_forms, form, 400);
        }
    }

    let mut all_form_of = true;
    for s in obj_list(v, "senses") {
        all_form_of &= s.get("form_of").is_some();
        let glosses: Vec<&str> = str_list(s, "glosses").collect();
        let Some(gloss) = glosses.last() else { continue };
        let mut sense = Sense {
            translations: vec![(*gloss).to_owned()],
            note: sense_note(s),
            ..Default::default()
        };
        for ex in obj_list(s, "examples").take(3) {
            if let Some(text) = ex.get("text").and_then(Value::as_str) {
                let tr = ex
                    .get("translation")
                    .or_else(|| ex.get("english"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                sense.examples.push(Example { text: text.to_owned(), translation: tr });
            }
        }
        for syn in obj_list(s, "synonyms") {
            if let Some(w) = syn.get("word").and_then(Value::as_str) {
                push_unique(&mut sense.synonyms, w, 8);
            }
        }
        e.senses.push(sense);
    }
    e.form_only = all_form_of;
    (!e.senses.is_empty()).then_some(e)
}

/// Abbreviation of grammatical tags shown next to a translation.
fn short_tags(t: &Value) -> String {
    let mut out = Vec::new();
    for tag in str_list(t, "tags") {
        let s = match tag {
            "masculine" => "m",
            "feminine" => "f",
            "neuter" => "n",
            "animate" => "anim",
            "inanimate" => "inan",
            "perfective" => "pf",
            "imperfective" => "impf",
            "plural" => "pl",
            _ => continue,
        };
        out.push(s);
    }
    out.join(" ")
}

/// Does a translation's language code belong to `lang`? Wiktionary files
/// most Chinese translations under Mandarin (`cmn`, sometimes `zh`).
fn translation_code_matches(code: &str, lang: &str) -> bool {
    code == lang || (lang == "zh" && code == "cmn")
}

/// A group of translations of an English word sharing one sense.
#[derive(Debug, Clone, PartialEq)]
pub struct TranslationGroup {
    pub sense: Option<String>,
    /// `(word, short grammatical tags)`
    pub words: Vec<(String, String)>,
}

/// Translations of an English kaikki entry into `lang`, grouped by sense.
pub fn english_translation_groups(v: &Value, lang: &str) -> Vec<TranslationGroup> {
    let mut groups: Vec<TranslationGroup> = Vec::new();
    let all = obj_list(v, "translations")
        .chain(obj_list(v, "senses").flat_map(|s| obj_list(s, "translations")));
    for t in all {
        let code = t
            .get("lang_code")
            .or_else(|| t.get("code"))
            .and_then(Value::as_str);
        if !code.is_some_and(|c| translation_code_matches(c, lang)) {
            continue;
        }
        let Some(word) = t.get("word").and_then(Value::as_str) else { continue };
        let sense = t
            .get("sense")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let item = (word.trim().to_owned(), short_tags(t));
        match groups.iter_mut().find(|g| g.sense == sense) {
            Some(g) => {
                if !g.words.iter().any(|w| w.0 == item.0) {
                    g.words.push(item);
                }
            }
            None => groups.push(TranslationGroup { sense, words: vec![item] }),
        }
    }
    groups
}

/// Format a translated word with its tags, e.g. `žlutá (f)`.
pub fn format_word(word: &str, tags: &str) -> String {
    if tags.is_empty() { word.to_owned() } else { format!("{word} ({tags})") }
}

/// Build an English→`lang` entry from an English kaikki entry, using its
/// translation tables. Returns `None` when there are no translations.
pub fn english_to_lang_entry(v: &Value, lang: &str) -> Option<Entry> {
    let word = v.get("word")?.as_str()?;
    let pos = v.get("pos").and_then(Value::as_str).unwrap_or("");
    let groups = english_translation_groups(v, lang);
    if groups.is_empty() {
        return None;
    }
    let mut e = Entry::new(word, pos, SOURCE);
    fill_sounds(v, &mut e);
    for g in groups {
        e.senses.push(Sense {
            translations: g.words.iter().map(|(w, t)| format_word(w, t)).collect(),
            note: g.sense,
            ..Default::default()
        });
    }
    for f in obj_list(v, "forms") {
        if let Some(form) = f.get("form").and_then(Value::as_str)
            && form != word
        {
            push_unique(&mut e.index_forms, form, 50);
        }
    }
    Some(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_czech_entry() {
        let v: Value = serde_json::from_str(
            r#"{"word":"žlutý","pos":"adj","lang_code":"cs",
                "head_templates":[{"expansion":"žlutý (comparative žlutější)"}],
                "sounds":[{"ipa":"[ˈʒlutiː]"},{"mp3_url":"https://x/a.mp3","ogg_url":"https://x/a.ogg"}],
                "hyphenations":[{"parts":["žlu","tý"]}],
                "forms":[{"form":"žlutější","tags":["comparative"]},
                         {"form":"cs-adecl","tags":["inflection-template"]},
                         {"form":"žlutou","tags":["accusative","feminine"]}],
                "senses":[{"glosses":["yellow"],"examples":[{"text":"Je žlutá.","translation":"It is yellow."}]}]}"#,
        )
        .unwrap();
        let e = entry_from_kaikki(&v).unwrap();
        assert_eq!(e.headword, "žlutý");
        assert_eq!(e.grammar, vec!["(comparative žlutější)"]);
        assert_eq!(e.ipa, vec!["[ˈʒlutiː]"]);
        assert_eq!(e.audio, vec!["https://x/a.mp3"]);
        assert_eq!(e.hyphenation.as_deref(), Some("žlu‧tý"));
        assert_eq!(e.index_forms, vec!["žlutější", "žlutou"]);
        assert_eq!(e.senses[0].translations, vec!["yellow"]);
        assert_eq!(e.senses[0].examples[0].translation.as_deref(), Some("It is yellow."));
    }

    #[test]
    fn groups_english_translations() {
        let v: Value = serde_json::from_str(
            r#"{"word":"dog","pos":"noun","translations":[
                {"lang_code":"cs","word":"pes","sense":"animal","tags":["masculine","animate"]},
                {"lang_code":"de","word":"Hund","sense":"animal","tags":["masculine"]},
                {"lang_code":"cs","word":"fena","sense":"female dog","tags":["feminine"]}]}"#,
        )
        .unwrap();
        let e = english_to_lang_entry(&v, "cs").unwrap();
        assert_eq!(e.senses.len(), 2);
        assert_eq!(e.senses[0].translations, vec!["pes (m anim)"]);
        assert_eq!(e.senses[0].note.as_deref(), Some("animal"));
        assert!(english_to_lang_entry(&v, "fr").is_none());
        let zh: Value = serde_json::from_str(
            r#"{"word":"dog","pos":"noun","translations":[{"lang_code":"cmn","word":"狗","sense":"animal"}]}"#,
        )
        .unwrap();
        assert_eq!(english_to_lang_entry(&zh, "zh").unwrap().senses[0].translations, vec!["狗"]);
    }

    #[test]
    fn indexes_readings() {
        let zh: Value = serde_json::from_str(
            r#"{"word":"狗","pos":"noun","sounds":[
                {"zh_pron":"gǒu (gou³)","tags":["Mandarin","Standard","Pinyin"]},
                {"zh_pron":"gau²","tags":["Cantonese","Jyutping"]}],
                "senses":[{"glosses":["dog"]}]}"#,
        )
        .unwrap();
        assert_eq!(entry_from_kaikki(&zh).unwrap().index_forms, vec!["gǒu"]);
        let ja: Value = serde_json::from_str(
            r#"{"word":"犬","pos":"noun","forms":[
                {"form":"犬","ruby":[["犬","いぬ"]],"tags":["canonical"]},
                {"form":"inu","tags":["romanization"]},
                {"form":"匹","tags":["counter"]}],
                "senses":[{"glosses":["dog"]}]}"#,
        )
        .unwrap();
        assert_eq!(entry_from_kaikki(&ja).unwrap().index_forms, vec!["いぬ", "inu"]);
        assert_eq!(dump_stem("grc").unwrap(), "kaikki.org-dictionary-AncientGreek");
    }

    #[test]
    fn builds_urls() {
        assert_eq!(
            word_url("cs", "žlutý").unwrap(),
            "https://kaikki.org/dictionary/Czech/meaning/%C5%BE/%C5%BEl/%C5%BElut%C3%BD.jsonl"
        );
        assert_eq!(
            word_url("grc", "λόγος").unwrap(),
            "https://kaikki.org/dictionary/Ancient%20Greek/meaning/%CE%BB/%CE%BB%CF%8C/%CE%BB%CF%8C%CE%B3%CE%BF%CF%82.jsonl"
        );
    }
}
