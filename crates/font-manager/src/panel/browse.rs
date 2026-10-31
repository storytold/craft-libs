//! The tabs and search, the filters, the sample toolbar and the list of family cards.

use std::sync::Arc;

use craft_ui::direction;
use egui::{CornerRadius, FontFamily, FontId, Id, Rect, Role, Sense, StrokeKind, Ui, UiBuilder, WidgetInfo, pos2, vec2};

use crate::index::{Category, Index, script_name, source_label};
use crate::panel::app::{Detail, FontsPanel, Origin, Pending, Preset, Show, Tab, category_label, family_key, regular, source_name, style_count};
use crate::panel::faces::{Owner, Shown};
use crate::panel::i18n::{self, fmt, tl, tn};
use crate::panel::jobs::Kind;
use crate::panel::widgets::{self, Cell, Glyph, Skin};
use crate::{FaceInfo, Source};

/// Space round a card's contents.
const CARD_PAD: f32 = 20.0;
const CARD_TOP: f32 = 14.0;
const CARD_GAP: f32 = 8.0;
const CARD_BOTTOM: f32 = 18.0;

/// What a card shows: `key` sets it apart from a family of the same name from another source.
struct Card<'a> {
    key: &'a str,
    name: &'a str,
    meta: &'a str,
    sample: &'a str,
    curated: bool,
    index_position: Option<usize>,
}

/// What a card shows at its reading end.
enum CardState {
    Running(Kind, Option<f32>),
    Failed(Kind, String, Option<Pending>),
    Installed,
    Removable(Vec<FaceInfo>),
    System,
    Offer { preview: bool },
}

