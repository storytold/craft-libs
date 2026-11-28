//! Regression coverage for group glow.

use craft_ui::testing::trial;
use craft_ui::theme::edge_glow;
use egui::{Color32, Rect, Shape, pos2};
#[trial]
fn glow_stays_inside_group_and_multiplies_alpha_without_opaque_edges() {
    let rect = Rect::from_min_size(pos2(20.0, 30.0), egui::vec2(100.0, 40.0));
    let shape = edge_glow(rect, Color32::from_rgba_unmultiplied(20, 100, 200, 100), 0.5, 9.0);
    assert!(matches!(shape, Shape::Mesh(_)), "expected mesh");
    let Shape::Mesh(mesh) = shape else {
        return;
    };
    assert_eq!(mesh.vertices.len(), 8);
    assert_eq!(mesh.indices.len(), 18);
    assert!(mesh.is_valid());
    assert!(mesh.vertices.iter().all(|v| rect.contains(v.pos) && v.color.a() <= 50));
    assert!(mesh.vertices.iter().skip(4).all(|v| v.color == Color32::TRANSPARENT));
    assert_eq!(mesh.vertices[0].color, Color32::TRANSPARENT);
    assert!(mesh.vertices[1].color.a() > 0);
}
#[trial]
fn hostile_input_and_zero_strength_are_inert() {
    for rect in [Rect::NOTHING, Rect::NAN, Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(0.0, 3.0))] {
        assert!(matches!(edge_glow(rect, Color32::WHITE, 1.0, 9.0), Shape::Noop));
    }
    let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(20.0, 10.0));
    for value in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
        assert!(matches!(edge_glow(rect, Color32::WHITE, value, 9.0), Shape::Noop));
        assert!(matches!(edge_glow(rect, Color32::WHITE, 1.0, value), Shape::Noop));
    }
}
