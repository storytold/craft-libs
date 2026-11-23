//! An opt-in primary numeric grammar never falls through to expression, relative or fallback parsing.

use craft_behavior::{Codec, NotAValue, NumberCodec, ParseNote, Reader};

fn absolute(text: &str) -> Option<f64> {
    text.parse().ok()
}

#[test]
fn codec_override_reads_complete_trimmed_values_absolutely() {
    let codec = NumberCodec::default().parse_with(|text| {
        assert_eq!(text, text.trim());
        absolute(text)
    });
    assert_eq!(codec.parse("  +12  ", &Some(50.0)), Ok(Some(12.0)));
    assert_eq!(codec.parse("-12", &Some(50.0)), Ok(Some(-12.0)));
    assert_eq!(codec.parse("  ", &Some(50.0)), Ok(None), "the existing empty-value convention stays intact");
    assert_eq!(codec.parse_tagged("12", &Some(50.0)), Ok((Some(12.0), ParseNote::default())));
    for text in ["1+2", "*2", "1.2.3", "12 widgets", "1 2", "1,2.3"] {
        assert_eq!(codec.parse(text, &Some(50.0)), Err(NotAValue), "reject the entire input {text:?}");
    }
}

#[test]
fn codec_override_rejection_never_falls_through_to_other_readers() {
    let codec = NumberCodec::new(Reader::new().unit("widgets", 2.0), 2).or_parse(|_| Some(999.0)).zoom_typing().parse_with(absolute);
    for text in ["1+2", "8/4", "2 widgets", "Auto", "1.2.3"] {
        assert_eq!(codec.parse(text, &Some(50.0)), Err(NotAValue), "no secondary reader may accept {text:?}");
    }
    assert_eq!(codec.parse("+12", &Some(50.0)), Ok(Some(12.0)));
    // Setter order does not silently replace the authoritative reader with the fallback reader.
    let codec = NumberCodec::default().parse_with(absolute).or_parse(|_| Some(999.0));
    assert_eq!(codec.parse("Auto", &None), Err(NotAValue));
}

#[test]
fn codec_override_rejects_nonfinite_callback_results() {
    let codec = NumberCodec::default().parse_with(absolute);
    for text in ["NaN", "inf", "-inf", "1e9999"] {
        assert_eq!(codec.parse(text, &None), Err(NotAValue), "non-finite {text:?}");
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let codec = NumberCodec::default().parse_with(move |_| Some(value));
        assert_eq!(codec.parse("anything", &None), Err(NotAValue));
    }
}

#[test]
fn codec_override_preserves_formatting_filter_equality_and_all_default_grammars() {
    let original = NumberCodec::new(Reader::new().decimal(','), 2).fixed().zoom_typing();
    let custom = original.clone().parse_with(absolute);
    assert_eq!(original.format(&Some(12.5)), custom.format(&Some(12.5)));
    assert_eq!(custom.format(&Some(12.5)), "12,50");
    for c in ['1', ',', '/', '+', '-', '*', 'w'] {
        assert_eq!(original.filter(c), custom.filter(c), "filter {c}");
    }
    assert_eq!(custom, custom.clone());
    assert_ne!(original, custom);
    let default = NumberCodec::default();
    assert_eq!(default.parse("+12", &Some(50.0)), Ok(Some(62.0)));
    assert_eq!(default.parse("1+2", &Some(50.0)), Ok(Some(3.0)));
    assert_eq!(default.clone().or_parse(|text| (text == "Auto").then_some(99.0)).parse("Auto", &None), Ok(Some(99.0)));
    assert_eq!(default.zoom_typing().parse("8/4", &None), Ok(Some(200.0)));
}
