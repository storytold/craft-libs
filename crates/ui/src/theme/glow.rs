use egui::{Color32, Rect, Shape, pos2};

/// An accent glow rising from the bottom edge of a toolbar group or inspector section.
///
/// This is painting only: it adds no widget, hit target, layout space, or application state.
/// Reserve a `Shape::Noop` before drawing the group's controls and replace it afterward with
/// this shape so it stays behind text. The caller supplies hover strength (usually egui's
/// `animate_bool_responsive`), color, clipping and motion preference. Stable group IDs keep
/// animations independent of translated headings. Hover should include the group's children
/// but respect popup/layer occlusion, e.g. `ui.rect_contains_pointer(group_rect)`.
///
/// `height` is bounded to the rectangle's height and at most 4096 logical points. Strength is
/// clamped to 0..1 and multiplies the supplied color's alpha. Non-finite/empty geometry or
/// non-positive strength/height produces `Shape::Noop`. The glow remains inside `rect`, fading
/// at its sides as well as upward, so it cannot cover neighbouring groups or leak past a panel.
pub fn edge_glow(rect: Rect, color: Color32, strength: f32, height: f32) -> Shape {
    if !rect.is_finite()
        || rect.width() <= 0.0
        || rect.height() <= 0.0
        || rect.size().max_elem() > 1_000_000.0
        || !strength.is_finite()
        || strength <= 0.0
        || !height.is_finite()
        || height <= 0.0
    {
        return Shape::Noop;
    }
    let height = height.min(rect.height()).min(4096.0);
    let soft = (rect.width() * 0.3).min(22.0);
    let xs = [rect.left(), rect.left() + soft, rect.right() - soft, rect.right()];
    let mut mesh = egui::Mesh::default();
    for (y, vertical) in [(rect.bottom(), 1.0), (rect.bottom() - height, 0.0)] {
        for (i, &x) in xs.iter().enumerate() {
            let side = if i == 0 || i == 3 { 0.0 } else { 1.0 };
            mesh.colored_vertex(pos2(x, y), color.gamma_multiply(strength.min(1.0) * vertical * side));
        }
    }
    for i in 0..3_u32 {
        mesh.add_triangle(i, i + 1, i + 5);
        mesh.add_triangle(i, i + 5, i + 4);
    }
    Shape::mesh(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
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
    #[test]
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
}
