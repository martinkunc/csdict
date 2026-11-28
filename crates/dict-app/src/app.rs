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

use crate::library::{CatalogState, Download, Group, Library, Slot};
use crate::online::{self, DEBOUNCE, Online, OnlineResult};
use crate::settings::{DictSetting, Dirs, Settings, Theme};
use crate::speech;

const TITLE_BAR_HEIGHT: f32 = 56.0;
const RESIZE_MARGIN: f32 = 5.0;
const LIST_LIMIT: usize = 60;
const ACCENT: Color32 = Color32::from_rgb(0x2f, 0x6f, 0xd6);

/// Entries of one dictionary of the shown group.
struct Part {
    /// Direction and source shown next to each entry.
    label: String,
    /// Language of the headwords (for pronunciation).
    lang: String,
    online: bool,
    entries: Arc<Vec<Entry>>,
}

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
    /// Download every missing dictionary of a group.
    Download(String),
    /// Remove every downloaded dictionary of a group.
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
    /// Group id of the shown tab.
    active_tab: Option<String>,
    entry_cache: HashMap<(String, String), Arc<Vec<Entry>>>,
    dirty: bool,
    scroll_list_to_selected: bool,

    button_side: ButtonSide,
    message: Arc<Mutex<Option<String>>>,
    catalog_edit: String,
}

impl App {
    /// `query`: word to look up at start (`csdict <word>`).
    pub fn new(cc: &eframe::CreationContext<'_>, query: Option<String>) -> Self {
        let dirs = Dirs::new();
        let settings = dirs.load_settings();
        let lib = Library::new(dirs.dicts.clone());
        apply_style(&cc.egui_ctx, settings.theme);
        cc.egui_ctx.set_zoom_factor(settings.zoom);
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
        let mut app = app;
        if let Some(q) = query {
            app.query = q;
            app.dirty = true;
        }
        if !app.settings.catalog.is_empty() {
            app.lib.refresh_catalog(app.settings.catalog.clone(), cc.egui_ctx.clone());
        }
        app
    }

    /// Selected dictionary groups in menu order.
    fn enabled_groups(&self) -> Vec<Group> {
        let groups = self.lib.groups();
        self.settings
            .dictionaries
            .iter()
            .filter(|d| d.enabled)
            .filter_map(|d| groups.iter().find(|g| g.id == d.id).cloned())
            .collect()
    }

