//! `dict-build` – turns raw Wiktionary (kaikki.org) and GNU/FDL slovník
//! dumps into `.csd` packs plus a `catalog.json` for the app.
//!
//! Expected layout of the raw directory (any file may be missing; packs
//! whose inputs are missing are skipped):
//!
//! ```text
//! raw/kaikki-cs/kaikki.org-dictionary-Czech.jsonl[.gz]
//! raw/kaikki-de/kaikki.org-dictionary-German.jsonl[.gz]
//! raw/kaikki-grc/kaikki.org-dictionary-AncientGreek.jsonl[.gz]
//! raw/kaikki-en/kaikki.org-dictionary-English.jsonl[.gz]
//! raw/slovnik/*/en-cs.txt
//! raw/slovnik/*/de-cs.txt
//! ```

mod slovnik;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dict_core::kaikki::{self, english_translation_groups, format_word};
use dict_core::normalize::fold;
use dict_core::{DictMeta, Dictionary, Entry, PackBuilder, Sense};
use serde_json::Value;

#[derive(Parser)]
#[command(about = "Build csdict dictionary packs")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build all packs whose inputs exist and write catalog.json.
    All {
        #[arg(long, default_value = "data/raw")]
        raw: PathBuf,
        #[arg(long, default_value = "dist")]
        out: PathBuf,
        /// Base URL where the packs will be hosted (written to catalog.json).
        #[arg(long, default_value = "")]
        base_url: String,
    },
    /// Look up a word in a pack (for testing).
    Lookup { pack: PathBuf, word: String },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::All { raw, out, base_url } => build_all(&raw, &out, &base_url),
        Cmd::Lookup { pack, word } => lookup(&pack, &word),
    }
}

fn lookup(pack: &Path, word: &str) -> Result<()> {
    let d = Dictionary::open(pack)?;
    println!("{} – {} entries, {} keys", d.meta().name, d.len(), d.key_count());
    let t = Instant::now();
    let hits = d.search(word, 20);
    let el = t.elapsed();
    for h in &hits {
        println!("  {:?} {} ({}) rank {}", h.kind, h.headword, h.pos, h.rank);
    }
    println!("search took {el:?}");
    if let Some(h) = hits.first() {
        println!("{:#?}", d.entry(h.entry_id)?);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Input helpers
// ---------------------------------------------------------------------------

fn find_input(dir: &Path, stem: &str) -> Option<PathBuf> {
    [format!("{stem}.jsonl.gz"), format!("{stem}.jsonl")]
        .into_iter()
        .map(|n| dir.join(n))
        .find(|p| p.exists())
}

fn find_glob(dir: &Path, file: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|d| {
        let p = d.path().join(file);
        p.exists().then_some(p)
    })
}

/// Stream lines of a (possibly gzipped) JSONL file.
fn for_each_line(path: &Path, mut f: impl FnMut(&str)) -> Result<usize> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader: Box<dyn Read> = if path.extension().is_some_and(|e| e == "gz") {
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut reader = BufReader::with_capacity(1 << 20, reader);
    let mut line = String::new();
    let mut n = 0;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        n += 1;
        if n % 200_000 == 0 {
            eprint!("\r  {n} lines…");
        }
        f(&line);
    }
    eprintln!("\r  {n} lines   ");
    Ok(n)
}

/// Pronunciation and inflection data of a word, used to enrich entries that
/// come from sources without it (slovník, pivot translations).
#[derive(Default, Clone)]
struct Extra {
    ipa: Vec<String>,
    audio: Vec<String>,
    hyphenation: Option<String>,
    forms: Vec<String>,
}

/// word → [(pos, extra)]
#[derive(Default)]
struct ExtraMap(HashMap<String, Vec<(String, Extra)>>);

impl ExtraMap {
    fn add(&mut self, e: &Entry) {
        if e.form_only {
            return;
        }
        let x = Extra {
            ipa: e.ipa.clone(),
            audio: e.audio.clone(),
            hyphenation: e.hyphenation.clone(),
            forms: e.index_forms.clone(),
        };
        self.0.entry(e.headword.clone()).or_default().push((e.pos.clone(), x));
    }

