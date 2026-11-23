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
