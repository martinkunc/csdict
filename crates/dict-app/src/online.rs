//! Online lookup for dictionaries that are not downloaded, using the
//! per-word JSON pages of kaikki.org (Wiktionary data) and the en-word.net
//! API (Open English WordNet).
//!
//! Requests are throttled so typing never floods the server:
//! * the app waits [`DEBOUNCE`] after the word last changed (typing or
//!   moving through the list) before asking;
//! * only the word currently shown is wanted – queued lookups for words the
//!   user has moved past are dropped before they hit the network (lookups
//!   of the same word in several dictionaries all stay wanted);
//! * all HTTP requests share one [`RateLimiter`] (burst 4, then ~1.5/s,
//!   max 2 in flight); HTTP 429/503 pause everything (Retry-After or 30 s);
//! * results (including "not found") are cached; failures can be retried
//!   with Enter.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use dict_core::kaikki::{self, english_translation_groups, format_word};
use dict_core::wordnet;
use dict_core::normalize::nfc;
use dict_core::{Entry, Sense};
use serde_json::Value;

use crate::library::USER_AGENT;
use crate::ratelimit::{Limits, RateLimiter};

#[derive(Clone, Debug)]
pub enum OnlineResult {
    Pending,
    Done(Arc<Vec<Entry>>),
    Failed(String),
}

type Key = (String, String); // (dictionary id, word)

enum Msg {
    Result(Key, OnlineResult),
    /// The lookup was abandoned before completing; forget the Pending marker.
    Cancelled(Key),
}

pub struct Online {
    results: HashMap<Key, OnlineResult>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    limiter: Arc<RateLimiter>,
    /// The word the UI currently shows; lookups of other words are
    /// cancelled while queued.
    wanted: Arc<Mutex<Option<String>>>,
}

/// Wait this long after the word last changed before going online.
pub const DEBOUNCE: Duration = Duration::from_millis(500);
/// Cached results kept before the cache is reset.
const CACHE_LIMIT: usize = 1000;
/// Back-off when the server returns 429/503 without Retry-After.
const DEFAULT_BACK_OFF: Duration = Duration::from_secs(30);

impl Online {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Online {
            results: HashMap::new(),
            tx,
            rx,
            limiter: Arc::new(RateLimiter::new(Limits::ONLINE)),
            wanted: Default::default(),
        }
    }

    pub fn poll(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Result(k, r) => {
                    self.results.insert(k, r);
                }
                Msg::Cancelled(k) => {
                    if matches!(self.results.get(&k), Some(OnlineResult::Pending)) {
                        self.results.remove(&k);
                    }
                }
            }
        }
    }

    pub fn get(&self, dict: &str, word: &str) -> Option<&OnlineResult> {
        self.results.get(&(dict.to_owned(), word.to_owned()))
    }

    /// Remaining server-requested pause, if any (shown in the UI).
    pub fn blocked_for(&self) -> Option<Duration> {
        self.limiter.blocked_for()
    }

    /// The user moved on (word still being typed): drop queued lookups.
    pub fn clear_wanted(&self) {
        *self.wanted.lock().unwrap() = None;
    }

    /// Forget a failed lookup so it is tried again.
    pub fn retry(&mut self, dict: &str, word: &str) {
        let key = (dict.to_owned(), word.to_owned());
        if matches!(self.results.get(&key), Some(OnlineResult::Failed(_))) {
            self.results.remove(&key);
        }
    }

    /// Start a lookup unless one is cached or running. Marks `word` as the
    /// wanted word, which cancels queued lookups of any other word.
    pub fn request(&mut self, dict: &str, src: &str, tgt: &str, word: &str, ctx: &egui::Context) {
        let key = (dict.to_owned(), word.to_owned());
        *self.wanted.lock().unwrap() = Some(word.to_owned());
        if self.results.contains_key(&key) {
            return;
        }
        if self.results.len() >= CACHE_LIMIT {
            self.results.retain(|_, r| matches!(r, OnlineResult::Pending));
        }
        self.results.insert(key.clone(), OnlineResult::Pending);
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        let (limiter, wanted) = (self.limiter.clone(), self.wanted.clone());
        let source = if dict == WORDNET_ID { Source::WordNet } else { Source::Wiktionary };
        let (src, tgt, word) = (src.to_owned(), tgt.to_owned(), word.to_owned());
        std::thread::spawn(move || {
            let fetcher = Fetcher {
                limiter: &limiter,
                still_wanted: &|| wanted.lock().unwrap().as_deref() == Some(word.as_str()),
            };
            let msg = match lookup_with(&fetcher, source, &src, &tgt, &word) {
                Ok(e) => Msg::Result(key, OnlineResult::Done(Arc::new(e))),
                Err(e) if e.is::<Abandoned>() => Msg::Cancelled(key),
                Err(e) => Msg::Result(key, OnlineResult::Failed(format!("{e:#}"))),
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }
}

#[derive(Debug)]
struct Abandoned;

impl std::fmt::Display for Abandoned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("lookup abandoned")
    }
}

