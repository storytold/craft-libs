//! Drawing the dialog: header, table, detail pane, conflict bar, messages and footer.
//!
//! Widgets are created in focus order (§2.10 of the dialog plan): set row, Show, Filter, Search,
//! ⌨, table, detail pane, conflict bar, footer. egui's Tab order is creation order, so the bars
//! under the table are drawn last even though their height is needed first: the view uses last
//! frame's height and asks egui to discard a frame whose height changed.

use std::ops::Range;
use std::sync::Arc;

use craft_ui::buttons::{IconButton, IconButtonStyle};
use craft_ui::layout::{ResizeHandle, SplitAxis, SplitSize, split_rect};
use craft_ui::tabs::{self, Tab};
use egui::Galley;
use egui::accesskit::Live;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Button, Color32, ComboBox, EventFilter, Id, Key as EguiKey, Label, Layout, Modifiers, Painter, Rect, Response, RichText, ScrollArea,
    Sense, Stroke, StrokeKind, TextEdit, TextStyle, Ui, UiBuilder, Vec2, WidgetInfo, WidgetText, WidgetType, pos2, vec2,
};

use crate::chord::Trigger;
use crate::command::CommandRef;
use crate::keymap::{Binding, MAX_BINDINGS, Problem};
use crate::scope::Scope;
use crate::ui::capture;
use crate::ui::rows::{AreaId, Filter, FixedLine, Line, Row};
use crate::ui::state::{Answer, DialogState, Env, Prompt, Recording, Target, binding_error};
use crate::ui::style::{self, DialogStyle};
use crate::ui::text::{trigger_text, visual_around};

#[path = "keyboard.rs"]
pub(crate) mod keyboard;
#[path = "review.rs"]
mod review;
#[path = "sets.rs"]
mod sets;

/// Below this width the detail pane goes under the table.
const NARROW: f32 = 720.0;
/// Smallest table or detail pane when dragging the divider.
const SPLIT_MIN: f32 = 120.0;
/// Icon buttons in rows and bars.
const ICON: f32 = 20.0;
/// Padding inside a chip.
const CHIP_PAD: f32 = 5.0;
const CHIP_GAP: f32 = 4.0;
/// The detail pane's New Shortcut field.
const FIELD_W: f32 = 160.0;
/// The widest Show, Filter or Set combo box; longer names are cut with "…".
const COMBO_MAX: f32 = 200.0;
/// Search field width.
const SEARCH_MIN: f32 = 120.0;
const SEARCH_MAX: f32 = 240.0;

/// One frame of the dialog body inside `ui`'s available rectangle.
pub(crate) fn body(ui: &mut Ui, env: &mut Env, state: &mut DialogState) {
    let style = env.options.style.unwrap_or_else(|| DialogStyle::from_visuals(ui.visuals()));
    let icons = style::icon_buttons(ui.visuals());
    let row_h = ui.text_style_height(&TextStyle::Body) + 6.0;
    let base = ui.id().with("craft_keymap_shortcuts");
    let mut d = Draw { env, state, style, icons, base, row_h };
    d.keys(ui);
    d.layout(ui);
}

struct Draw<'s, 'e> {
    env: &'s mut Env<'e>,
    state: &'s mut DialogState,
    style: DialogStyle,
    icons: IconButtonStyle,
    base: Id,
    row_h: f32,
}

fn fill(source: String, args: &[(&str, &str)]) -> String {
    craft_i18n::fmt(&source, args)
}

/// What a click in the table asked for, applied after the rows are drawn.
enum RowAction {
    Select(CommandRef),
    Record(CommandRef, Option<Binding>, usize),
    Reset(CommandRef),
    Toggle(String),
}

impl Draw<'_, '_> {
    // ---- Text ----

    /// The dialog's string, translated (logical order: for accessibility labels and `fill`).
    fn ui(&self, source: &str) -> String {
        self.env.text.ui(source).into_owned()
    }

    /// Display order for drawn text.
    fn vis(&self, s: &str) -> String {
        self.env.text.visual(s)
    }

    fn rtl(&self) -> bool {
        self.env.text.rtl()
    }

    /// Parts in logical order, each already in display order, put into display order: reversed in
    /// a right-to-left interface, where the path separator ` › ` becomes ` ‹ `.
    fn ordered<T>(&self, mut parts: Vec<(String, T)>) -> Vec<(String, T)> {
        if self.rtl() {
            parts.reverse();
            for (s, _) in &mut parts {
                if s == PATH_SEP {
                    *s = " ‹ ".into();
                }
            }
        }
        parts
    }

    /// [`ordered`](Self::ordered) parts as one string.
    fn joined(&self, parts: Vec<String>) -> String {
        self.ordered(parts.into_iter().map(|s| (s, ())).collect()).into_iter().map(|(s, ())| s).collect()
    }

