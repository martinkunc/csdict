//! Persistent user settings (dictionary order / selection, catalog URL, theme).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow the operating system's light / dark setting.
    #[default]
    System,
    Light,
    Dark,
}

impl From<Theme> for egui::ThemePreference {
    fn from(t: Theme) -> Self {
        match t {
            Theme::System => egui::ThemePreference::System,
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
        }
    }
}

/// One entry of the dictionary menu: a group id (see [`crate::library::GROUPS`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DictSetting {
    pub id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Dictionary groups in display order.
    pub dictionaries: Vec<DictSetting>,
    /// Where to get `catalog.json` (http(s) URL or local path).
    pub catalog: String,
    /// Look up words online in dictionaries that are not downloaded.
    pub online: bool,
    pub theme: Theme,
    /// UI zoom (Ctrl/Cmd + / − / 0).
    pub zoom: f32,
}

/// Default catalog location: the rolling `dictionaries` GitHub Release built
/// by `.github/workflows/build-dictionaries.yml`; override with
/// `CSDICT_CATALOG` or in the dictionaries menu.
pub const DEFAULT_CATALOG: &str = "https://github.com/martinkunc/csdict/releases/download/dictionaries/catalog.json";

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dictionaries: crate::library::GROUPS
                .iter()
                .map(|(id, ..)| DictSetting { id: (*id).to_owned(), enabled: *id == "cs-en" })
                .collect(),
            catalog: DEFAULT_CATALOG.to_owned(),
            online: true,
            theme: Theme::System,
            zoom: 1.0,
        }
    }
}

pub struct Dirs {
    pub config: PathBuf,
    pub dicts: PathBuf,
    pub cache: PathBuf,
}

impl Dirs {
    pub fn new() -> Self {
        let pd = directories::ProjectDirs::from("org", "csdict", "csdict");
        let home = std::env::var_os("CSDICT_HOME").map(PathBuf::from);
        let (config, data, cache) = match (&home, &pd) {
            // Portable mode: everything in one folder.
            (Some(h), _) => (h.clone(), h.clone(), h.join("cache")),
            (None, Some(p)) => (p.config_dir().to_owned(), p.data_dir().to_owned(), p.cache_dir().to_owned()),
            (None, None) => {
                let base = std::env::temp_dir().join("csdict");
                (base.clone(), base.clone(), base.join("cache"))
            }
        };
        let dirs = Dirs { config, dicts: data.join("dicts"), cache };
        for d in [&dirs.config, &dirs.dicts, &dirs.cache] {
            let _ = std::fs::create_dir_all(d);
        }
        dirs
    }

    fn settings_file(&self) -> PathBuf {
        self.config.join("settings.json")
    }

    pub fn load_settings(&self) -> Settings {
        let saved: Option<Settings> = std::fs::read(self.settings_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let first_run = saved.is_none();
        let mut s = saved.unwrap_or_default();
        if first_run {
            // Also select the dictionary of the system language (e.g. Spanish
            // ↔ English on a Spanish system), placed first.
            let lang = sys_locale::get_locale().unwrap_or_default();
            if let Some(id) = crate::library::group_for_language(&lang[..lang.len().min(2)].to_lowercase())
                && let Some(i) = s.dictionaries.iter().position(|d| d.id == id)
            {
                let mut d = s.dictionaries.remove(i);
                d.enabled = true;
                s.dictionaries.insert(0, d);
            }
        }
        // Single directions that are now part of a group (settings from
        // before grouping) are dropped; every built-in group is listed.
        s.dictionaries.retain(|d| !crate::library::is_grouped_member(&d.id));
        for (id, ..) in crate::library::GROUPS {
            if !s.dictionaries.iter().any(|d| d.id == *id) {
                s.dictionaries.push(DictSetting { id: (*id).to_owned(), enabled: false });
            }
        }
        if let Ok(c) = std::env::var("CSDICT_CATALOG") {
            s.catalog = c;
        }
        // A local catalog that has since gone (e.g. a workspace that was
        // moved or deleted) is detected anew rather than failing forever.
        let c = s.catalog.trim();
        let is_url = c.starts_with("http://") || c.starts_with("https://");
        if !is_url && !std::path::Path::new(c.strip_prefix("file://").unwrap_or(c)).is_file() {
            s.catalog.clear();
        }
        if s.catalog.trim().is_empty() {
            s.catalog = find_local_catalog().unwrap_or_else(|| DEFAULT_CATALOG.to_owned());
        }
        if !(0.5..=3.0).contains(&s.zoom) {
            s.zoom = 1.0;
        }
        s
    }

    pub fn save_settings(&self, s: &Settings) {
        if let Ok(json) = serde_json::to_vec_pretty(s) {
            let tmp = self.settings_file().with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, self.settings_file());
            }
        }
    }
}

/// Look for a locally built `dist/catalog.json`: in the current directory,
/// next to the executable, or in the workspace when run via `cargo run`
/// (executable in `target/<profile>/`).
pub fn find_local_catalog() -> Option<String> {
    let mut bases = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        bases.push(cwd);
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_owned())) {
        bases.extend(dir.ancestors().take(4).map(|p| p.to_owned()));
    }
    bases
        .iter()
        .flat_map(|b| [b.join("dist").join("catalog.json"), b.join("catalog.json")])
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}
