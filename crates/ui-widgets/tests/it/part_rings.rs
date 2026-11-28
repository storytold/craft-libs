//! The focus ring of focusable parts: Tab to each part of a story and the paint step rings it, as it rings a button.
//! A collection rings its focused parts but not itself, since its lead item shows the focus.

use craft_behavior::PartKey;
use craft_ui::testing::{Fixture, Registry, trial};
use egui::{Key, Modifiers, Rect, Shape};

/// The story named `name`, settled.
fn story(name: &str) -> Fixture {
    let registry = Registry::of(&[crate::register, craft_ui::testing::register]);
    let _ = registry.install_story_sets();
    let entry = registry.stories().iter().find(|entry| entry.story.name == name).unwrap_or_else(|| panic!("a story named {name}"));
    let mut fixture = Fixture::story(&entry.story);
    fixture.settle();
    fixture
}

/// Presses Tab until the stop `part` (`None` for a component's own stop) holds the focus, and returns its rect.
fn tab_to(fixture: &mut Fixture, part: Option<&'static str>) -> Option<Rect> {
    let wanted = part.map(PartKey::new);
    for _ in 0..24 {
        fixture.key(Key::Tab, Modifiers::NONE).settle();
        let Some((owner, focused)) = fixture.focused_stop() else { continue };
        if focused.map(|focused| focused.key) != wanted {
            continue;
        }
        return match focused {
            Some(focused) => fixture.part_rect(owner, focused),
            None => fixture.node_of(owner).map(|target| target.rect),
        };
    }
    None
}

/// Whether the latest frame paints a focus ring (a rectangle stroke at least two points wide) around `target`.
fn ringed(fixture: &mut Fixture, target: Rect) -> bool {
    fixture.frame_shapes().iter().any(|shape| match shape {
        Shape::Rect(rect) if rect.stroke.width >= 2.0 => rect.rect.expand(1.0).contains_rect(target),
        _ => false,
    })
}

/// Each focusable part, and each control that is its own stop, is ringed when Tab reaches it.
#[trial]
fn focused_parts_show_the_focus_ring() {
    const STOPS: &[(&str, Option<&str>)] = &[
        ("notice", Some("action")),
        ("toast_stack", Some("dismiss")),
        ("bar_items", Some("item")),
        ("list_row", None),
        ("list_row_trailing_items", Some("trailing")),
        ("list_row_status_cell", Some("status-action")),
        ("list_row_thumbnails_only", Some("link")),
        ("chip_field", Some("chip")),
        ("alt_text_field", Some("decorative")),
        ("tile", None),
        ("tile_actions", Some("action")),
        ("tile_grid_insert_gaps", Some("insert_gap")),
        ("thumbnail_slider", Some("out")),
        ("tab_strip_ribbon_end_items", Some("end")),
        ("table_one", Some("header")),
        ("table_editable_grid", Some("header_remove")),
    ];
    let mut unringed = Vec::new();
    for &(name, part) in STOPS {
        let mut fixture = story(name);
        let rect = tab_to(&mut fixture, part).unwrap_or_else(|| panic!("Tab reaches {part:?} in {name}"));
        if !ringed(&mut fixture, rect) {
            unringed.push(format!("{name} {part:?}"));
        }
    }
    assert!(unringed.is_empty(), "these focused stops show no ring: {unringed:?}");
}

/// A focused collection draws no ring around itself: its lead item shows the focus.
#[trial]
fn a_focused_collection_draws_no_ring_around_itself() {
    let mut ringed_whole = Vec::new();
    for name in ["tab_strip_ribbon_end_items", "table_one", "tile_grid_insert_gaps"] {
        let mut fixture = story(name);
        let rect = tab_to(&mut fixture, None).unwrap_or_else(|| panic!("Tab reaches the collection in {name}"));
        if ringed(&mut fixture, rect) {
            ringed_whole.push(name);
        }
    }
    assert!(ringed_whole.is_empty(), "these collections ring themselves: {ringed_whole:?}");
}
