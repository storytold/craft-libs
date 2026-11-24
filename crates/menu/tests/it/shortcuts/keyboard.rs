//! The keyboard pane, the set row's buttons and the shortcut column keep the look the dialog had before the port: readable keys
//! beside the clicked key's shortcuts, filled keys and a legend in the same fills, framed set buttons, a shortcut written as one
//! text, and the add button right after the chips of every row.

use craft_keymap::{Platform, Profiles};
use craft_ui::contract::Role;
use craft_ui::testing::{Target, trial};
use egui::Shape;

use super::fixture::Rig;

/// The key named `name` on the pane (its node is a button of that name).
fn cap(rig: &Rig, name: &str) -> Target {
    rig.named(name, Role::BUTTON)
}

/// The rects of the painted rectangles that have a fill.
fn filled(shapes: &[Shape]) -> Vec<(egui::Rect, egui::Color32)> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Rect(rect) if rect.fill.a() > 0 => Some((rect.visual_bounding_rect(), rect.fill)),
            _ => None,
        })
        .collect()
}

/// With the keyboard pane on, a key is readable (at least a control tall and wide) and the clicked key's shortcuts stand
/// beside the whole block of keys, not between them.
#[trial]
fn the_keyboard_keeps_readable_keys_beside_the_key_list() {
    let rig = Rig::tuned(Platform::Windows, |options| options.keyboard = true);
    let q = cap(&rig, "Q").rect;
    assert!(q.width() >= 24.0 && q.height() >= 24.0, "a key stays readable: {q:?}");
    let hint = rig.fixture.texts().iter().find(|text| text.text.starts_with("Click a key")).expect("the hint").rect;
    assert!(hint.left() > q.left() + 18.0 * q.width(), "the hint stands beside the whole block of keys (a key is {q:?}): {hint:?}");
}

/// Keys are painted with a fill of their own, and the legend words stand on fills too.
#[trial]
fn keys_and_the_legend_are_filled() {
    let mut rig = Rig::tuned(Platform::Windows, |options| options.keyboard = true);
    let q = cap(&rig, "Q").rect;
    let legend = rig.fixture.texts().iter().find(|text| text.text == "Application").expect("the legend word Application").rect;
    let fills = filled(&rig.fixture.frame_shapes());
    assert!(fills.iter().any(|(rect, _)| (rect.center() - q.center()).length() < 2.0 && rect.width() > 20.0), "the key Q has a fill: {q:?}");
    assert!(fills.iter().any(|(rect, _)| rect.contains_rect(legend)), "the legend word stands on a fill: {legend:?}");
}

/// The set row's actions are framed buttons.
#[trial]
fn the_set_row_buttons_are_framed() {
    let mut rig = Rig::mac_with_sets(Profiles::default());
    let new = rig.find("New Set\u{2026}").rect;
    let shapes = rig.fixture.frame_shapes();
    let framed = shapes.iter().any(|shape| match shape {
        Shape::Rect(rect) => {
            let r = rect.visual_bounding_rect();
            (r.center() - new.center()).length() < 3.0 && r.width() >= new.width() - 3.0 && (rect.fill.a() > 0 || !rect.stroke.is_empty())
        }
        _ => false,
    });
    assert!(framed, "New Set\u{2026} is drawn as a framed button: {new:?}");
}

/// A shortcut is one text (`Ctrl+S`), not a keycap for every key.
#[trial]
fn a_shortcut_is_written_as_one_text() {
    let rig = Rig::tuned(Platform::Windows, |_| {});
    let texts: Vec<&str> = rig.fixture.texts().iter().map(|text| text.text.as_str()).collect();
    assert!(texts.contains(&"Ctrl+S"), "Save's shortcut reads Ctrl+S: {texts:?}");
}

/// The add button stands right after the last text of its row's shortcut cell, whether the cell holds a chord or None.
#[trial]
fn the_add_button_follows_the_chips_of_every_row() {
    let rig = Rig::tuned(Platform::Windows, |_| {});
    let adds: Vec<Target> = rig
        .fixture
        .by_role(Role::BUTTON)
        .into_iter()
        .filter(|node| rig.fixture.tree().name_of(node.node).is_some_and(|name| name.starts_with("Add a shortcut to")))
        .collect();
    assert!(adds.len() >= 3, "rows with an add button: {}", adds.len());
    for add in adds {
        let before = rig
            .fixture
            .texts()
            .iter()
            .filter(|text| (text.rect.center().y - add.rect.center().y).abs() < 6.0 && text.rect.right() <= add.rect.left() + 1.0)
            .map(|text| text.rect.right())
            .fold(f32::MIN, f32::max);
        assert!(add.rect.left() - before < 24.0, "the add button {:?} follows its chips ({before})", add.rect);
    }
}

/// An app that names a width opens the dialog at it.
#[trial]
fn a_named_width_opens_the_dialog_at_it() {
    let rig = Rig::tuned(Platform::Windows, |options| options.width = Some(900.0));
    let dialog = rig.fixture.by_role(Role::DIALOG).into_iter().next().expect("the dialog");
    assert!((dialog.rect.width() - 900.0).abs() < 4.0, "the dialog opens at the width the app named: {:?}", dialog.rect);
}
