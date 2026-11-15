//! Reader for the Open English WordNet JSON release
//! (`english-wordnet-<year>-json.zip`, unpacked): `entries-*.json` map
//! lemma → part of speech → senses (synset ids), the other `*.json` files
//! (`noun.animal.json`, …) map synset id → synset.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dict_core::Entry;
use dict_core::wordnet::{SOURCE, ipa_list, pos_name, sense};
use serde_json::Value;

pub const LICENSE: &str =
    "Open English WordNet (https://en-word.net), CC BY 4.0; based on Princeton WordNet 3.0 (WordNet License).";

/// The unpacked release: `dir` itself or a sub-directory containing `entries-a.json`.
pub fn find(dir: &Path) -> Option<PathBuf> {
    if dir.join("entries-a.json").exists() {
        return Some(dir.to_owned());
    }
    std::fs::read_dir(dir).ok()?.flatten().map(|d| d.path()).find(|p| p.join("entries-a.json").exists())
}

fn read_json(path: &Path) -> Result<serde_json::Map<String, Value>> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    match serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))? {
        Value::Object(m) => Ok(m),
        _ => anyhow::bail!("{}: expected a JSON object", path.display()),
    }
}

pub fn load(dir: &Path) -> Result<Vec<Entry>> {
    eprintln!("reading WordNet from {}", dir.display());
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|d| d.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();

    // synset id → (lexname, synset)
    let mut synsets: HashMap<String, (String, Value)> = HashMap::new();
    let mut lemma_files = Vec::new();
    for f in files {
        let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_owned();
        if stem.starts_with("entries-") {
            lemma_files.push(f);
        } else if stem.contains('.') {
            for (id, s) in read_json(&f)? {
                synsets.insert(id, (stem.clone(), s));
            }
        }
    }

    let mut out = Vec::new();
    for f in lemma_files {
        for (lemma, by_pos) in read_json(&f)? {
            let Value::Object(by_pos) = by_pos else { continue };
            // Adjectives come as "a" and satellite "s": merge them.
            let mut merged: Vec<(&str, Entry)> = Vec::new();
            for (code, data) in &by_pos {
                let pos = pos_name(code);
                let i = match merged.iter().position(|(p, _)| *p == pos) {
                    Some(i) => i,
                    None => {
                        merged.push((pos, Entry::new(lemma.clone(), pos, SOURCE)));
                        merged.len() - 1
                    }
                };
                let e = &mut merged[i].1;
                if e.ipa.is_empty() {
                    e.ipa = ipa_list(data.get("pronunciation"));
                }
                for form in data.get("form").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                    if !e.index_forms.iter().any(|x| x == form) {
                        e.index_forms.push(form.to_owned());
                    }
                }
                for s in data.get("sense").and_then(Value::as_array).into_iter().flatten() {
                    let Some(id) = s.get("synset").and_then(Value::as_str) else { continue };
                    let Some((lex, syn)) = synsets.get(id) else { continue };
                    if let Some(sense) = sense(&lemma, syn, Some(lex)) {
                        e.senses.push(sense);
                    }
                }
            }
            merged.sort_by_key(|(p, _)| ["noun", "verb", "adj", "adv"].iter().position(|x| x == p));
            out.extend(merged.into_iter().map(|(_, e)| e).filter(|e| !e.senses.is_empty()));
        }
    }
    eprintln!("  {} WordNet entries from {} synsets", out.len(), synsets.len());
    Ok(out)
}
