//! The status bar every Craft app shows along the bottom of the window: context on the left
//! (page or slide, word count, language, messages), view switches and the [`ZoomBar`] on the
//! right, at one height with one text size.
//!
//! [`ZoomBar`]: crate::slider::ZoomBar

use egui::{Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

/// Colours and metrics; [`StatusBarStyle::from_visuals`] derives them from egui's visuals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusBarStyle {
    pub fill: Color32,
    pub border: Color32,
    pub text: Color32,
    pub hover: Color32,
    pub selected: Color32,
    pub icon: Color32,
    pub height: f32,
    pub font: f32,
}

impl StatusBarStyle {
    pub fn from_visuals(v: &egui::Visuals) -> Self {
        Self {
            fill: v.panel_fill,
            border: v.widgets.noninteractive.bg_stroke.color,
            text: v.weak_text_color(),
            hover: v.widgets.hovered.weak_bg_fill,
            selected: v.widgets.inactive.weak_bg_fill,
            icon: v.text_color(),
            height: 26.0,
            font: 12.0,
        }
    }
}

const PAD_X: f32 = 10.0;
const GAP: f32 = 4.0;

/// See the module documentation.
pub struct StatusBar<'a> {
    style: &'a StatusBarStyle,
}

impl<'a> StatusBar<'a> {
    pub fn new(style: &'a StatusBarStyle) -> Self {
        Self { style }
    }

    /// Draws the bar across the available width: `left` lays out left to right from the start,
    /// `right` right to left from the end (so its last item sits at the edge).
    pub fn show(self, ui: &mut Ui, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui)) -> Response {
        let s = self.style;
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), s.height), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::ZERO, s.fill);
        ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, s.border));
        let inner = rect.shrink2(vec2(PAD_X, 2.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(Layout::right_to_left(Align::Center)), |ui| {
            // Right-hand controls wider than the bar stop at its edge instead of reaching over the
            // panel beside it.
            ui.shrink_clip_rect(rect);
            ui.spacing_mut().item_spacing = vec2(GAP, 0.0);
            right(ui);
            // Never inverted: in a window too narrow for the right-hand controls the left zone is
            // simply empty.
            let rest = Rect::from_min_max(inner.min, pos2((ui.min_rect().left() - GAP * 2.0).max(inner.min.x), inner.bottom()));
            ui.scope_builder(egui::UiBuilder::new().max_rect(rest).layout(Layout::left_to_right(Align::Center)), |ui| {
                ui.shrink_clip_rect(rest);
                ui.spacing_mut().item_spacing = vec2(GAP * 3.0, 0.0);
                left(ui);
            });
        });
        response
    }
}

/// Context text that may be clickable (Word's word count opens Word Count).
pub fn item(ui: &mut Ui, style: &StatusBarStyle, text: &str) -> Response {
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), FontId::proportional(style.font), style.text));
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 8.0, style.height - 6.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(3), style.hover);
    }
    ui.painter().galley(pos2(rect.left() + 4.0, rect.center().y - galley.size().y / 2.0), galley, style.text);
    let (text, enabled) = (text.to_string(), ui.is_enabled());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &text));
    response
}

/// A view switch (Print Layout, Normal, Slide Sorter…): a 22×20 icon button, filled when
/// `selected`. `paint` draws the icon in the given colour.
pub fn view_button(ui: &mut Ui, style: &StatusBarStyle, selected: bool, label: &str, paint: impl FnOnce(&egui::Painter, Rect, Color32)) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(24.0, style.height - 6.0), Sense::click());
    if selected {
        ui.painter().rect(rect, CornerRadius::same(3), style.selected, Stroke::NONE, StrokeKind::Inside);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(3), style.hover);
    }
    paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(14.0, 14.0)), style.icon);
    let (label, enabled) = (label.to_string(), ui.is_enabled());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, enabled, selected, &label));
    response.on_hover_text(label)
}

/// A text label with an icon at its left (Notes, Comments, Focus).
pub fn text_button(ui: &mut Ui, style: &StatusBarStyle, label: &str, paint: impl FnOnce(&egui::Painter, Rect, Color32)) -> Response {
    let w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), FontId::proportional(style.font), style.text).size().x);
    let (rect, response) = ui.allocate_exact_size(vec2(w + 26.0, style.height - 6.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(3), style.hover);
    }
    paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 11.0, rect.center().y), vec2(13.0, 13.0)), style.icon);
    ui.painter().text(pos2(rect.left() + 21.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(style.font), style.text);
    let (label, enabled) = (label.to_string(), ui.is_enabled());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &label));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window narrower than the right-hand controls leaves the left zone empty, not inverted.
    #[test]
    fn a_narrow_bar_keeps_a_valid_left_zone() {
        let ctx = egui::Context::default();
        let style = StatusBarStyle::from_visuals(&egui::Visuals::light());
        let mut left_rect = None;
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(60.0, 40.0))), ..Default::default() };
        let mut full = ctx.run_ui(input, |ui| {
            StatusBar::new(&style).show(
                ui,
                |ui| left_rect = Some(ui.max_rect()),
                |ui| {
                    ui.allocate_exact_size(vec2(200.0, 10.0), Sense::hover());
                },
            );
        });
        full.textures_delta.clear();
        let r = left_rect.expect("the left zone is laid out");
        assert!(r.width() >= 0.0 && r.height() >= 0.0, "{r:?}");
    }

    /// Both zones paint and take input only inside the bar and inside the parent's clip: right-hand
    /// controls wider than the bar stop at its edge, and a bar in a clipped panel stays in it.
    #[test]
    fn both_zones_stay_inside_the_bar_and_the_parent_clip() {
        let ctx = egui::Context::default();
        let style = StatusBarStyle::from_visuals(&egui::Visuals::light());
        let (mut bar, mut left_clip, mut right_clip) = (Rect::NOTHING, Rect::NOTHING, Rect::NOTHING);
        let parent = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(150.0, 40.0));
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, 40.0))), ..Default::default() };
        let mut full = ctx.run_ui(input, |ui| {
            ui.set_clip_rect(parent);
            bar = StatusBar::new(&style)
                .show(
                    ui,
                    |ui| left_clip = ui.clip_rect(),
                    |ui| {
                        ui.allocate_exact_size(vec2(500.0, 10.0), Sense::hover());
                        right_clip = ui.clip_rect();
                    },
                )
                .rect;
        });
        full.textures_delta.clear();
        let inside = parent.intersect(bar);
        for (zone, clip) in [("left", left_clip), ("right", right_clip)] {
            assert!(inside.contains_rect(clip) || !clip.is_positive(), "{zone}: {clip:?} outside {inside:?}");
        }
    }
}
