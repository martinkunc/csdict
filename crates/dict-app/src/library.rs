//! Known dictionaries, the remote catalog, downloads and opened packs.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use dict_core::{DictMeta, Dictionary};
use serde::Deserialize;

/// Built-in list so the menu is useful before the catalog is fetched:
/// (id, name, source language, target language).
pub const BUILTIN: &[(&str, &str, &str, &str)] = &[
    ("cs-en", "Czech → English", "cs", "en"),
    ("en-cs", "English → Czech", "en", "cs"),
    ("cs-de", "Czech → German", "cs", "de"),
    ("de-cs", "German → Czech", "de", "cs"),
    ("cs-grc", "Czech → Ancient Greek", "cs", "grc"),
    ("grc-cs", "Ancient Greek → Czech", "grc", "cs"),
    ("de-en", "German → English", "de", "en"),
    ("en-de", "English → German", "en", "de"),
    ("grc-en", "Ancient Greek → English", "grc", "en"),
    ("en-grc", "English → Ancient Greek", "en", "grc"),
    ("es-en", "Spanish → English", "es", "en"),
    ("en-es", "English → Spanish", "en", "es"),
    ("fr-en", "French → English", "fr", "en"),
    ("en-fr", "English → French", "en", "fr"),
    ("ru-en", "Russian → English", "ru", "en"),
    ("en-ru", "English → Russian", "en", "ru"),
    ("pt-en", "Portuguese → English", "pt", "en"),
    ("en-pt", "English → Portuguese", "en", "pt"),
    ("ja-en", "Japanese → English", "ja", "en"),
    ("en-ja", "English → Japanese", "en", "ja"),
    ("ar-en", "Arabic → English", "ar", "en"),
    ("en-ar", "English → Arabic", "en", "ar"),
    ("zh-en", "Chinese → English", "zh", "en"),
    ("en-zh", "English → Chinese", "en", "zh"),
    ("hi-en", "Hindi → English", "hi", "en"),
    ("en-hi", "English → Hindi", "en", "hi"),
    ("ko-en", "Korean → English", "ko", "en"),
    ("en-ko", "English → Korean", "en", "ko"),
    ("en-wn", "English definitions (WordNet)", "en", "en"),
];

/// What the user selects and sees as one tab: both directions of a language
/// pair plus related sources, (group id, name, member dictionary ids).
/// Dictionaries not listed here (e.g. new ones from the catalog) form a
/// group of their own.
pub const GROUPS: &[(&str, &str, &[&str])] = &[
    ("cs-en", "Czech ↔ English", &["cs-en", "en-cs", "en-wn"]),
    ("cs-de", "Czech ↔ German", &["cs-de", "de-cs"]),
    ("cs-grc", "Czech ↔ Ancient Greek", &["cs-grc", "grc-cs"]),
    ("de-en", "German ↔ English", &["de-en", "en-de"]),
    ("grc-en", "Ancient Greek ↔ English", &["grc-en", "en-grc"]),
    ("es-en", "Spanish ↔ English", &["es-en", "en-es"]),
    ("fr-en", "French ↔ English", &["fr-en", "en-fr"]),
    ("ru-en", "Russian ↔ English", &["ru-en", "en-ru"]),
    ("pt-en", "Portuguese ↔ English", &["pt-en", "en-pt"]),
    ("ja-en", "Japanese ↔ English", &["ja-en", "en-ja"]),
    ("ar-en", "Arabic ↔ English", &["ar-en", "en-ar"]),
    ("zh-en", "Chinese ↔ English", &["zh-en", "en-zh"]),
    ("hi-en", "Hindi ↔ English", &["hi-en", "en-hi"]),
    ("ko-en", "Korean ↔ English", &["ko-en", "en-ko"]),
];

/// Group to enable on first start for a user whose system language is
/// `lang` (two-letter code), e.g. `es` → `es-en`.
pub fn group_for_language(lang: &str) -> Option<&'static str> {
    let id = format!("{lang}-en");
    GROUPS.iter().map(|g| g.0).find(|g| *g == id)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub members: Vec<String>,
}

/// Is `id` a member of a built-in group (and not a group id itself)?
pub fn is_grouped_member(id: &str) -> bool {
    !GROUPS.iter().any(|g| g.0 == id) && GROUPS.iter().any(|g| g.2.contains(&id))
}

