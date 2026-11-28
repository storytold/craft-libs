//! The painted chevron the overflow menu and the split button draw.

use egui::Ui;

/// A downward triangle at the right-hand end of a control.
///
/// Painted rather than typed. The chevron character is absent from the default
/// font set, where it renders as a missing-glyph box. A caller leaves room for
/// it by padding the label, then paints it over the response.
pub fn chevron(ui: &Ui, rect: egui::Rect, colour: egui::Color32) {
    let middle = rect.right_center() - egui::vec2(9.0, 0.0);
    let half = 3.5;
    ui.painter().add(egui::Shape::convex_polygon(
        vec![middle + egui::vec2(-half, -half * 0.6), middle + egui::vec2(half, -half * 0.6), middle + egui::vec2(0.0, half * 0.9)],
        colour,
        egui::Stroke::NONE,
    ));
}
