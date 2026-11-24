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
    for (raw, expected) in [(b"(text 'kept') (base-direction rtl)".as_slice(), Direction::Rtl), (b"(text 'kept')", Direction::Auto)] {
        let scan = gimp(raw, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.records[0].interpreted_direction(), Ok(expected));
    }
}