pub const USER_AGENT: &str = concat!("csdict/", env!("CARGO_PKG_VERSION"), " (personal dictionary app)");

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogItem {
    pub meta: DictMeta,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct Catalog {
    dictionaries: Vec<CatalogItem>,
}

#[derive(Debug, Clone)]
pub enum Download {
    Running { done: u64, total: u64 },
    Failed(String),
    Finished,
}

pub struct Slot {
    pub id: String,
    pub name: String,
    pub source_lang: String,
    pub target_lang: String,
    pub remote: Option<CatalogItem>,
    pub dict: Option<Arc<Dictionary>>,
    pub download: Option<Arc<Mutex<Download>>>,
}

impl Slot {
    pub fn is_downloaded(&self) -> bool {
        self.dict.is_some()
    }

    /// The catalog offers a different build than the downloaded one (the
    /// packs are rebuilt in place, so any difference means newer).
    pub fn update_available(&self) -> bool {
        match (&self.dict, &self.remote) {
            (Some(d), Some(r)) => d.meta().built != r.meta.built || d.meta().entries != r.meta.entries,
            _ => false,
        }
    }

    /// Short label of the direction, e.g. `English → Czech`, `English definitions`.
    pub fn direction(&self) -> String {
        let name = |c: &str| dict_core::kaikki::language_name(c).unwrap_or(c).to_owned();
        if self.source_lang == self.target_lang {
            format!("{} definitions", name(&self.source_lang))
        } else {
            format!("{} → {}", name(&self.source_lang), name(&self.target_lang))
        }
    }
}

pub struct Library {
    pub dir: PathBuf,
    pub slots: HashMap<String, Slot>,
    pub catalog_state: Arc<Mutex<CatalogState>>,
}

#[derive(Debug, Clone, Default)]
pub enum CatalogState {
    #[default]
    Idle,
    Loading,
    Loaded(Vec<CatalogItem>),
    Failed(String),
}

impl Library {
    pub fn new(dir: PathBuf) -> Self {
        let mut slots = HashMap::new();
        for (id, name, s, t) in BUILTIN {
            slots.insert(
                (*id).to_owned(),
                Slot {
                    id: (*id).to_owned(),
                    name: (*name).to_owned(),
                    source_lang: (*s).to_owned(),
                    target_lang: (*t).to_owned(),
                    remote: None,
                    dict: None,
                    download: None,
                },
            );
        }
        let mut lib = Library { dir, slots, catalog_state: Default::default() };
        lib.scan_local();
        lib
    }

    /// Built-in groups (members limited to known dictionaries), then one
    /// group for each other dictionary.
    pub fn groups(&self) -> Vec<Group> {
        let mut out: Vec<Group> = GROUPS
            .iter()
            .map(|(id, name, members)| Group {
                id: (*id).to_owned(),
                name: (*name).to_owned(),
                members: members.iter().filter(|m| self.slots.contains_key(**m)).map(|m| (*m).to_owned()).collect(),
            })
            .filter(|g| !g.members.is_empty())
            .collect();
        let mut rest: Vec<&Slot> = self
            .slots
            .values()
            .filter(|s| !out.iter().any(|g| g.id == s.id || g.members.contains(&s.id)))
            .collect();
        rest.sort_by(|a, b| a.id.cmp(&b.id));
        out.extend(rest.into_iter().map(|s| Group { id: s.id.clone(), name: s.name.clone(), members: vec![s.id.clone()] }));
        out
    }

    pub fn group(&self, id: &str) -> Option<Group> {
        self.groups().into_iter().find(|g| g.id == id)
    }

    fn pack_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.csd"))
    }

    /// Open every pack present in the dictionary directory.
    fn scan_local(&mut self) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "csd") {
                match Dictionary::open(&p) {
                    Ok(d) => self.insert_dict(d),
                    Err(err) => eprintln!("cannot open {}: {err}", p.display()),
                }
            }
        }
    }

    fn insert_dict(&mut self, d: Dictionary) {
        let m = d.meta().clone();
        let slot = self.slots.entry(m.id.clone()).or_insert_with(|| Slot {
            id: m.id.clone(),
            name: m.name.clone(),
            source_lang: m.source_lang.clone(),
            target_lang: m.target_lang.clone(),
            remote: None,
            dict: None,
            download: None,
        });
        slot.dict = Some(Arc::new(d));
    }

    /// Fetch the catalog in the background.
    pub fn refresh_catalog(&self, source: String, ctx: egui::Context) {
        let state = self.catalog_state.clone();
        *state.lock().unwrap() = CatalogState::Loading;
        std::thread::spawn(move || {
            let r = fetch_catalog(&source);
            *state.lock().unwrap() = match r {
                Ok(items) => CatalogState::Loaded(items),
                Err(e) => CatalogState::Failed(format!("{e:#}")),
            };
            ctx.request_repaint();
        });
    }

    /// Apply a freshly loaded catalog. Returns ids that are new to the user.
    pub fn apply_catalog(&mut self, items: &[CatalogItem]) -> Vec<String> {
        let mut new = Vec::new();
        for it in items {
            let m = &it.meta;
            let slot = self.slots.entry(m.id.clone()).or_insert_with(|| {
                new.push(m.id.clone());
                Slot {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    source_lang: m.source_lang.clone(),
                    target_lang: m.target_lang.clone(),
                    remote: None,
                    dict: None,
                    download: None,
                }
            });
            slot.remote = Some(it.clone());
        }
        new
    }

    pub fn start_download(&mut self, id: &str, ctx: egui::Context) {
        let path = self.pack_path(id);
        let Some(slot) = self.slots.get_mut(id) else { return };
        let Some(item) = slot.remote.clone() else { return };
        let state = Arc::new(Mutex::new(Download::Running { done: 0, total: item.size }));
        slot.download = Some(state.clone());
        std::thread::spawn(move || {
            let r = download(&item.url, &path, &state, &ctx);
            *state.lock().unwrap() = match r {
                Ok(()) => Download::Finished,
                Err(e) => Download::Failed(format!("{e:#}")),
            };
            ctx.request_repaint();
        });
    }

    /// Open packs whose download finished; drop finished download state.
    /// Returns whether any download finished.
    pub fn poll_downloads(&mut self) -> bool {
        let finished: Vec<String> = self
            .slots
            .values()
            .filter(|s| matches!(s.download.as_ref().map(|d| d.lock().unwrap().clone()), Some(Download::Finished)))
            .map(|s| s.id.clone())
            .collect();
        let any = !finished.is_empty();
        for id in finished {
            let path = self.pack_path(&id);
            if let Some(slot) = self.slots.get_mut(&id) {
                slot.download = None;
                // An update replaces an open pack: unmap it first (Windows
                // cannot replace a mapped file).
                slot.dict = None;
            }
            let renamed = std::fs::rename(part_path(&path), &path).with_context(|| format!("replace {}", path.display()));
            // Opens the new pack, or the old one again if it could not be replaced.
            let opened = Dictionary::open(&path).map(|d| self.insert_dict(d));
            if let Some(e) = renamed.err().or(opened.err().map(Into::into))
                && let Some(slot) = self.slots.get_mut(&id)
            {
                slot.download = Some(Arc::new(Mutex::new(Download::Failed(format!("{e:#}")))));
            }
        }
        any
    }

    pub fn remove(&mut self, id: &str) -> Result<()> {
        let path = self.pack_path(id);
        if let Some(slot) = self.slots.get_mut(id) {
            // Drop the memory map first (required on Windows).
            slot.dict = None;
            slot.download = None;
        }
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
        }
        Ok(())
    }
}