    /// List every group in the menu (new catalog dictionaries appear unselected).
    fn sync_group_settings(&mut self) {
        for g in self.lib.groups() {
            if !self.settings.dictionaries.iter().any(|d| d.id == g.id) {
                self.settings.dictionaries.push(DictSetting { id: g.id, enabled: false });
            }
        }
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
            let members: Vec<String> = self.enabled_groups().into_iter().flat_map(|g| g.members).collect();
            for id in &members {
                let Some(dict) = self.lib.slots.get(id).and_then(|s| s.dict.as_ref()) else { continue };
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

    /// Keep the current tab if it may have results, otherwise switch to the
    /// first group whose downloaded dictionaries have the word.
    fn pick_tab(&mut self) {
        let groups = self.enabled_groups();
        if groups.is_empty() {
            self.active_tab = None;
            return;
        }
        let cur = self.active_tab.clone().filter(|t| groups.iter().any(|g| &g.id == t));
        if let Some(g) = cur.as_ref().and_then(|t| groups.iter().find(|g| &g.id == t))
            && (g.members.iter().any(|m| !self.is_downloaded(m)) || self.offline_count(g) > 0)
        {
            return;
        }
        let first_hit = groups.iter().find(|g| self.offline_count(g) > 0).map(|g| g.id.clone());
        self.active_tab = first_hit.or(cur).or_else(|| Some(groups[0].id.clone()));
    }

    fn is_downloaded(&self, id: &str) -> bool {
        self.lib.slots.get(id).is_some_and(|s| s.is_downloaded())
    }

    /// Entries for the word in the downloaded dictionaries of a group.
    fn offline_count(&mut self, g: &Group) -> usize {
        let mut n = 0;
        for m in &g.members {
            if self.is_downloaded(m) {
                n += self.offline_entries(m).len();
            }
        }
        n
    }

    /// Entries known so far for the word in a group, and whether more may
    /// still come from online lookups.
    fn group_count(&mut self, g: &Group) -> (Option<usize>, bool) {
        let (mut known, mut pending) = (None, false);
        for m in &g.members {
            let n = if self.is_downloaded(m) {
                Some(self.offline_entries(m).len())
            } else {
                match self.online.get(m, &self.word) {
                    Some(OnlineResult::Done(e)) => Some(e.len()),
                    Some(OnlineResult::Failed(_)) => None,
                    _ => {
                        pending |= self.settings.online;
                        None
                    }
                }
            };
            if let Some(n) = n {
                known = Some(known.unwrap_or(0) + n);
            }
        }
        (known, pending)
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
            // The hint does not inherit `.font()`: give it the same size, and
            // centre both vertically (default alignment is top).
            let edit = TextEdit::singleline(&mut self.query)
                .font(FontId::proportional(24.0))
                .hint_text(RichText::new("Type a word…").size(24.0))
                .align(Align2::LEFT_CENTER)
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
                if let Some(g) = self.active_tab.as_ref().and_then(|t| self.lib.group(t)) {
                    for m in &g.members {
                        self.online.retry(m, &self.word);
                    }
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
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Dictionaries"));
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
                let max_list = (ui.ctx().content_rect().height() - 300.0).max(120.0);
                egui::ScrollArea::vertical().max_height(max_list).show(ui, |ui| {
                    egui::Grid::new("dict_grid").num_columns(3).spacing([12.0, 6.0]).striped(true).show(ui, |ui| {
                        let n = self.settings.dictionaries.len();
                        let groups = self.lib.groups();
                        for (i, d) in self.settings.dictionaries.iter().enumerate() {
                            let Some(g) = groups.iter().find(|g| g.id == d.id) else { continue };
                            let slots: Vec<&Slot> = g.members.iter().filter_map(|m| self.lib.slots.get(m)).collect();
                            let mut on = d.enabled;
                            if ui.checkbox(&mut on, &g.name).on_hover_text(group_tooltip(&slots)).changed() {
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
                                let downloads = download_states(&slots);
                                if let Some(f) = download_progress(&downloads) {
                                    ui.add(egui::ProgressBar::new(f).desired_width(140.0).show_percentage());
                                    return;
                                }
                                let have = slots.iter().filter(|s| s.is_downloaded()).count();
                                let missing: u64 = slots
                                    .iter()
                                    .filter(|s| !s.is_downloaded())
                                    .filter_map(|s| s.remote.as_ref().map(|r| r.size))
                                    .sum();
                                if have > 0 && ui.button("Remove").on_hover_text("Delete the downloaded dictionaries").clicked() {
                                    actions.push(MenuAction::Remove(g.id.clone()));
                                }
                                if missing > 0 && ui.button(format!("⬇ Download {:.1} MB", missing as f64 / 1e6)).clicked() {
                                    actions.push(MenuAction::Download(g.id.clone()));
                                }
                                let updates: u64 = slots
                                    .iter()
                                    .filter(|s| s.update_available())
                                    .filter_map(|s| s.remote.as_ref().map(|r| r.size))
                                    .sum();
                                if updates > 0
                                    && ui
                                        .button(format!("⟳ Update {:.1} MB", updates as f64 / 1e6))
                                        .on_hover_text("A newer build of this dictionary is available")
                                        .clicked()
                                {
                                    actions.push(MenuAction::Download(g.id.clone()));
                                }
                                let failed: Vec<&str> = downloads
                                    .iter()
                                    .filter_map(|d| if let Download::Failed(e) = d { Some(e.as_str()) } else { None })
                                    .collect();
                                if !failed.is_empty() {
                                    ui.label(RichText::new("failed").color(Color32::RED)).on_hover_text(failed.join("\n"));
                                } else if have == slots.len() {
                                    ui.label(RichText::new("Downloaded").color(Color32::from_rgb(0x2e, 0x9e, 0x4f)));
                                } else if have > 0 {
                                    ui.label(RichText::new(format!("{have}/{} downloaded", slots.len())))
                                        .on_hover_text("The rest is looked up online");
                                } else {
                                    ui.label(RichText::new("online").weak()).on_hover_text(
                                        "Not downloaded – words are looked up on kaikki.org (Wiktionary) and en-word.net (WordNet)",
                                    );
                                }
                            });
                            ui.end_row();
                        }
                    });
                });
                ui.separator();
                if ui.checkbox(&mut self.settings.online, "Look up online in dictionaries that are not downloaded").changed() {
                    self.save();
                }
                ui.horizontal(|ui| {
                    ui.label("Theme:");
                    let mut changed = false;
                    for (t, name) in [(Theme::System, "Same as system"), (Theme::Light, "Light"), (Theme::Dark, "Dark")] {
                        changed |= ui.selectable_value(&mut self.settings.theme, t, name).changed();
                    }
                    if changed {
                        apply_style(ui.ctx(), self.settings.theme);
                        self.save();
                    }
                });
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
                    ui.label("Open English WordNet (CC BY 4.0), https://en-word.net/.");
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
                MenuAction::Download(gid) => self.download_group(&gid, ui.ctx()),
                MenuAction::Remove(gid) => {
                    self.entry_cache.clear();
                    for id in self.lib.group(&gid).map(|g| g.members).unwrap_or_default() {
                        if let Err(e) = self.lib.remove(&id) {
                            *self.message.lock().unwrap() = Some(format!("{e:#}"));
                        }
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
                } else if !self.enabled_groups().iter().flat_map(|g| &g.members).any(|m| self.is_downloaded(m)) {
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
        let tabs = self.enabled_groups();
        if tabs.is_empty() {
            ui.add_space(20.0);
            ui.label("No dictionary selected. Open the D menu in the top-left corner to choose dictionaries.");
            return;
        }
        if self.active_tab.as_ref().is_none_or(|t| !tabs.iter().any(|g| &g.id == t)) {
            self.active_tab = Some(tabs[0].id.clone());
        }

        // Tab bar: one tab per group, with the number of entries found.
        ui.horizontal_wrapped(|ui| {
            for g in &tabs {
                let (count, pending) = if self.word.is_empty() { (None, false) } else { self.group_count(g) };
                let mut label = g.name.clone();
                if !g.members.iter().any(|m| self.is_downloaded(m)) {
                    label.push_str(" · online");
                }
                if let Some(n) = count {
                    label.push_str(&format!("  {n}{}", if pending { "…" } else { "" }));
                }
                let active = self.active_tab.as_deref() == Some(g.id.as_str());
                let mut text = RichText::new(label).size(14.0);
                if count == Some(0) && !pending && !active {
                    text = text.weak();
                }
                let tip: Vec<String> = g
                    .members
                    .iter()
                    .filter_map(|m| self.lib.slots.get(m))
                    .map(|s| format!("{} ({})", s.name, if s.is_downloaded() { "downloaded" } else { "online" }))
                    .collect();
                if ui.add(egui::Button::selectable(active, text)).on_hover_text(tip.join("\n")).clicked() {
                    self.active_tab = Some(g.id.clone());
                }
            }
        });
        ui.separator();

        let Some(group) = self.active_tab.as_ref().and_then(|t| tabs.iter().find(|g| &g.id == t)).cloned() else {
            return;
        };
        self.offline_only_hint(ui, &group);
        if self.word.is_empty() {
            return;
        }
        let word = self.word.clone();

        // Dictionaries that are not downloaded are looked up online once the
        // word has stopped changing.
        if self.settings.online {
            let missing: Vec<String> = group
                .members
                .iter()
                .filter(|m| !self.is_downloaded(m) && self.online.get(m, &word).is_none())
                .cloned()
                .collect();
            if !missing.is_empty() {
                let wait = DEBOUNCE.saturating_sub(self.last_edit.elapsed());
                if wait.is_zero() {
                    for m in &missing {
                        let slot = &self.lib.slots[m];
                        let (s, t) = (slot.source_lang.clone(), slot.target_lang.clone());
                        self.online.request(m, &s, &t, &word, ui.ctx());
                    }
                } else {
                    self.online.clear_wanted();
                    ui.ctx().request_repaint_after(wait);
                }
            }
        }

        // Collect entries of every dictionary of the group.
        let (mut parts, mut pending, mut failed, mut off) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for m in &group.members {
            let Some(slot) = self.lib.slots.get(m) else { continue };
            let (dir, lang) = (slot.direction(), slot.source_lang.clone());
            if slot.is_downloaded() {
                let entries = self.offline_entries(m);
                parts.push(Part { label: dir, lang, online: false, entries });
            } else if !self.settings.online {
                off.push(slot.name.clone());
            } else {
                match self.online.get(m, &word) {
                    Some(OnlineResult::Done(e)) => parts.push(Part { label: dir, lang, online: true, entries: e.clone() }),
                    Some(OnlineResult::Failed(e)) => failed.push(format!("{dir}: {e}")),
                    Some(OnlineResult::Pending) | None => pending.push(dir),
                }
            }
        }
        let found: usize = parts.iter().map(|p| p.entries.len()).sum();

        if !pending.is_empty() {
            let blocked = self.online.blocked_for();
            ui.horizontal(|ui| {
                ui.spinner();
                match blocked {
                    Some(d) => ui.label(format!(
                        "Server asked to slow down – waiting {} s before looking up “{word}”…",
                        d.as_secs() + 1
                    )),
                    None => ui.label(format!("Looking up “{word}” online ({})…", pending.join(", "))),
                };
            });
            if blocked.is_some() {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            }
        }
        for f in &failed {
            ui.label(RichText::new(format!("Online lookup failed – {f}")).color(Color32::RED));
        }
        if !failed.is_empty() {
            ui.label(RichText::new("Press Enter to try again.").weak());
        }
        if found == 0 {
            if pending.is_empty() {
                ui.add_space(8.0);
                if parts.is_empty() && failed.is_empty() {
                    ui.label(RichText::new("These dictionaries are not downloaded and online lookup is turned off.").weak());
                } else if failed.is_empty() {
                    ui.label(RichText::new(format!("“{word}” was not found in {}.", group.name)).weak());
                }
            }
            return;
        }
        if !off.is_empty() {
            ui.label(RichText::new(format!("Not downloaded, online lookup is off: {}", off.join(", "))).weak());
        }

        let mut lookup = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let mut first = true;
            for p in &parts {
                for e in p.entries.iter() {
                    if !first {
                        ui.add_space(6.0);
                        ui.separator();
                    }
                    first = false;
                    let mut label = format!("{} · {}", p.label, e.source);
                    if p.online {
                        label.push_str(" · online");
                    }
                    if let Some(w) = self.render_entry(ui, e, &p.lang, &label) {
                        lookup = Some(w);
                    }
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
    /// Status bar at the bottom of the window while dictionaries download:
    /// one line per dictionary group with its combined progress.
    fn download_status(&self, ui: &mut egui::Ui) {
        let running: Vec<(String, u64, u64)> = self
            .lib
            .groups()
            .into_iter()
            .filter_map(|g| {
                let slots: Vec<&Slot> = g.members.iter().filter_map(|m| self.lib.slots.get(m)).collect();
                download_bytes(&download_states(&slots)).map(|(d, t)| (g.name, d, t))
            })
            .collect();
        if running.is_empty() {
            return;
        }
        egui::Panel::bottom("downloads").show(ui, |ui| {
            ui.add_space(2.0);
            for (name, done, total) in &running {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Downloading {name}…"));
                    let f = if *total > 0 { *done as f32 / *total as f32 } else { 0.0 };
                    let text = format!("{:.1} / {:.1} MB", *done as f64 / 1e6, *total as f64 / 1e6);
                    ui.add(egui::ProgressBar::new(f).desired_width(260.0).text(text));
                });
            }
            ui.add_space(2.0);
        });
    }

    /// Start downloading every dictionary of a group that is not downloaded yet.
    fn download_group(&mut self, gid: &str, ctx: &egui::Context) {
        for id in self.lib.group(gid).map(|g| g.members).unwrap_or_default() {
            let slot = &self.lib.slots[&id];
            let busy = slot.download.as_ref().is_some_and(|d| matches!(*d.lock().unwrap(), Download::Running { .. }));
            if (!slot.is_downloaded() || slot.update_available()) && slot.remote.is_some() && !busy {
                self.lib.start_download(&id, ctx.clone());
            }
        }
    }

    /// Hint above the results when a dictionary of the group is not
    /// downloaded and contains sources that cannot be looked up online
    /// (e.g. the GNU/FDL slovník), with a button to download the group.
    fn offline_only_hint(&mut self, ui: &mut egui::Ui, g: &Group) {
        // source → directions in which it is missing
        let mut missing: Vec<(String, Vec<String>)> = Vec::new();
        let mut size = 0u64;
        let slots: Vec<&Slot> = g.members.iter().filter_map(|m| self.lib.slots.get(m)).collect();
        for s in slots.iter().filter(|s| !s.is_downloaded()) {
            let Some(remote) = &s.remote else { continue };
            size += remote.size;
            for src in remote.meta.sources.split(", ").filter(|x| !x.is_empty() && !online::available_online(x)) {
                match missing.iter_mut().find(|(x, _)| x == src) {
                    Some((_, dirs)) => dirs.push(s.direction()),
                    None => missing.push((src.to_owned(), vec![s.direction()])),
                }
            }
        }
        if missing.is_empty() {
            return;
        }
        let downloads = download_states(&slots);
        let mut start = false;
        egui::Frame::new()
            .fill(Color32::from_rgb(0xe0, 0x9a, 0x20).gamma_multiply(0.16))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    let what: Vec<String> = missing.iter().map(|(s, d)| format!("{s} ({})", d.join(", "))).collect();
                    ui.label(format!(
                        "{} cannot be looked up online, so {} translations are not shown until the dictionary is downloaded.",
                        what.join(" and "),
                        if missing.len() > 1 { "their" } else { "its" },
                    ));
                    if let Some(f) = download_progress(&downloads) {
                        ui.add(egui::ProgressBar::new(f).desired_width(160.0).show_percentage());
                        return;
                    }
                    let failed: Vec<&str> = downloads
                        .iter()
                        .filter_map(|d| if let Download::Failed(e) = d { Some(e.as_str()) } else { None })
                        .collect();
                    if !failed.is_empty() {
                        ui.label(RichText::new("Download failed").color(Color32::RED)).on_hover_text(failed.join("\n"));
                    }
                    let label = format!("⬇ Download {} ({:.1} MB)", g.name, size as f64 / 1e6);
                    if ui.button(label).on_hover_text("Download the complete dictionary for offline use").clicked() {
                        start = true;
                    }
                });
            });
        ui.add_space(4.0);
        if start {
            self.download_group(&g.id, ui.ctx());
        }
    }

    /// `label` names the dictionary direction and source of the entry.
    fn render_entry(&self, ui: &mut egui::Ui, e: &Entry, lang: &str, label: &str) -> Option<String> {
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
                egui::Frame::new()
                    .fill(ACCENT.gamma_multiply(0.14))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| ui.label(RichText::new(label).small().color(ACCENT)));
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
                .id_salt(("ety", &e.headword, &e.pos, label))
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

/// Download states of the given dictionaries (those with a download).
fn download_states(slots: &[&Slot]) -> Vec<Download> {
    slots.iter().filter_map(|s| s.download.as_ref().map(|d| d.lock().unwrap().clone())).collect()
}

/// Combined progress (0–1) of running downloads, `None` if none is running.
fn download_progress(downloads: &[Download]) -> Option<f32> {
    download_bytes(downloads).map(|(done, total)| if total > 0 { done as f32 / total as f32 } else { 0.0 })
}

/// Bytes (done, total) of running downloads, `None` if none is running.
fn download_bytes(downloads: &[Download]) -> Option<(u64, u64)> {
    let (mut done, mut total, mut running) = (0, 0, false);
    for dl in downloads {
        if let Download::Running { done: d, total: t } = dl {
            (done, total, running) = (done + d, total + t, true);
        }
    }
    running.then_some((done, total))
}

/// Hover text of a dictionary group in the menu: its dictionaries and their state.
fn group_tooltip(slots: &[&Slot]) -> String {
    slots
        .iter()
        .map(|s| {
            let state = match (&s.dict, &s.remote) {
                (Some(d), _) => format!("downloaded, {} entries", d.len()),
                (None, Some(r)) => format!("online; {} entries to download", r.meta.entries),
                (None, None) => "online".to_owned(),
            };
            format!("{} – {state}", s.name)
        })
        .collect::<Vec<_>>()
        .join("\n")
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
    // Only the window geometry is persisted by eframe; zoom and theme live
    // in settings.json and must not be overridden by egui's saved memory.
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.online.poll();
        if self.lib.poll_downloads() {
            // New or updated packs: cached entries may be stale.
            self.entry_cache.clear();
            self.dirty = true;
        }
        // Ctrl/Cmd + / − / 0 zoom is built into egui; remember it.
        if (ctx.zoom_factor() - self.settings.zoom).abs() > 0.001 {
            self.settings.zoom = ctx.zoom_factor();
            self.dirs.save_settings(&self.settings);
        }
        let loaded = match &*self.lib.catalog_state.lock().unwrap() {
            CatalogState::Loaded(items) => Some(items.clone()),
            _ => None,
        };
        if let Some(items) = loaded {
            self.lib.apply_catalog(&items);
            self.sync_group_settings();
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

        // Clone first: the lock must not be held while drawing (✕ locks it).
        let msg = self.message.lock().unwrap().clone();
        if let Some(msg) = msg {
            egui::Panel::bottom("message").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&msg).color(Color32::from_rgb(0xc0, 0x50, 0x20)));
                    if ui.small_button("✕").clicked() {
                        *self.message.lock().unwrap() = None;
                    }
                });
            });
        }

        self.download_status(ui);

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

