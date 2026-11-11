//! The optional `serde` feature (#64): what documents and settings store for an intent and for
//! the conversion options, and what they read back.
#![cfg(feature = "serde")]

use craft_cms::{Intent, TransformOptions};

#[test]
fn an_intent_is_stored_under_the_name_the_crate_already_gives_it() {
    for intent in Intent::ALL {
        let json = serde_json::to_string(&intent).unwrap();
        assert_eq!(json, format!("\"{}\"", intent.id()), "{intent:?}");
        assert_eq!(serde_json::from_str::<Intent>(&json).unwrap(), intent);
    }
    assert_eq!(serde_json::to_string(&Intent::RelativeColorimetric).unwrap(), "\"relative\"");
}

#[test]
fn every_spelling_parse_accepts_reads_back() {
    for name in ["relative", "relativeColorimetric", "relative-colorimetric", "Relative Colorimetric", "colorimetric", "1"] {
        let got: Intent = serde_json::from_str(&format!("\"{name}\"")).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(got, Intent::RelativeColorimetric, "{name}");
    }
    for (name, want) in [
        ("perceptual", Intent::Perceptual),
        ("saturation", Intent::Saturation),
        ("absolute", Intent::AbsoluteColorimetric),
        ("absoluteColorimetric", Intent::AbsoluteColorimetric),
    ] {
        assert_eq!(serde_json::from_str::<Intent>(&format!("\"{name}\"")).unwrap(), want, "{name}");
    }
}

#[test]
fn an_unknown_intent_is_an_error_that_names_the_choices() {
    let e = serde_json::from_str::<Intent>("\"photographic\"").unwrap_err().to_string();
    assert!(e.contains("photographic") && e.contains("perceptual") && e.contains("absolute"), "{e}");
    // Not a string at all.
    assert!(serde_json::from_str::<Intent>("1").is_err(), "the ICC number has `Intent::from_u32`");
}

#[test]
fn transform_options_fill_missing_fields_from_their_defaults() {
    let d = TransformOptions::default();
    assert_eq!(serde_json::from_str::<TransformOptions>("{}").unwrap(), d, "an empty table is the default");
    let one = serde_json::from_str::<TransformOptions>(r#"{"bpc": false}"#).unwrap();
    assert_eq!(one, TransformOptions { bpc: false, ..d });
    let all = serde_json::from_str::<TransformOptions>(r#"{"intent": "perceptual", "bpc": false, "precise-float": false}"#).unwrap();
    assert_eq!(all, TransformOptions { intent: Intent::Perceptual, bpc: false, precise_float: false });
    // And what it writes reads back as itself, under the names it wrote.
    let json = serde_json::to_string(&all).unwrap();
    assert_eq!(json, r#"{"intent":"perceptual","bpc":false,"precise-float":false}"#);
    assert_eq!(serde_json::from_str::<TransformOptions>(&json).unwrap(), all);
}
