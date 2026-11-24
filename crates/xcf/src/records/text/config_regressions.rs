use super::*;

#[test]
fn numeric_radices_preserve_source_and_decimal_fraction_interpretation() {
    for (literal, expected) in [
        ("010", 8.0),
        ("-010", -8.0),
        ("+010", 8.0),
        ("0x10", 16.0),
        ("-0Xf", -15.0),
        ("010.5", 10.5),
        ("010e2", 1000.0),
        ("\"010\"", 10.0),
        ("0", 0.0),
    ] {
        let raw = format!("(font-size {literal})");
        let scan = gimp(raw.as_bytes(), 71, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        let record = &scan.records[0];
        let config = record.config().unwrap();
        assert_eq!(config.number("font-size").unwrap(), Some(expected), "{literal}");
        let span = config.field("font-size").unwrap().unwrap()[0].span();
        assert_eq!(&raw.as_bytes()[span.start - 71..span.end - 71], literal.as_bytes());
        assert_eq!(record.raw, raw.as_bytes());
    }
    for literal in ["09", "0x", "0xg", "0x+1", "0xfffffffffffffffff", "NaN", "inf"] {
        let raw = format!("(font-size {literal})");
        let scan = gimp(raw.as_bytes(), 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.records[0].config().unwrap().number("font-size"), Err(FieldError::InvalidNumber), "{literal}");
    }
}

#[test]
fn block_comments_separate_tokens_but_quoted_markers_stay_literal() {
    let raw = br#"/* prolog */ (text/* inside */"kept /* literal */ # too")/*end*/(font-size 010/* radix */)"#;
    let scan = gimp(raw, 29, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.stopped, None);
    let record = &scan.records[0];
    assert_eq!(record.text().unwrap(), Some("kept /* literal */ # too"));
    assert_eq!(record.config().unwrap().number("font-size").unwrap(), Some(8.0));
    assert_eq!(record.raw, raw);
    for raw in [b"(text \"allocated\") /* missing".as_slice(), b"/*", b"/**"] {
        let mut budget = Budget::default();
        let retained = budget.retained_remaining();
        let work = budget.work_remaining();
        let scan = gimp(raw, 0, Limits::default(), &mut budget, &|| true).unwrap();
        assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed);
        assert!(scan.records.is_empty());
        assert_eq!(budget.retained_remaining(), retained);
        assert!(budget.work_remaining() < work);
    }
}

#[test]
fn block_comment_scanning_cancels_inside_scan_and_obeys_source_cap() {
    let raw = format!("/*{}*/(text 'kept')", "x".repeat(8192));
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        calls.get() < 2
    };
    let mut budget = Budget::default();
    // Enter the actual comment scanner so prevalidation cannot satisfy the test.
    let mut parser = Parser {
        bytes: raw.as_bytes(),
        source_offset: 0,
        at: 0,
        checked_at: 0,
        nodes: 0,
        limits: Limits::default(),
        budget: &mut budget,
        keep_going: &keep,
    };
    assert!(matches!(parser.space(), Err(Failure::Cancelled)));
    assert!(parser.at >= 256 && parser.at < raw.len() - 2);
    assert_eq!(calls.get(), 2);
    let scan = gimp(raw.as_bytes(), 0, Limits { bytes: raw.len() - 1, ..Limits::default() }, &mut budget, &|| true).unwrap();
    assert_eq!(scan.stopped.unwrap().reason, StopReason::PayloadLimit);
}

#[test]
fn modern_ordinary_escapes_keep_decoded_text_and_lexical_source() {
    let raw = br#"(text "A\qB\vC\nD\tE\\F\"G\303\251")"#;
    let scan = gimp(raw, 13, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.stopped, None);
    let record = &scan.records[0];
    assert_eq!(record.text().unwrap(), Some("AqBvC\nD\tE\\F\"G\u{e9}"));
    assert_eq!(record.raw, raw);
    let span = record.config().unwrap().field("text").unwrap().unwrap()[0].span();
    assert_eq!(&raw[span.start - 13..span.end - 13], &raw[6..raw.len() - 1]);
    for raw in [br#"(text "unfinished\q)"#.as_slice(), br#"(text "unfinished\"#] {
        let scan = gimp(raw, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed);
    }
}