fn apply_style(ctx: &egui::Context, theme: Theme) {
    // `System` follows the OS light / dark setting, also when it changes.
    ctx.set_theme(theme);
    ctx.all_styles_mut(|s| {
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
                    1 => minimize(&ctx),
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
                B::Min => minimize(&ctx),
                B::Max => ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized)),
                B::Close => ctx.send_viewport_cmd(ViewportCommand::Close),
            }
        }
    }
}

/// Minimizes the window.
///
/// On macOS this bypasses `ViewportCommand::Minimized`: that command makes egui
/// remember the window as minimized, and egui never re-reads that state on
/// macOS (see egui#3494), so after the window is restored from the Dock it
/// stays "invisible" to eframe, which then stops painting it: the app looks
/// frozen. Minimizing the NSWindow directly leaves egui's state alone; eframe
/// still notices through the window's occlusion events.
fn minimize(ctx: &egui::Context) {
    #[cfg(target_os = "macos")]
    if let Some(mtm) = objc2::MainThreadMarker::new()
        && let Some(window) = objc2_app_kit::NSApplication::sharedApplication(mtm).keyWindow()
    {
        window.miniaturize(None);
        return;
    }
    ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// The tests set process-wide environment variables: run them one at a time.
    static ENV: Mutex<()> = Mutex::new(());

    /// Point the app at a fresh, empty home folder (never the user's own).
    fn test_home(name: &str) -> (std::sync::MutexGuard<'static, ()>, std::path::PathBuf) {
        let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("csdict-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        // SAFETY: serialized by `ENV`; no other thread reads the environment meanwhile.
        unsafe { std::env::set_var("CSDICT_HOME", &home) };
        (guard, home)
    }

    fn step_until(h: &mut Harness<'_, App>, what: &str, mut done: impl FnMut(&mut Harness<'_, App>) -> bool) {
        for _ in 0..600 {
            h.step();
            if done(h) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    #[test]
    fn download_status_bar() {
        let (_env, home) = test_home("status");
        let mut h = Harness::builder()
            .with_size(vec2(960.0, 640.0))
            .allow_missing_glyphs()
            .build_eframe(|cc| App::new(cc, None));
        h.run_steps(2);
        assert!(h.query_by_label_contains("Downloading").is_none());
        let state = Arc::new(Mutex::new(Download::Running { done: 5_000_000, total: 20_000_000 }));
        h.state_mut().lib.slots.get_mut("en-wn").unwrap().download = Some(state.clone());
        h.run_steps(2);
        h.get_by_label("Downloading Czech ↔ English…");
        h.get_by_label_contains("5.0 / 20.0 MB");
        *state.lock().unwrap() = Download::Failed("x".into());
        h.run_steps(2);
        assert!(h.query_by_label_contains("Downloading").is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Start with nothing downloaded, download Czech ↔ English from the
    /// hint above the results, then open the D menu (this used to freeze).
    /// Needs the locally built `dist/` (skipped if absent).
    #[test]
    fn download_from_hint_then_open_menu() {
        let Some(catalog) = crate::settings::find_local_catalog() else { return };
        let (_env, home) = test_home("download");
        // SAFETY: serialized by `ENV` (held in `_env`).
        unsafe { std::env::set_var("CSDICT_CATALOG", &catalog) };
        let mut h = Harness::builder()
            .with_size(vec2(960.0, 640.0))
            .allow_missing_glyphs()
            .build_eframe(|cc| App::new(cc, Some("solicit".into())));

        step_until(&mut h, "the download hint", |h| h.query_by_label_contains("Download Czech ↔ English").is_some());
        if std::env::var_os("CSDICT_TEST_ONLINE").is_some() {
            // Let the online lookups finish first, as a user would.
            step_until(&mut h, "online results", |h| {
                ["cs-en", "en-cs", "en-wn"]
                    .iter()
                    .all(|id| matches!(h.state().online.get(id, "solicit"), Some(OnlineResult::Done(_))))
            });
        } else {
            h.state_mut().settings.online = false;
        }
        h.get_by_label_contains("Download Czech ↔ English").click();
        step_until(&mut h, "the downloads", |h| {
            ["cs-en", "en-cs", "en-wn"].iter().all(|id| h.state().lib.slots[*id].is_downloaded())
        });
        h.run_steps(5);
        assert!(h.query_by_label_contains("Download Czech ↔ English").is_none(), "hint still shown");

        h.get_by_label("Dictionaries").click();
        h.run_steps(5);
        h.get_by_label_contains("Czech ↔ German");
        // The last group is reachable (the list scrolls when it is long).
        h.get_by_label_contains("Korean ↔ English");
        let _ = std::fs::remove_dir_all(&home);
    }
}
