//! Controls that choose one entry from a list or a colour grid and show what is chosen.
//!
//! A row or swatch under the pointer is noted for the pass ([`note_hover`]) so an app can
//! preview it; the choice in effect is marked ([`set_current_pick`]). [`pick_rows`] draws a
//! list that marks the current entry, with an optional recent section. [`FrozenRecent`] keeps
//! that section's order while a menu stays open.

use std::ops::Range;

use egui::{vec2, Color32, Context, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui};

fn hover_key() -> Id {
    Id::unique("craft_ui_pick_hover")
}

fn current_key() -> Id {
    Id::unique("craft_ui_pick_current")
}

/// Notes the entry under the pointer. One per pass: the last drawn wins.
pub fn note_hover(ctx: &Context, value: String) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(hover_key(), (pass, value)));
}

/// The entry noted in this pass, read once. Take before drawing a list to drop what
/// another widget noted.
pub fn take_hover(ctx: &Context) -> Option<String> {
    let pass = ctx.cumulative_pass_nr();
    let seen = ctx.data_mut(|d| d.remove_temp::<(u64, String)>(hover_key()));
    seen.filter(|(p, _)| *p == pass).map(|(_, v)| v)
}

/// Sets the value the next swatches and rows mark as in effect. `None` marks nothing.
/// Clear it after drawing the menu.
pub fn set_current_pick(ctx: &Context, current: Option<String>) {
    ctx.data_mut(|d| match current {
        Some(c) => {
            d.insert_temp(current_key(), c);
        }
        None => d.remove::<String>(current_key()),
    });
}

pub fn current_pick(ctx: &Context) -> Option<String> {
    ctx.data(|d| d.get_temp::<String>(current_key()))
}

/// The colours a [`swatch`] paints its outline and current-choice ring with.
#[derive(Clone, Copy, Debug)]
pub struct SwatchStyle {
    pub accent: Color32,
    pub border: Color32,
}

/// The side of a [`swatch`], in points.
pub const SWATCH_SIZE: f32 = 15.0;

/// Uppercase `RRGGBB`: the value a swatch notes and compares to the current pick.
pub fn hex(c: Color32) -> String {
    format!("{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

/// A colour swatch. An accent ring marks it when the current pick is its colour.
/// Returns true when clicked.
pub fn swatch(ui: &mut Ui, color: Color32, style: SwatchStyle) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(SWATCH_SIZE, SWATCH_SIZE), Sense::click());
    let value = hex(color);
    ui.painter().rect_filled(r, 0.0, color);
    let (width, line) = if resp.hovered() { (1.5, style.accent) } else { (0.5, style.border) };
    ui.painter().rect_stroke(r, 0.0, Stroke::new(width, line), StrokeKind::Inside);
    if resp.hovered() {
        note_hover(ui.ctx(), value.clone());
    }
    if current_pick(ui.ctx()).is_some_and(|cur| cur.eq_ignore_ascii_case(&value)) {
        ui.painter().rect_stroke(r.expand(1.0), 0.0, Stroke::new(2.0, style.accent), StrokeKind::Outside);
    }
    resp.on_hover_text(format!("#{value}")).clicked()
}

/// A row of a list: a dim section heading nothing picks, or an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row<'a> {
    Heading(&'a str),
    Item(&'a String),
}

/// Headings of a list with a recent section; the app translates them.
#[derive(Clone, Copy, Debug)]
pub struct Sections<'a> {
    pub recent: &'a str,
    pub all: &'a str,
}

/// Rows of `n` entries under `recent` recent ones: the two headings and the recent
/// entries come on top when there are any.
pub fn section_rows(recent: usize, n: usize) -> usize {
    if recent == 0 {
        n
    } else {
        n.saturating_add(recent).saturating_add(2)
    }
}

/// Row `k`: the recent heading, the recent entries, the all heading, then `item(0..)`.
/// Without recent entries the rows are the items.
pub fn section_row<'a>(k: usize, recent: &'a [String], sections: Sections<'a>, item: &dyn Fn(usize) -> Option<&'a String>) -> Option<Row<'a>> {
    if recent.is_empty() {
        return item(k).map(Row::Item);
    }
    if k == 0 {
        return Some(Row::Heading(sections.recent));
    }
    if let Some(r) = recent.get(k.saturating_sub(1)) {
        return Some(Row::Item(r));
    }
    if k == recent.len().saturating_add(1) {
        return Some(Row::Heading(sections.all));
    }
    item(k.saturating_sub(recent.len()).saturating_sub(2)).map(Row::Item)
}

/// The row of entry `entry` in a list with `recent` recent entries.
pub fn highlight_row(entry: usize, recent: usize) -> usize {
    if recent == 0 {
        entry
    } else if entry < recent {
        entry.saturating_add(1)
    } else {
        entry.saturating_add(2)
    }
}

