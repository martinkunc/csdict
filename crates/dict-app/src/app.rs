//! The application window.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use dict_core::model::pos_label;
use dict_core::{Entry, MatchKind};
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Key, Layout, Modifiers, PopupCloseBehavior, Rect,
    ResizeDirection, RichText, Sense, Stroke, TextEdit, Vec2, ViewportCommand, pos2, vec2,
};

use crate::library::{CatalogState, Download, Library};
use crate::online::{DEBOUNCE, Online, OnlineResult};
use crate::settings::{DictSetting, Dirs, Settings};
use crate::speech;

const TITLE_BAR_HEIGHT: f32 = 56.0;
const RESIZE_MARGIN: f32 = 5.0;
const LIST_LIMIT: usize = 60;
const ACCENT: Color32 = Color32::from_rgb(0x2f, 0x6f, 0xd6);

/// One word in the result list (merged over all selected dictionaries).
struct ListItem {
    word: String,
    kind: MatchKind,
    rank: u32,
    pos: Vec<String>,
    /// Number of entries (over all dictionaries) behind this word; used to
    /// break ties – common words have entries in more sources.
    count: u32,
}

#[derive(Clone, Copy, PartialEq)]
enum ButtonSide {
    Left,
    Right,
}

enum MenuAction {
    Toggle(usize),
    Move(usize, isize),
    Download(String),
    Remove(String),
    ReloadCatalog,
}

pub struct App {
    dirs: Dirs,
    settings: Settings,
    lib: Library,
    online: Online,

    query: String,
    last_edit: Instant,
    results: Vec<ListItem>,
    selected: Option<usize>,
    /// Word shown in the result pane (list selection or typed text).
    word: String,
    active_tab: Option<String>,
    entry_cache: HashMap<(String, String), Arc<Vec<Entry>>>,
    dirty: bool,
    scroll_list_to_selected: bool,

