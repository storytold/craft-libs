//! The shape presets: catalog semantics, the glyph's paint and the menu's pick.

use craft_draw::geometry::presets::{PRESETS, ShapePreset};
use craft_draw::ui::shape_presets::ShapeMenu;
use craft_host::{AppInfo, Host};
use craft_i18n::{I18n, Lang};
use craft_theme::{Contrast, Mode};
use craft_ui::contract::Role;
use craft_ui::primitives::Glyph;
use craft_ui::testing::style::tokens;
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, Rect, Vec2, pos2};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

fn square() -> Rect {
    Rect::from_min_size(pos2(10.0, 10.0), Vec2::splat(20.0))
}

#[trial]
fn catalog_keys_are_unique_and_arrow_connector_semantics_remain_distinct() {
    let keys: std::collections::HashSet<_> = PRESETS.iter().map(|p| p.key()).collect();
    assert_eq!(keys.len(), PRESETS.len());
    assert!(ShapePreset::StraightArrowConnector.is_connector());
    assert!(ShapePreset::StraightArrowConnector.has_end_arrow());
    assert!(!ShapePreset::Arrow.is_connector());
    assert!(ShapePreset::Arrow.has_end_arrow());
    assert!(!ShapePreset::ElbowConnector.has_end_arrow());
}

#[trial]
fn every_procedural_preview_tessellates_and_invalid_geometry_is_inert() {
    let tokens = tokens(Mode::Light, Contrast::Standard);
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        for preset in PRESETS {
            preset.paint(ui.painter(), &tokens, square(), Color32::BLACK, false);
        }
    });
    out.textures_delta.clear();
    assert!(out.shapes.len() >= PRESETS.len(), "every preset paints something");
    for primitive in ctx.tessellate(out.shapes, out.pixels_per_point) {
        assert!(primitive.clip_rect.min.x >= 10.0 && primitive.clip_rect.min.y >= 10.0);
        assert!(primitive.clip_rect.max.x <= 30.0 && primitive.clip_rect.max.y <= 30.0);
        if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
            assert!(mesh.is_valid());
            assert!(mesh.vertices.iter().all(|v| v.pos.is_finite()));
        }
    }
    for bad in [Rect::NAN, Rect::NOTHING, Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(0.0, 20.0))] {
        let out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for preset in PRESETS {
                preset.paint(ui.painter(), &tokens, bad, Color32::BLACK, false);
            }
        });
        assert!(out.shapes.is_empty() || out.shapes.iter().all(|s| matches!(s.shape, egui::Shape::Noop)));
        out.drop_without_applying_deltas();
    }
}

#[trial]
fn concave_previews_fill_the_star_tips_and_heart_without_an_outline() {
    let tokens = tokens(Mode::Light, Contrast::Standard);
    let cross = |a: egui::Pos2, b: egui::Pos2, c: egui::Pos2| {
        let ab = b - a;
        let ac = c - a;
        ab.x * ac.y - ab.y * ac.x
    };
    for (preset, sample) in [(ShapePreset::Star, pos2(20.0, 14.0)), (ShapePreset::Heart, pos2(20.0, 20.0))] {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            preset.paint(ui.painter(), &tokens, square(), Color32::RED, false);
        });
        out.textures_delta.clear();
        let mut covered = false;
        for primitive in ctx.tessellate(out.shapes, out.pixels_per_point) {
            if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                assert!(mesh.is_valid());
                for triangle in mesh.indices.as_chunks::<3>().0 {
                    let [a, b, c] = [mesh.vertices[triangle[0] as usize], mesh.vertices[triangle[1] as usize], mesh.vertices[triangle[2] as usize]];
                    // The fill is the triangles of one colour that is neither the ink nor clear: the stroke's own
                    // triangles carry the ink or a clear feather.
                    let fill = a.color == b.color && b.color == c.color && a.color.a() > 0 && a.color != Color32::RED;
                    if !fill {
                        continue;
                    }
                    // One winding shows the fan's centre stays inside the contour's kernel.
                    assert!(cross(a.pos, b.pos, c.pos) > 0.0);
                    covered |= primitive.clip_rect.contains(sample)
                        && cross(a.pos, b.pos, sample) >= -0.0001
                        && cross(b.pos, c.pos, sample) >= -0.0001
                        && cross(c.pos, a.pos, sample) >= -0.0001;
                }
            }
        }
        assert!(covered, "{preset:?} must be filled where its contour is concave");
    }
}

#[trial]
fn capability_filtered_menu_uses_stable_identity_for_duplicate_localized_labels() {
    // A catalog that gives two presets the same text, so only the key tells their rows apart.
    let catalog = "@id\tshapePreset.rectangle\tObject\n@id\tshapePreset.ellipse\tObject\n".to_owned();
    let (i18n, problems) = I18n::from_owned([("fr".to_owned(), String::new(), catalog)], &["fr"]);
    assert!(problems.is_empty(), "{problems:?}");
    let i18n = Arc::new(i18n);
    let french = i18n.from_code("fr");
    assert!(french.is_some());
    let host = Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).i18n(i18n).lang(french.unwrap_or(Lang::EN)).build();
    let chosen = Rc::new(Cell::new(None));
    let seen = Rc::clone(&chosen);
    let mut fixture = Fixture::new(move |ui| {
        if let Some(preset) = ShapeMenu::new(|p| matches!(p, ShapePreset::Rectangle | ShapePreset::Ellipse)).show(ui) {
            seen.set(Some(preset));
        }
    });
    fixture.host(Arc::new(host));
    fixture.settle();
    let rows = fixture.by_role(Role::MENU_ITEM);
    assert_eq!(rows.len(), 2, "only the supported presets show");
    for row in &rows {
        assert_eq!(fixture.tree().name_of(row.node), Some("Object"));
    }
    fixture.click(&rows[1]);
    fixture.idle(2);
    assert_eq!(chosen.get(), Some(ShapePreset::Ellipse), "the second row is the second supported preset");
}
