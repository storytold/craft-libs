#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{Value, json};

use super::format::{self, FormatErrors, ThemeFile};
use super::semantic_tests::hex;
use super::*;
use crate::AppId;
use ColorToken as T;

/// A valid file that uses every section, with unknown keys at several levels.
fn full() -> Value {
    json!({
        "schema": "craft-design-theme", "schema_version": 1,
        "x_top": { "list": [1, 2, { "k": null }] },
        "id": "test.full", "name": "Full", "version": 3,
        "variants": "both", "contrast_intent": "low",
        "seeds": {
            "neutral": "#535353", "accent": "#78ADF6", "x_seed": "kept",
            "status": { "good": "#85DC6A", "warn": "#E8B046", "bad": "#FF8080", "info": "#78ADF6", "x_status": 1 }
        },
        "shape": {
            "radius": { "control": 3, "surface": 4, "window": 6, "panel": 0, "button": "full", "x_radius": 1 },
            "edge": { "bevel": { "width": 1 } }, "x_shape": true
        },
        "light": { "surface.panel": "#F0F0F0", "accent.solid": "#1062C4" },
        "dark": { "surface.panel": "#535353" },
        "high_contrast": { "light": { "text.normal": "#000000" }, "dark": {}, "x_hc": "kept" },
        "components": { "slider.thumb_height": 4.5 },
        "apps": {
            "photocraft": {
                "light": {}, "dark": { "surface.canvas": "#282828" },
                "high_contrast": { "dark": { "text.normal": "#FFFFFF" }, "x_app_hc": 1 },
                "components": { "slider.thumb_height": "full" }, "x_app": [true]
            },
            "x_app_entry": {}
        }
    })
}

fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap()
}

fn parse_value(v: &Value) -> Result<ThemeFile, FormatErrors> {
    format::parse(&bytes(v))
}

fn errors(v: &Value) -> Vec<String> {
    parse_value(v).unwrap_err().0.iter().map(ToString::to_string).collect()
}

#[test]
fn round_trip_keeps_values_and_unknown_keys() {
    let file = parse_value(&full()).unwrap();
    let t = &file.theme;
    assert_eq!((t.id.as_str(), t.name.as_str(), t.version), ("test.full", "Full", 3));
    assert_eq!((t.variants, t.contrast_intent), (Variants::Both, ContrastIntent::Low));
    assert_eq!(t.seeds.accent, hex("#78adf6"));
    let shape = t.shape.unwrap();
    assert_eq!(shape.radius.unwrap().button, 999.0);
    assert_eq!(shape.edge, Some(EdgeSeed::Bevel { width: 1.0 }));
    assert_eq!(t.pins.light.get(T::SurfacePanel), Some(hex("#f0f0f0")));
    assert_eq!(t.high_contrast.light.get(T::TextNormal), Some(hex("#000000")));
    assert_eq!(t.components.len(), 1);
    let photo = &t.apps[&AppId::parse("photocraft").unwrap()];
    assert_eq!(photo.pins.dark.get(T::SurfaceCanvas), Some(hex("#282828")));
    assert_eq!(photo.high_contrast.dark.get(T::TextNormal), Some(hex("#ffffff")));
    assert_eq!(photo.components.values().next(), Some(&999.0));

    let text = format::write(&file).unwrap();
    let written: Value = serde_json::from_str(&text).unwrap();
    let orig = full();
    for (path, expected) in [
        ("/x_top", &orig["x_top"]),
        ("/seeds/x_seed", &orig["seeds"]["x_seed"]),
        ("/seeds/status/x_status", &orig["seeds"]["status"]["x_status"]),
        ("/shape/x_shape", &orig["shape"]["x_shape"]),
        ("/shape/radius/x_radius", &orig["shape"]["radius"]["x_radius"]),
        ("/high_contrast/x_hc", &orig["high_contrast"]["x_hc"]),
        ("/apps/photocraft/x_app", &orig["apps"]["photocraft"]["x_app"]),
        ("/apps/photocraft/high_contrast/x_app_hc", &orig["apps"]["photocraft"]["high_contrast"]["x_app_hc"]),
        ("/apps/photocraft/high_contrast/dark/text.normal", &json!("#ffffff")),
        ("/apps/x_app_entry", &orig["apps"]["x_app_entry"]),
        ("/version", &json!(3)),
        ("/components/slider.thumb_height", &json!(4.5)),
        ("/shape/radius/button", &json!("full")),
        ("/dark/surface.panel", &json!("#535353")),
    ] {
        assert_eq!(written.pointer(path), Some(expected), "{path}");
    }
    assert!(text.starts_with("{\n  \"schema\": \"craft-design-theme\",\n  \"schema_version\": 1,\n  \"id\""), "{text}");
    let again = format::parse(text.as_bytes()).unwrap();
    assert_eq!(again.theme, file.theme);
    assert_eq!(format::write(&again).unwrap(), text, "writing is stable");
}

/// A theme file with the given light-mode pins.
fn with_light(pins: Value) -> Value {
    let mut v = full();
    v["light"] = pins;
    v
}

/// A reference to a derived token reads the value derived from the other references' final
/// values: `state.disabled_fill` is the panel, which here is itself a reference.
#[test]
fn a_reference_to_a_derived_token_reads_the_final_value() {
    let v = with_light(json!({ "surface.panel": "{surface.raised}", "surface.raised": "#d4d0c8", "text.muted": "{state.disabled_fill}" }));
    let file = parse_value(&v).unwrap();
    let a = Appearance { mode: Mode::Light, contrast: Contrast::Standard, size: Size::Compact, text_size: TextSize::Small, reduced_motion: false };
    let t = resolve(&file.theme, a, None).tokens;
    assert_eq!(t.color(T::SurfacePanel), hex("#d4d0c8"));
    assert_eq!(t.color(T::StateDisabledFill), hex("#d4d0c8"));
    assert_eq!(t.color(T::TextMuted), hex("#d4d0c8"));
}

/// A reference whose target is derived from the token it pins changes it on every round; that is
/// reported when the file is read, like a cycle of references.
#[test]
fn references_that_never_settle_are_errors() {
    let e = errors(&with_light(json!({ "state.selected": "{state.selected_hover}" })));
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(e[0].starts_with("light.\"state.selected\": references depend on themselves through derivation"), "{e:?}");
}

#[test]
fn reference_cycles_are_errors() {
    let e = errors(&with_light(json!({ "text.muted": "{text.faint}", "text.faint": "{text.muted}" })));
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(
        e[0].contains("references form a cycle: text.faint -> text.muted -> text.faint") || e[0].contains("text.muted -> text.faint -> text.muted"),
        "{e:?}"
    );
    let e = errors(&with_light(json!({ "text.muted": "{text.muted}" })));
    assert_eq!(e.len(), 1, "{e:?}");
}