#[test]
fn duplicate_directions_keep_ordered_forms_and_refuse_checked_interpretation() {
    for (fields, name) in [
        ("(base-direction ltr) (base-direction rtl)", "base-direction"),
        ("(base-dir-auto yes) (base-dir-auto no)", "base-dir-auto"),
        ("(base-direction rtl) (base-dir-auto yes) (base-dir-auto no)", "base-dir-auto"),
    ] {
        let raw = format!("(text 'kept') {fields} (font-size 12)");
        let scan = gimp(raw.as_bytes(), 31, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        let record = &scan.records[0];
        assert_eq!(record.text().unwrap(), Some("kept"));
        assert_eq!(record.interpreted_direction(), Err(FieldError::Duplicate));
        assert_eq!(record.direction, Direction::Auto);
        assert_eq!(record.raw, raw.as_bytes());
        let config = record.config().unwrap();
        assert_eq!(config.field(name).unwrap_err(), FieldError::Duplicate);
        assert_eq!(config.number("font-size").unwrap(), Some(12.0));
        let names: Vec<_> = config.forms.iter().map(|v| v.list().unwrap()[0].text().unwrap()).collect();
        assert_eq!(names.iter().filter(|n| **n == name).count(), 2);
    }
    for (raw, expected) in [
        (b"(text 'kept') (base-direction rtl) (base-dir-auto no)".as_slice(), Direction::Rtl),
        (b"(text 'kept') (base-dir-auto yes)", Direction::Auto),
    ] {
        let scan = gimp(raw, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.records[0].interpreted_direction(), Ok(Some(expected)));
    }
}

#[test]
fn duplicated_direction_does_not_hide_invalid_independent_or_repeated_values() {
    for fields in [
        "(base-direction sideways) (base-dir-auto yes) (base-dir-auto no)",
        "(base-direction ltr) (base-direction rtl) (base-dir-auto maybe)",
        "(base-direction ltr) (base-direction sideways)",
        "(base-dir-auto yes) (base-dir-auto maybe)",
        "(base-direction (rtl)) (base-dir-auto yes) (base-dir-auto no)",
        "(base-direction rtl) (base-direction ltr) (base-dir-auto)",
    ] {
        let raw = format!("(text 'retained before failure') {fields}");
        let mut budget = Budget::default();
        let retained = budget.retained_remaining();
        let work = budget.work_remaining();
        let scan = gimp(raw.as_bytes(), 0, Limits::default(), &mut budget, &|| true).unwrap();
        assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed, "{fields}");
        assert!(scan.records.is_empty());
        assert_eq!(budget.retained_remaining(), retained);
        assert!(budget.work_remaining() < work);
    }
}

