//! Every popup sits on one popup surface: a fill under its content and one shadow, whether a container mounts it as
//! a layer or a component shows it from its `overlay` hook, and whatever its body paints itself.

use craft_ui::testing::{Drive, Fixture, stories, trial};
use egui::Shape;

/// Each story, opened by its drive, with a text only its popup shows.
const OPEN: [(&str, &str); 4] = [
    // Hook form, a list body.
    ("select_open", "Screen"),
    // Hook form, an inline menu body.
    ("split_button_open", "Colour"),
    // Container form.
    ("popup_list", "Square"),
    // Hook form, a dock group body that paints its own panel.
    ("open_beside_its_column", "Panel 0"),
];

#[trial]
fn an_open_popup_paints_one_filled_surface_with_one_shadow_under_its_content() {
    for (name, row) in OPEN {
        let entry = stories().into_iter().find(|entry| entry.story.name == name).unwrap_or_else(|| panic!("story {name}"));
        let mut fixture = Fixture::story(&entry.story);
        fixture.settle();
        if let Drive::Custom(open) = entry.story.drive {
            open(&mut fixture);
        }
        fixture.settle();
        let shapes = fixture.frame_shapes();
        let shadows: Vec<usize> = (0..shapes.len()).filter(|at| matches!(&shapes[*at], Shape::Rect(rect) if rect.blur_width > 0.0)).collect();
        assert_eq!(shadows.len(), 1, "{name}: one shadow, under the popup");
        // A surface paints its shadow, then its fill.
        let surface = match shapes.get(shadows[0] + 1) {
            Some(Shape::Rect(fill)) if fill.blur_width == 0.0 && fill.fill.a() > 0 => fill.rect,
            other => panic!("{name}: the popup's surface is filled over its shadow, not {other:?}"),
        };
        let content = shapes.iter().find_map(|shape| match shape {
            Shape::Text(text) if text.galley.job.text.replace('&', "") == row => Some(text.galley.rect.translate(text.pos.to_vec2())),
            _ => None,
        });
        let content = content.unwrap_or_else(|| panic!("{name}: the popup shows {row}"));
        assert!(surface.contains_rect(content), "{name}: the surface {surface:?} lies under the popup's {row} at {content:?}");
    }
}
