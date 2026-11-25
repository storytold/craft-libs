//! The font name and size controls every Craft app with text shows the same way: two rounded
//! fields with the chevron inside, both typeable, each opening a list; then grow/shrink font and,
//! optionally, clear formatting.
//!
//! The application owns the font list, the size presets, previews and what a change does; the
//! control only edits the two values it is given and reports what happened.

use egui::{Align, Align2, Color32, CornerRadius, FontId, Id, Key, Rect, Response, Sense, Stroke, StrokeKind, TextEdit, Ui, pos2, vec2};

/// Colours and metrics; [`FontBoxStyle::from_visuals`] derives them from egui's visuals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontBoxStyle {
    pub fill: Color32,
    pub border: Color32,
    pub border_hover: Color32,
    pub focus: Color32,
    pub text: Color32,
    pub chevron: Color32,
    pub hover: Color32,
    /// The small marks of grow/shrink and clear formatting. Icons look the same in every app, so
    /// this is not the app's accent: by default it is the text colour.
    pub mark: Color32,
    pub radius: u8,
    pub height: f32,
    pub font: f32,
}

impl FontBoxStyle {
    pub fn from_visuals(v: &egui::Visuals) -> Self {
        Self {
            fill: v.extreme_bg_color,
            border: v.widgets.noninteractive.bg_stroke.color,
            border_hover: v.widgets.hovered.bg_stroke.color,
            focus: v.selection.bg_fill,
            text: v.text_color(),
            chevron: v.weak_text_color(),
            hover: v.widgets.hovered.weak_bg_fill,
            mark: v.text_color(),
            radius: 4,
            height: 24.0,
            font: 13.0,
        }
    }
}

/// Accessible names and tooltips, in the interface language.
#[derive(Clone, Copy, Debug)]
pub struct FontBoxLabels<'a> {
    pub font: &'a str,
    pub size: &'a str,
    pub grow: &'a str,
    pub shrink: &'a str,
    pub clear: &'a str,
}

/// What happened this frame.
#[derive(Debug, Default)]
pub struct FontBoxResponse {
    pub family_changed: bool,
    /// The size changed from the field, the list or grow/shrink.
    pub size_changed: bool,
    pub clear_clicked: bool,
}

/// Word's size list; [`FontBox::sizes`] replaces it.
pub const DEFAULT_SIZES: &[f32] = &[8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0];

/// Smallest and largest size the field accepts (Word's limits).
pub const MIN_SIZE: f32 = 1.0;
pub const MAX_SIZE: f32 = 1638.0;

type Preview<'a> = &'a dyn Fn(&str) -> Option<FontId>;

/// See the module documentation.
pub struct FontBox<'a> {
    family: &'a mut String,
    size: &'a mut Option<f32>,
    families: &'a [String],
    sizes: &'a [f32],
    style: &'a FontBoxStyle,
    labels: FontBoxLabels<'a>,
    family_width: f32,
    size_width: f32,
    grow_shrink: bool,
    clear: bool,
    preview: Option<Preview<'a>>,
    enabled: bool,
    hint: Option<(&'a str, f32)>,
}

impl<'a> FontBox<'a> {
    /// `family` empty or `size` `None` shows that field blank: a selection with mixed values.
    pub fn new(
        family: &'a mut String,
        size: &'a mut Option<f32>,
        families: &'a [String],
        style: &'a FontBoxStyle,
        labels: FontBoxLabels<'a>,
    ) -> Self {
        Self {
            family,
            size,
            families,
            sizes: DEFAULT_SIZES,
            style,
            labels,
            family_width: 150.0,
            size_width: 56.0,
            grow_shrink: true,
            clear: false,
            preview: None,
            enabled: true,
            hint: None,
        }
    }

    pub fn sizes(mut self, sizes: &'a [f32]) -> Self {
        self.sizes = sizes;
        self
    }