#[test]
fn separated_minus_requires_exactly_one_bare_unsigned_numeric_atom() {
    for (literal, expected) in [("- 3", -3.0), ("- /* gap */ 010", -8.0), ("- 0x10", -16.0), ("- .5", -0.5), ("- 1e2", -100.0), ("-3", -3.0)] {
        let raw = format!("(offset-x {literal})");
        let scan = gimp(raw.as_bytes(), 17, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        let record = &scan.records[0];
        assert_eq!(record.config().unwrap().number("offset-x"), Ok(Some(expected)), "{literal}");
        assert_eq!(record.raw, raw.as_bytes());
        let values = record.config().unwrap().field("offset-x").unwrap().unwrap();
        if literal.starts_with("- ") {
            assert_eq!(values.len(), 2);
            assert_eq!(values[0].text().unwrap(), "-");
        }
    }
    for literal in ["- -3", "- +3", "\"-\" 3", "- '3'", "- 3 4", "- words", "- inf"] {
        let raw = format!("(offset-x {literal})");
        let scan = gimp(raw.as_bytes(), 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(scan.records[0].config().unwrap().number("offset-x").is_err(), "{literal}");
    }
}

#[test]
fn typed_modern_text_terminates_at_nul_without_losing_the_full_atom() {
    for (literal, prefix, atom) in [
        (br#""A\000B""#.as_slice(), "A", b"A\0B".as_slice()),
        (br#""A\000\377""#, "A", b"A\0\xff"),
        (br#""\000after""#, "", b"\0after"),
        (br#""\303\251\000B""#, "\u{e9}", b"\xc3\xa9\0B"),
        (br#""unchanged""#, "unchanged", b"unchanged"),
    ] {
        let raw = [b"(text ".as_slice(), literal, b")"].concat();
        let scan = gimp(&raw, 23, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        let record = &scan.records[0];
        assert_eq!(record.text(), Ok(Some(prefix)));
        let value = &record.config().unwrap().field("text").unwrap().unwrap()[0];
        assert_eq!(value.bytes().unwrap(), atom);
        let span = value.span();
        assert_eq!(&raw[span.start - 23..span.end - 23], literal);
        assert_eq!(record.raw, raw);
    }
    let scan = gimp(br#"(text "\377\000valid tail")"#, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.records[0].text(), Err(FieldError::InvalidUtf8));
}

#[test]
fn omitted_automatic_policy_is_unresolved_without_producer_context() {
    for (fields, base, expected) in [
        ("", None, None),
        ("(base-direction rtl)", Some(Direction::Rtl), None),
        ("(base-direction rtl) (base-dir-auto yes)", Some(Direction::Rtl), Some(Direction::Auto)),
        ("(base-direction rtl) (base-dir-auto no)", Some(Direction::Rtl), Some(Direction::Rtl)),
        ("(base-dir-auto no)", None, Some(Direction::Ltr)),
    ] {
        let raw = format!("(text 'abc') {fields}");
        let scan = gimp(raw.as_bytes(), 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.records[0].base_direction, base);
        assert_eq!(scan.records[0].interpreted_direction(), Ok(expected));
    }
    let legacy = super::tests::legacy_bytes("abc", "iso8859");
    let scan = gdyntext(&legacy, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
    assert_eq!(scan.records[0].interpreted_direction(), Ok(Some(Direction::Auto)));
}

#[test]
fn direction_scan_cancels_after_parse_and_refunds_discarded_storage() {
    let raw = format!("(text 'allocated') {}", "(unknown 1) ".repeat(2048));
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        true
    };
    let scan = gimp(raw.as_bytes(), 0, Limits::default(), &mut Budget::default(), &keep).unwrap();
    let total = calls.get();
    let config = scan.records[0].config().unwrap();
    let inside_calls = std::cell::Cell::new(0);
    let stop_inside = || {
        inside_calls.set(inside_calls.get() + 1);
        inside_calls.get() < 2
    };
    assert!(matches!(direction(config, &stop_inside), Err(Failure::Cancelled)));
    assert_eq!(inside_calls.get(), 2, "the scan polls again after the first 64 forms");
    let mut budget = Budget::default();
    let retained = budget.retained_remaining();
    let work = budget.work_remaining();
    calls.set(0);
    let cancel_at_final_direction_poll = || {
        calls.set(calls.get() + 1);
        calls.get() < total
    };
    assert!(matches!(gimp(raw.as_bytes(), 0, Limits::default(), &mut budget, &cancel_at_final_direction_poll), Err(Error::Cancelled)));
    assert_eq!(calls.get(), total);
    assert_eq!(budget.retained_remaining(), retained);
    assert!(budget.work_remaining() < work);
    let retry = gimp(raw.as_bytes(), 0, Limits::default(), &mut budget, &|| true).unwrap();
    assert_eq!(retry.records[0].text(), Ok(Some("allocated")));
}
