use craft_cms::{
    Builtin, CacheLimits, Curve, Intent, Mat3, Profile, Transform, TransformOptions, cache_limits, cache_stats, cached, clear_cache, set_cache_limits,
};

#[test]
fn test_curve_constructors() {
    // Gamma
    let g22 = Curve::gamma(2.2).expect("valid gamma");
    assert_eq!(g22, Curve::Gamma(2.2));

    let g10 = Curve::gamma(1.0).expect("gamma 1.0 is identity");
    assert_eq!(g10, Curve::Identity);

    assert!(Curve::gamma(0.0).is_err());
    assert!(Curve::gamma(-2.2).is_err());
    assert!(Curve::gamma(f64::NAN).is_err());
    assert!(Curve::gamma(f64::INFINITY).is_err());

    // Parametric
    let p0 = Curve::parametric(0, [2.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]).expect("valid parametric type 0");
    assert_eq!(p0, Curve::Parametric { kind: 0, p: [2.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] });

    assert!(Curve::parametric(5, [0.0; 7]).is_err());
    assert!(Curve::parametric(0, [f64::NAN, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]).is_err());

    // Sampled table
    let table = Curve::from_table(vec![0.0, 0.5, 1.0]).expect("valid sampled table");
    assert_eq!(table, Curve::Table(vec![0.0, 0.5, 1.0]));

    assert!(Curve::from_table(vec![0.5]).is_err());
    assert!(Curve::from_table(vec![0.0, f32::NAN]).is_err());
}

#[test]
fn test_profile_constructors() {
    // Matrix shaper profile with D65 and sRGB-like primaries
    let white_d65 = [0.95047, 1.0, 1.08883];
    let matrix: Mat3 = [[0.4360747, 0.3850649, 0.1430804], [0.2225045, 0.7168786, 0.0606169], [0.0139322, 0.0971045, 0.7141733]];
    let trc = [Curve::gamma(2.2).expect("curve"), Curve::gamma(2.2).expect("curve"), Curve::gamma(2.2).expect("curve")];

    let custom_rgb = Profile::matrix_shaper(white_d65, matrix, trc).expect("matrix shaper profile");
    assert_eq!(custom_rgb.color_space, craft_cms::ColorSpace::Rgb);

    let srgb = Builtin::Srgb.profile();
    let xform = Transform::new(&custom_rgb, srgb, Intent::RelativeColorimetric, true).expect("transform from custom RGB to sRGB");

    let mut out = [0.0f32; 3];
    xform.eval(&[0.5, 0.5, 0.5], &mut out).expect("eval");
    assert!(out[0] > 0.0 && out[0] < 1.0);

    // Invalid matrix shaper inputs
    assert!(Profile::matrix_shaper([0.0, 1.0, 1.0], matrix, [Curve::Identity, Curve::Identity, Curve::Identity]).is_err());
    assert!(Profile::matrix_shaper([f64::NAN, 1.0, 1.0], matrix, [Curve::Identity, Curve::Identity, Curve::Identity]).is_err());
    let singular: Mat3 = [[0.0; 3]; 3];
    assert!(Profile::matrix_shaper(white_d65, singular, [Curve::Identity, Curve::Identity, Curve::Identity]).is_err());

    // Gray profile
    let gray = Profile::gray(white_d65, Curve::gamma(2.2).expect("curve")).expect("gray profile");
    assert_eq!(gray.color_space, craft_cms::ColorSpace::Gray);

    let gray_to_srgb = Transform::new(&gray, srgb, Intent::RelativeColorimetric, true).expect("transform from gray to sRGB");
    let mut srgb_out = [0.0f32; 3];
    gray_to_srgb.eval(&[0.5], &mut srgb_out).expect("eval");
    assert!((srgb_out[0] - srgb_out[1]).abs() < 1e-3);
    assert!((srgb_out[1] - srgb_out[2]).abs() < 1e-3);
}

#[test]
fn test_lru_transform_cache() {
    clear_cache();
    let initial_limits = cache_limits();
    assert_eq!(cache_stats().entries, 0);
    assert_eq!(cache_stats().total_bytes, 0);

    let srgb = Builtin::Srgb.profile();
    let p3 = Builtin::DisplayP3.profile();
    let adobe = Builtin::AdobeRgbCompat.profile();
    let rec2020 = Builtin::Rec2020.profile();

    let opts = TransformOptions::default();

    // Check table_bytes calculation
    let t1 = Transform::with_options(srgb, p3, opts).expect("transform");
    assert!(t1.table_bytes() > 0);

    // Set cache limit to 2 entries
    set_cache_limits(CacheLimits { max_entries: 2, max_bytes: 64 * 1024 * 1024 });

    // Insert entry 1: sRGB -> P3
    let _ = cached(srgb, p3, opts).expect("cached 1");
    assert_eq!(cache_stats().entries, 1);
    assert!(cache_stats().total_bytes > 0);

    // Insert entry 2: sRGB -> Adobe
    let _ = cached(srgb, adobe, opts).expect("cached 2");
    assert_eq!(cache_stats().entries, 2);

    // Touch entry 1 by querying it again (makes entry 2 the LRU)
    let _ = cached(srgb, p3, opts).expect("cached 1 hit");
    assert_eq!(cache_stats().entries, 2);

    // Insert entry 3: sRGB -> Rec2020 (should evict entry 2)
    let _ = cached(srgb, rec2020, opts).expect("cached 3");
    assert_eq!(cache_stats().entries, 2);

    // Byte budget eviction test: set limit so only 1 transform fits
    let single_bytes = cache_stats().total_bytes / 2;
    set_cache_limits(CacheLimits { max_entries: 10, max_bytes: single_bytes + 10 });
    assert_eq!(cache_stats().entries, 1);

    // Clear cache
    clear_cache();
    assert_eq!(cache_stats().entries, 0);
    assert_eq!(cache_stats().total_bytes, 0);

    // Restore initial limits
    set_cache_limits(initial_limits);
}

#[cfg(feature = "serde")]
#[test]
fn test_serde_intent_and_transform_options() {
    // Intent serialization / deserialization with kebab-case
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

    // TransformOptions with default fallback for missing fields
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
