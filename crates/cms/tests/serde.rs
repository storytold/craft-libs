#![cfg(feature = "serde")]

use craft_cms::{Intent, TransformOptions};

#[test]
fn test_serde_intent_kebab_case() {
    let intents = [
        (Intent::Perceptual, "\"perceptual\""),
        (Intent::RelativeColorimetric, "\"relative-colorimetric\""),
        (Intent::Saturation, "\"saturation\""),
        (Intent::AbsoluteColorimetric, "\"absolute-colorimetric\""),
    ];

    for (intent, json) in intents {
        let serialized = serde_json::to_string(&intent).expect("serialize intent");
        assert_eq!(serialized, json);
        let deserialized: Intent = serde_json::from_str(json).expect("deserialize intent");
        assert_eq!(deserialized, intent);
    }
}

#[test]
fn test_serde_transform_options_defaults() {
    let empty_json = "{}";
    let opts: TransformOptions = serde_json::from_str(empty_json).expect("deserialize empty options");
    assert_eq!(opts, TransformOptions::default());

    let partial_json = r#"{"intent": "perceptual"}"#;
    let opts_partial: TransformOptions = serde_json::from_str(partial_json).expect("deserialize partial");
    assert_eq!(opts_partial.intent, Intent::Perceptual);
    assert!(opts_partial.bpc);
    assert!(opts_partial.precise_float);

    let full_json = r#"{"intent":"saturation","bpc":false,"precise-float":false}"#;
    let opts_full: TransformOptions = serde_json::from_str(full_json).expect("deserialize full");
    assert_eq!(opts_full.intent, Intent::Saturation);
    assert!(!opts_full.bpc);
    assert!(!opts_full.precise_float);

    let reserialized = serde_json::to_string(&opts_full).expect("serialize options");
    assert_eq!(reserialized, full_json);
}
