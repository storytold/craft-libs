use super::*;
use crate::buttons::ButtonVisuals;
use egui::{Color32, CornerRadius, Stroke};
use egui_kittest::kittest::Queryable;

fn style() -> IconButtonStyle {
    let normal = ButtonVisuals { fill: Color32::TRANSPARENT, stroke: Stroke::NONE, icon: Color32::BLACK };
    IconButtonStyle {
        normal,
        hovered: ButtonVisuals { fill: Color32::LIGHT_BLUE, ..normal },
        pressed: None,
        selected: normal,
        corner_radius: CornerRadius::same(3),
        focus_stroke: Stroke::new(1.0, Color32::BLUE),
    }
}

#[test]
fn capability_filtered_menu_uses_stable_identity_for_duplicate_localized_labels() {
    let mut h = egui_kittest::Harness::builder().with_size(Vec2::new(400.0, 300.0)).build_ui_state(
        |ui, chosen: &mut Option<ShapePreset>| {
            if let Some(preset) = menu(
                ui,
                180.0,
                FontId::proportional(12.0),
                &style(),
                |p| matches!(p, ShapePreset::Rectangle | ShapePreset::Ellipse),
                |_| "Object".into(),
            ) {
                *chosen = Some(preset);
            }
        },
        None,
    );
    h.run_steps(2);
    let items: Vec<_> = h.get_all_by_label("Object").collect();
    assert_eq!(items.len(), 2);
    items[1].click();
    h.run_steps(2);
    assert_eq!(*h.state(), Some(ShapePreset::Ellipse));
}

#[test]
fn catalog_keys_are_unique_and_arrow_connector_semantics_remain_distinct() {
    let keys: std::collections::HashSet<_> = PRESETS.iter().map(|p| p.key()).collect();
    assert_eq!(keys.len(), PRESETS.len());
    assert!(ShapePreset::StraightArrowConnector.is_connector());
    assert!(ShapePreset::StraightArrowConnector.has_end_arrow());
    assert!(!ShapePreset::Arrow.is_connector());
    assert!(ShapePreset::Arrow.has_end_arrow());
    assert!(!ShapePreset::ElbowConnector.has_end_arrow());
}

#[test]
fn every_procedural_preview_tessellates_and_invalid_geometry_is_inert() {
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        for &preset in PRESETS {
            let rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), Vec2::splat(20.0));
            paint_preset(ui.painter(), rect, preset, Stroke::new(f32::NAN, Color32::BLACK), Color32::LIGHT_BLUE);
        }
    });
    out.textures_delta.clear();
    for primitive in ctx.tessellate(out.shapes, out.pixels_per_point) {
        assert!(primitive.clip_rect.min.x >= 10.0 && primitive.clip_rect.min.y >= 10.0);
        assert!(primitive.clip_rect.max.x <= 30.0 && primitive.clip_rect.max.y <= 30.0);
        if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
            assert!(mesh.is_valid());
            assert!(mesh.vertices.iter().all(|v| v.pos.is_finite()));
        }
    }
    let out = ctx.run_ui(egui::RawInput::default(), |ui| {
        for &preset in PRESETS {
            paint_preset(ui.painter(), egui::Rect::NAN, preset, Stroke::NONE, Color32::WHITE);
        }
    });
    assert!(out.shapes.is_empty() || out.shapes.iter().all(|s| matches!(s.shape, egui::Shape::Noop)));
    out.drop_without_applying_deltas();
}

#[test]
fn concave_previews_fill_the_star_tips_and_heart_without_an_outline() {
    let cross = |a: egui::Pos2, b: egui::Pos2, c: egui::Pos2| {
        let ab = b - a;
        let ac = c - a;
        ab.x * ac.y - ab.y * ac.x
    };
    for (preset, sample) in [(ShapePreset::Star, egui::pos2(20.0, 14.0)), (ShapePreset::Heart, egui::pos2(20.0, 20.0))] {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            paint_preset(ui.painter(), egui::Rect::from_min_size(egui::pos2(10.0, 10.0), Vec2::splat(20.0)), preset, Stroke::NONE, Color32::RED);
        });
        out.textures_delta.clear();
        let mut covered = false;
        for primitive in ctx.tessellate(out.shapes, out.pixels_per_point) {
            if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                assert!(mesh.is_valid());
                for triangle in mesh.indices.as_chunks::<3>().0 {
                    let [a, b, c] = [mesh.vertices[triangle[0] as usize], mesh.vertices[triangle[1] as usize], mesh.vertices[triangle[2] as usize]];
                    // Consistent winding verifies each fixed contour's center stays in its kernel.
                    assert!(cross(a.pos, b.pos, c.pos) > 0.0);
                    covered |= primitive.clip_rect.contains(sample)
                        && a.color == Color32::RED
                        && b.color == Color32::RED
                        && c.color == Color32::RED
                        && cross(a.pos, b.pos, sample) >= -0.0001
                        && cross(b.pos, c.pos, sample) >= -0.0001
                        && cross(c.pos, a.pos, sample) >= -0.0001;
                }
            }
        }
        assert!(covered, "{preset:?} must honor its fill with no outline");
    }
}
