//! The ribbon body of the Office-style Craft apps: one frame for the commands of the current tab,
//! split into groups with an optional caption and dialog launcher under each.
//!
//! The application owns the tabs, the commands, their icons and what each does. This module only
//! fixes the frame, the group geometry, captions, separators and toggle buttons, so every app's
//! ribbon has the same rhythm. [`fit`] holds the bands and the fitting rules: a tab whose groups no
//! longer fit folds them to one row instead of drawing past the card's edge.

use egui::{Align, Align2, Color32, CornerRadius, FontId, InnerResponse, Layout, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

pub mod fit;

use fit::{Bands, Level};

/// How the ribbon body sits in the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RibbonFrame {
    /// A rounded card inset from the window edges by `margin` on the left, right and bottom
    /// (Microsoft 365 on Windows, ONLYOFFICE).
    Card { margin: f32 },
    /// Edge to edge with a hairline under it (Microsoft 365 on macOS).
    Flush,
}

impl RibbonFrame {
    /// What Microsoft 365 does on each system: flat on macOS, a card with a 6 pt margin elsewhere
    /// (Windows, Linux, the web).
    pub fn platform_default() -> Self {
        if cfg!(target_os = "macos") { Self::Flush } else { Self::Card { margin: 6.0 } }
    }
}

/// Colours and metrics; [`RibbonStyle::from_visuals`] derives them from egui's visuals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RibbonStyle {
    /// Behind a card (the window chrome).
    pub window: Color32,
    pub body: Color32,
    pub border: Color32,
    pub caption: Color32,
    pub separator: Color32,
    pub hover: Color32,
    pub selected: Color32,
    pub selected_stroke: Color32,
    pub icon: Color32,
    pub radius: u8,
    /// Height of a group's command area (two rows of small buttons or one large button).
    pub content_height: f32,
    /// Height of the group-label band under the commands; Office keeps 18 pt.
    pub label_band: f32,
    pub caption_font: f32,
}

impl RibbonStyle {
    pub fn from_visuals(v: &egui::Visuals) -> Self {
        Self {
            window: v.panel_fill,
            body: v.extreme_bg_color,
            border: v.widgets.noninteractive.bg_stroke.color,
            caption: v.weak_text_color(),
            separator: v.widgets.noninteractive.bg_stroke.color,
            hover: v.widgets.hovered.weak_bg_fill,
            selected: v.selection.bg_fill.gamma_multiply(0.18),
            selected_stroke: v.selection.bg_fill,
            icon: v.text_color(),
            radius: 8,
            content_height: 56.0,
            label_band: fit::LABEL_BAND,
            caption_font: 11.0,
        }
    }
}

/// Space inside the body around the groups.
const PAD_X: f32 = 6.0;
const PAD_Y: f32 = 4.0;
const GROUP_PAD: f32 = 6.0;
/// Room the dialog launcher takes at the right of a caption.
const LAUNCHER_W: f32 = 18.0;
/// Space inside a folded group's one-row button, and the narrowest such button.
const COMPACT_PAD: f32 = 20.0;
const COMPACT_MIN: f32 = 52.0;
/// Room between a folded group's name and its chevron.
const CHEVRON: f32 = 10.0;

/// The ribbon body; see the module documentation.
pub struct Ribbon<'a> {
    style: &'a RibbonStyle,
    frame: RibbonFrame,
    captions: bool,
    /// The width each group measured on an earlier frame, in drawing order.
    groups: Option<&'a [fit::Group]>,
    collapse: fit::Collapse,
}

impl<'a> Ribbon<'a> {
    pub fn new(style: &'a RibbonStyle) -> Self {
        Self { style, frame: RibbonFrame::platform_default(), captions: captions_default(), groups: None, collapse: fit::Collapse::FirstMisfit }
    }

    pub fn frame(mut self, frame: RibbonFrame) -> Self {
        self.frame = frame;
        self
    }

    /// Group captions under the commands (Office's "Show group labels"); see [`captions_default`].
    pub fn captions(mut self, on: bool) -> Self {
        self.captions = on;
        self
    }

    /// The width each group measured on an earlier frame, in drawing order, so the row folds the
    /// groups that no longer fit instead of drawing them past the card's edge. Each
    /// [`GroupResponse`] reports the width it measured, for the next frame's plan. Omit it to draw
    /// every group as designed.
    pub fn groups(mut self, groups: &'a [fit::Group]) -> Self {
        self.groups = Some(groups);
        self
    }

    /// Which groups fold first when the row is too narrow; [`fit::Collapse::FirstMisfit`] by
    /// default. See [`fit::plan`].
    pub fn collapse(mut self, order: fit::Collapse) -> Self {
        self.collapse = order;
        self
    }

