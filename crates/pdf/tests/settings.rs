//! The PDF export settings: the saved JSON shape, the checks that refuse what a writer can't honour,
//! the warnings, the built-in presets and the permission bits.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_pdf::settings::{
    Changes, ColorConversion, Compatibility, PdfPreset, PdfSettings, Printing, ProfileCatalog, ProfileInclusion, ProfileKind, SecuritySettings,
    Standard, builtin_preset, builtin_presets, default_preset_name,
};
use serde_json::json;

const APP: &str = "VectorCraft";
const CMYK: &str = "Coated CMYK";

/// A catalog that knows one CMYK profile.
struct Catalog;

impl ProfileCatalog for Catalog {
    fn kind(&self, name: &str) -> Option<ProfileKind> {
        (name == CMYK).then_some(ProfileKind::Cmyk)
    }
    fn rgb(&self) -> &str {
        ""
    }
    fn cmyk(&self) -> &str {
        CMYK
    }
}

fn settings(v: serde_json::Value) -> PdfSettings {
    serde_json::from_value(v).unwrap()
}

/// The default settings, edited by `edit`.
fn with(edit: impl FnOnce(&mut PdfSettings)) -> PdfSettings {
    let mut s = PdfSettings::default();
    edit(&mut s);
    s
}

fn security(open: &str, permissions: &str) -> SecuritySettings {
    SecuritySettings { open_password: open.into(), permissions_password: permissions.into(), ..Default::default() }
}

#[test]
fn json_is_camel_case_and_partial_objects_keep_defaults() {
    let d = PdfSettings::default();
    let v = serde_json::to_value(&d).unwrap();
    assert!(v["compression"]["compressText"].is_boolean());
    assert!(v["longPagesInPoints"].is_boolean());
    assert_eq!(serde_json::from_value::<PdfSettings>(v).unwrap(), d, "round-trips");

    let partial = settings(json!({"marks": {"trim": true}}));
    assert_eq!(partial, with(|s| s.marks.trim = true));
    let nested = settings(json!({"compression": {"gray": {"downsample": "bicubic"}}}));
    assert_ne!(nested.compression.gray.downsample, d.compression.gray.downsample);
    assert_eq!(nested.compression.color, d.compression.color);
    assert_eq!(settings(json!({"longPagesInPoints": !d.long_pages_in_points})), with(|s| s.long_pages_in_points = !d.long_pages_in_points));

    assert!(serde_json::from_value::<PdfSettings>(json!({"compatibility": "9.9"})).is_err(), "a bad choice id");
    assert!(serde_json::from_value::<PdfSettings>(json!({"thumbnails": "yes"})).is_err(), "a wrongly typed field");
}

#[test]
fn the_profiles_setting_can_be_left_unset() {
    let read = |v: serde_json::Value| serde_json::from_value::<PdfSettings>(json!({"output": {"profiles": v}}));
    assert_eq!(settings(json!({})).output.profiles, None, "a missing key");
    assert_eq!(settings(json!({"output": {}})).output.profiles, None);
    assert_eq!(read(json!(null)).unwrap().output.profiles, None);
    assert_eq!(read(json!("default")).unwrap().output.profiles, None);
    assert_eq!(read(json!("all")).unwrap().output.profiles, Some(ProfileInclusion::All));
    assert!(read(json!("bogus")).is_err());
    assert!(read(json!(5)).is_err());

    assert!(serde_json::to_value(PdfSettings::default()).unwrap()["output"]["profiles"].is_null());
    let set = with(|s| s.output.profiles = Some(ProfileInclusion::TaggedSource));
    assert_eq!(serde_json::from_value::<PdfSettings>(serde_json::to_value(&set).unwrap()).unwrap(), set);
}

#[test]
fn passwords_are_never_serialized() {
    let s = with(|s| s.security = security("open-secret", "owner-secret"));
    let text = serde_json::to_string(&s).unwrap();
    for leaked in ["open-secret", "owner-secret", "openPassword", "permissionsPassword"] {
        assert!(!text.contains(leaked), "{leaked}");
    }
}

