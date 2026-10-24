//! Page-joining dialog tab painting on the existing accessible tab interaction.

use super::Tab;
use egui::{Align2, Color32, CornerRadius, FontId, Rect, Response, Stroke, StrokeKind, Ui, pos2};

/// A focused style adapter, not an application theme. The selected fill should match the page
/// below the strip. Draw the strip's baseline before its tabs: selected tabs erase their bottom
/// border to visually join that page. Applications own strip layout, overflow and page selection.
#[derive(Clone, Debug)]
pub struct DialogTabStyle {
    pub page: Color32,
    pub inactive: Color32,
    pub hovered: Color32,
    pub text: Color32,
    pub selected_text: Color32,
    /// Use this stroke for the strip baseline too. The selected seam covers half its width
    /// below the tab, matching a baseline centered on the bottom edge.
    pub border: Stroke,
    pub font: FontId,
    pub top_radius: u8,
}

impl Tab<'_> {
    /// Paint a raised dialog tab and return the same response as [`Tab::show_at`].
    ///
    /// Stable IDs, selected accessibility state, parent disabled state, keyboard activation and
    /// focus outlines all come from the existing tab. The application supplies localized text,
    /// geometry and style. Oversized labels clip within their tab instead of covering neighbours.
    /// The selected tab joins a page beginning at `rect.bottom()`; unselected tabs sit two points
    /// lower. No page is selected implicitly, and arrow-key navigation remains caller policy.
    pub fn show_dialog_at(self, ui: &mut Ui, rect: Rect, style: &DialogTabStyle) -> Response {
        let selected = self.selected;
        let label = self.label;
        self.show_at(ui, rect, |ui, response| {
            let inset = if selected { 0.0 } else { 2.0_f32.min(rect.height() * 0.25) };
            let shown = Rect::from_min_max(pos2(rect.left(), rect.top() + inset), rect.max);
            let width = if style.border.width.is_finite() { style.border.width.clamp(0.0, rect.width().min(rect.height()) * 0.5) } else { 0.0 };
            let stroke = Stroke::new(width, style.border.color);
            let fill = if selected {
                style.page
            } else if response.hovered() {
                style.hovered
            } else {
                style.inactive
            };
            let radius = CornerRadius { nw: style.top_radius, ne: style.top_radius, sw: 0, se: 0 };
            let mut paint_bounds = rect;
            if selected {
                paint_bounds.max.y += width * 0.5;
            }
            let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(paint_bounds));
            painter.rect(shown, radius, fill, stroke, StrokeKind::Inside);
            // Cover only the selected tab's bottom edge, preserving the vertical border corners.
            if selected && width > 0.0 && rect.width() > 2.0 * width {
                painter.rect_filled(
                    Rect::from_min_max(pos2(rect.left() + width, rect.bottom() - width), pos2(rect.right() - width, rect.bottom() + width * 0.5)),
                    0.0,
                    style.page,
                );
            }
            let text_rect = rect.shrink2(egui::vec2(2.0_f32.min(rect.width() * 0.25), 0.0));
            let color = if selected { style.selected_text } else { style.text };
            let font = FontId::new(if style.font.size.is_finite() { style.font.size.clamp(1.0, 512.0) } else { 12.0 }, style.font.family.clone());
            painter.with_clip_rect(text_rect.intersect(ui.clip_rect())).text(rect.center(), Align2::CENTER_CENTER, label, font, color);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Id, Vec2};
    use egui_kittest::kittest::Queryable;

    fn style() -> DialogTabStyle {
        DialogTabStyle {
            page: Color32::WHITE,
            inactive: Color32::LIGHT_GRAY,
            hovered: Color32::GRAY,
            text: Color32::BLACK,
            selected_text: Color32::BLACK,
            border: Stroke::new(1.0, Color32::DARK_GRAY),
            font: FontId::proportional(12.0),
            top_radius: 5,
        }
    }

    #[test]
    fn painting_preserves_accessible_selection_and_click_response_with_duplicate_labels() {
        let mut h = egui_kittest::Harness::builder().with_size(Vec2::new(400.0, 200.0)).build_ui_state(
            |ui, selected: &mut usize| {
                for index in 0..2 {
                    let (_, rect) = ui.allocate_space(Vec2::new(100.0, 26.0));
                    let response = Tab::new(Id::new(("dialog", index)), "Options", *selected == index).show_dialog_at(ui, rect, &style());
                    if response.clicked() {
                        *selected = index;
                    }
                }
            },
            0,
        );
        h.run_steps(2);
        let tabs: Vec<_> = h.get_all_by_label("Options").collect();
        assert_eq!(tabs.len(), 2);
        tabs[1].click();
        h.run_steps(2);
        assert_eq!(*h.state(), 1);
    }

    #[test]
    fn malformed_style_and_long_text_tessellate_inside_small_tab() {
        let ctx = egui::Context::default();
        let mut style = style();
        style.border.width = f32::NAN;
        style.font.size = f32::INFINITY;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let (_, rect) = ui.allocate_space(Vec2::new(5.0, 3.0));
            Tab::new(Id::new("small"), "A long localized label", true).show_dialog_at(ui, rect, &style);
        });
        out.textures_delta.clear();
        let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
    }

    #[test]
    fn selected_tab_erases_a_baseline_centered_on_its_bottom_edge() {
        let mut h = egui_kittest::Harness::builder().with_size(Vec2::new(120.0, 80.0)).wgpu().build_ui(|ui| {
            ui.painter().rect_filled(ui.max_rect(), 0.0, Color32::WHITE);
            let rect = Rect::from_min_size(egui::pos2(10.0, 10.0), Vec2::new(100.0, 26.0));
            let mut style = style();
            style.border = Stroke::new(2.0, Color32::BLACK);
            ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], style.border);
            Tab::new(Id::new("joined"), "Options", true).show_dialog_at(ui, rect, &style);
        });
        h.ctx.set_pixels_per_point(1.0);
        h.run_steps(3);
        let image = h.render();
        assert!(image.is_ok());
        if let Ok(image) = image {
            let pixel = image.get_pixel(60, 36);
            assert!(pixel[0] >= 245 && pixel[1] >= 245 && pixel[2] >= 245, "seam remains visible: {pixel:?}");
        }
    }
}
