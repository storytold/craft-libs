use super::*;

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
        }
    }
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