    button_side: ButtonSide,
    message: Arc<Mutex<Option<String>>>,
    catalog_edit: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let dirs = Dirs::new();
        let settings = dirs.load_settings();
        let lib = Library::new(dirs.dicts.clone());
        apply_style(&cc.egui_ctx, settings.dark_mode);
        let catalog_edit = settings.catalog.clone();
        let app = App {
            dirs,
            lib,
            online: Online::new(),
            query: String::new(),
            last_edit: Instant::now(),
            results: Vec::new(),
            selected: None,
            word: String::new(),
            active_tab: None,
            entry_cache: HashMap::new(),
            dirty: false,
            scroll_list_to_selected: false,
            button_side: detect_button_side(),
            message: Default::default(),
            catalog_edit,
            settings,
        };
        // `csdict <word>` starts with a lookup.
        let mut app = app;
        if let Some(q) = std::env::args().nth(1) {
            app.query = q;
            app.dirty = true;
        }
        if !app.settings.catalog.is_empty() {
            app.lib.refresh_catalog(app.settings.catalog.clone(), cc.egui_ctx.clone());
        }
        app
    }

    fn enabled(&self) -> impl Iterator<Item = &DictSetting> {
        self.settings.dictionaries.iter().filter(|d| d.enabled && self.lib.slots.contains_key(&d.id))
    }

    fn save(&self) {
        self.dirs.save_settings(&self.settings);
    }

    // ---------------------------------------------------------------- search

    fn run_search(&mut self) {
        self.dirty = false;
        self.entry_cache.clear();
        let q = self.query.trim().to_owned();
        let mut merged: HashMap<String, ListItem> = HashMap::new();
        if !q.is_empty() {
            for d in self.enabled() {
                let Some(dict) = self.lib.slots.get(&d.id).and_then(|s| s.dict.as_ref()) else { continue };
                for h in dict.search(&q, LIST_LIMIT) {
                    let it = merged.entry(h.headword.clone()).or_insert_with(|| ListItem {
                        word: h.headword.clone(),
                        kind: h.kind,
                        rank: h.rank,
                        pos: Vec::new(),
                        count: 0,
                    });
                    it.count += 1;
                    if h.rank < it.rank {
                        it.rank = h.rank;
                        it.kind = h.kind;
                    }
                    let p = pos_label(&h.pos).to_owned();
                    if !p.is_empty() && !it.pos.contains(&p) {
                        it.pos.push(p);
                    }
                }
            }
        }
        let mut list: Vec<ListItem> = merged.into_values().collect();
        list.sort_by(|a, b| {
            a.rank.cmp(&b.rank).then_with(|| b.count.cmp(&a.count)).then_with(|| a.word.cmp(&b.word))
        });
        list.truncate(LIST_LIMIT);
        self.results = list;
        self.selected = (!self.results.is_empty()).then_some(0);
        self.word = match self.selected {
            Some(i) => self.results[i].word.clone(),
            None => q,
        };
        self.scroll_list_to_selected = true;
        self.pick_tab();
    }

    fn select(&mut self, i: usize) {
        if let Some(it) = self.results.get(i) {
            self.selected = Some(i);
            self.word = it.word.clone();
            // Moving through the list also waits before going online.
            self.last_edit = Instant::now();
            self.scroll_list_to_selected = true;
            self.pick_tab();
        }
    }

    /// Keep the current tab if it has results, otherwise switch to the first
    /// downloaded dictionary that has the word.
    fn pick_tab(&mut self) {
        let ids: Vec<String> = self.enabled().map(|d| d.id.clone()).collect();
        if ids.is_empty() {
            self.active_tab = None;
            return;
        }
        let cur = self.active_tab.clone().filter(|t| ids.contains(t));
        if let Some(t) = &cur
            && (!self.is_downloaded(t) || !self.offline_entries(t).is_empty())
        {
            return;
        }
        let first_hit = ids.iter().find(|id| self.is_downloaded(id) && !self.offline_entries(id).is_empty());
        self.active_tab = first_hit.cloned().or(cur).or_else(|| ids.first().cloned());
    }

    fn is_downloaded(&self, id: &str) -> bool {
        self.lib.slots.get(id).is_some_and(|s| s.is_downloaded())
    }

    fn offline_entries(&mut self, id: &str) -> Arc<Vec<Entry>> {
        if self.word.is_empty() {
            return Arc::new(Vec::new());
        }
        let key = (id.to_owned(), self.word.clone());
        if let Some(e) = self.entry_cache.get(&key) {
            return e.clone();
        }
        let entries = self
            .lib
            .slots
            .get(id)
            .and_then(|s| s.dict.as_ref())
            .map(|d| d.entries_for(&self.word))
            .unwrap_or_default();
        let entries = Arc::new(entries);
        self.entry_cache.insert(key, entries.clone());
        entries
    }

    // ------------------------------------------------------------ title bar

    fn title_bar(&mut self, ui: &mut egui::Ui) {
        let bar_rect = ui.max_rect();
        // Background: drag to move, double-click to maximize. Widgets added
        // afterwards sit on top and take precedence.
        let bg = ui.interact(bar_rect, ui.scope_id().with("title_drag"), Sense::click_and_drag());
        if bg.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if bg.double_clicked() {
            let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!max));
        }

        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if self.button_side == ButtonSide::Left {
                window_buttons(ui, ButtonSide::Left);
            }
            self.dictionaries_menu(ui);

            // The search box takes 60 % of the window width, starting at 10 %
            // from the left edge (after the menu / buttons if they reach
            // further). The space left over stays part of the drag area.
            let right_w = if self.button_side == ButtonSide::Right { 3.0 * 40.0 + 8.0 } else { 8.0 };
            let win = ui.ctx().content_rect();
            let full = win.width();
            let lead = (win.left() + 0.10 * full - ui.cursor().left()).max(0.0);
            ui.add_space(lead);
            let w = (0.60 * full).min(ui.available_width() - right_w).max(100.0);

            // Arrow keys move through the result list while typing.
            let (down, up, enter, esc) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.key_pressed(Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                )
            });
            let edit = TextEdit::singleline(&mut self.query)
                .font(FontId::proportional(24.0))
                .hint_text("Type a word…")
                .margin(vec2(12.0, 8.0))
                .desired_width(w)
                .id_salt("search");
            let resp = ui.add(edit);
            if resp.changed() {
                self.last_edit = Instant::now();
                self.dirty = true;
            }
            if !ui.ctx().memory(|m| m.focused().is_some()) {
                resp.request_focus();
            }
            if esc {
                self.query.clear();
                self.dirty = true;
            }
            if let Some(sel) = self.selected {
                if down && sel + 1 < self.results.len() {
                    self.select(sel + 1);
                }
                if up && sel > 0 {
                    self.select(sel - 1);
                }
            }
            if enter {
                // Enter forces an online lookup of the typed word.
                self.last_edit = Instant::now() - DEBOUNCE;
                if self.results.is_empty() {
                    self.word = self.query.trim().to_owned();
                }
                if let Some(tab) = &self.active_tab {
                    self.online.retry(tab, &self.word);
                }
                resp.request_focus();
            }

            if self.button_side == ButtonSide::Right {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    window_buttons(ui, ButtonSide::Right);
                });
            }
        });
    }

    fn dictionaries_menu(&mut self, ui: &mut egui::Ui) {
        let (rect, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
        let fill = if resp.hovered() { ACCENT.gamma_multiply(0.85) } else { ACCENT };
        ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "D", FontId::proportional(24.0), Color32::WHITE);
        let resp = resp.on_hover_text("Dictionaries");

        let mut actions = Vec::new();
        #[cfg(feature = "screenshot")]
        if std::env::var_os("CSDICT_OPEN_MENU").is_some() && ui.ctx().cumulative_pass_nr() < 3 {
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&resp));
        }
        egui::Popup::menu(&resp)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .width(520.0)
            .show(|ui| {
                ui.set_min_width(520.0);
                ui.label(RichText::new("Dictionaries").strong().size(16.0));
                ui.add_space(4.0);
                egui::Grid::new("dict_grid").num_columns(3).spacing([12.0, 6.0]).striped(true).show(ui, |ui| {
                    let n = self.settings.dictionaries.len();
                    for (i, d) in self.settings.dictionaries.iter().enumerate() {
                        let Some(slot) = self.lib.slots.get(&d.id) else { continue };
                        let mut on = d.enabled;
                        let entries = slot.dict.as_ref().map(|x| x.len()).or(slot.remote.as_ref().map(|r| r.meta.entries as usize));
                        let tip = match entries {
                            Some(n) => format!("{} entries", n),
                            None => "not in catalog".to_owned(),
                        };
                        if ui.checkbox(&mut on, &slot.name).on_hover_text(tip).changed() {
                            actions.push(MenuAction::Toggle(i));
                        }
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if i > 0 && icon_button(ui, Icon::Up).on_hover_text("Move up").clicked() {
                                actions.push(MenuAction::Move(i, -1));
                            }
                            if i == 0 {
                                ui.add_space(24.0);
                            }
                            if i + 1 < n && icon_button(ui, Icon::Down).on_hover_text("Move down").clicked() {
                                actions.push(MenuAction::Move(i, 1));
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let dl = slot.download.as_ref().map(|d| d.lock().unwrap().clone());
                            match (&dl, slot.is_downloaded()) {
                                (Some(Download::Running { done, total }), _) => {
                                    let f = if *total > 0 { *done as f32 / *total as f32 } else { 0.0 };
                                    ui.add(egui::ProgressBar::new(f).desired_width(140.0).show_percentage());
                                }
                                (_, true) => {
                                    if ui.button("Remove").on_hover_text("Delete the downloaded dictionary").clicked() {
                                        actions.push(MenuAction::Remove(d.id.clone()));
                                    }
                                    ui.label(RichText::new("Downloaded").color(Color32::from_rgb(0x2e, 0x9e, 0x4f)));
                                }
                                (dl, false) => {
                                    if let Some(item) = &slot.remote {
                                        let label = format!("⬇ Download {:.1} MB", item.size as f64 / 1e6);
                                        if ui.button(label).clicked() {
                                            actions.push(MenuAction::Download(d.id.clone()));
                                        }
                                    }
                                    if let Some(Download::Failed(e)) = dl {
                                        ui.label(RichText::new("failed").color(Color32::RED)).on_hover_text(e);
                                    } else {
                                        ui.label(RichText::new("online").weak())
                                            .on_hover_text("Not downloaded – words are looked up on kaikki.org (Wiktionary)");
                                    }
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
                ui.separator();
                if ui.checkbox(&mut self.settings.online, "Look up online in dictionaries that are not downloaded").changed() {
                    self.save();
                }
                if ui.checkbox(&mut self.settings.dark_mode, "Dark mode").changed() {
                    apply_style(ui.ctx(), self.settings.dark_mode);
                    self.save();
                }
                ui.horizontal(|ui| {
                    ui.label("Catalog:");
                    let r = ui.add(
                        TextEdit::singleline(&mut self.catalog_edit)
                            .hint_text("URL or path of catalog.json")
                            .desired_width(330.0),
                    );
                    if ui.button("⟳ Reload").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
                        actions.push(MenuAction::ReloadCatalog);
                    }
                });
                match &*self.lib.catalog_state.lock().unwrap() {
                    CatalogState::Idle => {
                        ui.label(
                            RichText::new(
                                "No catalog found – enter the URL or path of catalog.json above \
                                 (build one with `cargo run --release -p dict-build -- all`).",
                            )
                            .weak(),
                        );
                    }
                    CatalogState::Loading => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Loading catalog…");
                        });
                    }
                    CatalogState::Loaded(items) => {
                        ui.label(RichText::new(format!("{} dictionaries available", items.len())).weak());
                    }
                    CatalogState::Failed(e) => {
                        ui.label(RichText::new(format!("Catalog error: {e}")).color(Color32::RED));
                    }
                }
                ui.collapsing("Sources & licenses", |ui| {
                    ui.label("Wiktionary content (CC BY-SA 4.0) via wiktextract / kaikki.org.");
                    ui.label("Založeno na svobodném anglicko-českém slovníku, https://www.svobodneslovniky.cz/ (GNU FDL).");
                    ui.label(format!("Dictionaries are stored in {}", self.dirs.dicts.display()));
                });
            });

        for a in actions {
            match a {
                MenuAction::Toggle(i) => {
                    self.settings.dictionaries[i].enabled ^= true;
                    self.dirty = true;
                }
                MenuAction::Move(i, d) => {
                    let j = (i as isize + d) as usize;
                    self.settings.dictionaries.swap(i, j);
                }
                MenuAction::Download(id) => self.lib.start_download(&id, ui.ctx().clone()),
                MenuAction::Remove(id) => {
                    self.entry_cache.clear();
                    if let Err(e) = self.lib.remove(&id) {
                        *self.message.lock().unwrap() = Some(format!("{e:#}"));
                    }
                    self.dirty = true;
                }
                MenuAction::ReloadCatalog => {
                    self.settings.catalog = self.catalog_edit.trim().to_owned();
                    if !self.settings.catalog.is_empty() {
                        self.lib.refresh_catalog(self.settings.catalog.clone(), ui.ctx().clone());
                    }
                }
            }
            self.save();
        }
    }

    // ------------------------------------------------------------ word list

    fn word_list(&mut self, ui: &mut egui::Ui) {
        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if self.results.is_empty() {
                ui.add_space(8.0);
                let msg = if self.query.trim().is_empty() {
                    "Start typing to search."
                } else if !self.enabled().any(|d| self.is_downloaded(&d.id)) {
                    "No downloaded dictionary selected – results are looked up online."
                } else {
                    "No matches."
                };
                ui.label(RichText::new(msg).weak());
                return;
            }
            let mut shown_similar = false;
            for (i, it) in self.results.iter().enumerate() {
                if it.kind == MatchKind::Similar && !shown_similar {
                    shown_similar = true;
                    ui.add_space(6.0);
                    ui.label(RichText::new("Similar words").small().weak());
                    ui.separator();
                }
                let selected = self.selected == Some(i);
                let mut text = RichText::new(&it.word).size(16.0);
                if it.kind == MatchKind::Exact || it.kind == MatchKind::Form {
                    text = text.strong();
                }
                let r = ui
                    .add(egui::Button::selectable(selected, text).min_size(vec2(ui.available_width(), 26.0)))
                    .on_hover_text(it.pos.join(", "));
                if selected && self.scroll_list_to_selected {
                    r.scroll_to_me(None);
                }
                if r.clicked() {
                    clicked = Some(i);
                }
            }
        });
        self.scroll_list_to_selected = false;
        if let Some(i) = clicked {
            self.select(i);
        }
    }

    // ---------------------------------------------------------- result pane

    fn result_pane(&mut self, ui: &mut egui::Ui) {
        let tabs: Vec<(String, String, String, bool)> = self
            .enabled()
            .filter_map(|d| self.lib.slots.get(&d.id))
            .map(|s| (s.id.clone(), s.name.clone(), s.source_lang.clone(), s.is_downloaded()))
            .collect();
        if tabs.is_empty() {
            ui.add_space(20.0);
            ui.label("No dictionary selected. Open the D menu in the top-left corner to choose dictionaries.");
            return;
        }
        if self.active_tab.as_ref().is_none_or(|t| !tabs.iter().any(|x| &x.0 == t)) {
            self.active_tab = Some(tabs[0].0.clone());
        }

        // Tab bar.
        ui.horizontal_wrapped(|ui| {
            for (id, name, _, downloaded) in &tabs {
                let count = if self.word.is_empty() {
                    None
                } else if *downloaded {
                    Some(self.offline_entries(id).len())
                } else {
                    match self.online.get(id, &self.word) {
                        Some(OnlineResult::Done(e)) => Some(e.len()),
                        _ => None,
                    }
                };
                let mut label = name.clone();
                if !*downloaded {
                    label.push_str(" · online");
                }
                if let Some(n) = count {
                    label.push_str(&format!("  {n}"));
                }
                let active = self.active_tab.as_deref() == Some(id.as_str());
                let mut text = RichText::new(label).size(14.0);
                if count == Some(0) && !active {
                    text = text.weak();
                }
                let tip = if *downloaded { "Downloaded dictionary" } else { "Looked up online (not downloaded)" };
                if ui.add(egui::Button::selectable(active, text)).on_hover_text(tip).clicked() {
                    self.active_tab = Some(id.clone());
                }
            }
        });
        ui.separator();

        let Some(tab) = self.active_tab.clone() else { return };
        let (_, _, src_lang, downloaded) = tabs.iter().find(|t| t.0 == tab).cloned().unwrap();
        if self.word.is_empty() {
            return;
        }

        let entries: Arc<Vec<Entry>> = if downloaded {
            self.offline_entries(&tab)
        } else if !self.settings.online {
            ui.label(RichText::new("This dictionary is not downloaded and online lookup is turned off.").weak());
            return;
        } else {
            let slot = &self.lib.slots[&tab];
            match self.online.get(&tab, &self.word) {
                Some(OnlineResult::Done(e)) => Arc::new(e.clone()),
                Some(OnlineResult::Pending) => {
                    let blocked = self.online.blocked_for();
                    ui.horizontal(|ui| {
                        ui.spinner();
                        match blocked {
                            Some(d) => ui.label(format!(
                                "Server asked to slow down – waiting {} s before looking up “{}”…",
                                d.as_secs() + 1,
                                self.word
                            )),
                            None => ui.label(format!("Looking up “{}” online…", self.word)),
                        };
                    });
                    if blocked.is_some() {
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                    }
                    return;
                }
                Some(OnlineResult::Failed(e)) => {
                    ui.label(RichText::new(format!("Online lookup failed: {e}")).color(Color32::RED));
                    ui.label(RichText::new("Press Enter to try again.").weak());
                    return;
                }
                None => {
                    let wait = DEBOUNCE.saturating_sub(self.last_edit.elapsed());
                    if wait.is_zero() {
                        let (s, t) = (slot.source_lang.clone(), slot.target_lang.clone());
                        self.online.request(&tab, &s, &t, &self.word.clone(), ui.ctx());
                    } else {
                        self.online.clear_wanted();
                        ui.ctx().request_repaint_after(wait);
                    }
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Waiting…");
                    });
                    return;
                }
            }
        };

        if entries.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("“{}” was not found in this dictionary.", self.word)).weak());
            return;
        }

        let mut lookup = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for (i, e) in entries.iter().enumerate() {
                if i > 0 {
                    ui.add_space(6.0);
                    ui.separator();
                }
                if let Some(w) = self.render_entry(ui, e, &src_lang) {
                    lookup = Some(w);
                }
            }
        });
        if let Some(w) = lookup {
            self.query = w;
            self.last_edit = Instant::now();
            self.dirty = true;
        }
    }

    /// Render one entry; returns a word the user asked to look up.
    fn render_entry(&self, ui: &mut egui::Ui, e: &Entry, lang: &str) -> Option<String> {
        let mut lookup = None;
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&e.headword).size(26.0).strong());
            if !e.pos.is_empty() {
                ui.label(RichText::new(pos_label(&e.pos)).italics().size(15.0).color(ACCENT));
            }
            for g in e.grammar.iter().filter(|g| g.chars().count() <= 24) {
                ui.label(RichText::new(g).size(15.0).weak());
            }
            if let Some(ipa) = e.ipa.first() {
                let r = ui.label(RichText::new(ipa).size(15.0));
                if e.ipa.len() > 1 {
                    r.on_hover_text(e.ipa.join("\n"));
                }
            }
            if let Some(h) = &e.hyphenation {
                ui.label(RichText::new(h).size(13.0).weak()).on_hover_text("Hyphenation");
            }
            let tip = if e.audio.is_empty() { "Speak (text-to-speech)" } else { "Play pronunciation" };
            if icon_button(ui, Icon::Speaker).on_hover_text(tip).clicked() {
                let msg = self.message.clone();
                let ctx = ui.ctx().clone();
                speech::pronounce(e.headword.clone(), lang.to_owned(), e.audio.clone(), self.dirs.cache.clone(), move |err| {
                    *msg.lock().unwrap() = Some(format!("Pronunciation failed: {err}"));
                    ctx.request_repaint();
                });
            }
            if icon_button(ui, Icon::Copy).on_hover_text("Copy entry").clicked() {
                ui.ctx().copy_text(entry_text(e));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(&e.source).small().weak());
            });
        });
        for g in e.grammar.iter().filter(|g| g.chars().count() > 24) {
            ui.label(RichText::new(g).weak());
        }
        ui.add_space(4.0);

        for (n, s) in e.senses.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label(RichText::new(format!("{}.  ", n + 1)).size(16.0).weak());
                if let Some(note) = &s.note {
                    ui.label(RichText::new(format!("({note})  ")).italics().weak());
                }
                for (k, t) in s.translations.iter().enumerate() {
                    let r = ui.add(egui::Label::new(RichText::new(t).size(17.0)).selectable(true));
                    r.context_menu(|ui| {
                        if ui.button("Copy").clicked() {
                            ui.ctx().copy_text(t.clone());
                            ui.close();
                        }
                        if ui.button(format!("Look up “{}”", strip_tags(t))).clicked() {
                            lookup = Some(strip_tags(t).to_owned());
                            ui.close();
                        }
                    });
                    if k + 1 < s.translations.len() {
                        ui.label(RichText::new(", ").size(17.0));
                    }
                }
            });
            for ex in &s.examples {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(24.0);
                    ui.label(RichText::new(&ex.text).italics());
                    if let Some(t) = &ex.translation {
                        ui.label(RichText::new(format!("— {t}")).weak());
                    }
                });
            }
            if !s.synonyms.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(24.0);
                    ui.label(RichText::new(format!("synonyms: {}", s.synonyms.join(", "))).weak());
                });
            }
        }
        if let Some(et) = &e.etymology {
            ui.add_space(4.0);
            egui::CollapsingHeader::new(RichText::new("Etymology").weak())
                .id_salt(("ety", &e.headword, &e.pos, &e.source))
                .show(ui, |ui| {
                    ui.label(et);
                });
        }
        lookup
    }
}

