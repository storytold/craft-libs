use super::*;

#[test]
fn single_quoted_configuration_strings_are_literal_and_keep_lexical_spans() {
    let raw = br#"(text 'hello world\n "quoted"') (font 'Example Family')"#;
    let scan = gimp(raw, 17, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.stopped, None);
    let record = &scan.records[0];
    assert_eq!(record.text().unwrap(), Some("hello world\\n \"quoted\""));
    let atom = &record.config().unwrap().field("text").unwrap().unwrap()[0];
    assert!(matches!(atom, Value::Atom { bytes: Cow::Borrowed(_), quoted: true, .. }));
    let span = atom.span();
    assert_eq!(&raw[span.start - 17..span.end - 17], br#"'hello world\n "quoted"'"#);
    let double = gimp(br#"(text "hello\nworld")"#, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(double.records[0].text().unwrap(), Some("hello\nworld"));
    let malformed = gimp(b"(text 'unclosed)", 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(malformed.stopped.unwrap().reason, StopReason::Malformed);
}

#[test]
fn equivalent_auto_boolean_spellings_preserve_direction_and_saved_policy() {
    for (spellings, automatic) in [(["yes", "true", "TRUE", "YeS"], true), (["no", "false", "FALSE", "No"], false)] {
        for spelling in spellings {
            let raw = format!("(text 'hello') (base-direction ttb-rtl-upright) (base-dir-auto {spelling})");
            let scan = gimp(raw.as_bytes(), 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
            assert_eq!(scan.stopped, None, "{spelling}");
            let record = &scan.records[0];
            assert_eq!(record.base_direction, Some(Direction::TtbRtlUpright));
            assert_eq!(record.automatic_direction, Some(automatic));
            assert_eq!(record.direction, if automatic { Direction::Auto } else { Direction::TtbRtlUpright });
        }
    }
    for spelling in ["maybe", "1", "0"] {
        let raw = format!("(base-dir-auto {spelling})");
        let scan = gimp(raw.as_bytes(), 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed);
    }
}

#[test]
fn equivalent_legacy_escape_spellings_decode_identically_without_losing_source() {
    for content in [b"\\303\\251".to_vec(), [vec![0xc3], b"\\251".to_vec()].concat()] {
        let mut raw = super::tests::legacy_bytes("?", "iso8859");
        raw.splice(6..7, content.iter().copied());
        let scan = gdyntext(&raw, 10, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        let record = &scan.records[0];
        assert_eq!(record.text().unwrap(), Some("é"));
        assert_eq!(record.raw, raw);
        let Source::Gdyntext(legacy) = &record.source else { panic!("legacy") };
        assert_eq!(legacy.encoding, LegacyEncoding::Utf8);
        let span = legacy.spans[0];
        assert_eq!(&raw[span.start - 10..span.end - 10], content);
    }
}

#[test]
fn legacy_separator_scans_poll_in_missing_and_final_fields() {
    let bytes = vec![b'x'; 8192];
    for tail in [bytes.clone(), [bytes, b"}{next".to_vec()].concat()] {
        let calls = std::cell::Cell::new(0);
        let keep = || {
            calls.set(calls.get() + 1);
            calls.get() < 2
        };
        assert!(matches!(legacy_separator(&tail, &keep), Err(Failure::Cancelled)));
        assert_eq!(calls.get(), 2);
    }
    let missing = format!("GDT10{{{}}}", "x".repeat(8192)).into_bytes();
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        calls.get() < 3
    };
    assert!(matches!(gdyntext(&missing, 0, Limits::default(), &mut Budget::default(), &keep), Err(Error::Cancelled)));
    let malformed = gdyntext(&missing, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(malformed.stopped.unwrap().reason, StopReason::Malformed);
}

#[test]
fn explicit_vertical_directions_remain_distinct_from_automatic_direction() {
    for (name, expected) in [
        ("ttb-rtl", Direction::TtbRtl),
        ("ttb-rtl-upright", Direction::TtbRtlUpright),
        ("ttb-ltr", Direction::TtbLtr),
        ("ttb-ltr-upright", Direction::TtbLtrUpright),
    ] {
        for suffix in ["", " (base-dir-auto no)", " (base-dir-auto yes)"] {
            let raw = format!("(text \"vertical\") (base-direction {name}){suffix}");
            let scan = gimp(raw.as_bytes(), 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
            assert_eq!(scan.stopped, None);
            assert_eq!(scan.records[0].direction, if suffix.ends_with("yes)") { Direction::Auto } else { expected });
            assert_eq!(scan.records[0].base_direction, Some(expected));
            assert_eq!(scan.records[0].automatic_direction, if suffix.is_empty() { None } else { Some(suffix.ends_with("yes)")) });
        }
    }
}

#[test]
fn source_direction_absence_is_distinct_from_an_explicit_paragraph_policy() {
    for (source, direction, base, automatic) in [
        ("", Direction::Auto, None, None),
        ("(base-dir-auto no)", Direction::Ltr, None, Some(false)),
        ("(base-dir-auto yes)", Direction::Auto, None, Some(true)),
        ("(base-direction rtl) (base-dir-auto yes)", Direction::Auto, Some(Direction::Rtl), Some(true)),
    ] {
        let scan = gimp(source.as_bytes(), 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        let record = &scan.records[0];
        assert_eq!((record.direction, record.base_direction, record.automatic_direction), (direction, base, automatic));
    }
    let raw = super::tests::legacy_bytes("horizontal legacy", "iso8859");
    let scan = gdyntext(&raw, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    let record = &scan.records[0];
    assert_eq!((record.direction, record.base_direction, record.automatic_direction), (Direction::Auto, None, None));
}

#[test]
fn gdyntext_vertical_tab_is_legacy_only() {
    let raw = super::tests::legacy_bytes("A\\vB", "iso8859");
    let scan = gdyntext(&raw, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.stopped, None);
    assert_eq!(scan.records[0].text().unwrap(), Some("A\u{b}B"));
    let modern = gimp(br#"(text "A\vB")"#, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(modern.stopped.unwrap().reason, StopReason::Malformed);
}

#[test]
fn raw_latin1_text_uses_its_declared_encoding_and_keeps_original_spans() {
    for charset in ["iso8859", "unknown"] {
        let mut raw = super::tests::legacy_bytes("caf?", charset);
        let index = raw.iter().position(|byte| *byte == b'?').unwrap();
        raw[index] = 0xe9;
        let scan = gdyntext(&raw, 31, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        if charset == "unknown" {
            assert!(scan.records.is_empty());
            assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed);
            continue;
        }
        assert_eq!(scan.stopped, None);
        let record = &scan.records[0];
        assert_eq!(record.text().unwrap(), Some("café"));
        assert_eq!(record.raw, raw);
        let Source::Gdyntext(legacy) = &record.source else { panic!("legacy") };
        assert_eq!(legacy.encoding, LegacyEncoding::Latin1);
        assert_eq!(legacy.fields[0], "café");
        assert!(matches!(legacy.fields[0], Cow::Owned(_)));
        assert!(legacy.fields.iter().skip(1).all(|field| matches!(field, Cow::Borrowed(_))));
        let span = legacy.spans[0];
        assert_eq!(&raw[span.start - 31..span.end - 31], b"caf\xe9");
    }
    let mut raw = super::tests::legacy_bytes("valid", "iso8859");
    let metadata = raw.windows(3).position(|window| window == b"}{1").unwrap() + 2;
    raw[metadata] = 0xe9;
    let scan = gdyntext(&raw, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed);
}

#[test]
fn cancelled_text_releases_discarded_storage_but_keeps_work_charged() {
    for legacy in [false, true] {
        let text = "x".repeat(4096);
        let raw = if legacy { super::tests::legacy_bytes(&text, "iso8859") } else { format!("(text \"{text}\")").into_bytes() };
        let mut budget = Budget::default();
        let retained = budget.retained_remaining();
        let work = budget.work_remaining();
        let calls = std::cell::Cell::new(0);
        let keep = || {
            calls.set(calls.get() + 1);
            calls.get() < if legacy { 10 } else { 4 }
        };
        let cancelled =
            if legacy { gdyntext(&raw, 0, Limits::default(), &mut budget, &keep) } else { gimp(&raw, 0, Limits::default(), &mut budget, &keep) };
        assert!(matches!(cancelled, Err(Error::Cancelled)));
        assert_eq!(budget.retained_remaining(), retained);
        assert!(budget.work_remaining() < work);
        let retry = if legacy {
            gdyntext(&raw, 0, Limits::default(), &mut budget, &|| true)
        } else {
            gimp(&raw, 0, Limits::default(), &mut budget, &|| true)
        }
        .unwrap();
        assert_eq!(retry.stopped, None);
        assert_eq!(retry.records[0].text().unwrap(), Some(text.as_str()));
    }
}