    /// The card's bands: the command band over the group-label band, with no tab strip, which the
    /// application draws above it. With captions off the card has no label band.
    pub fn bands(&self) -> Bands {
        Bands { tabs: 0.0, commands: self.style.content_height, labels: if self.captions { self.style.label_band } else { 0.0 } }
    }

    /// The height the body takes, frame included.
    pub fn height(&self) -> f32 {
        let body = self.bands().card() + PAD_Y * 2.0;
        match self.frame {
            RibbonFrame::Card { margin } => body + margin,
            RibbonFrame::Flush => body + 1.0,
        }
    }

    /// Draws the frame across the available width and lays `add`'s groups out left to right.
    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        let width = ui.available_width();
        let (outer, response) = ui.allocate_exact_size(vec2(width, self.height()), Sense::hover());
        let s = self.style;
        let body = match self.frame {
            RibbonFrame::Card { margin } => {
                ui.painter().rect_filled(outer, CornerRadius::ZERO, s.window);
                let r = Rect::from_min_max(pos2(outer.left() + margin, outer.top()), pos2(outer.right() - margin, outer.bottom() - margin));
                ui.painter().rect(r, CornerRadius::same(s.radius), s.body, Stroke::new(1.0, s.border), StrokeKind::Inside);
                r
            }
            RibbonFrame::Flush => {
                ui.painter().rect_filled(outer, CornerRadius::ZERO, s.body);
                ui.painter().hline(outer.x_range(), outer.bottom() - 0.5, Stroke::new(1.0, s.border));
                Rect::from_min_max(outer.min, pos2(outer.right(), outer.bottom() - 1.0))
            }
        };
        let inner_rect = body.shrink2(vec2(PAD_X, PAD_Y));
        let captions = self.captions;
        // The row folds the groups that no longer fit. Groups are planned in drawing order, so a
        // group reads its level by its position on the row.
        let levels = self.groups.map(|groups| fit::plan(groups, inner_rect.width(), 0.0, self.collapse).levels).unwrap_or_default();
        let inner = ui
            .scope_builder(egui::UiBuilder::new().max_rect(inner_rect).layout(Layout::left_to_right(Align::Min)), |ui| {
                // Within the parent's clip, never past it: a ribbon partly inside a clipped panel
                // paints and takes input only where both allow.
                ui.shrink_clip_rect(body);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                ui.data_mut(|d| {
                    d.insert_temp(captions_id(), captions);
                    d.insert_temp(levels_id(), levels);
                    d.insert_temp(group_index_id(), 0usize);
                });
                add(ui)
            })
            .inner;
        InnerResponse::new(inner, response)
    }
}

/// Whether group captions show unless the app says otherwise: what Microsoft 365 does on each
/// system (on for Windows, Linux and the web, off on macOS).
pub fn captions_default() -> bool {
    !cfg!(target_os = "macos")
}

fn captions_id() -> egui::Id {
    egui::Id::unique("craft_ribbon_captions")
}

/// The levels this frame's plan chose, in drawing order.
fn levels_id() -> egui::Id {
    egui::Id::unique("craft_ribbon_levels")
}

/// How many groups of this frame's row have been drawn: the next group's position.
fn group_index_id() -> egui::Id {
    egui::Id::unique("craft_ribbon_group_index")
}

/// What a group reports.
pub struct GroupResponse<R> {
    /// The group's content, when it drew: a folded group draws its content only in its drop-down.
    pub inner: Option<R>,
    /// The dialog launcher (↘ next to the caption) was clicked.
    pub launcher_clicked: bool,
    /// The level this frame's plan gave the group.
    pub level: Level,
    /// The width the group took, for the next frame's plan.
    pub width: f32,
}

/// One group: commands, then the caption, then a separator on its right.
pub struct RibbonGroup<'a> {
    caption: &'a str,
    style: &'a RibbonStyle,
    launcher: Option<&'a str>,
    separator: bool,
}

impl<'a> RibbonGroup<'a> {
    pub fn new(caption: &'a str, style: &'a RibbonStyle) -> Self {
        Self { caption, style, launcher: None, separator: true }
    }

    /// Adds the dialog launcher with this tooltip (for example "Font Settings").
    pub fn launcher(mut self, tooltip: &'a str) -> Self {
        self.launcher = Some(tooltip);
        self
    }

    /// Omit the separator (the last group).
    pub fn last(mut self) -> Self {
        self.separator = false;
        self
    }