#[derive(Clone, Copy)]
enum Icon {
    Speaker,
    Copy,
    Up,
    Down,
}

/// Small painted icon button (does not depend on emoji fonts).
fn icon_button(ui: &mut egui::Ui, icon: Icon) -> egui::Response {
    let size = match icon {
        Icon::Up | Icon::Down => vec2(22.0, 20.0),
        _ => vec2(30.0, 28.0),
    };
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    let v = ui.style().interact(&r);
    ui.painter().rect_filled(rect, CornerRadius::same(6), v.weak_bg_fill);
    let c = rect.center();
    let stroke = Stroke::new(1.6, v.fg_stroke.color);
    let p = ui.painter();
    match icon {
        Icon::Speaker => {
            let body = vec![
                c + vec2(-8.0, -3.0),
                c + vec2(-4.0, -3.0),
                c + vec2(1.0, -8.0),
                c + vec2(1.0, 8.0),
                c + vec2(-4.0, 3.0),
                c + vec2(-8.0, 3.0),
            ];
            p.add(egui::Shape::convex_polygon(body, v.fg_stroke.color, Stroke::NONE));
            for (rad, n) in [(5.0f32, 6), (9.0, 8)] {
                let pts: Vec<_> = (0..=n)
                    .map(|i| {
                        let a = -0.8 + 1.6 * i as f32 / n as f32;
                        c + vec2(1.0 + rad * a.cos(), rad * a.sin())
                    })
                    .collect();
                p.add(egui::Shape::line(pts, stroke));
            }
        }
        Icon::Up | Icon::Down => {
            let d = if matches!(icon, Icon::Up) { -1.0 } else { 1.0 };
            let tri = vec![c + vec2(-5.0, -3.0 * d), c + vec2(5.0, -3.0 * d), c + vec2(0.0, 3.5 * d)];
            p.add(egui::Shape::convex_polygon(tri, v.fg_stroke.color, Stroke::NONE));
        }
        Icon::Copy => {
            let back = Rect::from_center_size(c + vec2(-2.5, -2.5), vec2(10.0, 12.0));
            let front = Rect::from_center_size(c + vec2(2.5, 2.5), vec2(10.0, 12.0));
            p.rect_stroke(back, CornerRadius::same(2), stroke, egui::StrokeKind::Middle);
            p.rect_filled(front, CornerRadius::same(2), v.weak_bg_fill);
            p.rect_stroke(front, CornerRadius::same(2), stroke, egui::StrokeKind::Middle);
        }
    }
    r
}