    pub fn widths(mut self, family: f32, size: f32) -> Self {
        self.family_width = family;
        self.size_width = size;
        self
    }

    /// Show the grow/shrink buttons (on by default).
    pub fn grow_shrink(mut self, on: bool) -> Self {
        self.grow_shrink = on;
        self
    }

    /// Show the clear formatting button.
    pub fn clear(mut self, on: bool) -> Self {
        self.clear = on;
        self
    }

    /// Draw each family in the list in its own face, when the app has it loaded.
    pub fn preview(mut self, preview: Preview<'a>) -> Self {
        self.preview = Some(preview);
        self
    }

    /// Off when there is no text to format (nothing selected on a slide, a picture selected): the
    /// control stays in place, lightly dimmed and inert, instead of turning into empty boxes.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// What a disabled control shows, dimmed: usually the theme's body font and size. Without a
    /// hint a disabled control shows its values as they are.
    pub fn hint(mut self, family: &'a str, size: f32) -> Self {
        self.hint = Some((family, size));
        self
    }

    pub fn show(self, ui: &mut Ui) -> FontBoxResponse {
        let mut out = FontBoxResponse::default();
        let id = ui.scope_id().with("craft_font_box").with(ui.next_auto_id());
        let enabled = self.enabled;
        let hint = if enabled { None } else { self.hint };
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let style = *self.style;
                let shown = match hint {
                    Some((family, _)) if self.family.is_empty() => family.to_string(),
                    _ => self.family.clone(),
                };
                // The lists are only built while a popup is open, not every frame.
                let families = self.families;
                let family_list = || families.to_vec();
                if let Some(v) =
                    field(ui, id.with("family"), &style, self.family_width, &shown, self.labels.font, &family_list, Some(&shown), self.preview)
                    && !v.trim().is_empty()
                    && v.trim() != self.family.as_str()
                {
                    *self.family = v.trim().to_string();
                    out.family_changed = true;
                }
                let shown = self.size.or(hint.map(|(_, size)| size)).map(format_size).unwrap_or_default();
                let sizes = self.sizes;
                let size_list = || sizes.iter().map(|s| format_size(*s)).collect();
                if let Some(v) = field(ui, id.with("size"), &style, self.size_width, &shown, self.labels.size, &size_list, Some(&shown), None)
                    && let Some(s) = parse_size(&v)
                    && Some(s) != *self.size
                {
                    *self.size = Some(s);
                    out.size_changed = true;
                }
                if self.grow_shrink {
                    if glyph_button(ui, &style, Glyph::Grow, self.labels.grow).clicked() {
                        *self.size = Some(step(*self.size, self.sizes, true));
                        out.size_changed = true;
                    }
                    if glyph_button(ui, &style, Glyph::Shrink, self.labels.shrink).clicked() {
                        *self.size = Some(step(*self.size, self.sizes, false));
                        out.size_changed = true;
                    }
                }
                if self.clear && glyph_button(ui, &style, Glyph::Clear, self.labels.clear).clicked() {
                    out.clear_clicked = true;
                }
            })
        });
        out
    }
}

/// `11`, `10.5`; never `11.0`.
pub fn format_size(size: f32) -> String {
    if (size - size.round()).abs() < 0.01 { format!("{}", size.round() as i32) } else { format!("{:.1}", size) }
}

/// A typed size: a number (comma or dot), rounded to the nearest half point and clamped to Word's
/// limits. Anything else is `None` and the field keeps its old value.
pub fn parse_size(text: &str) -> Option<f32> {
    let t = text.trim().trim_end_matches("pt").trim().replace(',', ".");
    let v: f32 = t.parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    Some(((v * 2.0).round() / 2.0).clamp(MIN_SIZE, MAX_SIZE))
}