/// `source` is an http(s) URL or a local path.
fn read_source(source: &str) -> Result<Vec<u8>> {
    if source.starts_with("http://") || source.starts_with("https://") {
        let mut resp = ureq::get(source).header("User-Agent", USER_AGENT).call()?;
        Ok(resp.body_mut().with_config().limit(256 << 20).read_to_vec()?)
    } else {
        let p = source.strip_prefix("file://").unwrap_or(source);
        Ok(std::fs::read(p).with_context(|| format!("read {p}"))?)
    }
}

/// Resolve `url` relative to the catalog location.
fn resolve(base: &str, url: &str) -> String {
    if url.contains("://") || Path::new(url).is_absolute() {
        return url.to_owned();
    }
    match base.rfind('/') {
        Some(i) => format!("{}/{url}", &base[..i]),
        None => url.to_owned(),
    }
}

fn fetch_catalog(source: &str) -> Result<Vec<CatalogItem>> {
    let bytes = read_source(source)?;
    let mut cat: Catalog = serde_json::from_slice(&bytes).context("invalid catalog.json")?;
    for it in &mut cat.dictionaries {
        it.url = resolve(source, &it.url);
    }
    Ok(cat.dictionaries)
}

fn part_path(dest: &Path) -> PathBuf {
    dest.with_extension("csd.part")
}