impl std::error::Error for Abandoned {}

/// Performs rate-limited, cancellable HTTP fetches for one lookup.
pub struct Fetcher<'a> {
    limiter: &'a RateLimiter,
    still_wanted: &'a dyn Fn() -> bool,
}

impl Fetcher<'_> {
    /// Download and parse the kaikki.org JSONL page of `word` in `lang`.
    fn fetch_word(&self, lang: &str, word: &str) -> Result<Vec<Value>> {
        let Some(url) = kaikki::word_url(lang, word) else {
            anyhow::bail!("online lookup is not available for language '{lang}'");
        };
        let text = self.get(&url)?.unwrap_or_default();
        Ok(text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
    }

    /// Ask en-word.net for the synsets of `lemma` (JSON array).
    fn fetch_wordnet(&self, lemma: &str) -> Result<Value> {
        match self.get(&wordnet::api_url(lemma))? {
            Some(text) => Ok(serde_json::from_str(&text)?),
            None => Ok(Value::Array(Vec::new())),
        }
    }

    /// Rate-limited GET; `None` on HTTP 404.
    fn get(&self, url: &str) -> Result<Option<String>> {
        let _permit = self.limiter.acquire(self.still_wanted).map_err(|_| Abandoned)?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .http_status_as_error(false)
            .build()
            .into();
        let mut resp = agent.get(url).header("User-Agent", USER_AGENT).call()?;
        match resp.status().as_u16() {
            200 => {}
            404 => return Ok(None),
            s @ (429 | 503) => {
                let wait = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok()?.trim().parse::<u64>().ok())
                    .map(|secs| Duration::from_secs(secs.clamp(1, 600)))
                    .unwrap_or(DEFAULT_BACK_OFF);
                self.limiter.back_off(wait);
                anyhow::bail!("server is busy (HTTP {s}); pausing online lookups for {} s", wait.as_secs());
            }
            s => anyhow::bail!("server returned HTTP {s}"),
        }
        Ok(Some(resp.body_mut().with_config().limit(16 << 20).read_to_string()?))
    }
}

/// Can entries of this pack source (as named in the catalog, e.g.
/// `Wiktionary (via English)`, `GNU/FDL slovník`) be looked up online?
pub fn available_online(source: &str) -> bool {
    source.starts_with("Wiktionary") || source.contains("WordNet")
}

/// Id of the WordNet dictionary (looked up on en-word.net, not kaikki.org).
pub const WORDNET_ID: &str = "en-wn";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    Wiktionary,
    WordNet,
}

/// English glosses suitable as pivot words (`dog`, `to run` → `run`).
fn pivot_words(entries: &[Entry]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in entries {
        for s in &e.senses {
            for g in &s.translations {
                for part in g.split([';', ',']) {
                    let w = part.split('(').next().unwrap_or("").trim();
                    let w = w.strip_prefix("to ").unwrap_or(w);
                    let w = w.strip_prefix("a ").or_else(|| w.strip_prefix("an ")).unwrap_or(w);
                    if !w.is_empty() && w.split_whitespace().count() <= 2 && !out.iter().any(|x| x == w) {
                        out.push(w.to_owned());
                    }
                }
            }
        }
    }
    out.truncate(4);
    out
}