/// Grow/shrink: the next preset up or down; past the ends of the list, one point at a time
/// (Word's behaviour). A mixed size starts from 11.
pub fn step(size: Option<f32>, sizes: &[f32], up: bool) -> f32 {
    let v = size.unwrap_or(11.0);
    let next = if up { sizes.iter().copied().find(|s| *s > v + 0.01) } else { sizes.iter().rev().copied().find(|s| *s < v - 0.01) };
    next.unwrap_or(if up { v.floor() + 1.0 } else { v.ceil() - 1.0 }).clamp(MIN_SIZE, MAX_SIZE)
}

const CHEVRON_W: f32 = 18.0;

/// One rounded field with the chevron inside. Returns the committed text (Enter, a list pick or
/// leaving the field after typing).
#[allow(clippy::too_many_arguments)]
fn field(
    ui: &mut Ui,
    id: Id,
    style: &FontBoxStyle,
    width: f32,
    shown: &str,
    label: &str,
    list: &dyn Fn() -> Vec<String>,
    current: Option<&str>,
    preview: Option<Preview<'_>>,
) -> Option<String> {
    let (rect, _) = ui.allocate_exact_size(vec2(width, style.height), Sense::hover());
    let open_id = id.with("open");
    let buf_id = id.with("buf");
    // What the list is narrowed to. It is kept apart from the field's focus, so a press on a row
    // (which takes focus from the field first) leaves the rows where they were until the release.
    let filter_id = id.with("filter");
    let popup_id = id.with("popup");
    let mut open = ui.is_enabled() && ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    let editing = ui.memory(|m| m.has_focus(id.with("edit")));
    let mut buf = if editing { ui.data(|d| d.get_temp::<String>(buf_id)).unwrap_or_else(|| shown.to_string()) } else { shown.to_string() };

    let chevron = Rect::from_min_max(pos2(rect.right() - CHEVRON_W, rect.top()), rect.max);
    let text_rect = Rect::from_min_max(pos2(rect.left() + 6.0, rect.top()), pos2(chevron.left(), rect.bottom()));
    // The fill goes under the text edit; the border is drawn over it below, once its state is known.
    ui.painter().rect_filled(rect, CornerRadius::same(style.radius), style.fill);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_rect).layout(egui::Layout::left_to_right(Align::Center)));
    let edit = child.add(
        TextEdit::singleline(&mut buf)
            .id(id.with("edit"))
            .frame(egui::Frame::NONE)
            .margin(vec2(0.0, 0.0))
            .align(egui::Align2::LEFT_CENTER)
            .font(FontId::proportional(style.font))
            .text_color(style.text)
            .desired_width(text_rect.width()),
    );
    let edit = edit.on_hover_text(label);
    let chev = ui.interact(chevron, id.with("chevron"), Sense::click());
    let hovered = edit.hovered() || chev.hovered();

    let painter = ui.painter();
    let stroke = if edit.has_focus() || open {
        Stroke::new(1.5, style.focus)
    } else if hovered {
        Stroke::new(1.0, style.border_hover)
    } else {
        Stroke::new(1.0, style.border)
    };
    if chev.hovered() {
        painter.rect_filled(chevron.shrink(2.0), CornerRadius::same(style.radius.saturating_sub(1)), style.hover);
    }
    painter.rect_stroke(rect, CornerRadius::same(style.radius), stroke, StrokeKind::Inside);
    let c = chevron.center();
    painter.line(vec![pos2(c.x - 4.0, c.y - 2.0), pos2(c.x, c.y + 2.0), pos2(c.x + 4.0, c.y - 2.0)], Stroke::new(1.2, style.chevron));
    chev.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, ui.is_enabled(), label));

    let mut committed = None;
    if edit.has_focus() {
        ui.data_mut(|d| d.insert_temp(buf_id, buf.clone()));
        if edit.changed() {
            open = true;
            ui.data_mut(|d| d.insert_temp(filter_id, buf.clone()));
        }
    }
    // A click inside last frame's list takes focus from the field (egui does it on the release)
    // before the row sees the click: that is a pick in progress, not leaving the field.
    let popup_rect = if open { ui.ctx().memory(|m| m.area_rect(popup_id)) } else { None };
    let (pointer_event, at) = ui.input(|i| (i.pointer.any_pressed() || i.pointer.any_released(), i.pointer.interact_pos()));
    let press_in_popup = pointer_event && at.zip(popup_rect).is_some_and(|(p, r)| r.contains(p));
    if edit.lost_focus() && !press_in_popup {
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            open = false;
        } else if buf != shown {
            committed = Some(buf.clone());
            open = false;
        }
        ui.data_mut(|d| d.remove::<String>(buf_id));
    }
    if chev.clicked() {
        open = !open;
        ui.data_mut(|d| d.remove::<String>(filter_id));
    }

    if open {
        // While typing, the list narrows to what matches; opened from the chevron it shows all.
        let filter = ui.data(|d| d.get_temp::<String>(filter_id)).filter(|f| f != shown).unwrap_or_default().to_lowercase();
        let list = list();
        let area = egui::Area::new(popup_id).order(egui::Order::Foreground).fixed_pos(rect.left_bottom() + vec2(0.0, 2.0)).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(rect.width().max(120.0));
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for value in list.iter().filter(|t| filter.is_empty() || t.to_lowercase().contains(&filter)) {
                        let font = preview.and_then(|p| p(value)).unwrap_or(FontId::proportional(style.font));
                        let rt = egui::RichText::new(value).font(font);
                        if ui.add(egui::Button::selectable(current == Some(value.as_str()), rt).min_size(vec2(ui.available_width(), 0.0))).clicked() {
                            committed = Some(value.clone());
                        }
                    }
                });
            });
        });
        let clicked_outside = ui.input(|i| i.pointer.any_pressed())
            && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p) && !rect.contains(p));
        if committed.is_some() || clicked_outside || ui.input(|i| i.key_pressed(Key::Escape)) {
            open = false;
            if committed.is_some() {
                ui.memory_mut(|m| m.surrender_focus(id.with("edit")));
            }
        }
    }
    if !open {
        ui.data_mut(|d| d.remove::<String>(filter_id));
    }
    ui.data_mut(|d| d.insert_temp(open_id, open));
    committed
}