    /// Draws the group at the level this frame's plan gave it, in drawing order.
    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> GroupResponse<R> {
        let index = ui.data_mut(|d| {
            let index = d.get_temp::<usize>(group_index_id()).unwrap_or(0);
            d.insert_temp(group_index_id(), index + 1);
            index
        });
        let level = ui.data(|d| d.get_temp::<Vec<Level>>(levels_id())).and_then(|levels| levels.get(index).copied()).unwrap_or_default();
        if level == Level::Compact { self.show_folded(ui, add) } else { self.show_rows(ui, add, level) }
    }

    /// The group in its rows, under its caption.
    fn show_rows<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R, level: Level) -> GroupResponse<R> {
        let s = self.style;
        let captions = ui.data(|d| d.get_temp::<bool>(captions_id())).unwrap_or(true);
        let bands = Bands { tabs: 0.0, commands: s.content_height, labels: if captions { s.label_band } else { 0.0 } };
        let top = ui.cursor().min;
        let content = Rect::from_min_size(pos2(top.x + GROUP_PAD, top.y), vec2((ui.available_width() - GROUP_PAD).max(0.0), s.content_height));
        let mut used = content;
        let inner = ui
            .scope_builder(egui::UiBuilder::new().max_rect(content).layout(Layout::left_to_right(Align::Center)), |ui| {
                ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                let r = add(ui);
                used = ui.min_rect();
                r
            })
            .inner;
        let caption_w = if captions {
            ui.fonts_mut(|f| f.layout_no_wrap(self.caption.to_string(), FontId::proportional(s.caption_font), s.caption).size().x)
        } else {
            0.0
        };
        let launcher = if self.launcher.is_some() { LAUNCHER_W } else { 0.0 };
        let width = fit::group_width(used.width(), caption_w, launcher, GROUP_PAD);
        let rect = Rect::from_min_size(top, vec2(width, bands.card()));
        // The group's own id, so two groups with the same caption get distinct launchers.
        let group_id = ui.advance_cursor_after_rect(rect);
        let mut launcher_clicked = false;
        if captions {
            launcher_clicked = caption_row(ui, s, bands.label_band(rect), self.caption, self.launcher, group_id.with("launcher"));
        }
        if self.separator {
            let x = rect.right() - 0.5;
            ui.painter().vline(x, (rect.top() + 4.0)..=(rect.bottom() - 4.0), Stroke::new(1.0, s.separator));
        }
        GroupResponse { inner: Some(inner), launcher_clicked, level, width }
    }

    /// The group folded to one row: its name and a chevron in the command band, its name again in
    /// the label band, and the whole group in the drop-down the row opens.
    fn show_folded<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> GroupResponse<R> {
        let s = self.style;
        let captions = ui.data(|d| d.get_temp::<bool>(captions_id())).unwrap_or(true);
        let bands = Bands { tabs: 0.0, commands: s.content_height, labels: if captions { s.label_band } else { 0.0 } };
        let top = ui.cursor().min;
        let text_w = ui.fonts_mut(|f| f.layout_no_wrap(self.caption.to_string(), FontId::proportional(s.caption_font), s.icon).size().x);
        let width = fit::compact_width(text_w, 0.0, CHEVRON, COMPACT_PAD, COMPACT_MIN).max(text_w + GROUP_PAD * 2.0);
        let rect = Rect::from_min_size(top, vec2(width, bands.card()));
        let group_id = ui.advance_cursor_after_rect(rect);
        let button = bands.command_band(rect);
        let resp = ui.interact(button, group_id.with("folded"), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(button, CornerRadius::same(4), s.hover);
        }
        let name = pos2(button.center().x - CHEVRON * 0.5, button.center().y);
        ui.painter().text(name, Align2::CENTER_CENTER, self.caption, FontId::proportional(s.caption_font), s.icon);
        let c = pos2(name.x + text_w * 0.5 + CHEVRON * 0.5, button.center().y - 1.0);
        let k = Stroke::new(1.0, s.icon);
        ui.painter().line_segment([pos2(c.x - 3.0, c.y - 1.5), pos2(c.x, c.y + 1.5)], k);
        ui.painter().line_segment([pos2(c.x, c.y + 1.5), pos2(c.x + 3.0, c.y - 1.5)], k);
        let (label, enabled) = (self.caption.to_string(), ui.is_enabled());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &label));
        let resp = resp.on_hover_text(self.caption);
        let mut launcher_clicked = false;
        let mut inner = None;
        // The drop-down sits above the card and holds the group as designed; its width is capped at
        // the window, so a wide folded group wraps inside a narrow one.
        egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::IgnoreClicks).show(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
            ui.set_max_width((ui.ctx().content_rect().width() - 16.0).max(80.0));
            let content = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                    add(ui)
                })
                .inner;
            if captions {
                let (band, _) = ui.allocate_exact_size(vec2(ui.min_rect().width(), s.label_band), Sense::hover());
                launcher_clicked = caption_row(ui, s, band, self.caption, self.launcher, group_id.with("popup_launcher"));
            }
            inner = Some(content);
        });
        if captions {
            caption_row(ui, s, bands.label_band(rect), self.caption, None, group_id.with("band"));
        }
        if self.separator {
            let x = rect.right() - 0.5;
            ui.painter().vline(x, (rect.top() + 4.0)..=(rect.bottom() - 4.0), Stroke::new(1.0, s.separator));
        }
        GroupResponse { inner, launcher_clicked, level: Level::Compact, width }
    }
}