/// How [`pick_rows`] marks rows.
#[derive(Clone, Copy, Debug)]
pub struct RowStyle {
    /// Fill of the current and highlighted rows.
    pub fill: Color32,
    /// The bar at the left of the current row of a preview list.
    pub accent: Color32,
}

/// Draws an entry in its own look.
pub type PreviewFn = dyn Fn(&mut Ui, &str) -> Response;

/// Fetches the row at an index.
pub type RowFn<'a> = dyn Fn(usize) -> Option<Row<'a>> + 'a;

/// What [`pick_rows`] draws.
pub struct PickRows<'a> {
    pub width: f32,
    pub current: &'a str,
    pub range: Range<usize>,
    pub row: &'a RowFn<'a>,
    /// Row marked instead of `current`, for keyboard walking.
    pub highlight: Option<usize>,
    /// Draws an entry with its own look (a font name in its font); `None` draws text.
    pub preview: Option<&'a PreviewFn>,
    pub row_height: f32,
    /// How many recent entries head the list; 0 without the section.
    pub recent_rows: usize,
    pub style: RowStyle,
}

fn heading_row(ui: &mut Ui, text: &str, size: egui::Vec2, rule: bool) {
    let weak = ui.visuals().weak_text_color();
    let line = ui.visuals().widgets.noninteractive.bg_stroke;
    if rule {
        ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), line);
    }
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(size);
        ui.add_space(6.0);
        ui.add(egui::Label::new(egui::RichText::new(text).small().color(weak)).selectable(false));
    });
}

fn mark_row(ui: &Ui, row_height: f32, fill: Color32, bar: Option<Color32>) {
    let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), row_height));
    ui.painter().rect_filled(r, 3.0, fill);
    if let Some(accent) = bar {
        ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(3.0, row_height)), 1.5, accent);
    }
}

/// Draws the rows in `range`. The entry under the pointer is noted ([`note_hover`]). A
/// current entry in the recent section is marked there only: marked again below it would
/// look as if two were chosen. Returns the clicked entry.
pub fn pick_rows(ui: &mut Ui, options: PickRows<'_>) -> Option<String> {
    let PickRows { width, current, range, row, highlight, preview, row_height, recent_rows, style } = options;
    let mut out = None;
    ui.set_min_width(width + 60.0);
    let current_is_recent = (1..=recent_rows).any(|k| matches!(row(k), Some(Row::Item(r)) if r == current));
    for k in range {
        let it = match row(k) {
            Some(Row::Item(it)) => it,
            Some(Row::Heading(h)) => {
                heading_row(ui, h, vec2(width + 60.0, row_height), k > 0);
                continue;
            }
            None => continue,
        };
        let lit = highlight == Some(k);
        let in_all = recent_rows > 0 && k > recent_rows.saturating_add(1);
        let is_current = it == current && !(in_all && current_is_recent);
        let resp = match preview {
            Some(p) => {
                if lit {
                    mark_row(ui, row_height, style.fill, None);
                }
                if is_current {
                    mark_row(ui, row_height, style.fill, Some(style.accent));
                }
                p(ui, it)
            }
            None => ui.selectable_label(if highlight.is_some() { lit } else { is_current }, it.as_str()),
        };
        if resp.hovered() {
            note_hover(ui.ctx(), it.clone());
        }
        if resp.clicked() {
            out = Some(it.clone());
        }
    }
    out
}

/// The recent section's order, held while a menu stays open: a pick moves the entry to the
/// top for the next time the menu opens, not under the pointer.
#[derive(Clone, Copy, Debug)]
pub struct FrozenRecent {
    id: Id,
}

impl FrozenRecent {
    pub fn new(id: Id) -> Self {
        Self { id }
    }

    /// The order to draw: the frozen one after a pick, else `recent`.
    pub fn order(self, ctx: &Context, recent: &[String]) -> Vec<String> {
        ctx.data(|d| d.get_temp(self.id)).unwrap_or_else(|| recent.to_vec())
    }

    /// Freezes `drawn` after a pick and thaws it otherwise. Call once per frame, after the menu.
    pub fn settle(self, ctx: &Context, picked: bool, drawn: &[String]) {
        ctx.data_mut(|d| {
            if picked {
                d.insert_temp(self.id, drawn.to_vec());
            } else {
                d.remove::<Vec<String>>(self.id);
            }
        });
    }
}

/// [`pick_rows`] for the list of a menu: a pick closes the menu.
pub fn pick_rows_in_menu(ui: &mut Ui, options: PickRows<'_>) -> Option<String> {
    let picked = pick_rows(ui, options);
    if picked.is_some() {
        ui.close();
    }
    picked
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