fn strip_tags(t: &str) -> &str {
    t.split(" (").next().unwrap_or(t).trim()
}

fn entry_text(e: &Entry) -> String {
    let mut s = format!("{} ({})", e.headword, pos_label(&e.pos));
    if let Some(ipa) = e.ipa.first() {
        s.push_str(&format!(" {ipa}"));
    }
    s.push('\n');
    for (i, sense) in e.senses.iter().enumerate() {
        s.push_str(&format!("{}. ", i + 1));
        if let Some(n) = &sense.note {
            s.push_str(&format!("({n}) "));
        }
        s.push_str(&sense.translations.join(", "));
        s.push('\n');
    }
    s
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.online.poll();
        let had = self.lib.slots.values().filter(|s| s.is_downloaded()).count();
        self.lib.poll_downloads();
        if self.lib.slots.values().filter(|s| s.is_downloaded()).count() != had {
            self.dirty = true;
        }
        let loaded = match &*self.lib.catalog_state.lock().unwrap() {
            CatalogState::Loaded(items) => Some(items.clone()),
            _ => None,
        };
        if let Some(items) = loaded {
            let new = self.lib.apply_catalog(&items);
            for id in new {
                self.settings.dictionaries.push(DictSetting { id, enabled: false });
            }
            // Keep the state as Loaded but avoid re-applying every frame.
            *self.lib.catalog_state.lock().unwrap() = CatalogState::Loaded(Vec::new());
            if !items.is_empty() {
                *self.lib.catalog_state.lock().unwrap() = CatalogState::Loaded(items);
            }
        }
        if self.dirty {
            self.run_search();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        handle_resize(ui.ctx());

        let bar_frame = egui::Frame::new()
            .fill(ui.visuals().panel_fill)
            .inner_margin(egui::Margin::symmetric(10, 8));
        egui::Panel::top("title_bar")
            .exact_size(TITLE_BAR_HEIGHT)
            .frame(bar_frame)
            .show(ui, |ui| self.title_bar(ui));

        if let Some(msg) = self.message.lock().unwrap().clone() {
            egui::Panel::bottom("message").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&msg).color(Color32::from_rgb(0xc0, 0x50, 0x20)));
                    if ui.small_button("✕").clicked() {
                        *self.message.lock().unwrap() = None;
                    }
                });
            });
        }

        egui::Panel::left("words")
            .resizable(true)
            .default_size(230.0)
            .size_range(140.0..=500.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                self.word_list(ui);
            });
        egui::CentralPanel::default().show(ui, |ui| self.result_pane(ui));
    }
}