    fn get(&self, word: &str, pos: &str) -> Option<&Extra> {
        let list = self.0.get(word)?;
        list.iter()
            .find(|(p, _)| p == pos)
            .or_else(|| list.first())
            .map(|(_, x)| x)
    }

    fn enrich(&self, e: &mut Entry) {
        let Some(x) = self.get(&e.headword, &e.pos) else { return };
        if e.ipa.is_empty() {
            e.ipa = x.ipa.clone();
        }
        if e.audio.is_empty() {
            e.audio = x.audio.clone();
        }
        if e.hyphenation.is_none() {
            e.hyphenation = x.hyphenation.clone();
        }
        if e.index_forms.is_empty() {
            e.index_forms = x.forms.clone();
        }
    }

    /// All inflected forms of all words (to drop redundant form-only entries).
    fn all_forms(&self) -> HashSet<String> {
        self.0
            .values()
            .flatten()
            .flat_map(|(_, x)| x.forms.iter().map(|f| fold(f)))
            .collect()
    }
}

/// Entries of one language from its kaikki dump.
struct LangData {
    entries: Vec<Entry>,
    extra: ExtraMap,
}

fn load_kaikki_lang(path: &Path) -> Result<LangData> {
    eprintln!("reading {}", path.display());
    let mut entries = Vec::new();
    let mut extra = ExtraMap::default();
    for_each_line(path, |line| {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return };
        if let Some(e) = kaikki::entry_from_kaikki(&v) {
            extra.add(&e);
            entries.push(e);
        }
    })?;
    // Drop "inflection of X" entries already reachable through X's forms.
    let forms = extra.all_forms();
    let before = entries.len();
    entries.retain(|e| !(e.form_only && forms.contains(&fold(&e.headword))));
    eprintln!("  {} entries ({} redundant form entries dropped)", entries.len(), before - entries.len());
    Ok(LangData { entries, extra })
}

// ---------------------------------------------------------------------------
// English dump: en→cs entries and cs↔de / cs↔grc pivots
// ---------------------------------------------------------------------------

const PIVOT_LANGS: &[&str] = &["cs", "de", "grc"];
const PIVOT_PAIRS: &[(&str, &str)] = &[("cs", "de"), ("de", "cs"), ("cs", "grc"), ("grc", "cs")];

#[derive(Default)]
struct EnglishData {
    en_cs: Vec<Entry>,
    /// (src, tgt) → entries keyed by (word, pos)
    pivots: HashMap<(String, String), HashMap<(String, String), Entry>>,
    extra: ExtraMap,
}

