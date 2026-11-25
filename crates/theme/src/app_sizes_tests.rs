#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{Value, json};

use super::*;
use crate::app_sizes::FOLLOWS;
use crate::sizes::{Preset, Slot, leaves};

fn sizes(v: &Value) -> AppSizes {
    AppSizes::parse(v.to_string().as_bytes()).unwrap()
}

#[test]
fn written_sizes_parse_back_to_the_same_overrides() {
    let original = sizes(&json!({
        "all": { "size.control.md": 28.5, "radius.button": "full" },
        "compact": { "space.pad.regular": 6, "typography.body.size": 15 },
    }));
    let text = original.to_json();
    assert!(text.starts_with("{\n  \"all\": {"), "{text}");
    assert!(!text.contains("comfortable"), "an empty section is left out");
    assert_eq!(AppSizes::parse(text.as_bytes()).unwrap(), original);
    assert_eq!(AppSizes::default().to_json(), "{}\n");
}

/// An app that sets only the control height keeps its push buttons a control high; one that sets the button height
/// gets that height.
#[test]
fn a_push_button_follows_the_control_height_unless_the_app_sets_its_own() {
    let base = crate::sizes::builtin(Size::Compact).base;
    let applied = |v: &Value| {
        let mut preset = base;
        sizes(v).apply_base(Size::Compact, &mut preset);
        preset.size
    };
    let follows = applied(&json!({ "all": { "size.control.md": 40 } }));
    assert!(follows.control.md > base.size.button, "the override raises the control height");
    assert_eq!(follows.button, follows.control.md, "the button follows the control height");
    let own = applied(&json!({ "all": { "size.control.md": 40, "size.button": 30 } }));
    assert!(own.button < own.control.md, "the app's own button height wins");
}

/// A page gap follows the gap it stands in for when the app sets only that one, and the page's New and Open follow a
/// push button down the chain to the control height; the app's own page values win.
#[test]
fn a_page_follows_the_values_it_stands_in_for_unless_the_app_sets_its_own() {
    let base = crate::sizes::builtin(Size::Compact).base;
    let applied = |v: &Value| {
        let mut preset = base;
        sizes(v).apply_base(Size::Compact, &mut preset);
        preset
    };
    let follows = applied(&json!({ "all": { "space.gap.section": 30, "size.control.md": 40 } }));
    assert!(follows.space.gap.section > base.space.gap.section, "the override widens the section gap");
    assert_eq!(follows.size.page.gap.section, follows.space.gap.section, "the page's section gap follows it");
    assert_eq!(follows.size.page.button, follows.size.control.md, "New and Open follow the push button to the control height");
    let own = applied(&json!({ "all": { "space.gap.section": 30, "size.page.gap.section": 40 } }));
    assert!(own.size.page.gap.section > own.space.gap.section, "the app's own page gap wins");
}

/// Each built-in size set gives a following value (`FOLLOWS`) the value it follows, so a new follower changes no app
/// that leaves it unset; an app's override reaches a follower through every step (a dialog's footer button follows the
/// push button, which follows the control height).
#[test]
fn a_follower_starts_at_the_value_it_follows_and_takes_its_override() {
    let value = |preset: &mut Preset, path: &str| {
        let want = format!("base.{path}");
        leaves(preset).into_iter().find_map(|l| match l.slot {
            Slot::Num(x) | Slot::Length(x) if l.path == want => Some(*x),
            _ => None,
        })
    };
    for size in [Size::Compact, Size::Comfortable] {
        let base = crate::sizes::builtin(size).base;
        for (own, of) in FOLLOWS {
            let mut preset = base;
            assert_eq!(value(&mut preset, own), value(&mut preset, of), "{size:?}: {own} starts at {of}");
        }
        let mut preset = base;
        sizes(&json!({ "all": { "size.control.md": 40, "space.pad.regular": 20 } })).apply_base(size, &mut preset);
        assert_eq!(value(&mut preset, "size.dialog.button_height"), Some(40.0), "{size:?}: through size.button");
        assert_eq!(value(&mut preset, "size.row.menu"), Some(40.0), "{size:?}");
        assert_eq!(value(&mut preset, "space.pad.dialog"), Some(20.0), "{size:?}");
    }
}
