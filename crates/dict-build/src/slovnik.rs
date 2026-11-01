//! Parser for the GNU/FDL slovník text format
//! (`left \t right \t grammar \t notes \t author`, `#` comments).

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use dict_core::{Entry, Sense};

pub const SOURCE: &str = "GNU/FDL";

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub left: String,
    pub right: String,
    pub pos: String,
    /// Domain tags and notes, e.g. `[tech.] pl.; přeneseně`.
    pub note: Option<String>,
    /// Row lists irregular forms (`get/got/gotten`) rather than a translation.
    pub irregular: bool,
    /// German article (`der`/`die`/`das`) of the left word, if given.
    pub article: Option<String>,
}

fn map_pos(tok: &str) -> Option<&'static str> {
    Some(match tok {
        "n:" => "noun",
        "v:" => "verb",
        "adj:" => "adj",
        "adv:" => "adv",
        "prep:" => "prep",
        "conj:" => "conj",
        "pron:" => "pron",
        "interj:" => "intj",
        "num:" => "num",
        _ => return None,
    })
}

pub fn parse_line(line: &str) -> Option<Row> {
    if line.starts_with('#') || line.trim().is_empty() {
        return None;
    }
    let cols: Vec<&str> = line.trim_end_matches(['\n', '\r']).split('\t').collect();
    let left = cols.first()?.trim();
    let right = cols.get(1)?.trim();
    if left.is_empty() || right.is_empty() {
        return None;
    }
    let mut pos = "";
    let mut tags = Vec::new();
    let mut article = None;
    for tok in cols.get(2).copied().unwrap_or("").split_whitespace() {
        match map_pos(tok) {
            Some(p) => {
                if pos.is_empty() || pos == "noun" {
                    pos = p;
                }
            }
            None if matches!(tok, "der" | "die" | "das") => {
                article = Some(tok.to_owned());
                if pos.is_empty() {
                    pos = "noun";
                }
            }
            _ => tags.push(tok),
        }
    }
    let irregular = tags.contains(&"[neprav.]") && right.contains('/');
    let mut note = tags.join(" ");
    // Column 4 holds notes separated by `;`; `pre2005`/`post2005` mark the
    // German spelling reform and are not interesting.
    for extra in cols.get(3).copied().unwrap_or("").split(';').map(str::trim) {
        if extra.is_empty() || extra == "pre2005" || extra == "post2005" {
            continue;
        }
        if !note.is_empty() {
            note.push_str("; ");
        }
        note.push_str(extra);
    }
    Some(Row {
        left: left.to_owned(),
        right: right.to_owned(),
        pos: pos.to_owned(),
        note: (!note.is_empty()).then_some(note),
        irregular,
        article,
    })
}

pub fn parse(path: &Path) -> Result<Vec<Row>> {
    let text = std::fs::read_to_string(path)?;
    Ok(text.lines().filter_map(parse_line).collect())
}

/// Group rows into entries. `reverse = false` gives left→right entries
/// (en→cs), `true` gives right→left (cs→en).
pub fn group(rows: &[Row], reverse: bool) -> Vec<Entry> {
    let mut map: HashMap<(String, String), Entry> = HashMap::new();
    let mut order: Vec<(String, String)> = Vec::new();
    for r in rows {
        if r.irregular && reverse {
            continue;
        }
        let (head, tr) = if reverse { (&r.right, &r.left) } else { (&r.left, &r.right) };
        let key = (head.clone(), r.pos.clone());
        let e = map.entry(key.clone()).or_insert_with(|| {
            order.push(key);
            Entry::new(head.clone(), r.pos.clone(), SOURCE)
        });
        if !reverse
            && let Some(a) = &r.article
            && !e.grammar.contains(a)
        {
            e.grammar.push(a.clone());
        }
        if r.irregular {
            if !e.grammar.contains(tr) {
                e.grammar.push(tr.clone());
                e.index_forms.extend(tr.split('/').map(str::to_owned));
            }
            continue;
        }
        match e.senses.iter_mut().find(|s| s.note == r.note) {
            Some(s) => {
                if !s.translations.contains(tr) {
                    s.translations.push(tr.clone());
                }
            }
            None => e.senses.push(Sense { translations: vec![tr.clone()], note: r.note.clone(), ..Default::default() }),
        }
    }
    order
        .into_iter()
        .filter_map(|k| map.remove(&k))
        .map(|mut e| {
            // Plain (un-annotated) senses first.
            e.senses.sort_by_key(|s| s.note.is_some());
            e
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rows() {
        let r = parse_line("dog\tpes\tn: [zoo.]\t\tweb\n").unwrap();
        assert_eq!(r.left, "dog");
        assert_eq!(r.right, "pes");
        assert_eq!(r.pos, "noun");
        assert_eq!(r.note.as_deref(), Some("[zoo.]"));
        assert!(parse_line("# comment").is_none());
        let r = parse_line("get\tget/got/gotten\tv: [neprav.]\t\tx").unwrap();
        assert!(r.irregular);
        let r = parse_line("Aal\túhoř\tder\tpl. Aale; post2005\trs").unwrap();
        assert_eq!(r.article.as_deref(), Some("der"));
        assert_eq!(r.pos, "noun");
        assert_eq!(r.note.as_deref(), Some("pl. Aale"));
    }

    #[test]
    fn groups_both_directions() {
        let rows: Vec<Row> = [
            "dog\tpes\tn:\t\t",
            "dog\tskoba\tn:\t\t",
            "dog\tpronásledovat\tv:\t\t",
            "hound\tpes\tn:\t\t",
            "get\tget/got/gotten\tv: [neprav.]\t\t",
            "get\tdostat\tv:\t\t",
        ]
        .iter()
        .filter_map(|l| parse_line(l))
        .collect();
        let fwd = group(&rows, false);
        assert_eq!(fwd.len(), 4);
        assert_eq!(fwd[0].senses[0].translations, vec!["pes", "skoba"]);
        let get = fwd.iter().find(|e| e.headword == "get").unwrap();
        assert_eq!(get.grammar, vec!["get/got/gotten"]);
        assert!(get.index_forms.contains(&"gotten".to_string()));
        let rev = group(&rows, true);
        let pes = rev.iter().find(|e| e.headword == "pes").unwrap();
        assert_eq!(pes.senses[0].translations, vec!["dog", "hound"]);
    }
}
