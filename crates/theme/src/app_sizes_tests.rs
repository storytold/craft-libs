#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{Value, json};

use super::*;

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