/// Look `word` up online in the `src`→`tgt` direction.
pub fn lookup_with(f: &Fetcher<'_>, source: Source, src: &str, tgt: &str, word: &str) -> Result<Vec<Entry>> {
    let word = nfc(word);
    let started = Instant::now();
    if source == Source::WordNet {
        let mut e = wordnet::entries_from_api(&word, &f.fetch_wordnet(&word)?);
        let lower = word.to_lowercase();
        if e.is_empty() && lower != word {
            e = wordnet::entries_from_api(&lower, &f.fetch_wordnet(&lower)?);
        }
        return Ok(e);
    }
    if tgt == "en" {
        return Ok(f.fetch_word(src, &word)?.iter().filter_map(kaikki::entry_from_kaikki).collect());
    }
    if src == "en" {
        return Ok(f.fetch_word("en", &word)?
            .iter()
            .filter_map(|v| kaikki::english_to_lang_entry(v, tgt))
            .collect());
    }
    // Neither side is English: go through the English glosses.
    let source: Vec<Entry> = f.fetch_word(src, &word)?.iter().filter_map(kaikki::entry_from_kaikki).collect();
    let mut out: Vec<Entry> = Vec::new();
    for en in pivot_words(&source) {
        if started.elapsed() > Duration::from_secs(15) {
            break;
        }
        for v in f.fetch_word("en", &en)? {
            let pos = v.get("pos").and_then(Value::as_str).unwrap_or("").to_owned();
            for g in english_translation_groups(&v, tgt) {
                let note = match &g.sense {
                    Some(s) => format!("via English “{en}”: {s}"),
                    None => format!("via English “{en}”"),
                };
                let e = match out.iter_mut().position(|e| e.pos == pos) {
                    Some(i) => &mut out[i],
                    None => {
                        let mut e = Entry::new(word.clone(), pos.clone(), "Wiktionary (via English, online)");
                        if let Some(s) = source.iter().find(|s| s.pos == pos).or(source.first()) {
                            e.ipa = s.ipa.clone();
                            e.audio = s.audio.clone();
                        }
                        out.push(e);
                        out.last_mut().unwrap()
                    }
                };
                e.senses.push(Sense {
                    translations: g.words.iter().map(|(w, t)| format_word(w, t)).collect(),
                    note: Some(note),
                    ..Default::default()
                });
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(src: &str, tgt: &str, word: &str) -> Result<Vec<Entry>> {
        let limiter = RateLimiter::new(Limits::ONLINE);
        let source = if tgt == "wn" { Source::WordNet } else { Source::Wiktionary };
        lookup_with(&Fetcher { limiter: &limiter, still_wanted: &|| true }, source, src, tgt, word)
    }

    #[test]
    fn sources_available_online() {
        assert!(available_online("Wiktionary") && available_online("Wiktionary (via English)"));
        assert!(available_online("Open English WordNet"));
        assert!(!available_online("GNU/FDL slovník"));
    }

    #[test]
    fn abandoned_lookup_makes_no_request() {
        let limiter = RateLimiter::new(Limits::ONLINE);
        let f = Fetcher { limiter: &limiter, still_wanted: &|| false };
        let err = lookup_with(&f, Source::Wiktionary, "cs", "en", "pes").unwrap_err();
        assert!(err.is::<Abandoned>());
    }

    /// Hits kaikki.org; run with `cargo test -p dict-app -- --ignored`.
    #[test]
    #[ignore]
    fn online_lookups() {
        let cs_en = lookup("cs", "en", "pes").unwrap();
        assert!(cs_en.iter().any(|e| e.senses.iter().any(|s| s.translations.iter().any(|t| t.contains("dog")))));
        let en_cs = lookup("en", "cs", "dog").unwrap();
        assert!(en_cs.iter().any(|e| e.senses.iter().any(|s| s.translations.iter().any(|t| t.starts_with("pes")))));
        let cs_de = lookup("cs", "de", "pes").unwrap();
        assert!(cs_de.iter().any(|e| e.senses.iter().any(|s| s.translations.iter().any(|t| t.starts_with("Hund")))), "{cs_de:#?}");
        assert!(lookup("cs", "en", "qqqxxyz").unwrap().is_empty());
        let wn = lookup("en", "wn", "Solicit").unwrap();
        assert!(wn.iter().any(|e| e.pos == "verb" && e.senses.iter().any(|s| s.synonyms.iter().any(|x| x == "accost"))), "{wn:#?}");
        assert!(lookup("en", "wn", "qqqxxyz").unwrap().is_empty());
    }
}
