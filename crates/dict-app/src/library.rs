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
    ("grc-en", "Ancient Greek → English", "grc", "en"),
];

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
    pub fn poll_downloads(&mut self) {
        let finished: Vec<String> = self
            .slots
            .values()
            .filter(|s| matches!(s.download.as_ref().map(|d| d.lock().unwrap().clone()), Some(Download::Finished)))
            .map(|s| s.id.clone())
            .collect();
        for id in finished {
            let path = self.pack_path(&id);
            if let Some(slot) = self.slots.get_mut(&id) {
                slot.download = None;
            }
            match Dictionary::open(&path) {
                Ok(d) => self.insert_dict(d),
                Err(e) => {
                    if let Some(slot) = self.slots.get_mut(&id) {
                        slot.download = Some(Arc::new(Mutex::new(Download::Failed(e.to_string()))));
                    }
                }
            }
        }
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

fn download(url: &str, dest: &Path, state: &Mutex<Download>, ctx: &egui::Context) -> Result<()> {
    let part = dest.with_extension("csd.part");
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
    std::fs::rename(&part, dest)?;
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
        lib.remove("cs-grc").unwrap();
        assert!(!dir.join("cs-grc.csd").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_relative_urls() {
        assert_eq!(resolve("https://x.org/a/catalog.json", "cs-en.csd"), "https://x.org/a/cs-en.csd");
        assert_eq!(resolve("/data/dist/catalog.json", "cs-en.csd"), "/data/dist/cs-en.csd");
        assert_eq!(resolve("https://x.org/catalog.json", "https://y.org/z.csd"), "https://y.org/z.csd");
    }
}