// ---------------------------------------------------------------------------
// Window chrome
// ---------------------------------------------------------------------------

fn apply_style(ctx: &egui::Context, dark: bool) {
    ctx.set_visuals(if dark { egui::Visuals::dark() } else { egui::Visuals::light() });
    ctx.global_style_mut(|s| {
        s.spacing.button_padding = vec2(8.0, 4.0);
        s.interaction.selectable_labels = true;
    });
}

/// Which side the window buttons go on.
fn detect_button_side() -> ButtonSide {
    if let Ok(v) = std::env::var("CSDICT_BUTTONS") {
        return if v == "left" { ButtonSide::Left } else { ButtonSide::Right };
    }
    if cfg!(target_os = "macos") {
        return ButtonSide::Left;
    }
    if cfg!(target_os = "linux") {
        // GNOME-style setting, e.g. "close,minimize,maximize:" (left) or
        // "appmenu:minimize,maximize,close" (right).
        if let Ok(out) = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.wm.preferences", "button-layout"])
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some((left, _)) = s.trim().trim_matches('\'').split_once(':')
                && left.contains("close")
            {
                return ButtonSide::Left;
            }
        }
    }
    ButtonSide::Right
}

fn window_buttons(ui: &mut egui::Ui, side: ButtonSide) {
    let ctx = ui.ctx().clone();
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    if side == ButtonSide::Left && cfg!(target_os = "macos") {
        // macOS "traffic lights": close, minimize, zoom.
        ui.spacing_mut().item_spacing.x = 8.0;
        let colors = [
            Color32::from_rgb(0xff, 0x5f, 0x57),
            Color32::from_rgb(0xfe, 0xbc, 0x2e),
            Color32::from_rgb(0x28, 0xc8, 0x40),
        ];
        let glyphs = ["×", "−", "+"];
        let group_hover = ui.rect_contains_pointer(Rect::from_min_size(ui.cursor().min, vec2(60.0, TITLE_BAR_HEIGHT)));
        for (k, c) in colors.iter().enumerate() {
            let (rect, r) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::click());
            ui.painter().circle_filled(rect.center(), 6.0, *c);
            if group_hover {
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyphs[k], FontId::proportional(11.0), Color32::from_black_alpha(160));
            }
            if r.clicked() {
                match k {
                    0 => ctx.send_viewport_cmd(ViewportCommand::Close),
                    1 => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                    _ => ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized)),
                }
            }
        }
        ui.add_space(6.0);
        return;
    }

    // Windows / Linux style. In a right-to-left layout the first button is
    // the rightmost, so order depends on the side.
    #[derive(Clone, Copy)]
    enum B {
        Min,
        Max,
        Close,
    }
    let order = match side {
        ButtonSide::Right => [B::Close, B::Max, B::Min],
        ButtonSide::Left => [B::Close, B::Min, B::Max],
    };
    ui.spacing_mut().item_spacing.x = 0.0;
    for b in order {
        let (rect, r) = ui.allocate_exact_size(vec2(40.0, 32.0), Sense::click());
        let hover_fill = match b {
            B::Close => Color32::from_rgb(0xe8, 0x11, 0x23),
            _ => ui.visuals().widgets.hovered.weak_bg_fill,
        };
        if r.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(4), hover_fill);
        }
        let color = if r.hovered() && matches!(b, B::Close) { Color32::WHITE } else { ui.visuals().text_color() };
        let stroke = Stroke::new(1.2, color);
        let c = rect.center();
        let p = ui.painter();
        match b {
            B::Min => {
                p.line_segment([pos2(c.x - 5.0, c.y), pos2(c.x + 5.0, c.y)], stroke);
            }
            B::Max if maximized => {
                p.rect_stroke(Rect::from_center_size(c + vec2(-1.5, 1.5), Vec2::splat(8.0)), 0.0, stroke, egui::StrokeKind::Middle);
                p.line_segment([pos2(c.x - 3.5, c.y - 4.5), pos2(c.x + 4.5, c.y - 4.5)], stroke);
                p.line_segment([pos2(c.x + 4.5, c.y - 4.5), pos2(c.x + 4.5, c.y + 3.5)], stroke);
            }
            B::Max => {
                p.rect_stroke(Rect::from_center_size(c, Vec2::splat(10.0)), 0.0, stroke, egui::StrokeKind::Middle);
            }
            B::Close => {
                p.line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], stroke);
                p.line_segment([c + vec2(5.0, -5.0), c + vec2(-5.0, 5.0)], stroke);
            }
        }
        let r = r.on_hover_text(match b {
            B::Min => "Minimize",
            B::Max if maximized => "Restore",
            B::Max => "Maximize",
            B::Close => "Close",
        });
        if r.clicked() {
            match b {
                B::Min => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                B::Max => ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized)),
                B::Close => ctx.send_viewport_cmd(ViewportCommand::Close),
            }
        }
    }
}

/// Without OS decorations we provide our own resize borders.
fn handle_resize(ctx: &egui::Context) {
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    if maximized {
        return;
    }
    let rect = ctx.content_rect();
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let m = RESIZE_MARGIN;
    let (l, r) = (pos.x <= rect.left() + m, pos.x >= rect.right() - m);
    let (t, b) = (pos.y <= rect.top() + m, pos.y >= rect.bottom() - m);
    let dir = match (l, r, t, b) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::West,
        (_, true, ..) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (_, _, _, true) => ResizeDirection::South,
        _ => return,
    };
    let cursor = match dir {
        ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
    };
    ctx.set_cursor_icon(cursor);
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
    }
}
