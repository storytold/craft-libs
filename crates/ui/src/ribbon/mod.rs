//! The ribbon body of the Office-style Craft apps: one frame for the commands of the current tab,
//! split into groups with an optional caption and dialog launcher under each.
//!
//! The application owns the tabs, the commands, their icons and what each does. This module only
//! fixes the frame, the group geometry, captions, separators and toggle buttons, so every app's
//! ribbon has the same rhythm.

use egui::{Align, Align2, Color32, CornerRadius, FontId, InnerResponse, Layout, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

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
            caption_font: 11.0,
        }
    }
}

/// Space inside the body around the groups.
const PAD_X: f32 = 6.0;
const PAD_Y: f32 = 4.0;
const CAPTION_H: f32 = 16.0;
const GROUP_PAD: f32 = 6.0;
/// Room the dialog launcher takes at the right of a caption.
const LAUNCHER_W: f32 = 18.0;

/// The ribbon body; see the module documentation.
pub struct Ribbon<'a> {
    style: &'a RibbonStyle,
    frame: RibbonFrame,
    captions: bool,
}

impl<'a> Ribbon<'a> {
    pub fn new(style: &'a RibbonStyle) -> Self {
        Self { style, frame: RibbonFrame::platform_default(), captions: captions_default() }
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

    /// The height the body takes, frame included.
    pub fn height(&self) -> f32 {
        let body = self.style.content_height + PAD_Y * 2.0 + if self.captions { CAPTION_H } else { 0.0 };
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
        let inner = ui
            .scope_builder(egui::UiBuilder::new().max_rect(inner_rect).layout(Layout::left_to_right(Align::Min)), |ui| {
                ui.set_clip_rect(body);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                ui.data_mut(|d| d.insert_temp(captions_id(), captions));
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
    egui::Id::new("craft_ribbon_captions")
}

/// What a group reports.
pub struct GroupResponse<R> {
    pub inner: R,
    /// The dialog launcher (↘ next to the caption) was clicked.
    pub launcher_clicked: bool,
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

    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> GroupResponse<R> {
        let s = self.style;
        let captions = ui.data(|d| d.get_temp::<bool>(captions_id())).unwrap_or(true);
        let top = ui.cursor().min;
        let height = s.content_height + if captions { CAPTION_H } else { 0.0 };
        let content = Rect::from_min_size(pos2(top.x + GROUP_PAD, top.y), vec2(ui.available_width() - GROUP_PAD, s.content_height));
        let mut used = content;
        let inner = ui
            .scope_builder(egui::UiBuilder::new().max_rect(content).layout(Layout::left_to_right(Align::Center)), |ui| {
                ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                let r = add(ui);
                used = ui.min_rect();
                r
            })
            .inner;
        let mut width = used.width().max(0.0) + GROUP_PAD * 2.0;
        let caption_w = if captions {
            ui.fonts_mut(|f| f.layout_no_wrap(self.caption.to_string(), FontId::proportional(s.caption_font), s.caption).size().x)
                + if self.launcher.is_some() { LAUNCHER_W } else { 0.0 }
        } else {
            0.0
        };
        width = width.max(caption_w + GROUP_PAD * 2.0);
        let rect = Rect::from_min_size(top, vec2(width, height));
        ui.advance_cursor_after_rect(rect);
        let mut launcher_clicked = false;
        if captions {
            let cap = Rect::from_min_max(pos2(rect.left(), rect.bottom() - CAPTION_H), rect.max);
            // The caption is centred in what the launcher leaves, so a group no wider than its
            // caption does not draw one over the other.
            let text_area = if self.launcher.is_some() { Rect::from_min_max(cap.min, pos2(cap.right() - LAUNCHER_W, cap.bottom())) } else { cap };
            ui.painter().text(text_area.center(), Align2::CENTER_CENTER, self.caption, FontId::proportional(s.caption_font), s.caption);
            if let Some(tip) = self.launcher {
                let lr = Rect::from_center_size(pos2(cap.right() - 9.0, cap.center().y), vec2(14.0, 14.0));
                let resp = ui.interact(lr, ui.id().with(("launcher", self.caption)), Sense::click());
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
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tip));
                launcher_clicked = resp.on_hover_text(tip).clicked();
            }
        }
        if self.separator {
            let x = rect.right() - 0.5;
            ui.painter().vline(x, (rect.top() + 4.0)..=(rect.bottom() - 4.0), Stroke::new(1.0, s.separator));
        }
        GroupResponse { inner, launcher_clicked }
    }
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
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, selected, &label));
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
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &label));
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
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &label));
    response
}

#[cfg(test)]
mod tests;
