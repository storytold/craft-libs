//! Shift+Up and Shift+Down in a Full picker field add or take away exactly ten from the value, clamped at the range's
//! ends; Up and Down without Shift step one (photocraft#16743).

use craft_ui::testing::trial;
use egui::{Key, Modifiers};

use super::test_rig::Rig;

/// The field's text after typing `start` into `field` and pressing `keys` in it.
fn stepped(field: &str, start: &str, keys: &[(Key, Modifiers)]) -> String {
    let mut rig = Rig::open("#336699");
    rig.type_tab(field, start);
    for (key, modifiers) in keys {
        rig.key_in(field, *key, *modifiers);
    }
    rig.text(field)
}

#[trial]
fn shift_arrows_step_exactly_ten_from_the_value() {
    let (up, down, shift) = (Key::ArrowUp, Key::ArrowDown, Modifiers::SHIFT);
    assert_eq!(stepped("Hue", "182", &[(up, shift)]), "192");
    assert_eq!(stepped("Hue", "183", &[(up, shift)]), "193");
    assert_eq!(stepped("Hue", "183", &[(down, shift)]), "173");
    assert_eq!(stepped("Hue", "180", &[(up, Modifiers::NONE), (up, Modifiers::NONE), (up, shift)]), "192", "the reference's H 180, Up, Up, Shift+Up");
    assert_eq!(stepped("Red", "250", &[(up, shift)]), "255", "clamped at the top");
    assert_eq!(stepped("Red", "5", &[(down, shift)]), "0", "clamped at the bottom");
    assert_eq!(stepped("Saturation", "47", &[(down, shift)]), "37");
}

#[trial]
fn plain_arrows_still_step_one() {
    assert_eq!(stepped("Red", "100", &[(Key::ArrowUp, Modifiers::NONE)]), "101");
    assert_eq!(stepped("Red", "100", &[(Key::ArrowDown, Modifiers::NONE)]), "99");
    assert_eq!(stepped("Red", "0", &[(Key::ArrowDown, Modifiers::NONE)]), "0", "R 0, Down stays 0");
    assert_eq!(stepped("Red", "255", &[(Key::ArrowUp, Modifiers::NONE)]), "255");
}