#[test]
fn checks_refuse_what_a_writer_cant_honour() {
    let refused = |s: PdfSettings, id: &str| {
        let e = s.check(&()).unwrap_err();
        assert_eq!(e.id, id, "{}", e.message);
    };
    let at = |standard: Standard, compatibility: Compatibility| with(|s| (s.standard, s.compatibility) = (standard, compatibility));

    // A standard with a version it forbids.
    refused(at(Standard::PdfX4, Compatibility::Pdf17), "standardVersion");
    refused(at(Standard::PdfA2b, Compatibility::Pdf13), "standardVersion");
    refused(at(Standard::PdfA2b, Compatibility::Pdf20), "standardVersion");
    for standard in [Standard::PdfX1a, Standard::PdfX3] {
        for version in [Compatibility::Pdf13, Compatibility::Pdf14] {
            assert!(at(standard, version).check(&()).is_ok(), "{standard:?} at {version:?}");
        }
    }

    // Editing data and PDF layers.
    refused(with(|s| (s.standard, s.preserve_editing) = (Standard::PdfA2b, true)), "standardEditing");
    for standard in [Standard::PdfX1a, Standard::PdfX3] {
        refused(with(|s| (s.standard, s.compatibility, s.create_layers) = (standard, Compatibility::Pdf13, true)), "standardLayers");
    }
    assert!(with(|s| (s.standard, s.compatibility, s.create_layers) = (Standard::PdfX4, Compatibility::Pdf16, true)).check(&()).is_ok());

    // Passwords.
    for sec in [security("pw", ""), security("", "pw")] {
        refused(with(|s| (s.standard, s.compatibility, s.security) = (Standard::PdfX4, Compatibility::Pdf16, sec)), "standardPassword");
    }
    refused(with(|s| s.security = security("same", "same")), "passwordsEqual");
    refused(with(|s| (s.compatibility, s.security) = (Compatibility::Pdf16, security("pässword", ""))), "passwordLegacy");
    refused(with(|s| (s.compatibility, s.security) = (Compatibility::Pdf14, security(&"x".repeat(33), ""))), "passwordLegacy");
    refused(with(|s| s.security = security(&"x".repeat(128), "")), "passwordLong");

    // Ranges.
    refused(with(|s| s.compression.color.ppi = 5.0), "compressionPpi");
    refused(with(|s| s.bleed.top = 100.0), "bleedSide");
    refused(with(|s| s.marks.weight = 5.0), "marksWeight");
    refused(with(|s| s.advanced.font_subset_percent = 101.0), "fontSubsetPercent");

    // What needs the catalog.
    let intent = |standard: Standard, name: &str| {
        with(|s| (s.standard, s.compatibility, s.output.output_intent) = (standard, standard.version(), name.into()))
    };
    refused(intent(Standard::PdfX4, "Nope"), "intentUnknownX4");
    assert!(intent(Standard::PdfX4, CMYK).check(&Catalog).is_ok());
    let destination = |name: &str| {
        with(|s| {
            s.output.conversion = ColorConversion::Destination;
            s.output.destination = name.into();
        })
    };
    refused(destination("Nope"), "destinationUnknown");
    assert!(destination(CMYK).check(&Catalog).is_ok());
}

#[test]
fn options_not_applied_yet_come_back_as_setting_notes() {
    let ids = |s: PdfSettings| s.warnings(&Catalog).into_iter().map(|w| w.id).collect::<Vec<_>>();
    assert!(ids(PdfSettings::default()).is_empty());
    assert!(ids(with(|s| (s.standard, s.compatibility, s.output.output_intent) = (Standard::PdfX4, Compatibility::Pdf16, CMYK.into()))).is_empty());

    assert_eq!(ids(with(|s| (s.compatibility, s.create_layers) = (Compatibility::Pdf14, true))), ["layersVersion"]);
    assert_eq!(ids(with(|s| s.advanced.font_subset_percent = 50.0)), ["fontSubsetThreshold"]);
    assert_eq!(ids(with(|s| s.output.output_intent = "Nope".into())), ["intentNotEmbedded"]);
    assert_eq!(ids(with(|s| s.output.registry = "http://www.color.org".into())), ["conditionNeedsIntent"]);
    assert_eq!(ids(with(|s| (s.standard, s.output.trapped) = (Standard::PdfA2b, true))), ["pdfaOutputIntent"]);
    assert_eq!(ids(with(|s| s.security.copy = false)), ["permissionsNeedPassword"]);
}

#[test]
fn built_in_presets() {
    let all = builtin_presets(APP);
    assert_eq!(all[0].name, default_preset_name(APP));
    assert!(all[0].settings.preserve_editing, "the app default keeps the document editable");
    for p in &all {
        assert_eq!(p.settings.check_values(), Ok(()), "{}", p.name);
        assert!(!p.description.is_empty(), "{}", p.name);
        let back: PdfPreset = serde_json::from_value(serde_json::to_value(p).unwrap()).unwrap();
        assert_eq!(&back, p, "round-trips");
        if p.settings.standard.is_pdfx() {
            assert_eq!(p.settings.check(&()), Ok(()), "{}", p.name);
        }
    }
    assert!(all.iter().any(|p| p.settings.standard == Standard::PdfX4));
    assert_eq!(builtin_preset("default", APP), Some(all[0].clone()));
    assert_eq!(builtin_preset("no such preset", APP), None);
}

#[test]
fn permission_bits_follow_the_options() {
    let bits = |sec: SecuritySettings| sec.permission_bits() as u32;
    let bit = |n: u32| 1u32 << (n - 1);
    let reserved = 0xFFFF_F0C0;
    // Without a permissions password everything is allowed.
    assert_eq!(bits(SecuritySettings { printing: Printing::None, copy: false, ..security("open", "") }), 0xFFFF_FFFC);
    // Print (3), modify (4), copy (5), annotate (6), forms (9), accessibility (10), assemble (11),
    // high-quality print (12).
    assert_eq!(bits(security("", "p")), reserved | [3, 4, 5, 6, 9, 10, 11, 12].iter().map(|n| bit(*n)).sum::<u32>());
    let none = SecuritySettings { printing: Printing::None, changes: Changes::None, copy: false, screen_reader: false, ..security("", "p") };
    assert_eq!(bits(none.clone()), reserved);
    assert_eq!(bits(SecuritySettings { printing: Printing::Low, ..none.clone() }), reserved | bit(3));
    assert_eq!(bits(SecuritySettings { printing: Printing::High, ..none.clone() }), reserved | bit(3) | bit(12));
    for (changes, set) in [
        (Changes::Pages, bit(11)),
        (Changes::Forms, bit(9)),
        (Changes::Comments, bit(6) | bit(9)),
        (Changes::Any, bit(4) | bit(6) | bit(9) | bit(11)),
    ] {
        assert_eq!(bits(SecuritySettings { changes, ..none.clone() }), reserved | set, "{changes:?}");
    }
    assert_eq!(bits(SecuritySettings { screen_reader: true, ..none.clone() }), reserved | bit(10));
    assert_eq!(bits(SecuritySettings { copy: true, ..none }), reserved | bit(5) | bit(10), "copying lets screen readers read");
}
