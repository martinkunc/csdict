//! Conversion of Open English WordNet data (https://en-word.net) into
//! [`Entry`] values. Used both by the offline builder (release JSON) and by
//! the app's online lookup (`https://en-word.net/api/lemma/<word>`); both
//! describe synsets with `definition`, `example`, `members` and
//! `partOfSpeech`.

use serde_json::Value;

use crate::model::{Entry, Example, Sense};

pub const SOURCE: &str = "WordNet";

/// Online lookup URL of `lemma`.
pub fn api_url(lemma: &str) -> String {
    format!("https://en-word.net/api/lemma/{}", urlencoding::encode(lemma))
}

/// WordNet part-of-speech letter → the short form used in [`Entry::pos`].
pub fn pos_name(code: &str) -> &'static str {
    match code {
        "n" => "noun",
        "v" => "verb",
        "a" | "s" => "adj",
        "r" => "adv",
        _ => "",
    }
}

/// `[{"value": "dɒɡ", "variety": "GB"}]` → `["/dɒɡ/ (GB)"]`.
pub fn ipa_list(pron: Option<&Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in pron.and_then(Value::as_array).into_iter().flatten() {
        let Some(v) = p.get("value").and_then(Value::as_str) else { continue };
        let s = match p.get("variety").and_then(Value::as_str) {
            Some(var) => format!("/{v}/ ({var})"),
            None => format!("/{v}/"),
        };
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Member lemmas of a synset (strings in the dump, objects in the API).
fn members(synset: &Value) -> impl Iterator<Item = &str> {
    synset
        .get("members")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.as_str().or_else(|| m.get("lemma").and_then(Value::as_str)))
}

/// One sense of `lemma` from its synset. `lexname` (e.g. `verb.communication`)
/// becomes the note (`communication`).
pub fn sense(lemma: &str, synset: &Value, lexname: Option<&str>) -> Option<Sense> {
    let defs: Vec<&str> = synset.get("definition")?.as_array()?.iter().filter_map(Value::as_str).collect();
    if defs.is_empty() {
        return None;
    }
    let examples = synset
        .get("example")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().or_else(|| x.get("text").and_then(Value::as_str)))
        .take(3)
        .map(|t| Example { text: t.to_owned(), translation: None })
        .collect();
    let note = lexname
        .and_then(|l| l.split_once('.'))
        .map(|(_, d)| d.to_owned())
        .filter(|d| d != "all" && d != "Tops");
    Some(Sense {
        translations: vec![defs.join("; ")],
        note,
        examples,
        synonyms: members(synset).filter(|m| *m != lemma).map(str::to_owned).collect(),
    })
}

/// Entries (one per part of speech) from the en-word.net API response for
/// `lemma`: an array of synsets.
pub fn entries_from_api(lemma: &str, synsets: &Value) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for s in synsets.as_array().into_iter().flatten() {
        let pos = pos_name(s.get("partOfSpeech").and_then(Value::as_str).unwrap_or(""));
        let Some(sense) = sense(lemma, s, s.get("lexname").and_then(Value::as_str)) else { continue };
        let e = match out.iter().position(|e| e.pos == pos) {
            Some(i) => &mut out[i],
            None => {
                let mut e = Entry::new(lemma, pos, SOURCE);
                // Pronunciation is given per member.
                if let Some(m) = s
                    .get("members")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .find(|m| m.get("lemma").and_then(Value::as_str) == Some(lemma))
                {
                    e.ipa = ipa_list(m.get("pronunciation"));
                }
                out.push(e);
                out.last_mut().unwrap()
            }
        };
        e.senses.push(sense);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_api_response() {
        let v: Value = serde_json::from_str(
            r#"[{"id":"1-v","lexname":"verb.communication","partOfSpeech":"v",
                 "members":[{"lemma":"solicit","pronunciation":[{"value":"səˈlɪsɪt"}]},{"lemma":"beg"}],
                 "definition":["make a petition"],"example":["She is too shy to solicit"]},
                {"id":"2-n","lexname":"noun.Tops","partOfSpeech":"n",
                 "members":[{"lemma":"solicit"}],"definition":["a thing"]}]"#,
        )
        .unwrap();
        let e = entries_from_api("solicit", &v);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].pos, "verb");
        assert_eq!(e[0].ipa, ["/səˈlɪsɪt/"]);
        assert_eq!(e[0].senses[0].translations, ["make a petition"]);
        assert_eq!(e[0].senses[0].note.as_deref(), Some("communication"));
        assert_eq!(e[0].senses[0].synonyms, ["beg"]);
        assert_eq!(e[0].senses[0].examples[0].text, "She is too shy to solicit");
        assert_eq!(e[1].senses[0].note, None);
    }
}
