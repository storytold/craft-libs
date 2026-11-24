use super::*;

#[test]
fn registered_base_direction_names_keep_exact_source_and_automatic_policy() {
    for (name, expected) in [
        ("GIMP_TEXT_DIRECTION_LTR", Direction::Ltr),
        ("GIMP_TEXT_DIRECTION_RTL", Direction::Rtl),
        ("GIMP_TEXT_DIRECTION_TTB_RTL", Direction::TtbRtl),
        ("GIMP_TEXT_DIRECTION_TTB_RTL_UPRIGHT", Direction::TtbRtlUpright),
        ("GIMP_TEXT_DIRECTION_TTB_LTR", Direction::TtbLtr),
        ("GIMP_TEXT_DIRECTION_TTB_LTR_UPRIGHT", Direction::TtbLtrUpright),
    ] {
        let spelling = name;
        for automatic in [false, true] {
            let raw = format!("(text \"abc\") (base-direction {spelling}) (base-dir-auto {})", if automatic { "yes" } else { "no" });
            let scan = gimp(raw.as_bytes(), 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
            assert_eq!(scan.stopped, None, "{raw}");
            let record = &scan.records[0];
            assert_eq!(record.base_direction, Some(expected));
            assert_eq!(record.automatic_direction, Some(automatic));
            assert_eq!(record.direction, if automatic { Direction::Auto } else { expected });
            assert_eq!(record.config().unwrap().scalar("base-direction").unwrap(), Some(spelling));
            assert_eq!(record.raw, raw.as_bytes());
        }
    }
    for atom in ["GIMP_TEXT_DIRECTION_AUTO", "GIMP_TEXT_DIRECTION_RTL_EXTRA", "GIMP_TEXT_DIRECTION_", "OTHER_DIRECTION_RTL"] {
        let raw = format!("(base-direction {atom})");
        let scan = gimp(raw.as_bytes(), 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed, "{atom}");
    }
}

#[test]
fn legacy_ordinary_character_escapes_preserve_decoded_text_and_original_fields() {
    for (input, expected) in [("A\\}B", "A}B"), ("A\\qB", "AqB"), ("A\\nB", "A\nB"), ("A\\\\B", "A\\B")] {
        let raw = super::tests::legacy_bytes(input, "iso8859");
        let scan = gdyntext(&raw, 19, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        let record = &scan.records[0];
        assert_eq!(record.text().unwrap(), Some(expected));
        assert_eq!(record.raw, raw);
        let Source::Gdyntext(legacy) = &record.source else { panic!("legacy") };
        assert_eq!(legacy.fields[0], input);
        let span = legacy.spans[0];
        assert_eq!(&raw[span.start - 19..span.end - 19], input.as_bytes());
    }
    let modern = gimp(br#"(text "A\qB")"#, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(modern.stopped, None);
    assert_eq!(modern.records[0].text().unwrap(), Some("AqB"));
}

#[test]
fn registered_numeric_base_directions_match_nicknames_and_keep_source_atoms() {
    for (value, expected) in
        [Direction::Ltr, Direction::Rtl, Direction::TtbRtl, Direction::TtbRtlUpright, Direction::TtbLtr, Direction::TtbLtrUpright]
            .into_iter()
            .enumerate()
    {
        for spelling in [value.to_string(), format!("0{value:o}"), format!("0x{value:x}"), format!("0X{value:X}")] {
            for automatic in [false, true] {
                let raw = format!("(text \"abc\") (base-direction {spelling}) (base-dir-auto {})", if automatic { "yes" } else { "no" });
                let scan = gimp(raw.as_bytes(), 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
                assert_eq!(scan.stopped, None, "{raw}");
                let record = &scan.records[0];
                assert_eq!(record.base_direction, Some(expected));
                assert_eq!(record.automatic_direction, Some(automatic));
                assert_eq!(record.direction, if automatic { Direction::Auto } else { expected });
                assert_eq!(record.config().unwrap().scalar("base-direction").unwrap(), Some(spelling.as_str()));
            }
        }
    }
    for atom in ["6", "999", "0x100000000", "08", "0b1", "+1", "-0", "1.0", "\"1\"", "'1'"] {
        let raw = format!("(base-direction {atom})");
        let scan = gimp(raw.as_bytes(), 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed, "{atom}");
    }
    let raw = format!("{}1", "0".repeat(4096));
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        calls.get() < 2
    };
    assert!(matches!(integer_direction(&raw, &keep), Err(Failure::Cancelled)));
    assert_eq!(calls.get(), 2);
}

#[test]
fn bare_atoms_end_at_comments_but_quoted_hashes_remain_literal() {
    for raw in [b"(base-direction rtl#note\n) (text \"#literal\")".as_slice(), b"(base-direction rtl #note\n) (text '#literal')"] {
        let scan = gimp(raw, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        assert_eq!(scan.records[0].direction, Direction::Rtl);
        assert_eq!(scan.records[0].text().unwrap(), Some("#literal"));
    }
    let scan = gimp(b"(base-direction rtl#note)", 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed, "the comment consumes the closing delimiter without a newline");
}

#[test]
fn both_text_readers_decode_octal_as_a_byte_and_keep_lexical_source() {
    for (input, decoded, modern_text, legacy_text) in [
        (r"X\101Y", b"XAY".as_slice(), Some("XAY"), "XAY"),
        (r"X\501Y", b"XAY".as_slice(), Some("XAY"), "XAY"),
        (r"X\377Y", b"X\xffY".as_slice(), None, "X\u{ff}Y"),
        (r"X\400Y", b"X\0Y".as_slice(), Some("X"), "X"),
    ] {
        let modern = format!("(text \"{input}\")");
        let scan = gimp(modern.as_bytes(), 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        let record = &scan.records[0];
        assert_eq!(record.raw, modern.as_bytes());
        let atom = &record.config().unwrap().field("text").unwrap().unwrap()[0];
        assert_eq!(atom.bytes().unwrap(), decoded);
        let span = atom.span();
        assert_eq!(&modern[span.start..span.end], format!("\"{input}\""));
        if let Some(expected) = modern_text {
            assert_eq!(record.text().unwrap(), Some(expected));
        } else {
            assert!(record.text().is_err());
        }

        let legacy = super::tests::legacy_bytes(input, "iso8859");
        let scan = gdyntext(&legacy, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        let record = &scan.records[0];
        assert_eq!(record.raw, legacy);
        assert_eq!(record.text().unwrap(), Some(legacy_text));
        let Source::Gdyntext(fields) = &record.source else { panic!("legacy") };
        assert_eq!(fields.fields[0], input);
    }
}

#[test]
fn gdyntext_cancellation_at_publication_refunds_retention_only() {
    let raw = super::tests::legacy_bytes("valid text", "iso8859");
    let mut budget = Budget::default();
    let retained = budget.retained_remaining();
    let work = budget.work_remaining();
    let polls = std::cell::Cell::new(0);
    let keep = || {
        polls.set(polls.get() + 1);
        // One admission, a poll for each of the eight fields and their scans,
        // and one decoded-text poll precede the final publication check.
        polls.get() <= 18
    };
    assert!(matches!(gdyntext(&raw, 0, Limits::default(), &mut budget, &keep), Err(crate::Error::Cancelled)));
    assert_eq!(budget.retained_remaining(), retained);
    assert!(budget.work_remaining() < work);
    let scan = gdyntext(&raw, 0, Limits::default(), &mut budget, &|| true).unwrap();
    assert_eq!(scan.records[0].text().unwrap(), Some("valid text"));
}

#[test]
fn literal_legacy_nul_is_rejected_in_every_field_except_the_optional_terminator() {
    let original = super::tests::legacy_bytes("valid text", "iso8859");
    let scan = gdyntext(&original, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    let Source::Gdyntext(legacy) = &scan.records[0].source else { panic!("legacy") };
    let body = original.strip_prefix(b"GDT10{").unwrap().strip_suffix(b"}").unwrap();
    let starts = [6].into_iter().chain(body.windows(2).enumerate().filter_map(|(index, pair)| (pair == b"}{").then_some(index + 8)));
    let starts: Vec<_> = starts.collect();
    assert_eq!(starts.len(), 8);
    assert_eq!(legacy.spans.iter().map(|span| span.start).collect::<Vec<_>>(), starts);
    for start in starts {
        let mut raw = original.clone();
        raw.splice(start + 1..start + 1, b"\0junk".iter().copied());
        let scan = gdyntext(&raw, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(scan.records.is_empty());
        assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed);
    }
    let mut terminal = original;
    terminal.push(0);
    let scan = gdyntext(&terminal, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.stopped, None);
    assert_eq!(scan.records[0].text().unwrap(), Some("valid text"));
    terminal.push(0);
    let scan = gdyntext(&terminal, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed);
    assert!(matches!(legacy_separator(b"\0", &|| false), Err(Failure::Cancelled)));
}

#[test]
fn literal_nul_declines_modern_configuration_but_escaped_nul_remains_source_data() {
    for raw in [b"(text before\0after)".as_slice(), b"(text \"before\")\0(text \"after\")", b"(text 'before\0after')"] {
        let scan = gimp(raw, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(scan.records.is_empty());
        assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed);
    }
    let scan = gimp(b"(text \"kept\")\0", 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.records[0].text().unwrap(), Some("kept"));
    let scan = gimp(br#"(unknown "before\000after")"#, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.records[0].config().unwrap().field("unknown").unwrap().unwrap()[0].bytes().unwrap(), b"before\0after");
}

#[test]
fn literal_and_escaped_gdyntext_quotation_marks_have_identical_decoded_text() {
    for text in ["He said \"hello\"", "He said \\\"hello\\\""] {
        let raw = super::tests::legacy_bytes(text, "iso8859");
        let scan = gdyntext(&raw, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        assert_eq!(scan.records[0].text().unwrap(), Some("He said \"hello\""));
        assert_eq!(scan.records[0].raw, raw);
    }
    let scan = gimp(br#"(text "a\"b")"#, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.records[0].text().unwrap(), Some("a\"b"));
}

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
    assert_eq!(modern.stopped, None);
    assert_eq!(modern.records[0].text().unwrap(), Some("AvB"));
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
    cancelled_owned_text(false);
}

#[test]
fn cancelled_legacy_text_refunds_real_decoded_storage_and_allows_same_budget_retry() {
    cancelled_owned_text(true);
}

#[test]
fn public_gimp_cancellation_refunds_decoded_storage_and_retries_the_same_budget() {
    cancelled_public_owned_text(false);
}

#[test]
fn public_gdyntext_cancellation_refunds_decoded_storage_and_retries_the_same_budget() {
    cancelled_public_owned_text(true);
}

fn cancelled_public_owned_text(legacy: bool) {
    let source = |text: &str| {
        if legacy { super::tests::legacy_bytes(text, "iso8859") } else { format!("(text \"{text}\")").into_bytes() }
    };
    let read = |raw: &[u8], budget: &mut Budget, keep: &dyn Fn() -> bool| {
        // Discard returned borrows inside the call so the same helper works
        // with the independently allocated empty and long source fixtures.
        let scan = if legacy { gdyntext(raw, 37, Limits::default(), budget, keep) } else { gimp(raw, 37, Limits::default(), budget, keep) }?;
        Ok::<_, Error>((scan.stopped, scan.records.first().map(|record| record.text().unwrap().unwrap().to_owned())))
    };
    let mut empty_budget = Budget::default();
    let empty_storage = empty_budget.retained_remaining();
    assert_eq!(read(&source(""), &mut empty_budget, &|| true).unwrap(), (None, Some(String::new())));
    let empty_storage = empty_storage - empty_budget.retained_remaining();
    let text = "x".repeat(8192);
    let raw = source(&text);

    // Public behavioral phase oracle: enough storage for the entire empty
    // record plus 1 KiB still fails while retaining the long decoded value.
    // This passes every preliminary framing/NUL/separator scan. No callback
    // count from those scans is hard-coded into the cancellation test.
    let mut limited = Budget::new(128 << 20, empty_storage + 1024);
    let phase_calls = std::cell::Cell::new(0);
    let phase = read(&raw, &mut limited, &|| {
        phase_calls.set(phase_calls.get() + 1);
        true
    })
    .unwrap();
    assert_eq!(phase.0.unwrap().reason, StopReason::RetainedLimit);
    assert_eq!(phase.1, None);
    // Modern single-quoted values borrow their contents: the same allowance
    // completes the long record when decoded-byte storage is unnecessary.
    if !legacy {
        let borrowed = format!("(text '{text}')");
        let mut budget = Budget::new(128 << 20, empty_storage + 1024);
        assert_eq!(read(borrowed.as_bytes(), &mut budget, &|| true).unwrap(), (None, Some(text.clone())));
    }

    let mut budget = Budget::default();
    let retained = budget.retained_remaining();
    let work = budget.work_remaining();
    let calls = std::cell::Cell::new(0);
    let stop = phase_calls.get() + 1;
    let cancelled = read(&raw, &mut budget, &|| {
        calls.set(calls.get() + 1);
        calls.get() < stop
    });
    assert!(matches!(cancelled, Err(Error::Cancelled)));
    assert_eq!(calls.get(), stop, "cancel after the decoded-storage phase");
    assert_eq!(budget.retained_remaining(), retained, "the public reader must refund discarded decoded output");
    assert!(budget.work_remaining() < work, "attempted parsing remains charged");
    let work_after_cancel = budget.work_remaining();
    assert_eq!(read(&raw, &mut budget, &|| true).unwrap(), (None, Some(text)));
    assert!(budget.retained_remaining() < retained, "successful retry owns retained output");
    assert!(budget.work_remaining() < work_after_cancel);
}

fn cancelled_owned_text(legacy: bool) {
    let text = "x".repeat(4096);
    let raw = if legacy { super::tests::legacy_bytes(&text, "iso8859") } else { format!("(text \"{text}\")").into_bytes() };
    let mut budget = Budget::default();
    let retained = budget.retained_remaining();
    let work = budget.work_remaining();
    admit(&raw, Limits::default(), &mut budget, &|| true).unwrap_or_else(|_| panic!("fixture admitted"));
    let quoted = if legacy { text.clone() } else { format!("{text}\"") };
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        calls.get() < 2
    };
    let result = {
        // Enter the actual shared quoted decoder directly, after framing,
        // so new separator/NUL checks cannot move cancellation before any
        // allocation. Both public readers use the same finalizer below.
        let mut parser = Parser {
            bytes: quoted.as_bytes(),
            source_offset: 0,
            at: 0,
            checked_at: 0,
            nodes: 0,
            limits: Limits::default(),
            budget: &mut budget,
            keep_going: &keep,
        };
        let result = parser.quoted(!legacy);
        assert!(parser.budget.retained_remaining() < retained, "decoded storage really was charged before cancellation");
        result.map(|_| panic!("fixture must cancel during quoted decoding"))
    };
    let cancelled = finish(result, 0, retained, &mut budget);
    assert!(matches!(cancelled, Err(Error::Cancelled)));
    assert_eq!(budget.retained_remaining(), retained);
    assert!(budget.work_remaining() < work);
    let retry =
        if legacy { gdyntext(&raw, 0, Limits::default(), &mut budget, &|| true) } else { gimp(&raw, 0, Limits::default(), &mut budget, &|| true) }
            .unwrap();
    assert_eq!(retry.stopped, None);
    assert_eq!(retry.records[0].text().unwrap(), Some(text.as_str()));
}