/// A group's caption centred in its label band, with the dialog launcher in the room reserved at
/// the band's trailing edge. Returns whether the launcher was clicked.
fn caption_row(ui: &mut Ui, s: &RibbonStyle, band: Rect, caption: &str, launcher: Option<&str>, id: egui::Id) -> bool {
    let (text_area, square) = fit::caption_slots(band, if launcher.is_some() { LAUNCHER_W } else { 0.0 });
    ui.painter().text(text_area.center(), Align2::CENTER_CENTER, caption, FontId::proportional(s.caption_font), s.caption);
    let (Some(tip), Some(lr)) = (launcher, square) else { return false };
    let resp = ui.interact(lr, id, Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(lr, CornerRadius::same(2), s.hover);
    }
    let c = lr.center();
    let k = Stroke::new(1.0, s.caption);
    ui.painter().line_segment([pos2(c.x - 3.5, c.y - 3.5), pos2(c.x + 3.0, c.y + 3.0)], k);
    ui.painter().line(vec![pos2(c.x + 3.0, c.y - 1.0), pos2(c.x + 3.0, c.y + 3.0), pos2(c.x - 1.0, c.y + 3.0)], k);
    ui.painter().rect_stroke(
        Rect::from_center_size(c, vec2(9.0, 9.0)),
        CornerRadius::ZERO,
        Stroke::new(1.0, s.caption.gamma_multiply(0.5)),
        StrokeKind::Inside,
    );
    let enabled = ui.is_enabled();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, tip));
    resp.on_hover_text(tip).clicked()
}

/// A square toggle (bold, italic, alignment…): hover fill, selected fill and outline. `paint`
/// draws the glyph in the given colour inside the given rect.
pub fn toggle(ui: &mut Ui, style: &RibbonStyle, selected: bool, label: &str, paint: impl FnOnce(&egui::Painter, Rect, Color32)) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect(rect, CornerRadius::same(4), style.selected, Stroke::new(1.0, style.selected_stroke), StrokeKind::Inside);
    } else if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(4), style.hover);
    }
    paint(painter, rect.shrink(4.0), style.icon);
    let (label, enabled) = (label.to_string(), ui.is_enabled());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, selected, &label));
    response.on_hover_text(label)
}

/// A large button: a 32 px icon over a one- or two-line label.
pub fn large_button(ui: &mut Ui, style: &RibbonStyle, label: &str, paint: impl FnOnce(&egui::Painter, Rect, Color32)) -> Response {
    let text_w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), FontId::proportional(12.0), style.icon).size().x);
    let (rect, response) = ui.allocate_exact_size(vec2(text_w.max(40.0) + 8.0, style.content_height), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), style.hover);
    }
    let icon = Rect::from_center_size(pos2(rect.center().x, rect.top() + 20.0), vec2(28.0, 28.0));
    paint(ui.painter(), icon, style.icon);
    ui.painter().text(pos2(rect.center().x, rect.bottom() - 10.0), Align2::CENTER_CENTER, label, FontId::proportional(12.0), style.icon);
    let (label, enabled) = (label.to_string(), ui.is_enabled());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &label));
    response
}

/// A small button: a 16 px icon and a label on one row.
pub fn small_button(ui: &mut Ui, style: &RibbonStyle, label: &str, paint: impl FnOnce(&egui::Painter, Rect, Color32)) -> Response {
    let text_w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), FontId::proportional(12.0), style.icon).size().x);
    let (rect, response) = ui.allocate_exact_size(vec2(text_w + 30.0, 24.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), style.hover);
    }
    paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 13.0, rect.center().y), vec2(16.0, 16.0)), style.icon);
    ui.painter().text(pos2(rect.left() + 26.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(12.0), style.icon);
    let (label, enabled) = (label.to_string(), ui.is_enabled());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &label));
    response
}

#[cfg(test)]
mod tests;