fn load_english(path: &Path, want_extra: &HashSet<String>) -> Result<EnglishData> {
    eprintln!("reading {} (translations)", path.display());
    let mut d = EnglishData::default();
    for_each_line(path, |line| {
        let has_translations = line.contains("\"translations\"");
        if !has_translations && !want_extra.is_empty() {
            // Only parse lines we may need for pronunciation data.
            let Some(w) = line.find("\"word\": \"").map(|i| &line[i + 9..]) else { return };
            let w = w.split('"').next().unwrap_or("");
            if !want_extra.contains(w) {
                return;
            }
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { return };
        let Some(word) = v.get("word").and_then(Value::as_str) else { return };
        let pos = v.get("pos").and_then(Value::as_str).unwrap_or("").to_owned();

        if want_extra.contains(word)
            && let Some(e) = kaikki::entry_from_kaikki(&v)
        {
            d.extra.add(&e);
        }
        if !has_translations {
            return;
        }
        if let Some(e) = kaikki::english_to_lang_entry(&v, "cs") {
            d.en_cs.push(e);
        }

        let groups: HashMap<&str, Vec<kaikki::TranslationGroup>> = PIVOT_LANGS
            .iter()
            .map(|l| (*l, english_translation_groups(&v, l)))
            .collect();
        for (src, tgt) in PIVOT_PAIRS {
            let (gs, gt) = (&groups[src], &groups[tgt]);
            if gs.is_empty() || gt.is_empty() {
                continue;
            }
            let map = d.pivots.entry((src.to_string(), tgt.to_string())).or_default();
            for s in gs {
                // Translations under the same sense box line up across languages.
                let Some(t) = gt.iter().find(|t| t.sense == s.sense) else { continue };
                let translations: Vec<String> = t.words.iter().map(|(w, tg)| format_word(w, tg)).collect();
                let note = match &s.sense {
                    Some(sense) => format!("via English “{word}”: {sense}"),
                    None => format!("via English “{word}”"),
                };
                for (sw, _) in &s.words {
                    let e = map
                        .entry((sw.clone(), pos.clone()))
                        .or_insert_with(|| Entry::new(sw.clone(), pos.clone(), "Wiktionary (via English)"));
                    if !e.senses.iter().any(|x| x.translations == translations) && e.senses.len() < 24 {
                        e.senses.push(Sense { translations: translations.clone(), note: Some(note.clone()), ..Default::default() });
                    }
                }
            }
        }
    })?;
    eprintln!("  en→cs: {} entries", d.en_cs.len());
    for (k, v) in &d.pivots {
        eprintln!("  pivot {}→{}: {} entries", k.0, k.1, v.len());
    }
    Ok(d)
}

// ---------------------------------------------------------------------------
// Pack assembly
// ---------------------------------------------------------------------------

const LICENSE_WIKTIONARY: &str = "Wiktionary content, CC BY-SA 4.0 (https://en.wiktionary.org), extracted by wiktextract / kaikki.org.";
const LICENSE_SLOVNIK: &str = "Založeno na svobodném anglicko-českém slovníku, https://www.svobodneslovniky.cz/ – GNU FDL 1.1+.";

fn lang_name(code: &str) -> &str {
    match code {
        "cs" => "Czech",
        "en" => "English",
        "de" => "German",
        "grc" => "Ancient Greek",
        other => other,
    }
}

struct PackSpec {
    src: &'static str,
    tgt: &'static str,
    entries: Vec<Entry>,
    sources: Vec<&'static str>,
    licenses: Vec<&'static str>,
}

impl PackSpec {
    fn new(src: &'static str, tgt: &'static str) -> Self {
        PackSpec { src, tgt, entries: Vec::new(), sources: Vec::new(), licenses: Vec::new() }
    }
    fn add(&mut self, entries: impl IntoIterator<Item = Entry>, source: &'static str, license: &'static str) {
        let before = self.entries.len();
        self.entries.extend(entries);
        if self.entries.len() > before {
            if !self.sources.contains(&source) {
                self.sources.push(source);
            }
            if !self.licenses.contains(&license) {
                self.licenses.push(license);
            }
        }
    }
}

fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Civil-from-days (Howard Hinnant).
    let z = secs / 86_400 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn build_all(raw: &Path, out: &Path, base_url: &str) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let started = Instant::now();

    let en_cs_txt = find_glob(&raw.join("slovnik"), "en-cs.txt");
    let de_cs_txt = find_glob(&raw.join("slovnik"), "de-cs.txt");
    let en_cs_rows = en_cs_txt.as_deref().map(slovnik::parse).transpose()?.unwrap_or_default();
    let de_cs_rows = de_cs_txt.as_deref().map(slovnik::parse).transpose()?.unwrap_or_default();
    eprintln!("slovník: {} en-cs rows, {} de-cs rows", en_cs_rows.len(), de_cs_rows.len());

    let load = |dir: &str, stem: &str| -> Result<Option<LangData>> {
        find_input(&raw.join(dir), stem).map(|p| load_kaikki_lang(&p)).transpose()
    };
    let cs = load("kaikki-cs", "kaikki.org-dictionary-Czech")?;
    let de = load("kaikki-de", "kaikki.org-dictionary-German")?;
    let grc = load("kaikki-grc", "kaikki.org-dictionary-AncientGreek")?;

    let want_en: HashSet<String> = en_cs_rows.iter().map(|r| r.left.clone()).collect();
    let en = find_input(&raw.join("kaikki-en"), "kaikki.org-dictionary-English")
        .map(|p| load_english(&p, &want_en))
        .transpose()?;

    let mut specs: Vec<PackSpec> = Vec::new();
    let take_pivot = |en: &mut Option<EnglishData>, s: &str, t: &str| -> Vec<Entry> {
        en.as_mut()
            .and_then(|d| d.pivots.remove(&(s.to_owned(), t.to_owned())))
            .map(|m| m.into_values().collect())
            .unwrap_or_default()
    };
    let mut en = en;

    // cs → en
    let mut p = PackSpec::new("cs", "en");
    if let Some(cs) = &cs {
        p.add(cs.entries.iter().cloned(), "Wiktionary", LICENSE_WIKTIONARY);
    }
    p.add(slovnik::group(&en_cs_rows, true), "GNU/FDL slovník", LICENSE_SLOVNIK);
    specs.push(p);

    // en → cs
    let mut p = PackSpec::new("en", "cs");
    if let Some(en) = &mut en {
        p.add(std::mem::take(&mut en.en_cs), "Wiktionary", LICENSE_WIKTIONARY);
    }
    p.add(slovnik::group(&en_cs_rows, false), "GNU/FDL slovník", LICENSE_SLOVNIK);
    specs.push(p);

    // de → en, grc → en
    for (code, data) in [("de", &de), ("grc", &grc)] {
        if let Some(d) = data {
            let mut p = PackSpec::new(code, "en");
            p.add(d.entries.iter().cloned(), "Wiktionary", LICENSE_WIKTIONARY);
            specs.push(p);
        }
    }

    // cs ↔ de, cs ↔ grc
    let mut p = PackSpec::new("de", "cs");
    p.add(slovnik::group(&de_cs_rows, false), "GNU/FDL slovník", LICENSE_SLOVNIK);
    p.add(take_pivot(&mut en, "de", "cs"), "Wiktionary (via English)", LICENSE_WIKTIONARY);
    specs.push(p);
    let mut p = PackSpec::new("cs", "de");
    p.add(slovnik::group(&de_cs_rows, true), "GNU/FDL slovník", LICENSE_SLOVNIK);
    p.add(take_pivot(&mut en, "cs", "de"), "Wiktionary (via English)", LICENSE_WIKTIONARY);
    specs.push(p);
    let mut p = PackSpec::new("grc", "cs");
    p.add(take_pivot(&mut en, "grc", "cs"), "Wiktionary (via English)", LICENSE_WIKTIONARY);
    specs.push(p);
    let mut p = PackSpec::new("cs", "grc");
    p.add(take_pivot(&mut en, "cs", "grc"), "Wiktionary (via English)", LICENSE_WIKTIONARY);
    specs.push(p);

    let mut catalog = Vec::new();
    for mut spec in specs {
        if spec.entries.is_empty() {
            eprintln!("skipping {}-{}: no input data", spec.src, spec.tgt);
            continue;
        }
        let extra = match spec.src {
            "cs" => cs.as_ref().map(|d| &d.extra),
            "de" => de.as_ref().map(|d| &d.extra),
            "grc" => grc.as_ref().map(|d| &d.extra),
            "en" => en.as_ref().map(|d| &d.extra),
            _ => None,
        };
        if let Some(x) = extra {
            spec.entries.iter_mut().for_each(|e| x.enrich(e));
        }
        let id = format!("{}-{}", spec.src, spec.tgt);
        let meta = DictMeta {
            id: id.clone(),
            name: format!("{} → {}", lang_name(spec.src), lang_name(spec.tgt)),
            source_lang: spec.src.into(),
            target_lang: spec.tgt.into(),
            sources: spec.sources.join(", "),
            license: spec.licenses.join("\n"),
            built: today(),
            entries: 0,
        };
        let mut b = PackBuilder::new(meta);
        for e in spec.entries {
            b.add(e);
        }
        let file = format!("{id}.csd");
        let path = out.join(&file);
        let t = Instant::now();
        let meta = b.write(&path)?;
        let size = std::fs::metadata(&path)?.len();
        eprintln!("wrote {} – {} entries, {:.1} MB in {:?}", path.display(), meta.entries, size as f64 / 1e6, t.elapsed());
        let url = if base_url.is_empty() { file.clone() } else { format!("{}/{file}", base_url.trim_end_matches('/')) };
        catalog.push(serde_json::json!({ "meta": meta, "url": url, "size": size }));
    }
    let catalog = serde_json::json!({ "version": 1, "dictionaries": catalog });
    std::fs::write(out.join("catalog.json"), serde_json::to_vec_pretty(&catalog)?)?;
    eprintln!("done in {:?}", started.elapsed());
    Ok(())
}