/// Download into `<dest>.part` and validate it; [`Library::poll_downloads`]
/// moves it into place.
fn download(url: &str, dest: &Path, state: &Mutex<Download>, ctx: &egui::Context) -> Result<()> {
    let part = part_path(dest);
    let mut reader: Box<dyn Read> = if url.starts_with("http://") || url.starts_with("https://") {
        let resp = ureq::get(url).header("User-Agent", USER_AGENT).call()?;
        if let Some(len) = resp.headers().get("content-length").and_then(|v| v.to_str().ok()?.parse::<u64>().ok())
            && let Download::Running { total, .. } = &mut *state.lock().unwrap()
        {
            *total = len;
        }
        Box::new(resp.into_body().into_reader())
    } else {
        let p = url.strip_prefix("file://").unwrap_or(url);
        Box::new(std::fs::File::open(p).with_context(|| format!("open {p}"))?)
    };
    let mut out = std::fs::File::create(&part)?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        if let Download::Running { done: d, .. } = &mut *state.lock().unwrap() {
            *d = done;
        }
        ctx.request_repaint();
    }
    out.sync_all()?;
    drop(out);
    if let Err(e) = Dictionary::read_meta(&part) {
        let _ = std::fs::remove_file(&part);
        bail!("downloaded file is not a valid dictionary: {e}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full download flow against the locally built `dist/` (skipped if absent).
    #[test]
    fn downloads_from_local_catalog() {
        let Some(cat) = crate::settings::find_local_catalog() else { return };
        let dir = std::env::temp_dir().join(format!("csdict-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut lib = Library::new(dir.clone());
        let items = fetch_catalog(&cat).unwrap();
        lib.apply_catalog(&items);
        let ctx = egui::Context::default();
        lib.start_download("cs-grc", ctx);
        for _ in 0..200 {
            lib.poll_downloads();
            if lib.slots["cs-grc"].is_downloaded() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let slot = &lib.slots["cs-grc"];
        assert!(slot.is_downloaded(), "download state: {:?}", slot.download.as_ref().map(|d| d.lock().unwrap().clone()));
        assert!(!slot.dict.as_ref().unwrap().entries_for("slovo").is_empty());
        assert!(!slot.update_available());

        // A rebuilt pack in the catalog is offered as an update and replaces
        // the open one.
        lib.slots.get_mut("cs-grc").unwrap().remote.as_mut().unwrap().meta.built = "2000-01-01".into();
        assert!(lib.slots["cs-grc"].update_available());
        lib.start_download("cs-grc", egui::Context::default());
        for _ in 0..200 {
            if lib.poll_downloads() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let slot = &lib.slots["cs-grc"];
        assert!(slot.download.is_none(), "download state: {:?}", slot.download.as_ref().map(|d| d.lock().unwrap().clone()));
        assert!(!slot.dict.as_ref().unwrap().entries_for("slovo").is_empty());
        assert!(!dir.join("cs-grc.csd.part").exists());
        lib.remove("cs-grc").unwrap();
        assert!(!dir.join("cs-grc.csd").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn groups_cover_every_dictionary_once() {
        let lib = Library::new(std::env::temp_dir().join("csdict-test-no-such-dir"));
        let groups = lib.groups();
        assert_eq!(groups[0].members, ["cs-en", "en-cs", "en-wn"]);
        assert_eq!(groups.len(), GROUPS.len(), "every built-in dictionary belongs to a built-in group");
        assert_eq!(group_for_language("es"), Some("es-en"));
        assert_eq!(group_for_language("cs"), Some("cs-en"));
        assert_eq!(group_for_language("xx"), None);
        for id in lib.slots.keys() {
            assert_eq!(groups.iter().filter(|g| g.members.contains(id)).count(), 1, "{id}");
        }
        assert!(is_grouped_member("en-cs") && !is_grouped_member("cs-en") && !is_grouped_member("xx-yy"));
    }

    #[test]
    fn resolves_relative_urls() {
        assert_eq!(resolve("https://x.org/a/catalog.json", "cs-en.csd"), "https://x.org/a/cs-en.csd");
        assert_eq!(resolve("/data/dist/catalog.json", "cs-en.csd"), "/data/dist/cs-en.csd");
        assert_eq!(resolve("https://x.org/catalog.json", "https://y.org/z.csd"), "https://y.org/z.csd");
    }
}