#[derive(Clone, Copy)]
enum Glyph {
    Grow,
    Shrink,
    Clear,
}

/// A 24×24 button with an `A` and a small mark: ▲ grow, ▼ shrink, an eraser for clear.
fn glyph_button(ui: &mut Ui, style: &FontBoxStyle, glyph: Glyph, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(style.height, style.height), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(style.radius), style.hover);
    }
    let (size, mark) = match glyph {
        Glyph::Grow => (15.0, 1.0),
        Glyph::Shrink => (12.0, -1.0),
        Glyph::Clear => (14.0, 0.0),
    };
    let a = rect.center() + vec2(-3.0, 1.0);
    painter.text(a, Align2::CENTER_CENTER, "A", FontId::proportional(size), style.text);
    let tip = pos2(rect.center().x + 6.0, rect.center().y - 4.0);
    match glyph {
        Glyph::Grow | Glyph::Shrink => {
            let h = 3.0 * mark;
            painter.add(egui::Shape::convex_polygon(
                vec![pos2(tip.x - 3.0, tip.y + h * 0.5), pos2(tip.x + 3.0, tip.y + h * 0.5), pos2(tip.x, tip.y - h * 0.5)],
                style.mark,
                Stroke::NONE,
            ));
        }
        Glyph::Clear => {
            let r = Rect::from_center_size(pos2(rect.center().x + 5.0, rect.center().y + 5.0), vec2(8.0, 5.0));
            painter.rect_filled(r, CornerRadius::same(1), style.mark);
        }
    }
    let (label, enabled) = (label.to_string(), ui.is_enabled());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &label));
    response.on_hover_text(label)
}

#[cfg(test)]
mod tests;