    /// Display-order `parts` on one line within `width`, cut with "…" when they do not fit: at
    /// the end, or at the start in a right-to-left interface, where the logical end of the text is
    /// on the left. Returns the galley and whether it was cut.
    fn one_line(&self, painter: &Painter, parts: &[(String, TextFormat)], width: f32) -> (Arc<Galley>, bool) {
        let width = width.max(0.0);
        let job = |skip: usize| {
            let mut job = LayoutJob::default();
            if let (true, Some((_, f))) = (skip > 0, parts.first()) {
                job.append("…", 0.0, f.clone());
            }
            let mut left = skip;
            for (s, f) in parts {
                let n = s.chars().count();
                if left >= n {
                    left -= n;
                    continue;
                }
                let start = s.char_indices().nth(left).map_or(s.len(), |(i, _)| i);
                left = 0;
                job.append(&s[start..], 0.0, f.clone());
            }
            job
        };
        let full = painter.layout_job(job(0));
        if full.size().x <= width + 0.5 {
            return (full, false);
        }
        if !self.rtl() {
            let mut cut = job(0);
            cut.wrap = TextWrapping::truncate_at_width(width);
            return (painter.layout_job(cut), true);
        }
        // The fewest characters to drop from the start so that "…" and the rest fit.
        let total: usize = parts.iter().map(|(s, _)| s.chars().count()).sum();
        let (mut lo, mut hi) = (1, total);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if painter.layout_job(job(mid)).size().x <= width + 0.5 { hi = mid } else { lo = mid + 1 }
        }
        (painter.layout_job(job(lo)), true)
    }

    /// `s` in display order with `chord`, text inside it, left as it is: chord text never goes
    /// through [`Text::visual`](crate::ui::Text::visual).
    fn vis_chord(&self, s: &str, chord: &str) -> String {
        visual_around(self.env.text, s, chord)
    }

    /// `text` after a ⚠, in display order.
    fn warned(&self, text: String) -> String {
        self.joined(vec!["⚠".into(), " ".into(), text])
    }

    /// Horizontal placement inside `rect`, mirrored in a right-to-left interface.
    fn mirror(&self, rect: Rect) -> Mirror {
        Mirror { on: self.rtl(), sum: rect.min.x + rect.max.x }
    }

    fn chord(&self, t: &Trigger) -> String {
        trigger_text(self.env.text, t, self.env.keymap.platform())
    }

    fn scope_name(&self, s: &Scope) -> String {
        if s.is_app() { self.ui("Application") } else { self.env.text.segment(s.as_str()).into_owned() }
    }

    /// A binding as text, with its scope when that is not the command's home scope.
    fn binding_text(&self, b: &Binding, home: Option<&Scope>) -> String {
        let chord = self.chord(&b.trigger);
        match &b.scope {
            Some(s) if Some(s) != home => fill(self.ui("{chord} ({scope})"), &[("chord", &chord), ("scope", &self.scope_name(s))]),
            _ => chord,
        }
    }

    /// A menu path in display order: "File › Export", "Export ‹ File" right to left.
    fn path_text(&self, path: &[String]) -> String {
        let mut parts = Vec::new();
        for (i, s) in path.iter().enumerate() {
            if i > 0 {
                parts.push(PATH_SEP.to_string());
            }
            parts.push(self.vis(&self.env.text.segment(s)));
        }
        self.joined(parts)
    }

    fn label_of(&self, command: &CommandRef) -> String {
        self.env.keymap.bindable(command).map_or_else(|| command.id.clone(), |b| self.env.text.command(b).into_owned())
    }

    /// "press keys", or the chord waiting for a second press.
    fn recording_text(&self) -> String {
        match self.state.recording.as_ref().and_then(|r| r.waiting.as_ref()) {
            Some((t, _)) => {
                let chord = self.chord(t);
                self.vis_chord(&fill(self.ui("{chord} (press again for a double press)"), &[("chord", &chord)]), &chord)
            }
            None => self.ui("press keys"),
        }
    }

    /// Reports a control to [`Options::on_widget`](crate::ui::Options::on_widget).
    fn tag(&self, id: impl FnOnce() -> String, rect: Rect, label: &str) {
        if let Some(f) = &self.env.options.on_widget {
            f(&id(), rect, label);
        }
    }

    fn button(&self, ui: &mut Ui, enabled: bool, text: &str, id: &str) -> bool {
        let r = ui.add_enabled(enabled, Button::new(self.vis(text)));
        self.tag(|| id.to_string(), r.rect, text);
        r.clicked()
    }

    /// The width a [`Button`] with `text` takes.
    fn button_width(&self, ui: &Ui, text: &str) -> f32 {
        let pad = ui.spacing().button_padding.x * 2.0;
        let g = WidgetText::from(self.vis(text)).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, TextStyle::Button);
        (g.size().x + pad).max(ui.spacing().interact_size.x)
    }

    fn icon_button(&self, ui: &mut Ui, label: &str, selected: bool, paint: fn(&Painter, Rect, Color32)) -> Response {
        IconButton::new(label, Vec2::splat(ICON), &self.icons).selected(selected).show(ui, |ui, rect, color| paint(ui.painter(), rect, color))
    }

    /// A row control: clickable and labeled but not a Tab stop (the table is one).
    fn row_button(&self, ui: &mut Ui, rect: Rect, id: Id, label: &str, enabled: bool, paint: fn(&Painter, Rect, Color32)) -> Response {
        let resp = ui.interact(rect, id, if enabled { Sense::CLICK } else { Sense::hover() });
        resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
        let v = if !enabled {
            self.icons.normal
        } else if resp.is_pointer_button_down_on() {
            self.icons.pressed.unwrap_or(self.icons.hovered)
        } else if resp.hovered() {
            self.icons.hovered
        } else {
            self.icons.normal
        };
        if v.fill != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, self.icons.corner_radius, v.fill);
        }
        let color = if enabled { v.icon } else { self.style.dim.gamma_multiply(0.6) };
        paint(ui.painter(), rect, color);
        resp
    }

    /// Marks a text field of the dialog, so Escape, Enter, Undo and Delete leave it alone.
    fn text_field(&mut self, r: &Response) {
        if r.has_focus() {
            self.state.text_focus = true;
        }
    }

    fn recording_target(&self) -> Option<&Target> {
        self.state.recording.as_ref().map(|r: &Recording| &r.target)
    }

    // ---- Keys when nothing records ----

    /// Escape, Undo/Redo and Enter (§2.5). Keys while recording were taken by `DialogState::input`.
    fn keys(&mut self, ui: &Ui) {
        if self.state.is_recording() {
            return;
        }
        let text_focus = self.state.text_focus;
        let popup = egui::Popup::is_any_open(ui.ctx());
        if !text_focus && !popup && ui.input_mut(|i| i.consume_key(Modifiers::NONE, EguiKey::Escape)) {
            if self.state.typing.take().is_some() {
                self.state.type_error = None;
            } else {
                self.state.escape(self.env);
            }
            return;
        }
        if text_focus || !self.env.options.editable || self.state.modal() {
            return;
        }
        let mac = self.env.keymap.platform().is_mac();
        let (undo, redo) = ui.input_mut(|i| {
            let redo = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, EguiKey::Z) || (!mac && i.consume_key(Modifiers::COMMAND, EguiKey::Y));
            let undo = !redo && i.consume_key(Modifiers::COMMAND, EguiKey::Z);
            (undo, redo)
        });
        if redo {
            self.state.redo(self.env);
        } else if undo {
            self.state.undo(self.env);
        }
        if self.env.options.enter_is_ok
            && self.state.prompt.is_none()
            && ui.memory(|m| m.focused().is_none())
            && ui.input_mut(|i| i.consume_key(Modifiers::NONE, EguiKey::Enter))
        {
            self.state.enter(self.env);
        }
    }

    // ---- Layout ----

    fn layout(&mut self, ui: &mut Ui) {
        let avail = ui.available_rect_before_wrap();
        let size = |v: f32, or: f32| if v.is_finite() { v.clamp(0.0, 4096.0) } else { or };
        let full = Rect::from_min_size(avail.min, vec2(size(avail.width(), 800.0), size(avail.height(), 600.0)));
        self.state.text_focus = false;
        let gap = ui.spacing().item_spacing.y;
        let column = self.column();
        ui.scope_builder(UiBuilder::new().max_rect(full).layout(column), |ui| {
            self.sets_row(ui);
            // Show, Filter and Search act on the table, which the import review replaces.
            ui.add_enabled_ui(self.state.review.is_none(), |ui| self.header(ui));
            let rest = ui.available_rect_before_wrap();
            let bottom_h = self.state.bottom_h.clamp(0.0, rest.height());
            let mid_bottom = (rest.max.y - bottom_h - gap).max(rest.min.y);
            let mid = Rect::from_min_max(rest.min, pos2(rest.max.x, mid_bottom));
            if !self.review(ui, mid) {
                let prompt = self.state.prompt.is_some();
                ui.scope(|ui| {
                    if prompt {
                        ui.disable();
                    }
                    self.middle(ui, mid);
                });
            }
            let top = (mid.max.y + gap).min(rest.max.y);
            let bottom = Rect::from_min_max(pos2(rest.min.x, top), rest.max);
            let used = ui.scope_builder(UiBuilder::new().max_rect(bottom).layout(column), |ui| self.bottom(ui)).response.rect.height();
            if (used - self.state.bottom_h).abs() > 0.5 {
                self.state.bottom_h = used;
                ui.ctx().request_discard("shortcuts dialog: height of the bars under the table");
            }
        });
        ui.advance_cursor_after_rect(full);
    }

    /// Top to bottom, aligned to the reading side. In a right-to-left interface egui's
    /// `horizontal` rows inside such a column also run right to left.
    fn column(&self) -> Layout {
        Layout::top_down(if self.env.text.rtl() { Align::Max } else { Align::Min })
    }

    fn child(&mut self, ui: &mut Ui, rect: Rect, f: impl FnOnce(&mut Self, &mut Ui)) {
        let column = self.column();
        ui.scope_builder(UiBuilder::new().max_rect(rect).layout(column), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            f(self, ui);
        });
    }

    // ---- Header: Show, Filter, Search ----

    fn area_key(id: &AreaId) -> String {
        match id {
            AreaId::All => "all".into(),
            AreaId::Menu(m) => format!("menu.{m}"),
            AreaId::Tools => "tools".into(),
            AreaId::Panel(p) => format!("panel.{p}"),
            AreaId::Other => "other".into(),
            AreaId::App(a) => (*a).to_string(),
            AreaId::Fixed => "fixed".into(),
        }
    }

    fn header(&mut self, ui: &mut Ui) {
        if self.env.options.area_tabs {
            self.area_tabs(ui);
        }
        // Show and Filter, then Search, ⌨ and Keyboard; Search goes to a row of its own when the
        // two do not fit side by side.
        let gap = ui.spacing().item_spacing.x;
        let text_w = |d: &Self, ui: &Ui, s: &str| d.button_width(ui, s) - ui.spacing().button_padding.x * 2.0;
        let mut first = text_w(self, ui, &self.ui("Filter:")) + COMBO_MAX + 2.0 * gap;
        if !self.env.options.area_tabs {
            first += text_w(self, ui, &self.ui("Show:")) + COMBO_MAX + 2.0 * gap;
        }
        let toggle = if self.env.options.keyboard { self.button_width(ui, &self.ui("Keyboard")) + gap } else { 0.0 };
        let second = text_w(self, ui, &self.ui("Search:")) + SEARCH_MIN + ICON + toggle + 3.0 * gap;
        let one_row = first + second <= ui.available_width();
        ui.horizontal(|ui| {
            if !self.env.options.area_tabs {
                self.area_combo(ui);
            }
            self.filter_combo(ui);
            if one_row {
                self.search(ui, toggle);
            }
        });
        if !one_row {
            ui.horizontal(|ui| self.search(ui, toggle));
        }
    }

    /// A combo box at most [`COMBO_MAX`] wide, its text cut with "…" and shown in full on hover.
    fn combo(&self, ui: &mut Ui, salt: &str, label: &Response, title: &str, current: &str, items: impl FnOnce(&mut Ui)) -> Response {
        let full = self.vis(current);
        let natural = self.button_width(ui, &full) + ui.spacing().icon_width + ui.spacing().icon_spacing;
        let width = natural.clamp(ui.spacing().combo_width, COMBO_MAX);
        let r = ui
            .allocate_ui(vec2(width, ui.spacing().interact_size.y), |ui| {
                ComboBox::from_id_salt(self.base.with(salt)).width(width).truncate().selected_text(full.clone()).show_ui(ui, items).response
            })
            .inner
            .labelled_by(label.id);
        self.tag(|| format!("shortcuts.{salt}"), r.rect, title);
        if natural > width { r.on_hover_text(full) } else { r }
    }

    fn filter_combo(&mut self, ui: &mut Ui) {
        let filter_label = self.ui("Filter:");
        let l = ui.label(self.vis(&filter_label));
        let current = self.state.filter.label(self.env.text);
        let mut pick = None;
        self.combo(ui, "filter", &l, &filter_label, &current, |ui| {
            for f in Filter::ALL {
                let label = f.label(self.env.text);
                if ui.selectable_label(self.state.filter == f, self.vis(&label)).clicked() {
                    pick = Some(f);
                }
            }
        });
        if let Some(f) = pick {
            self.state.set_filter(f);
        }
    }

    /// Search, search by key and the keyboard pane's toggle (`toggle` wide, 0 without it).
    fn search(&mut self, ui: &mut Ui, toggle: f32) {
        let search_label = self.ui("Search:");
        let l = ui.label(self.vis(&search_label));
        let mut q = self.state.query.clone();
        let gap = ui.spacing().item_spacing.x;
        let width = (ui.available_width() - ICON - toggle - 2.0 * gap).clamp(SEARCH_MIN.min(ui.available_width()), SEARCH_MAX);
        let edit = ui.add(TextEdit::singleline(&mut q).id(self.base.with("search")).desired_width(width)).labelled_by(l.id);
        self.text_field(&edit);
        self.tag(|| "shortcuts.search".into(), edit.rect, &search_label);
        if edit.changed() {
            self.state.set_query(q);
        }

        let key_label = self.ui("Search by key");
        let searching = self.recording_target() == Some(&Target::Search);
        let r = self.icon_button(ui, &key_label, searching, paint_keyboard).on_hover_text(self.vis(&key_label));
        self.tag(|| "shortcuts.searchKey".into(), r.rect, &key_label);
        if r.clicked() {
            if searching {
                self.state.stop_recording();
            } else {
                self.state.record(self.env, Target::Search);
                r.request_focus();
            }
        }
        if self.recording_target() == Some(&Target::Search) {
            if r.lost_focus() {
                self.state.stop_recording();
            } else if r.has_focus() || r.clicked() {
                capture::lock_focus(ui, r.id);
            }
        } else if r.has_focus() {
            capture::unlock_focus(ui, r.id);
        }
        self.keyboard_toggle(ui);
    }

    fn area_combo(&mut self, ui: &mut Ui) {
        let show = self.ui("Show:");
        let l = ui.label(self.vis(&show));
        let current = self.state.areas.iter().find(|a| a.id == self.state.area).map_or_else(String::new, |a| a.label.clone());
        let mut pick = None;
        let areas = self.state.areas.clone();
        self.combo(ui, "area", &l, &show, &current, |ui| {
            for a in &areas {
                let r = ui.selectable_label(a.id == self.state.area, self.vis(&a.label));
                self.tag(|| format!("shortcuts.area.{}", Self::area_key(&a.id)), r.rect, &a.label);
                if r.clicked() {
                    pick = Some(a.id.clone());
                }
            }
        });
        if let Some(a) = pick {
            self.state.set_area(a);
        }
    }

    /// Show as tabs, with the ones that do not fit in a menu.
    fn area_tabs(&mut self, ui: &mut Ui) {
        let areas = self.state.areas.clone();
        let pad = ui.spacing().button_padding.x * 2.0;
        let font = TextStyle::Button.resolve(ui.style());
        let natural: Vec<f32> =
            areas.iter().map(|a| ui.painter().layout_no_wrap(self.vis(&a.label), font.clone(), Color32::WHITE).size().x + pad).collect();
        let selected = areas.iter().position(|a| a.id == self.state.area).unwrap_or(0);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ui.spacing().interact_size.y + 4.0), Sense::hover());
        let fit = tabs::fit(&natural, selected, rect.width(), 48.0, ICON + 8.0);
        let m = self.mirror(rect);
        let mut x = rect.min.x;
        let mut pick = None;
        for &(i, w) in &fit.shown {
            let Some(a) = areas.get(i) else { continue };
            let r = m.rect(Rect::from_min_size(pos2(x, rect.min.y), vec2(w, rect.height())));
            x += w;
            let on = i == selected;
            let id = self.base.with(("area", i));
            let text = self.vis(&a.label);
            let accent = self.style.accent;
            let resp = Tab::new(id, &a.label, on).show_at(ui, r, |ui, resp| {
                let v = ui.visuals();
                let color = if on { v.strong_text_color() } else { v.text_color() };
                if resp.hovered() && !on {
                    ui.painter().rect_filled(r, 2.0, v.widgets.hovered.weak_bg_fill);
                }
                if on {
                    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(2.0, accent));
                }
                let (g, _) = self.one_line(ui.painter(), &[(text, TextFormat::simple(font.clone(), color))], r.width() - 4.0);
                ui.painter().galley(r.center() - g.size() / 2.0, g, color);
            });
            self.tag(|| format!("shortcuts.area.{}", Self::area_key(&a.id)), resp.rect, &a.label);
            if resp.clicked() {
                pick = Some(a.id.clone());
            }
        }
        if !fit.overflow.is_empty() {
            let r = m.rect(Rect::from_min_size(pos2(rect.max.x - fit.overflow_width, rect.min.y), vec2(fit.overflow_width, rect.height())));
            let more = self.ui("More");
            ui.scope_builder(UiBuilder::new().max_rect(r), |ui| {
                ui.menu_button(self.vis(&more), |ui| {
                    for &i in &fit.overflow {
                        if let Some(a) = areas.get(i)
                            && ui.button(self.vis(&a.label)).clicked()
                        {
                            pick = Some(a.id.clone());
                        }
                    }
                });
            });
        }
        if let Some(a) = pick {
            self.state.set_area(a);
        }
    }

    // ---- Table and detail pane ----

    /// The table and detail pane, with the keyboard pane under both when it is on.
    fn middle(&mut self, ui: &mut Ui, mid: Rect) {
        let Some(kb_h) = self.keyboard_height(mid.height()) else {
            self.table_and_detail(ui, mid);
            return;
        };
        let gap = ui.spacing().item_spacing.y;
        let top = Rect::from_min_max(mid.min, pos2(mid.max.x, mid.max.y - kb_h - gap));
        let pane = Rect::from_min_max(pos2(mid.min.x, mid.max.y - kb_h), mid.max);
        self.table_and_detail(ui, top);
        self.child(ui, pane, |d, ui| d.keyboard_pane(ui));
    }

    fn table_and_detail(&mut self, ui: &mut Ui, mid: Rect) {
        let wide = mid.width() >= NARROW;
        let axis = if wide { SplitAxis::Horizontal } else { SplitAxis::Vertical };
        // Right to left: the detail pane goes on the left.
        let swap = wide && self.env.text.rtl();
        let ratio = if swap { 1.0 - self.state.split } else { self.state.split };
        let extent = if wide { mid.width() } else { mid.height() };
        let Some(split) = split_rect(mid, axis, SplitSize::Ratio(ratio), 8.0, SPLIT_MIN.min(extent / 3.0)) else { return };
        let (table, detail) = if swap { (split.second, split.first) } else { (split.first, split.second) };
        self.child(ui, table, |d, ui| d.table(ui));
        let label = self.ui("Detail pane size");
        let resp = ResizeHandle::new(self.base.with("split"), axis, split.divider, &label, style::divider(ui.visuals()))
            .value(split.first_size(), 0.0..=split.available())
            .show(ui);
        let line = ui.visuals().widgets.noninteractive.bg_stroke;
        let c = split.divider.center();
        let ends = if wide {
            [pos2(c.x, split.divider.min.y), pos2(c.x, split.divider.max.y)]
        } else {
            [pos2(split.divider.min.x, c.y), pos2(split.divider.max.x, c.y)]
        };
        ui.painter().line_segment(ends, line);
        if resp.delta != 0.0
            && let Some(SplitSize::Ratio(r)) = split.resized(SplitSize::Ratio(ratio), resp.delta, SPLIT_MIN)
        {
            self.state.split = if swap { 1.0 - r } else { r };
        }
        self.child(ui, detail, |d, ui| d.detail(ui));
    }

    /// Width of the shortcut column: the widest row of chips plus "+", within 40 % of the table.
    fn shortcut_width(&self, ui: &Ui, width: f32) -> f32 {
        let font = TextStyle::Body.resolve(ui.style());
        let chip_w = |text: &str| ui.painter().layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE).size().x + 2.0 * CHIP_PAD + CHIP_GAP;
        let chips = self
            .state
            .lines_cached()
            .iter()
            .filter_map(|l| if let Line::Row(r) = l { Some(r.chips.iter().map(|c| chip_w(&c.text)).sum::<f32>()) } else { None })
            .fold(0.0, f32::max);
        (chips + ICON + CHIP_GAP).max(110.0).min(width * 0.4)
    }

    fn table(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        let table_id = self.base.with("table");
        let focus = ui.interact(rect, table_id, Sense::focusable_noninteractive());
        let commands = self.ui("Commands");
        focus.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, &commands));
        let has_focus = focus.has_focus();
        if has_focus && !self.state.is_recording() {
            self.table_keys(ui);
        }
        let recording_cell = matches!(self.recording_target(), Some(Target::Cell { .. }));
        if recording_cell && focus.lost_focus() {
            self.state.stop_recording();
        }
        if has_focus {
            let filter = if recording_cell {
                capture::FOCUS_LOCK
            } else {
                EventFilter { vertical_arrows: true, horizontal_arrows: true, tab: false, escape: false }
            };
            ui.memory_mut(|m| m.set_focus_lock_filter(table_id, filter));
        }

        let selected = self.state.selected.clone();
        let built = self.state.lines(self.env);
        let (count, matches) = (built.lines.len(), built.matches);
        let position = selected.as_ref().and_then(|c| built.position(c));
        let row_h = self.row_h;
        let shortcut_w = self.shortcut_width(ui, rect.width());

        // Column titles.
        let (head, _) = ui.allocate_exact_size(vec2(rect.width(), row_h), Sense::hover());
        let font = TextStyle::Body.resolve(ui.style());
        let dim = self.style.dim;
        let m = self.mirror(head);
        let key_x = head.max.x - shortcut_w - ICON - 4.0;
        let titles = [(self.ui("Command"), head.min.x + 6.0, key_x - head.min.x - 12.0), (self.ui("Shortcut"), key_x, shortcut_w)];
        for (title, x, w) in titles {
            let (g, _) = self.one_line(ui.painter(), &[(self.vis(&title), TextFormat::simple(font.clone(), dim))], w);
            ui.painter().galley(pos2(m.x(x, g.size().x), head.center().y - g.size().y / 2.0), g, dim);
        }
        ui.painter().line_segment([head.left_bottom(), head.right_bottom()], ui.visuals().widgets.noninteractive.bg_stroke);

        if matches == 0 {
            ui.add_space(8.0);
            ui.label(RichText::new(self.vis(&self.ui("No matching commands"))).color(dim));
        }

        let mut scroll = ScrollArea::vertical().id_salt(self.base.with("rows")).auto_shrink([false, false]);
        if std::mem::take(&mut self.state.reveal)
            && let Some(pos) = position
        {
            let inside = self.state.visible.start < pos && pos + 1 < self.state.visible.end;
            if !inside {
                let view = ui.available_height();
                scroll = scroll.vertical_scroll_offset((pos as f32 * row_h - (view - row_h) / 2.0).max(0.0));
            }
        }
        let mut action = None;
        let mut shown = 0..0;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            scroll.show_rows(ui, row_h, count, |ui, range| {
                shown = range.clone();
                action = self.rows(ui, range, shortcut_w, has_focus);
            });
        });
        // Screen readers follow the selected row while the table has focus.
        let row_id = position.filter(|p| shown.contains(p)).and_then(|p| match self.state.lines_cached().get(p) {
            Some(Line::Row(r)) => Some(self.base.with(("row", &r.key))),
            _ => None,
        });
        if let Some(id) = row_id {
            ui.ctx().accesskit_node_builder(table_id, |node| node.set_active_descendant(id.accesskit_id()));
        }
        self.state.visible = shown;
        if has_focus {
            ui.painter().rect_stroke(rect, 2.0, ui.visuals().selection.stroke, StrokeKind::Inside);
        }

        if let Some(a) = action {
            // A group header is a Tab stop of its own and keeps focus.
            if !matches!(a, RowAction::Toggle(_)) {
                focus.request_focus();
            }
            match a {
                RowAction::Select(c) => self.state.select_row(&c),
                RowAction::Record(c, old, chip) => {
                    self.state.record(self.env, Target::Cell { command: c, old });
                    self.state.chip = chip;
                }
                RowAction::Reset(c) => {
                    self.state.select_row(&c);
                    self.state.use_default(self.env, &c);
                }
                RowAction::Toggle(name) => self.state.toggle_group(&name),
            }
            if self.state.is_recording() {
                ui.memory_mut(|m| m.set_focus_lock_filter(table_id, capture::FOCUS_LOCK));
            }
        }
    }

    /// Up/Down, Home/End, Left/Right between chips, Enter records, Delete removes (§2.10).
    fn table_keys(&mut self, ui: &Ui) {
        let keys = [
            EguiKey::ArrowUp,
            EguiKey::ArrowDown,
            EguiKey::Home,
            EguiKey::End,
            EguiKey::ArrowLeft,
            EguiKey::ArrowRight,
            EguiKey::Enter,
            EguiKey::Delete,
        ];
        let pressed = ui.input_mut(|i| keys.map(|k| i.consume_key(Modifiers::NONE, k)));
        let [up, down, home, end, left, right, enter, delete] = pressed;
        if !pressed.contains(&true) {
            return;
        }
        let rows: Vec<(CommandRef, Vec<Binding>)> = self
            .state
            .lines(self.env)
            .lines
            .iter()
            .filter_map(|l| if let Line::Row(r) = l { Some((r.command.clone(), r.chips.iter().map(|c| c.binding.clone()).collect())) } else { None })
            .collect();
        let at = self.state.selected.as_ref().and_then(|s| rows.iter().position(|(c, _)| c == s));
        let to = if up {
            Some(at.map_or(0, |i| i.saturating_sub(1)))
        } else if down {
            Some(at.map_or(0, |i| (i + 1).min(rows.len().saturating_sub(1))))
        } else if home {
            Some(0)
        } else if end {
            Some(rows.len().saturating_sub(1))
        } else {
            None
        };
        if let Some((command, _)) = to.and_then(|i| rows.get(i)) {
            self.state.select_row(command);
            self.state.reveal = true;
            return;
        }
        let Some((command, bindings)) = at.and_then(|i| rows.get(i)) else { return };
        let chip = self.state.chip.min(bindings.len().saturating_sub(1));
        if left {
            self.state.chip = chip.saturating_sub(1);
        } else if right {
            self.state.chip = (chip + 1).min(bindings.len().saturating_sub(1));
        } else if enter && self.env.options.editable {
            self.state.record(self.env, Target::Cell { command: command.clone(), old: bindings.get(chip).cloned() });
            self.state.chip = chip;
        } else if delete
            && self.env.options.editable
            && let Some(b) = bindings.get(chip)
        {
            self.state.remove(self.env, command, b);
            self.state.chip = chip.saturating_sub(1);
        }
    }

    fn rows(&mut self, ui: &mut Ui, range: Range<usize>, shortcut_w: f32, focused: bool) -> Option<RowAction> {
        let lines: Vec<Line> = self.state.lines(self.env).lines.get(range).map(<[Line]>::to_vec).unwrap_or_default();
        let mut action = None;
        for line in lines {
            let (_, rect) = ui.allocate_space(vec2(ui.available_width(), self.row_h));
            let a = match &line {
                Line::Header { name, collapsed } => self.group_header(ui, rect, name, *collapsed),
                Line::Row(r) => self.row(ui, rect, r, shortcut_w, focused),
                Line::Fixed(f) => {
                    self.fixed_row(ui, rect, f, shortcut_w);
                    None
                }
            };
            action = action.or(a);
        }
        action
    }

    /// A group's header; a Tab stop, toggled with Enter or Space.
    fn group_header(&self, ui: &mut Ui, rect: Rect, name: &str, collapsed: bool) -> Option<RowAction> {
        let resp = ui.interact(rect, self.base.with(("group", name)), Sense::click());
        resp.widget_info(|| WidgetInfo::labeled(WidgetType::CollapsingHeader, true, name));
        ui.ctx().accesskit_node_builder(resp.id, |node| node.set_expanded(!collapsed));
        if resp.has_focus() {
            ui.painter().rect_stroke(rect, 2.0, ui.visuals().selection.stroke, StrokeKind::Inside);
        }
        let v = ui.visuals();
        let color = v.strong_text_color();
        let m = self.mirror(rect);
        let tri = m.rect(Rect::from_center_size(pos2(rect.min.x + 10.0, rect.center().y), Vec2::splat(8.0)));
        let points = match (collapsed, m.on) {
            (true, false) => vec![tri.left_top(), tri.right_center(), tri.left_bottom()],
            (true, true) => vec![tri.right_top(), tri.right_bottom(), tri.left_center()],
            (false, _) => vec![tri.left_top(), tri.right_top(), tri.center_bottom()],
        };
        ui.painter().add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
        let font = TextStyle::Body.resolve(ui.style());
        let (g, cut) = self.one_line(ui.painter(), &[(self.vis(name), TextFormat::simple(font, color))], rect.width() - 24.0);
        ui.painter().galley(pos2(m.x(rect.min.x + 20.0, g.size().x), rect.center().y - g.size().y / 2.0), g, color);
        let clicked = resp.clicked();
        if cut {
            resp.on_hover_text(self.vis(name));
        }
        clicked.then(|| RowAction::Toggle(name.to_string()))
    }

    fn row(&self, ui: &mut Ui, rect: Rect, r: &Row, shortcut_w: f32, focused: bool) -> Option<RowAction> {
        let editable = self.env.options.editable;
        let selected = self.state.selected.as_ref() == Some(&r.command);
        let resp = ui.interact(rect, self.base.with(("row", &r.key)), Sense::CLICK);
        let mut a11y = r.label.clone();
        if r.cycles {
            a11y = fill(self.ui("{label} (cycles)"), &[("label", &a11y)]);
        }
        if r.changed {
            a11y = fill(self.ui("{label} (Changed)"), &[("label", &a11y)]);
        }
        resp.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &a11y));
        ui.ctx().accesskit_node_builder(resp.id, |node| node.set_selected(selected));
        self.tag(|| format!("shortcuts.row.{}", r.key), rect, &r.label);
        let v = ui.visuals();
        if selected {
            let fill = if focused { v.selection.bg_fill } else { v.selection.bg_fill.gamma_multiply(0.5) };
            ui.painter().rect_filled(rect, 0.0, fill);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, v.widgets.hovered.weak_bg_fill);
        }
        let m = self.mirror(rect);
        if r.changed {
            ui.painter().rect_filled(m.rect(Rect::from_min_size(rect.min + vec2(1.0, 3.0), vec2(3.0, rect.height() - 6.0))), 1.0, self.style.changed);
        }
        let text_color = if selected { v.strong_text_color() } else { v.text_color() };
        let strong = v.strong_text_color();
        let dim = self.style.dim;
        let font = TextStyle::Body.resolve(ui.style());

        // Label column.
        let label_w = (rect.width() - shortcut_w - ICON - 12.0).max(0.0);
        let fmt = |c: Color32| TextFormat::simple(font.clone(), c);
        let mut parts = Vec::new();
        if r.conflict {
            parts.extend([("⚠".to_string(), fmt(self.style.warning)), (" ".into(), fmt(dim))]);
        }
        for p in &r.prefix {
            parts.extend([(self.vis(p), fmt(dim)), (PATH_SEP.into(), fmt(dim))]);
        }
        parts.push((self.vis(&r.label), fmt(if r.changed { strong } else { text_color })));
        if r.cycles {
            parts.extend([(" ".into(), fmt(dim)), (self.vis(&self.ui("(cycles)")), fmt(dim))]);
        }
        let (g, _) = self.one_line(ui.painter(), &self.ordered(parts), label_w);
        ui.painter().galley(pos2(m.x(rect.min.x + 8.0, g.size().x), rect.center().y - g.size().y / 2.0), g, text_color);

        // Shortcut column: chips, then "+".
        let mut action = None;
        let col = Rect::from_min_max(pos2(rect.max.x - shortcut_w - ICON - 4.0, rect.min.y), pos2(rect.max.x - ICON - 4.0, rect.max.y));
        let painter = ui.painter_at(m.rect(col));
        let mut x = col.min.x;
        let rec = match self.recording_target() {
            Some(Target::Cell { command, old }) if *command == r.command => Some(old.clone()),
            _ => None,
        };
        let chip_h = rect.height() - 4.0;
        let mut cells: Vec<(Option<Binding>, String, bool)> =
            r.chips.iter().map(|c| (Some(c.binding.clone()), c.text.clone(), rec.as_ref() == Some(&Some(c.binding.clone())))).collect();
        if rec == Some(None) {
            cells.push((None, String::new(), true));
        }
        for (n, (binding, text, recording)) in cells.iter().enumerate() {
            let shown = if *recording { self.recording_text() } else { text.clone() };
            let g = painter.layout_no_wrap(shown.clone(), font.clone(), text_color);
            let w = g.size().x + 2.0 * CHIP_PAD;
            let chip = m.rect(Rect::from_min_size(pos2(x, rect.min.y + 2.0), vec2(w, chip_h)));
            x += w + CHIP_GAP;
            let changed = r.chips.get(n).is_some_and(|c| c.changed);
            let stroke = if *recording {
                Stroke::new(1.5, self.style.accent)
            } else if changed {
                Stroke::new(1.0, self.style.changed)
            } else {
                v.widgets.inactive.bg_stroke
            };
            let current_chip = selected && focused && n == self.state.chip;
            painter.rect(chip, 3.0, v.widgets.inactive.weak_bg_fill, if current_chip { v.selection.stroke } else { stroke }, StrokeKind::Inside);
            painter.galley(pos2(chip.min.x + CHIP_PAD, chip.center().y - g.size().y / 2.0), g, v.text_color());
            let waiting = self.state.recording.as_ref().is_some_and(|r| r.waiting.is_some());
            let label = if *recording && waiting {
                fill(self.ui("{label}: {chord}"), &[("label", &r.label), ("chord", &shown)])
            } else if *recording {
                fill(self.ui("{label}: press keys"), &[("label", &r.label)])
            } else {
                fill(self.ui("{label}: {chord}"), &[("label", &r.label), ("chord", text)])
            };
            let resp =
                ui.interact(chip.intersect(m.rect(col)), self.base.with(("cell", &r.key, n)), if editable { Sense::CLICK } else { Sense::hover() });
            resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, editable, &label));
            self.tag(|| format!("shortcuts.cell.{}.{n}", r.key), chip, &label);
            if resp.clicked() {
                action = Some(RowAction::Record(r.command.clone(), binding.clone(), n));
            }
        }
        if cells.is_empty() {
            // An empty cell to click for the first shortcut.
            let cell = m.rect(Rect::from_min_max(pos2(col.min.x, rect.min.y), pos2(col.max.x - ICON - CHIP_GAP, rect.max.y)));
            let label = fill(self.ui("{label}: {chord}"), &[("label", &r.label), ("chord", &self.ui("None"))]);
            let resp = ui.interact(cell, self.base.with(("cell", &r.key, 0)), if editable { Sense::CLICK } else { Sense::hover() });
            resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, editable, &label));
            self.tag(|| format!("shortcuts.cell.{}.0", r.key), cell, &label);
            if resp.clicked() {
                action = Some(RowAction::Record(r.command.clone(), None, 0));
            }
            x = col.min.x;
        }
        if editable && r.chips.len() < MAX_BINDINGS && rec != Some(None) {
            let plus = m.rect(Rect::from_min_size(pos2(x.min(col.max.x - ICON), rect.center().y - ICON / 2.0), Vec2::splat(ICON)));
            let label = fill(self.ui("Add a shortcut to {label}"), &[("label", &r.label)]);
            let resp = self.row_button(ui, plus, self.base.with(("add", &r.key)), &label, true, paint_plus);
            self.tag(|| format!("shortcuts.add.{}", r.key), plus, &label);
            if resp.clicked() {
                action = Some(RowAction::Record(r.command.clone(), None, r.chips.len()));
            }
        }
        if editable {
            let reset = m.rect(Rect::from_min_size(pos2(rect.max.x - ICON - 2.0, rect.center().y - ICON / 2.0), Vec2::splat(ICON)));
            let label = fill(self.ui("Restore the default for {label}"), &[("label", &r.label)]);
            let resp = self.row_button(ui, reset, self.base.with(("reset", &r.key)), &label, r.changed, paint_reset);
            self.tag(|| format!("shortcuts.reset.{}", r.key), reset, &label);
            if resp.clicked() {
                action = Some(RowAction::Reset(r.command.clone()));
            }
        }

        let clicked = resp.clicked();
        let mut tip = vec![self.vis(&r.label)];
        if let Some(b) = self.env.keymap.bindable(&r.command) {
            if let Some(path) = b.paths.first().filter(|p| !p.is_empty()) {
                tip.push(self.path_text(path));
            }
            if let Some(h) = self.env.text.hint(b) {
                tip.push(self.vis(&h));
            }
        }
        resp.on_hover_text(tip.join("\n"));
        action.or_else(|| clicked.then(|| RowAction::Select(r.command.clone())))
    }

    /// A read-only row from `Options::fixed`, struck through when a command took its chord.
    fn fixed_row(&self, ui: &mut Ui, rect: Rect, f: &FixedLine, shortcut_w: f32) {
        let v = ui.visuals();
        let font = TextStyle::Body.resolve(ui.style());
        let color = if f.now_used_by.is_some() { self.style.dim } else { v.text_color() };
        let mut fmt = TextFormat::simple(font.clone(), color);
        if f.now_used_by.is_some() {
            fmt.strikethrough = Stroke::new(1.0, color);
        }
        let m = self.mirror(rect);
        let label_w = rect.width() - shortcut_w - ICON - 12.0;
        let (g, cut) = self.one_line(ui.painter(), &[(self.vis(&f.label), fmt.clone())], label_w);
        ui.painter().galley(pos2(m.x(rect.min.x + 8.0, g.size().x), rect.center().y - g.size().y / 2.0), g, color);
        let (keys, _) = self.one_line(ui.painter(), &[(f.keys.clone(), fmt)], shortcut_w);
        ui.painter().galley(pos2(m.x(rect.max.x - shortcut_w - ICON - 4.0, keys.size().x), rect.center().y - keys.size().y / 2.0), keys, color);
        let resp = ui.interact(rect, self.base.with(("fixed", &f.label, &f.keys)), Sense::hover());
        let mut tip = Vec::new();
        if cut {
            tip.push(self.vis(&f.label));
        }
        match &f.now_used_by {
            Some(by) => {
                let label = fill(self.ui("{label}: {chord} (Now used by {holder})"), &[("label", &f.label), ("chord", &f.keys), ("holder", by)]);
                resp.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &label));
                tip.push(self.vis(&fill(self.ui("Now used by {label}"), &[("label", by)])));
            }
            None => {
                let label = fill(self.ui("{label}: {chord}"), &[("label", &f.label), ("chord", &f.keys)]);
                resp.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &label));
            }
        }
        if !tip.is_empty() {
            resp.on_hover_text(tip.join("\n"));
        }
    }

    fn detail(&mut self, ui: &mut Ui) {
        ScrollArea::vertical().id_salt(self.base.with("detail")).auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(4.0);
            let Some(command) = self.state.selected.clone().filter(|c| self.env.keymap.bindable(c).is_some()) else {
                ui.label(RichText::new(self.vis(&self.ui("Select a command to see and change its shortcuts"))).color(self.style.dim));
                return;
            };
            self.detail_of(ui, &command);
        });
    }

    fn detail_of(&mut self, ui: &mut Ui, command: &CommandRef) {
        let Some(b) = self.env.keymap.bindable(command).cloned() else { return };
        let editable = self.env.options.editable;
        let label = self.env.text.command(&b).into_owned();
        let home = Some(&b.scope);
        let dim = self.style.dim;
        ui.add(Label::new(RichText::new(self.vis(&label)).strong().size(ui.text_style_height(&TextStyle::Body) * 1.15)).wrap());
        let path: Vec<String> = b.paths.first().map(|p| p.iter().filter(|s| !s.is_empty()).cloned().collect()).unwrap_or_default();
        let place =
            if path.is_empty() { b.command.id.clone() } else { self.joined(vec![self.path_text(&path), " · ".into(), b.command.id.clone()]) };
        ui.add(Label::new(RichText::new(place).color(dim)).wrap());
        if let Some(h) = self.env.text.hint(&b) {
            ui.add(Label::new(RichText::new(self.vis(&h)).color(dim)).wrap());
        }
        ui.add_space(6.0);

        // Current shortcuts, each with Remove.
        ui.label(self.vis(&self.ui("Current shortcuts:")));
        let bindings = self.env.keymap.bindings(command);
        let defaults = self.env.keymap.default_bindings(command);
        if bindings.is_empty() {
            ui.label(RichText::new(self.vis(&self.ui("None"))).color(dim));
        }
        let mut remove = None;
        for (n, binding) in bindings.iter().enumerate() {
            let text = self.binding_text(binding, home);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let shown = if defaults.contains(binding) { fill(self.ui("{chord} (Default)"), &[("chord", &text)]) } else { text.clone() };
                ui.label(self.vis_chord(&shown, &self.chord(&binding.trigger)));
                if editable {
                    let a11y = fill(self.ui("Remove {chord} from {label}"), &[("chord", &text), ("label", &label)]);
                    let r = self.icon_button(ui, &a11y, false, paint_cross).on_hover_text(self.vis(&a11y));
                    self.tag(|| format!("shortcuts.remove.{n}"), r.rect, &a11y);
                    if r.clicked() {
                        remove = Some(binding.clone());
                    }
                }
            });
        }
        if let Some(binding) = remove {
            self.state.remove(self.env, command, &binding);
        }
        if !editable {
            self.defaults_line(ui, &defaults, home);
            return;
        }
        ui.add_space(6.0);
        self.new_shortcut(ui, command, home);
        self.context(ui, command);
        self.holders(ui);
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if self.button(ui, self.state.field.is_some(), &self.ui("Assign"), "shortcuts.assign") {
                self.state.assign_field(self.env);
            }
            if self.button(ui, DialogState::can_use_default(self.env, command), &self.ui("Use Default"), "shortcuts.useDefault") {
                self.state.use_default(self.env, command);
            }
            if self.button(ui, !bindings.is_empty(), &self.ui("Clear"), "shortcuts.clear") {
                self.state.clear(self.env, command);
            }
        });
        self.defaults_line(ui, &defaults, home);
    }

    fn defaults_line(&self, ui: &mut Ui, defaults: &[Binding], home: Option<&Scope>) {
        let chords =
            if defaults.is_empty() { self.ui("None") } else { defaults.iter().map(|d| self.binding_text(d, home)).collect::<Vec<_>>().join(", ") };
        let text = self.vis_chord(&fill(self.ui("Default: {chord}"), &[("chord", &chords)]), &chords);
        ui.add(Label::new(RichText::new(text).color(self.style.dim)).wrap());
    }

    /// New Shortcut: a field that records when focused or clicked, or typed entry.
    fn new_shortcut(&mut self, ui: &mut Ui, command: &CommandRef, home: Option<&Scope>) {
        let title = self.ui("New shortcut:");
        let title_id = ui.label(self.vis(&title)).id;
        let field_id = self.base.with("new");
        let recording = matches!(self.recording_target(), Some(Target::Field { command: c }) if c == command);
        if let Some(mut typed) = self.state.typing.clone() {
            let type_id = self.base.with("type");
            let edit = ui.add(TextEdit::singleline(&mut typed).id(type_id).desired_width(FIELD_W).hint_text("Cmd+Shift+K"));
            let edit = edit.labelled_by(title_id);
            // Focus once, on the frame after Type… was clicked.
            if ui.data_mut(|d| d.remove_temp::<bool>(type_id)).is_some() {
                edit.request_focus();
            }
            self.text_field(&edit);
            self.tag(|| "shortcuts.type".into(), edit.rect, &title);
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(EguiKey::Enter));
            self.state.typing = Some(typed.clone());
            if enter {
                match self.state.type_binding(self.env, command, &typed) {
                    Ok(()) => {
                        self.state.typing = None;
                        self.state.type_error = None;
                    }
                    Err(e) => self.state.type_error = Some(e),
                }
            }
            if let Some(e) = &self.state.type_error {
                ui.add(Label::new(RichText::new(self.warned(self.vis(e))).color(self.style.warning)).wrap());
            }
            return;
        }
        ui.horizontal(|ui| {
            let shown = if recording {
                self.recording_text()
            } else {
                self.state.field.as_ref().filter(|f| f.command == *command).map_or_else(String::new, |f| self.binding_text(&f.binding, home))
            };
            let size = vec2(FIELD_W, ui.spacing().interact_size.y);
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let resp = ui.interact(rect, field_id, Sense::click());
            let value = if shown.is_empty() { self.ui("None") } else { shown.clone() };
            let a11y = fill(self.ui("New shortcut: {chord}"), &[("chord", &value)]);
            resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &a11y));
            let v = ui.visuals();
            let stroke = if recording || resp.has_focus() { v.selection.stroke } else { v.widgets.inactive.bg_stroke };
            ui.painter().rect(rect, 3.0, v.extreme_bg_color, stroke, StrokeKind::Inside);
            let color = if recording { self.style.dim } else { v.text_color() };
            ui.painter().text(rect.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, shown, TextStyle::Body.resolve(ui.style()), color);
            self.tag(|| "shortcuts.new".into(), rect, &title);
            if !recording && (resp.clicked() || resp.gained_focus()) {
                self.state.record(self.env, Target::Field { command: command.clone() });
                resp.request_focus();
                capture::lock_focus(ui, field_id);
            } else if recording {
                if resp.lost_focus() {
                    self.state.stop_recording();
                } else {
                    capture::lock_focus(ui, field_id);
                }
            } else if resp.has_focus() {
                capture::unlock_focus(ui, field_id);
            }
            if ui.link(self.vis(&self.ui("Type…"))).clicked() {
                self.state.stop_recording();
                self.state.typing = Some(String::new());
                self.state.type_error = None;
                let type_id = self.base.with("type");
                ui.data_mut(|d| d.insert_temp(type_id, true));
            }
        });
    }

    fn context(&mut self, ui: &mut Ui, command: &CommandRef) {
        let contexts = DialogState::contexts(self.env, command);
        let name = |s: &Option<Scope>| match s {
            None => self.ui("Default"),
            Some(s) => self.scope_name(s),
        };
        let current = name(&self.state.context);
        let names: Vec<String> = contexts.iter().map(name).collect();
        let title = self.ui("Context:");
        let mut pick = None;
        ui.horizontal(|ui| {
            let l = ui.label(self.vis(&title));
            let combo = ComboBox::from_id_salt(self.base.with("context")).selected_text(self.vis(&current)).show_ui(ui, |ui| {
                for (s, n) in contexts.iter().zip(&names) {
                    if ui.selectable_label(*s == self.state.context, self.vis(n)).clicked() {
                        pick = Some(s.clone());
                    }
                }
            });
            let combo = combo.response.labelled_by(l.id);
            self.tag(|| "shortcuts.context".into(), combo.rect, &title);
        });
        if let Some(s) = pick {
            self.state.set_context(self.env, s);
        }
    }

    /// "Currently assigned to:" for the field's chord, with links to the holders.
    fn holders(&mut self, ui: &mut Ui) {
        let Some(f) = self.state.field.clone() else { return };
        if f.holders.is_empty() && f.shadows.is_empty() {
            ui.label(RichText::new(self.vis(&self.ui("Not assigned"))).color(self.style.dim));
            return;
        }
        ui.label(self.vis(&self.ui("Currently assigned to:")));
        let mut go = None;
        for h in &f.holders {
            let label = self.label_of(&h.command);
            let home = self.env.keymap.bindable(&h.command).map(|b| b.scope.clone());
            let text = match &h.binding.scope {
                Some(s) if Some(s) != home.as_ref() => fill(self.ui("{label} ({scope})"), &[("label", &label), ("scope", &self.scope_name(s))]),
                _ => label,
            };
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                if ui.link(self.warned(self.vis(&text))).clicked() {
                    go = Some(h.command.clone());
                }
            });
        }
        for h in &f.shadows {
            let label = self.label_of(&h.command);
            let home = self.env.keymap.bindable(&h.command).map(|b| b.scope.clone()).unwrap_or_else(Scope::app);
            let scope = self.scope_name(h.binding.scope_or(&home));
            let text = fill(self.ui("{label} ({scope}, both work)"), &[("label", &label), ("scope", &scope)]);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                if ui.link(self.vis(&text)).clicked() {
                    go = Some(h.command.clone());
                }
            });
        }
        if let Some(c) = go {
            self.state.go_to(&c);
        }
    }

    // ---- Under the table ----

    fn bottom(&mut self, ui: &mut Ui) {
        self.prompt(ui);
        self.conflict_bar(ui);
        self.message(ui);
        self.info(ui);
        self.footer(ui);
    }

    fn prompt(&mut self, ui: &mut Ui) {
        let Some(p) = self.state.prompt.clone() else { return };
        let (text, yes, no) = match &p {
            Prompt::ResetAll => (self.ui("Restore every default shortcut in this set?"), self.ui("Reset All"), None),
            Prompt::DeleteSet(set) => (fill(self.ui("Delete the set “{set}”?"), &[("set", set)]), self.ui("Delete"), None),
            Prompt::SaveChanges { set, .. } => {
                (fill(self.ui("Save changes to “{set}”?"), &[("set", set)]), self.ui("Save"), Some(self.ui("Don't Save")))
            }
        };
        ui.add(Label::new(RichText::new(self.vis(&text)).strong()).wrap());
        let mut answer = None;
        ui.horizontal_wrapped(|ui| {
            if self.button(ui, true, &yes, "shortcuts.prompt.yes") {
                answer = Some(Answer::Yes);
            }
            if let Some(no) = &no
                && self.button(ui, true, no, "shortcuts.prompt.no")
            {
                answer = Some(Answer::No);
            }
            if self.button(ui, true, &self.ui("Cancel"), "shortcuts.prompt.cancel") {
                answer = Some(Answer::Cancel);
            }
        });
        if let Some(a) = answer {
            self.state.answer(self.env, a);
        }
    }

    fn conflict_bar(&mut self, ui: &mut Ui) {
        let Some(c) = self.state.conflict.clone() else { return };
        let text = self.vis_chord(&c.text, &self.chord(&c.binding.trigger));
        let r = ui.add(Label::new(RichText::new(text).color(self.style.warning)).wrap());
        ui.ctx().accesskit_node_builder(r.id, |node| node.set_live(Live::Polite));
        let mut act = 0;
        ui.horizontal_wrapped(|ui| {
            if self.button(ui, true, &self.ui("Take It"), "shortcuts.conflict.take") {
                act = 1;
            }
            if self.button(ui, true, &self.ui("Take It and Go to Conflict"), "shortcuts.conflict.goto") {
                act = 2;
            }
            if self.env.options.allow_keep && self.button(ui, true, &self.ui("Keep Both"), "shortcuts.conflict.keep") {
                act = 3;
            }
            let label = self.ui("Cancel the new shortcut");
            let r = self.icon_button(ui, &label, false, paint_cross).on_hover_text(self.vis(&label));
            self.tag(|| "shortcuts.conflict.cancel".into(), r.rect, &label);
            if r.clicked() {
                act = 4;
            }
        });
        match act {
            1 => self.state.take_it(self.env),
            2 => self.state.take_it_and_go(self.env),
            3 => self.state.keep_both(self.env),
            4 => self.state.dismiss_conflict(),
            _ => {}
        }
    }

    fn message(&mut self, ui: &mut Ui) {
        let Some(m) = self.state.message.clone() else { return };
        let mut act = 0;
        ui.horizontal_wrapped(|ui| {
            let text = self.vis_chord(&m.text, m.chord.as_deref().unwrap_or_default());
            let text = if m.refused { self.warned(text) } else { text };
            let color = if m.refused { self.style.warning } else { ui.visuals().text_color() };
            let r = ui.add(Label::new(RichText::new(text).color(color)).wrap());
            ui.ctx().accesskit_node_builder(r.id, |node| node.set_live(Live::Polite));
            if m.goto.is_some() && self.button(ui, true, &self.ui("Go to Conflict"), "shortcuts.message.goto") {
                act = 1;
            }
            if m.undo && self.button(ui, self.state.can_undo() && !self.state.modal(), &self.ui("Undo"), "shortcuts.message.undo") {
                act = 2;
            }
            let label = self.ui("Dismiss");
            let r = self.icon_button(ui, &label, false, paint_cross).on_hover_text(self.vis(&label));
            self.tag(|| "shortcuts.message.dismiss".into(), r.rect, &label);
            if r.clicked() {
                act = 3;
            }
        });
        match (act, &m.goto) {
            (1, Some(c)) => self.state.go_to(c),
            (2, _) => self.state.undo(self.env),
            (3, _) => self.state.message = None,
            _ => {}
        }
    }

    /// Clashes, shadows and problems in the keymap as a whole.
    fn info(&mut self, ui: &mut Ui) {
        let built = self.state.lines(self.env);
        let (clashes, shadows) = (built.clashes, built.shadows);
        let problems = self.env.keymap.problems();
        if clashes == 0 && shadows == 0 && problems.is_empty() {
            return;
        }
        let mut conflicts = false;
        ui.horizontal_wrapped(|ui| {
            if clashes > 0 {
                let n = clashes as u64;
                let text = self.env.text.plural(n, "{n} shortcut is assigned more than once", "{n} shortcuts are assigned more than once");
                conflicts = ui.link(self.warned(self.vis(&text))).clicked();
            }
            if shadows > 0 {
                let n = shadows as u64;
                let text = self.env.text.plural(n, "{n} shortcut also works in another scope", "{n} shortcuts also work in another scope");
                ui.label(RichText::new(self.vis(&text)).color(self.style.dim));
            }
        });
        if conflicts {
            self.state.set_filter(Filter::Conflicts);
        }
        if !problems.is_empty() {
            let n = problems.len() as u64;
            let title = self.env.text.plural(n, "{n} saved shortcut could not be used", "{n} saved shortcuts could not be used");
            egui::CollapsingHeader::new(self.vis(&title)).id_salt(self.base.with("problems")).show(ui, |ui| {
                ScrollArea::vertical().max_height(self.row_h * 4.0).show(ui, |ui| {
                    for p in &problems {
                        ui.add(Label::new(self.vis(&self.problem(p))).wrap());
                    }
                });
            });
        }
    }

    fn problem(&self, p: &Problem) -> String {
        match p {
            Problem::UnknownCommand { key } => fill(self.ui("{key}: no command has this name"), &[("key", key)]),
            Problem::BadBinding { key, text, error } => {
                let error = binding_error(self.env, error);
                fill(self.ui("{key}: “{text}” is not a shortcut ({error})"), &[("key", key), ("text", text), ("error", &error)])
            }
            Problem::Reserved { key, text } => fill(self.ui("{key}: {text} is reserved by the system"), &[("key", key), ("text", text)]),
            Problem::UnknownScope { key, text } => fill(self.ui("{key}: the scope in “{text}” is unknown"), &[("key", key), ("text", text)]),
            Problem::DuplicateKey { key } => fill(self.ui("{key}: listed more than once"), &[("key", key)]),
            Problem::Dropped { key, .. } => fill(self.ui("{key}: part of the entry was dropped"), &[("key", key)]),
            Problem::TooManyCommands { ignored } => {
                self.env.text.plural(*ignored as u64, "{n} entry was ignored: too many commands", "{n} entries were ignored: too many commands")
            }
        }
    }

    fn footer(&mut self, ui: &mut Ui) {
        let editable = self.env.options.editable;
        let right: Vec<(String, &str)> = if editable {
            vec![(self.ui("Cancel"), "shortcuts.cancel"), (self.ui("OK"), "shortcuts.ok")]
        } else {
            vec![(self.ui("Close"), "shortcuts.close")]
        };
        let idle = !self.state.modal();
        ui.horizontal(|ui| {
            if editable {
                if self.button(ui, idle && self.state.can_undo(), &self.ui("Undo"), "shortcuts.undo") {
                    self.state.undo(self.env);
                }
                if self.button(ui, idle && self.state.can_redo(), &self.ui("Redo"), "shortcuts.redo") {
                    self.state.redo(self.env);
                }
                if self.button(ui, idle && !self.env.keymap.overrides().is_empty(), &self.ui("Reset All…"), "shortcuts.resetAll") {
                    self.state.ask_reset_all();
                }
            }
            // With sets, Summarize… is in the set row, which a read-only dialog does not show.
            if (self.env.sets.is_none() || !editable) && self.env.options.summary {
                let label = self.ui("Summarize…");
                if self.button(ui, true, &label, "shortcuts.summarize") {
                    self.state.summarize(self.env);
                }
            }
            let gap = ui.spacing().item_spacing.x;
            let width: f32 = right.iter().map(|(t, _)| self.button_width(ui, t) + gap).sum();
            ui.add_space((ui.available_width() - width).max(0.0));
            for (text, id) in &right {
                // OK waits for the open prompt or review; Cancel and Close never do.
                if self.button(ui, idle || *id != "shortcuts.ok", text, id) {
                    if editable && *id == "shortcuts.ok" {
                        self.state.ok(self.env);
                    } else {
                        self.state.cancel(self.env);
                    }
                }
            }
        });
    }
}