impl FontsPanel {
    pub(crate) fn top_bar(&mut self, ui: &mut Ui) {
        let s = self.skin.clone();
        ui.allocate_ui_with_layout(vec2(ui.available_width(), s.h()), direction::row(ui), |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let mut tab = self.tab;
            let browse = self.catalogue.as_ref().map_or(0, |c| c.index.families().len());
            let options = [
                Cell { value: Tab::Catalogue, label: tl!("Browse"), count: Some(browse) },
                Cell { value: Tab::Installed, label: tl!("Installed"), count: Some(self.installed.len()) },
            ];
            if widgets::cells_row(ui, &s, &mut tab, &options) {
                self.set_tab(tab);
            }
            let (text, hint) = match self.tab {
                Tab::Catalogue => (&mut self.query.text, tl!("Search fonts by name")),
                Tab::Installed => (&mut self.installed_text, tl!("Search installed fonts")),
            };
            let field = widgets::field(ui, &s, text, hint, tl!("Search"), ui.available_width(), true);
            if std::mem::take(&mut self.focus_search) {
                field.request_focus();
            }
            if field.changed() {
                self.focus_row = None;
            }
        });
    }

    pub(crate) fn sidebar(&mut self, ui: &mut Ui) {
        let s = self.skin.clone();
        egui::ScrollArea::vertical().id_salt("craft-fonts.filters").auto_shrink([false, false]).show(ui, |ui| {
            ui.with_layout(direction::column(ui), |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                match self.tab {
                    Tab::Catalogue => self.catalogue_filters(ui, &s),
                    Tab::Installed => self.installed_filters(ui, &s),
                }
                if self.downloads_switch {
                    ui.add_space(14.0);
                    ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), s.hairline());
                    ui.add_space(14.0);
                    widgets::label(ui, &s, tl!("Downloads"), s.p.faint);
                    let now = ui.input(|i| i.time);
                    let mut allowed = self.allowed(now);
                    if ui.checkbox(&mut allowed, tl!("Allow downloads")).changed() {
                        let _ = self.set_downloads_allowed(allowed);
                    }
                    let note = tl!("From each font's source, only when you install or preview a font.");
                    ui.add(egui::Label::new(egui::RichText::new(note).size(12.0).color(s.p.muted)).wrap());
                }
            });
        });
    }

    fn catalogue_filters(&mut self, ui: &mut Ui, s: &Skin) {
        let Ok(c) = &self.catalogue else { return };
        let index = Arc::clone(&c.index);
        let width = ui.available_width();
        widgets::label(ui, s, tl!("Category"), s.p.faint);
        let mut options = vec![Cell { value: None, label: tl!("All categories"), count: Some(self.category_counts[0]) }];
        for (n, c) in Category::ALL.iter().enumerate() {
            options.push(Cell { value: Some(*c), label: crate::panel::app::category_name(*c), count: self.category_counts.get(n + 1).copied() });
        }
        if widgets::cells_column(ui, s, &mut self.query.category, &options) {
            self.focus_row = None;
        }

        ui.add_space(12.0);
        widgets::label(ui, s, tl!("Language"), s.p.faint);
        let language_label = |i: usize| {
            index.languages().get(i).map_or_else(String::new, |l| match l.autonym.as_deref() {
                Some(autonym) if autonym != l.name => format!("{} · {autonym}", l.name),
                _ => l.name.clone(),
            })
        };
        let selected = self.query.language.map_or_else(|| tl!("Any language").to_owned(), language_label);
        let before = self.query.language;
        let search = &c.search;
        let language_text = &mut self.language_text;
        let query = &mut self.query;
        let row = ui.spacing().interact_size.y;
        let header = |ui: &mut Ui| {
            let field =
                widgets::field(ui, s, language_text, tl!("Name in English or in the language"), tl!("Find a language"), ui.available_width(), true);
            if field.gained_focus() || ui.memory(|m| m.focused().is_none()) {
                field.request_focus();
            }
            ui.add_space(4.0);
            search.languages(language_text)
        };
        let rows = |ui: &mut Ui, range: std::ops::Range<usize>, matches: Vec<usize>| {
            for n in range {
                if n == 0 {
                    ui.selectable_value(&mut query.language, None, tl!("Any language"));
                } else if let Some(&i) = matches.get(n - 1) {
                    ui.selectable_value(&mut query.language, Some(i), language_label(i));
                }
            }
        };
        let inner = widgets::select_box("craft-fonts.language", &selected, width).show_rows_with_header(ui, header, row, |m| m.len() + 1, rows);
        widgets::name(ui, &inner.response, tl!("Language"));
        if self.query.language != before {
            self.focus_row = None;
        }

        ui.add_space(12.0);
        widgets::label(ui, s, tl!("Script"), s.p.faint);
        let any = tl!("Any script");
        let selected = self.query.script.as_deref().and_then(|id| script_name(&index, id)).unwrap_or(any).to_owned();
        let query = &mut self.query;
        widgets::select(ui, "craft-fonts.script", tl!("Script"), &selected, width, |ui| {
            ui.selectable_value(&mut query.script, None, any);
            for &i in c.search.scripts() {
                if let Some(script) = index.scripts().get(i) {
                    ui.selectable_value(&mut query.script, Some(script.id.clone()), &script.name);
                }
            }
        });

        if index.sources().len() > 1 {
            ui.add_space(12.0);
            widgets::label(ui, s, tl!("Source"), s.p.faint);
            let all: usize = self.source_counts.iter().sum();
            let mut options = vec![Cell { value: None, label: tl!("All sources"), count: Some(all) }];
            for (n, source) in index.sources().iter().enumerate() {
                options.push(Cell { value: Some(source.id.clone()), label: &source.name, count: self.source_counts.get(n).copied() });
            }
            let mut chosen = self.source.clone();
            if widgets::cells_column(ui, s, &mut chosen, &options) {
                self.source = chosen;
                self.focus_row = None;
            }
        }

        ui.add_space(12.0);
        widgets::label(ui, s, tl!("Show"), s.p.faint);
        let options = [
            Cell { value: Show::All, label: tl!("All fonts"), count: Some(self.show_counts[0]) },
            Cell { value: Show::Installed, label: tl!("Installed"), count: Some(self.show_counts[1]) },
            Cell { value: Show::NotInstalled, label: tl!("Not installed"), count: Some(self.show_counts[2]) },
        ];
        widgets::cells_column(ui, s, &mut self.show, &options);

        let filtered = self.query.category.is_some()
            || self.query.language.is_some()
            || self.query.script.is_some()
            || self.show != Show::All
            || self.source.is_some();
        if filtered {
            ui.add_space(6.0);
            if widgets::link(ui, s, tl!("Clear filters")).clicked() {
                self.clear_filters();
            }
        }
    }

    fn clear_filters(&mut self) {
        self.query.category = None;
        self.query.language = None;
        self.query.script = None;
        self.source = None;
        self.show = Show::All;
        self.focus_row = None;
    }

    fn installed_filters(&mut self, ui: &mut Ui, s: &Skin) {
        widgets::label(ui, s, tl!("Source"), s.p.faint);
        let store = self.installed.iter().filter(|f| f.source == Source::Store).count();
        let options = [
            Cell { value: Origin::All, label: tl!("All sources"), count: Some(self.installed.len()) },
            Cell { value: Origin::Store, label: tl!("Font store"), count: Some(store) },
            Cell { value: Origin::System, label: tl!("System"), count: Some(self.installed.len() - store) },
        ];
        widgets::cells_column(ui, s, &mut self.origin, &options);
        ui.add_space(8.0);
        let note = tl!("Every Craft app uses the fonts in the font store. System fonts are added and removed with your system's own font tools.");
        ui.add(egui::Label::new(egui::RichText::new(note).size(12.0).color(s.p.muted)).wrap());
    }

    pub(crate) fn main(&mut self, ui: &mut Ui) {
        let s = self.skin.clone();
        self.toolbar(ui, &s);
        ui.add_space(12.0);
        if let Some(notice) = self.notice.clone() {
            widgets::note(ui, &s, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(notice).color(s.p.bad)).wrap());
            });
            ui.add_space(10.0);
        }
        match self.tab {
            Tab::Catalogue => {
                self.download_note(ui, &s);
                self.catalogue_list(ui, &s);
            }
            Tab::Installed => self.installed_list(ui, &s),
        }
    }

    fn toolbar(&mut self, ui: &mut Ui, s: &Skin) {
        ui.allocate_ui_with_layout(vec2(ui.available_width(), s.h()), direction::row(ui), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let preset = self.settings.preset;
            let preset_name = |p: Preset| match p {
                Preset::Sentence => tl!("Sentence"),
                Preset::Alphabet => tl!("Alphabet"),
                Preset::Numerals => tl!("Numerals"),
            };
            let settings = &mut self.settings;
            widgets::select(ui, "craft-fonts.preset", tl!("Sample"), preset_name(preset), 128.0, |ui| {
                for p in [Preset::Sentence, Preset::Alphabet, Preset::Numerals] {
                    if ui.selectable_label(preset == p && settings.sample.is_empty(), preset_name(p)).clicked() {
                        settings.preset = p;
                        settings.sample.clear();
                    }
                }
            });
            // The size control at the reading end, the sample text filling the room between.
            let size_w = 176.0;
            let hint = self.sample_for(["Latn"]);
            let field_w = (ui.available_width() - size_w - 10.0).max(120.0);
            widgets::field(ui, s, &mut self.settings.sample, &hint, tl!("Sample text"), field_w, false);
            ui.allocate_ui_with_layout(vec2(size_w, s.h()), direction::row(ui), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                widgets::slider(ui, s, &mut self.settings.size, 12.0..=120.0, 112.0, tl!("Size"));
                let value = fmt("{size} px", &[("size", &i18n::number(self.settings.size.round() as u64))]);
                widgets::label(ui, s, &value, s.p.muted);
            });
        });
    }

    fn download_note(&mut self, ui: &mut Ui, s: &Skin) {
        let now = ui.input(|i| i.time);
        if self.allowed(now) || self.settings.hide_download_note {
            return;
        }
        let mut allow = false;
        let mut hide = false;
        let text = tl!("Fonts that aren't installed show a sample once you preview them. Nothing is downloaded until you allow it.");
        widgets::note_with_actions(
            ui,
            s,
            |ui| {
                ui.label(egui::RichText::new(tl!("Downloads are off")).color(s.p.ink_strong));
                ui.add(egui::Label::new(egui::RichText::new(text).size(12.5).color(s.p.muted)).wrap());
            },
            &[tl!("Allow downloads")],
            |ui, n| match n {
                0 => allow = widgets::button(ui, s, tl!("Allow downloads"), widgets::Kind::Secondary, 28.0, 0.0).clicked(),
                _ => hide = widgets::glyph_button(ui, s, Glyph::Close, tl!("Hide this note"), 28.0).clicked(),
            },
            true,
        );
        ui.add_space(10.0);
        if allow {
            let _ = self.set_downloads_allowed(true);
        }
        if hide {
            self.settings.hide_download_note = true;
        }
    }

    fn row_height(&self) -> f32 {
        CARD_TOP + self.skin.h() + CARD_GAP + self.settings.size * 1.32 + CARD_BOTTOM
    }

    /// The scroll area for `count` cards; `card` draws row `n` into the rect it is given.
    fn cards(&mut self, ui: &mut Ui, count: usize, mut card: impl FnMut(&mut FontsPanel, &mut Ui, usize, Rect) -> egui::Response) {
        let row = self.row_height();
        let mut area = egui::ScrollArea::vertical().id_salt(("craft-fonts.cards", self.tab == Tab::Catalogue)).auto_shrink([false, false]);
        let viewport_h = ui.available_height();
        if std::mem::take(&mut self.scroll_to_focus)
            && let Some(focus) = self.focus_row
        {
            let offset = ui.ctx().data(|d| d.get_temp::<f32>(Id::unique(("craft-fonts.offset", self.tab == Tab::Catalogue)))).unwrap_or(0.0);
            let top = focus as f32 * row;
            if top < offset {
                area = area.vertical_scroll_offset(top);
            } else if top + row > offset + viewport_h {
                area = area.vertical_scroll_offset(top + row - viewport_h);
            }
        }
        let output = area.show_rows(ui, row, count, |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let width = ui.available_width();
            let mut stepped = false;
            for n in range {
                let (rect, _) = ui.allocate_exact_size(vec2(width, row), Sense::hover());
                // Neighbouring cards share their hairline.
                let rect = if n == 0 { rect } else { rect.with_min_y(rect.top() - 1.0) };
                let resp = card(self, ui, n, rect);
                if self.focus_row == Some(n) && self.move_focus {
                    self.move_focus = false;
                    resp.request_focus();
                }
                if resp.has_focus() && !stepped {
                    // The arrow keys step through the cards here, not egui's spatial focus moves.
                    let filter = egui::EventFilter { vertical_arrows: true, ..Default::default() };
                    ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, filter));
                    self.focus_row = Some(n);
                    let step = ui.input(|i| {
                        if i.key_pressed(egui::Key::ArrowDown) {
                            1
                        } else if i.key_pressed(egui::Key::ArrowUp) {
                            -1
                        } else {
                            0
                        }
                    });
                    let next = n as i64 + step;
                    if step != 0 && (0..count as i64).contains(&next) {
                        self.focus_row = Some(next as usize);
                        self.scroll_to_focus = true;
                        self.move_focus = true;
                        stepped = true;
                        ui.ctx().request_repaint();
                    }
                }
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(Id::unique(("craft-fonts.offset", self.tab == Tab::Catalogue)), output.state.clamped_offset().y));
    }

    fn empty(&mut self, ui: &mut Ui, s: &Skin, title: &str, body: &str, clear: bool) {
        ui.add_space(48.0);
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            widgets::label(ui, s, title, s.p.muted);
            ui.add(egui::Label::new(egui::RichText::new(body).color(s.p.faint)).wrap());
            if clear {
                ui.add_space(4.0);
                if widgets::button(ui, s, tl!("Clear search and filters"), widgets::Kind::Secondary, s.h(), 0.0).clicked() {
                    self.query.text.clear();
                    self.clear_filters();
                }
            }
        });
    }

    fn catalogue_list(&mut self, ui: &mut Ui, s: &Skin) {
        let index = match &self.catalogue {
            Ok(c) => Arc::clone(&c.index),
            Err(e) => {
                let e = e.clone();
                widgets::note(ui, s, |ui| {
                    let text = fmt("The font indexes can't be read: {error}", &[("error", &e)]);
                    ui.add(egui::Label::new(egui::RichText::new(text).color(s.p.bad)).wrap());
                });
                return;
            }
        };
        self.source_problems(ui, s, &index);
        let count = tn(self.results.len() as u64, "{n} family", "{n} families");
        widgets::label(ui, s, &count, s.p.faint);
        ui.add_space(8.0);
        if self.results.is_empty() {
            self.empty(ui, s, tl!("No fonts match"), tl!("Try another name, or fewer filters."), true);
            return;
        }
        let results = self.results.clone();
        self.cards(ui, results.len(), |me, ui, n, rect| {
            let i = results.get(n).copied().unwrap_or_default();
            me.catalogue_card(ui, &index, i, rect)
        });
    }

    /// Sources that couldn't be loaded, in a note above the list; the others show as usual.
    fn source_problems(&mut self, ui: &mut Ui, s: &Skin, index: &Index) {
        let problems = index.problems();
        if problems.is_empty() {
            return;
        }
        let now = ui.input(|i| i.time);
        let allowed = self.allowed(now);
        let fetching = self.fetching_sources();
        let buttons: &[&str] = if allowed && !fetching { &[tl!("Try again")] } else { &[] };
        let mut retry = false;
        widgets::note_with_actions(
            ui,
            s,
            |ui| {
                let title = tn(problems.len() as u64, "{n} font source couldn't be loaded", "{n} font sources couldn't be loaded");
                ui.label(egui::RichText::new(title).color(s.p.ink_strong));
                for p in problems.iter().take(4) {
                    let line = fmt("{source}: {error}", &[("source", &p.name), ("error", &p.error)]);
                    ui.add(egui::Label::new(egui::RichText::new(line).size(12.5).color(s.p.muted)).wrap());
                }
            },
            buttons,
            |ui, _| retry = widgets::button(ui, s, tl!("Try again"), widgets::Kind::Secondary, 28.0, 0.0).clicked(),
            false,
        );
        ui.add_space(10.0);
        if retry {
            let ctx = ui.ctx().clone();
            self.fetch_sources(&ctx);
        }
    }

    fn installed_list(&mut self, ui: &mut Ui, s: &Skin) {
        if !self.catalog.is_ready() {
            direction::horizontal(ui, |ui| {
                ui.spinner();
                ui.label(egui::RichText::new(tl!("Looking for installed fonts…")).color(s.p.muted));
            });
            ui.ctx().request_repaint();
            return;
        }
        let shown = self.installed_shown();
        let count = tn(shown.len() as u64, "{n} family", "{n} families");
        widgets::label(ui, s, &count, s.p.faint);
        ui.add_space(8.0);
        if shown.is_empty() {
            let body = if self.origin == Origin::Store && self.installed_text.trim().is_empty() {
                tl!("Fonts you install from Browse go here, for every Craft app.")
            } else {
                tl!("Try another name.")
            };
            self.empty(ui, s, tl!("No installed fonts match"), body, false);
            return;
        }
        self.cards(ui, shown.len(), |me, ui, n, rect| {
            let i = shown.get(n).copied().unwrap_or_default();
            me.installed_card(ui, i, rect)
        });
    }

    fn catalogue_state(&self, source: &str, family: &str, faces_installed: bool) -> CardState {
        let key = family_key(source, family);
        if let Some(job) = self.jobs.of(source, family) {
            return CardState::Running(job.kind, job.fraction());
        }
        if let Some(f) = self.failed.get(&key) {
            return CardState::Failed(f.kind, f.error.clone(), f.retry.clone());
        }
        if faces_installed {
            return CardState::Installed;
        }
        CardState::Offer { preview: !self.previewed.contains_key(&key) }
    }

    fn catalogue_card(&mut self, ui: &mut Ui, index: &Index, i: usize, rect: Rect) -> egui::Response {
        let s = self.skin.clone();
        let Some(family) = index.families().get(i) else { return ui.interact(rect, Id::unique(("craft-fonts.card", i)), Sense::hover()) };
        let key = family_key(&family.source, &family.family);
        let installed = self.installed_by_name.get(&family.family.to_lowercase()).and_then(|f| regular(f)).cloned();
        let previewed = self.previewed.get(&key).and_then(|f| regular(f)).cloned();
        let state = self.catalogue_state(&family.source, &family.family, installed.is_some());
        let (source, curated) = source_label(index, family);
        let meta = [tn(style_count(family) as u64, "{n} style", "{n} styles"), category_label(family), source.to_owned()].join(" · ");
        let sample = self.sample_for(family.scripts.iter().map(String::as_str));
        let face = match (installed, previewed) {
            (Some(face), _) => Some((Owner::Installed, face)),
            (None, Some(face)) => Some((Owner::Preview, face)),
            (None, None) => None,
        };
        let card = Card { key: &key, name: &family.family, meta: &meta, sample: &sample, curated, index_position: Some(i) };
        let resp = self.card(ui, &s, rect, card, face, state);
        if resp.clicked() {
            self.detail = Some(Detail::Family(i));
        }
        resp
    }

    fn installed_card(&mut self, ui: &mut Ui, i: usize, rect: Rect) -> egui::Response {
        let s = self.skin.clone();
        let Some(family) = self.installed.get(i).cloned() else { return ui.interact(rect, Id::unique(("craft-fonts.installed", i)), Sense::hover()) };
        let meta = [tn(family.faces.len() as u64, "{n} style", "{n} styles"), source_name(family.source).to_owned()].join(" · ");
        let state = match self.jobs.of("", &family.name) {
            Some(job) => CardState::Running(job.kind, job.fraction()),
            None => match self.failed.get(&family_key("", &family.name)) {
                Some(f) => CardState::Failed(f.kind, f.error.clone(), None),
                None if family.removable() => CardState::Removable(family.faces.clone()),
                None => CardState::System,
            },
        };
        let face = regular(&family.faces).cloned().map(|f| (Owner::Installed, f));
        let scripts = self.index().and_then(|index| index.family(&family.name).map(|f| f.scripts.clone())).unwrap_or_default();
        let sample = self.sample_for(scripts.iter().map(String::as_str));
        let key = format!("{:?}\n{}", family.source, family.name);
        let card = Card { key: &key, name: &family.name, meta: &meta, sample: &sample, curated: true, index_position: None };
        let resp = self.card(ui, &s, rect, card, face, state);
        if resp.clicked() {
            let in_index = self.index().and_then(|index| index.families().iter().position(|f| f.family.eq_ignore_ascii_case(&family.name)));
            self.detail = Some(match in_index {
                Some(i) if family.source == Source::Store => Detail::Family(i),
                _ => Detail::Installed(family.name.clone(), family.source),
            });
        }
        resp
    }

    /// One family card: its name, styles and source on one line with its state at the end, and
    /// a sample in the face below.
    fn card(&mut self, ui: &mut Ui, s: &Skin, rect: Rect, card: Card<'_>, face: Option<(Owner, FaceInfo)>, state: CardState) -> egui::Response {
        let Card { key, name, meta, sample, curated, index_position } = card;
        let p = s.p;
        let rtl = direction::is_rtl(ui);
        let id = Id::unique(("craft-fonts.card", key));
        // The whole card opens the details; added first, so the buttons on it keep their clicks.
        let resp = ui.interact(rect, id, Sense::click());
        resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, name));
        let hovered = ui.rect_contains_pointer(rect);
        let lift = ui.ctx().animate_bool_with_time(id.with("hover"), hovered || resp.has_focus(), ui.style().animation_time * 2.0);
        let painter = ui.painter_at(rect.expand(1.0));
        painter.rect(rect, CornerRadius::ZERO, p.bg.lerp_to_gamma(p.sunken, 0.55 * lift), s.hairline(), StrokeKind::Inside);
        if lift > 0.0 {
            let inside = rect.shrink(1.0);
            let w = inside.width() * lift;
            let x = if rtl { inside.right() - w } else { inside.left() };
            painter.rect_filled(Rect::from_min_size(pos2(x, inside.top()), vec2(w, 2.0)), CornerRadius::ZERO, p.accent);
        }
        widgets::focus_ring(ui, s, rect.shrink(1.0), &resp);

        let inner = rect.shrink2(vec2(CARD_PAD, 0.0));
        let header = Rect::from_min_size(pos2(inner.left(), rect.top() + CARD_TOP), vec2(inner.width(), s.h()));
        let mut end = ui.new_child(UiBuilder::new().max_rect(header).layout(direction::trailing(ui)));
        end.spacing_mut().item_spacing.x = 10.0;
        self.card_state(&mut end, s, name, index_position, state);
        if !curated {
            widgets::badge(&mut end, s, tl!("Not curated"), s.p.warn, false)
                .on_hover_text(tl!("This family comes from a source you added. Craft doesn't review it."));
        }
        let end_w = end.min_rect().width();

        let name_galley = ui.painter().layout_no_wrap(name.to_owned(), FontId::new(16.0, s.look.strong.clone()), p.ink_strong);
        let name_w = name_galley.size().x.min((header.width() - end_w - 24.0).max(0.0));
        let start = direction::start_x(ui, header);
        let f = direction::forward(ui);
        let name_x = if rtl { start - name_w } else { start };
        ui.painter().with_clip_rect(header).galley(pos2(name_x, header.center().y - name_galley.size().y / 2.0), name_galley, p.ink_strong);
        let room = header.width() - name_w - end_w - 40.0;
        if room > 24.0 {
            let meta_galley = s.caps_galley(ui, meta, if s.look.caps { 10.5 } else { 12.0 }, s.look.label.clone(), p.faint, Some(room));
            let x = start + f * (name_w + 14.0);
            let x = if rtl { x - meta_galley.size().x } else { x };
            ui.painter().galley(pos2(x, header.center().y - meta_galley.size().y / 2.0 + 1.0), meta_galley, p.faint);
        }

        let sample_rect = Rect::from_min_max(pos2(inner.left(), header.bottom() + CARD_GAP), pos2(inner.right(), rect.bottom() - CARD_BOTTOM + 4.0));
        self.paint_sample(ui, s, sample_rect, face.as_ref(), sample);
        resp
    }

    fn card_state(&mut self, ui: &mut Ui, s: &Skin, name: &str, index_position: Option<usize>, state: CardState) {
        let p = s.p;
        let h = s.h() - 4.0;
        let ctx = ui.ctx().clone();
        match state {
            CardState::Running(kind, fraction) => {
                let what = match kind {
                    Kind::Install => tl!("Installing"),
                    Kind::Remove => tl!("Removing"),
                    Kind::Preview => tl!("Downloading"),
                };
                let text = match fraction {
                    Some(f) => format!("{what} {}", fmt("{percent} %", &[("percent", &i18n::number((f * 100.0).round() as u64))])),
                    None => format!("{what}…"),
                };
                let (rect, resp) = ui.allocate_exact_size(vec2(150.0, h), Sense::hover());
                let galley = s.label_galley(ui, &text, p.muted);
                widgets::paint_end(ui, rect.with_max_y(rect.bottom() - 8.0), galley, p.muted);
                let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 3.0), rect.max);
                widgets::progress_bar(ui, s, bar, fraction);
                resp.widget_info(|| WidgetInfo::labeled(Role::ProgressIndicator, true, &text));
            }
            CardState::Failed(kind, error, retry) => {
                if let Some(retry) = retry
                    && widgets::button(ui, s, tl!("Retry"), widgets::Kind::Secondary, h, 0.0).clicked()
                {
                    self.ask(&ctx, retry);
                }
                let what = match kind {
                    Kind::Install => tl!("Couldn't install"),
                    Kind::Remove => tl!("Couldn't remove"),
                    Kind::Preview => tl!("Couldn't download"),
                };
                widgets::label(ui, s, what, p.bad).on_hover_text(error);
            }
            CardState::Installed => {
                widgets::badge(ui, s, tl!("Installed"), p.good, true);
            }
            CardState::System => {}
            CardState::Removable(faces) => {
                let remove = widgets::button(ui, s, tl!("Remove"), widgets::Kind::Danger, h, 0.0);
                widgets::name(ui, &remove, &fmt("Remove {family}", &[("family", name)]));
                if remove.clicked() {
                    self.remove(&ctx, name, &faces, true);
                }
            }
            CardState::Offer { preview } => {
                let Some(i) = index_position else { return };
                let install = widgets::button(ui, s, tl!("Install"), widgets::Kind::Secondary, h, 0.0);
                widgets::name(ui, &install, &fmt("Install {family}", &[("family", name)]));
                if install.clicked() {
                    self.ask(&ctx, Pending::Install(i));
                }
                if preview {
                    let button = widgets::link(ui, s, tl!("Preview"));
                    widgets::name(ui, &button, &fmt("Preview {family}", &[("family", name)]));
                    if button.clicked() {
                        self.ask(&ctx, Pending::Preview(i));
                    }
                }
            }
        }
    }

    /// `text` in the face, on one line; the interface font in a faint ink where the face isn't
    /// here (not installed and not previewed).
    pub(crate) fn paint_sample(&mut self, ui: &mut Ui, s: &Skin, rect: Rect, face: Option<&(Owner, FaceInfo)>, text: &str) {
        let size = self.settings.size;
        let (family, color) = match face {
            Some((owner, face)) => {
                let catalog = match owner {
                    Owner::Installed => Arc::clone(&self.catalog),
                    Owner::Preview => Arc::clone(&self.previews),
                };
                match self.faces.family(ui.ctx(), &catalog, *owner, face) {
                    Shown::Ready(family) => (family, s.p.ink),
                    Shown::Loading => return,
                    Shown::Unavailable => (FontFamily::Proportional, s.p.faint),
                }
            }
            None => (FontFamily::Proportional, s.p.faint.gamma_multiply(0.8)),
        };
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), FontId::new(size, family), color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width());
        let galley = ui.painter().layout_job(job);
        // Sample text keeps its own direction whatever the interface's.
        let x = if craft_i18n::base_direction(text) == Some(craft_i18n::Direction::Rtl) { rect.right() - galley.size().x } else { rect.left() };
        let y = rect.center().y - galley.size().y / 2.0;
        ui.painter().with_clip_rect(rect.expand2(vec2(0.0, 6.0))).galley(pos2(x, y), galley, color);
    }
}
