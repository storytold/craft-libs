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
        "variants": "both", "contrast_intent": "low", "icon_pack": "craft.tabler",
        "seeds": {
            "neutral": "#535353", "accent": "#78ADF6", "x_seed": "kept",
            "status": { "good": "#85DC6A", "warn": "#E8B046", "bad": "#FF8080", "info": "#78ADF6", "x_status": 1 }
        },
        "shape": {
            "radius": { "control": 3, "surface": 4, "window": 6, "panel": 0, "button": "full", "x_radius": 1 },
            "edge": { "bevel": { "width": 1 } }, "knob": "tab", "wells": "round", "x_shape": true
        },
        "light": { "surface.panel": "#F0F0F0", "accent.solid": "#1062C4" },
        "dark": { "surface.panel": "#535353" },
        "high_contrast": { "light": { "text.normal": "#000000" }, "dark": {}, "x_hc": "kept" },
        "apps": {
            "photocraft": {
                "light": {}, "dark": { "surface.canvas": "#282828" },
                "high_contrast": { "dark": { "text.normal": "#FFFFFF" }, "x_app_hc": 1 },
                "x_app": [true]
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
    assert_eq!(t.icon_pack.as_ref().map(IconPackId::as_str), Some("craft.tabler"));
    assert_eq!(t.seeds.accent, hex("#78adf6"));
    let shape = t.shape.unwrap();
    assert_eq!(shape.radius.unwrap().button, 999.0);
    assert_eq!(shape.edge, Some(EdgeSeed::Bevel { width: 1.0 }));
    assert_eq!(shape.knob, Some(KnobShape::Tab));
    assert_eq!(shape.wells, Some(WellShape::Round));
    assert_eq!(t.pins.light.get(T::SurfacePanel), Some(hex("#f0f0f0")));
    assert_eq!(t.high_contrast.light.get(T::TextNormal), Some(hex("#000000")));
    let photo = &t.apps[&AppId::parse("photocraft").unwrap()];
    assert_eq!(photo.pins.dark.get(T::SurfaceCanvas), Some(hex("#282828")));
    assert_eq!(photo.high_contrast.dark.get(T::TextNormal), Some(hex("#ffffff")));

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
        ("/icon_pack", &json!("craft.tabler")),
        ("/shape/knob", &json!("tab")),
        ("/shape/wells", &json!("round")),
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

/// An app's own `shape` replaces the theme's for that app only (WordCraft's rounded-rectangle buttons under
/// ArtCraft's capsules), and it is written back.
#[test]
fn an_apps_own_shape_replaces_the_themes_for_that_app() {
    let mut v = full();
    v["apps"]["photocraft"]["shape"] = json!({ "button_radius": 4 });
    let file = parse_value(&v).unwrap();
    let id = AppId::parse("photocraft").unwrap();
    assert_eq!(file.theme.apps.get(&id).and_then(|app| app.shape).and_then(|shape| shape.button_radius), Some(4.0));
    let appearance = super::semantic_tests::app(Mode::Light, Contrast::Standard);
    let own = crate::resolve(&file.theme, appearance, Some(&id)).tokens.shape.radius.button;
    let theirs = crate::resolve(&file.theme, appearance, None).tokens.shape.radius.button;
    assert_eq!(own, 4.0, "the app's buttons take its own radius");
    assert_ne!(theirs, 4.0, "other apps keep the theme's");
    let written: Value = serde_json::from_str(&format::write(&file).unwrap()).unwrap();
    assert_eq!(written.pointer("/apps/photocraft/shape/button_radius"), Some(&json!(4.0)));
    let mut bad = full();
    bad["apps"]["photocraft"]["shape"] = json!({ "button_radius": -1 });
    assert!(!errors(&bad).is_empty(), "an app's shape is checked as a theme's");
}

/// A pack set on a file without one is written; clearing it removes the key and keeps the file's unknown keys; a
/// value that is not a well-formed pack id is an error at its key.
#[test]
fn the_icon_pack_is_added_cleared_and_checked() {
    let mut v = full();
    v.as_object_mut().unwrap().remove("icon_pack");
    let mut file = parse_value(&v).unwrap();
    assert_eq!(file.theme.icon_pack, None);
    file.theme.icon_pack = Some(IconPackId::parse("user.rounded").unwrap());
    let added: Value = serde_json::from_str(&format::write(&file).unwrap()).unwrap();
    assert_eq!(added.pointer("/icon_pack"), Some(&json!("user.rounded")));

    let mut file = parse_value(&full()).unwrap();
    file.theme.icon_pack = None;
    let cleared: Value = serde_json::from_str(&format::write(&file).unwrap()).unwrap();
    assert_eq!(cleared.pointer("/icon_pack"), None);
    assert_eq!(cleared.pointer("/x_top"), full().pointer("/x_top"), "unknown keys stay");

    for bad in [json!(3), json!("Bad Pack"), json!("x")] {
        let mut v = full();
        v["icon_pack"] = bad.clone();
        let e = errors(&v);
        assert_eq!(e.len(), 1, "{bad}: {e:?}");
        assert!(e[0].starts_with("icon_pack: "), "{bad}: {e:?}");
    }
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

/// `shape.locator` sets the reference point's squares: left out, a value is the Size preset's handle size; set, the
/// tokens take it and the file writes it back; a value of the wrong kind or out of range is an error at its key.
#[test]
fn the_locator_shape_is_read_resolved_written_and_checked() {
    let appearance = super::semantic_tests::app(Mode::Dark, Contrast::Standard);
    let plain = crate::resolve(&parse_value(&full()).unwrap().theme, appearance, None).tokens;
    let handle = plain.size.handle;
    assert_eq!((plain.shape.locator.cell, plain.shape.locator.square), (handle.md, handle.sm), "no locator: the preset's handles");

    let mut v = full();
    v["shape"]["locator"] = json!({ "square": 5 });
    let file = parse_value(&v).unwrap();
    let locator = crate::resolve(&file.theme, appearance, None).tokens.shape.locator;
    assert_eq!((locator.cell, locator.square), (handle.md, 5.0), "a set value over the preset's");
    let written: Value = serde_json::from_str(&format::write(&file).unwrap()).unwrap();
    assert_eq!(written.pointer("/shape/locator/square"), Some(&json!(5.0)));
    assert_eq!(written.pointer("/shape/locator/cell"), None, "an unset value is not written");

    for (key, bad) in [("cell", json!(0)), ("cell", json!(33)), ("square", json!("small")), ("square", json!(17))] {
        let mut v = full();
        v["shape"]["locator"] = json!({ key: bad });
        let found = errors(&v);
        assert!(found.iter().any(|e| e.contains(&format!("shape.locator.{key}"))), "{key} = {bad}: {found:?}");
    }
}

/// photocraft#16426: `shape.grip` sets the dock grips' marks. Each value is optional and keeps the default's when
/// left out; the theme's tokens take them, the file writes them back, and a value of the wrong kind or out of range
/// is an error at its key.
#[test]
fn the_grip_shape_is_read_resolved_written_and_checked() {
    let appearance = super::semantic_tests::app(Mode::Dark, Contrast::Standard);
    let plain = parse_value(&full()).unwrap();
    assert_eq!(crate::resolve(&plain.theme, appearance, None).tokens.shape.grip, GripShape::DEFAULT, "no grip: the default");
    assert_eq!(
        GripShape::DEFAULT,
        GripShape { mark: GripMark::Ticks, count: 11, length: 5.0, width: 1.0, gap: 1.0 },
        "eleven ticks, five points tall: apart from the reference application's ten of four"
    );

    let mut v = full();
    v["shape"]["grip"] = json!({ "mark": "dots", "count": 6, "width": 2, "gap": 3, "x_grip": 1 });
    let file = parse_value(&v).unwrap();
    let grip = crate::resolve(&file.theme, appearance, None).tokens.shape.grip;
    assert_eq!(grip, GripShape { mark: GripMark::Dots, count: 6, length: 5.0, width: 2.0, gap: 3.0 }, "set values over the default");
    let written: Value = serde_json::from_str(&format::write(&file).unwrap()).unwrap();
    assert_eq!(written.pointer("/shape/grip/mark"), Some(&json!("dots")));
    assert_eq!(written.pointer("/shape/grip/count"), Some(&json!(6)));
    assert_eq!(written.pointer("/shape/grip/length"), None, "an unset value is not written");
    assert_eq!(written.pointer("/shape/grip/x_grip"), Some(&json!(1)), "unknown keys stay");
    assert_eq!(format::parse(format::write(&file).unwrap().as_bytes()).unwrap().theme, file.theme);

    for (key, bad) in [
        ("mark", json!("stripes")),
        ("count", json!(0)),
        ("count", json!(2.5)),
        ("count", json!(33)),
        ("length", json!(-1)),
        ("width", json!("thin")),
        ("gap", json!(100)),
    ] {
        let mut v = full();
        v["shape"]["grip"] = json!({ key: bad });
        let found = errors(&v);
        assert!(found.iter().any(|e| e.contains(&format!("shape.grip.{key}")) || e.contains(key)), "{key} = {bad}: {found:?}");
    }
}