/// Separator between menu path parts, mirrored by [`Draw::ordered`].
const PATH_SEP: &str = " › ";

/// Places rectangles laid out left to right inside a span, mirrored in a right-to-left interface.
#[derive(Clone, Copy)]
struct Mirror {
    on: bool,
    /// The span's left plus right edge.
    sum: f32,
}

impl Mirror {
    fn rect(self, r: Rect) -> Rect {
        if self.on { Rect::from_x_y_ranges((self.sum - r.max.x)..=(self.sum - r.min.x), r.y_range()) } else { r }
    }

    /// The left edge of something `width` wide whose left-to-right left edge is `x`.
    fn x(self, x: f32, width: f32) -> f32 {
        if self.on { self.sum - x - width } else { x }
    }
}

// ---- Icons, drawn so they do not depend on the app's fonts ----

fn square(r: Rect, f: f32) -> Rect {
    Rect::from_center_size(r.center(), Vec2::splat(r.width().min(r.height()) * f))
}

fn paint_plus(p: &Painter, r: Rect, c: Color32) {
    let r = square(r, 0.5);
    let s = Stroke::new(1.5, c);
    p.line_segment([r.center_top(), r.center_bottom()], s);
    p.line_segment([r.left_center(), r.right_center()], s);
}

fn paint_cross(p: &Painter, r: Rect, c: Color32) {
    let r = square(r, 0.45);
    let s = Stroke::new(1.5, c);
    p.line_segment([r.left_top(), r.right_bottom()], s);
    p.line_segment([r.right_top(), r.left_bottom()], s);
}

fn paint_reset(p: &Painter, r: Rect, c: Color32) {
    let r = square(r, 0.55);
    let (center, radius) = (r.center(), r.width() / 2.0);
    let s = Stroke::new(1.5, c);
    let at = |a: f32| center + radius * Vec2::angled(a);
    let start = -std::f32::consts::FRAC_PI_2;
    let points: Vec<_> = (0..=12).map(|i| at(start + i as f32 / 12.0 * 1.6 * std::f32::consts::PI)).collect();
    p.add(egui::Shape::line(points, s));
    let tip = at(start);
    p.line_segment([tip, tip + vec2(radius * 0.6, -radius * 0.35)], s);
    p.line_segment([tip, tip + vec2(radius * 0.5, radius * 0.45)], s);
}

fn paint_keyboard(p: &Painter, r: Rect, c: Color32) {
    let r = Rect::from_center_size(r.center(), vec2(r.width() * 0.7, r.height() * 0.5));
    p.rect_stroke(r, 2.0, Stroke::new(1.2, c), StrokeKind::Inside);
    for row in 0..2 {
        for col in 0..4 {
            let x = r.min.x + r.width() * (col as f32 + 0.75) / 4.5;
            let y = r.min.y + r.height() * (row as f32 + 1.0) / 4.0;
            p.circle_filled(pos2(x, y), 0.9, c);
        }
    }
    p.line_segment(
        [pos2(r.min.x + r.width() * 0.3, r.max.y - r.height() * 0.22), pos2(r.max.x - r.width() * 0.3, r.max.y - r.height() * 0.22)],
        Stroke::new(1.2, c),
    );
}
