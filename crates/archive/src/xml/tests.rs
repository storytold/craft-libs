//! Tests for the XML nesting and DTD check.

use crate::xml::{self, Depth, Dtd, Error, Limits};

fn limits(max_depth: usize, dtd: Dtd) -> Limits {
    Limits { max_depth, dtd }
}

/// A document nested `depth` elements deep.
fn nested(depth: usize) -> Vec<u8> {
    let mut text = String::new();
    for index in 0..depth {
        text.push_str(&format!("<e{index}>"));
    }
    for index in (0..depth).rev() {
        text.push_str(&format!("</e{index}>"));
    }
    text.into_bytes()
}

#[test]
fn a_document_at_the_limit_passes_and_one_level_more_fails() {
    let accepting = limits(8, Dtd::Refuse);
    assert!(xml::check(&nested(7), &accepting).is_ok());
    assert!(xml::check(&nested(8), &accepting).is_ok(), "a document nested exactly to the limit is accepted");
    assert_eq!(xml::check(&nested(9), &accepting), Err(Error::TooDeep { depth: 9, limit: 8 }));

    // The limit is the document's deepest nesting, not its element count: siblings are free.
    let wide = format!("<r>{}</r>", "<e/>".repeat(10_000));
    assert!(xml::check(wide.as_bytes(), &limits(2, Dtd::Refuse)).is_ok());

    // A limit of zero refuses any start tag, and a self-closing tag still opens one.
    assert_eq!(xml::check(b"<r/>", &limits(0, Dtd::Refuse)), Err(Error::TooDeep { depth: 1, limit: 0 }));
    assert_eq!(xml::check(b"<r>", &limits(0, Dtd::Refuse)), Err(Error::TooDeep { depth: 1, limit: 0 }));
    assert!(xml::check(b"text only", &limits(0, Dtd::Refuse)).is_ok());

    // A self-closing tag and its spaced form both still occupy one level.
    assert!(xml::check(b"<r><e/></r>", &limits(2, Dtd::Refuse)).is_ok());
    assert_eq!(xml::check(b"<r><e/></r>", &limits(1, Dtd::Refuse)), Err(Error::TooDeep { depth: 2, limit: 1 }));
    assert_eq!(xml::check(b"<r><e /></r>", &limits(1, Dtd::Refuse)), Err(Error::TooDeep { depth: 2, limit: 1 }));
    assert!(xml::check(b"<r><e a=\"1\" /></r>", &limits(2, Dtd::Refuse)).is_ok());
}

#[test]
fn comments_cdata_processing_instructions_and_quotes_cannot_fake_depth() {
    let deep = "<a>".repeat(200);
    let decoys = format!(
        "<?xml version=\"1.0\"?><r><!-- {deep} --><![CDATA[{deep}]]><?pi {deep}?><e x=\"{deep}\" y='{deep}'/>
         <f z=\"a &gt; b\"/></r>"
    );
    assert!(xml::check(decoys.as_bytes(), &limits(3, Dtd::Refuse)).is_ok());

    // A quoted `>` does not end a tag, so the markup after it is text, not elements.
    assert!(xml::check(b"<a x=\"<b><c>\">text</a>", &limits(1, Dtd::Refuse)).is_ok());
}

#[test]
fn malformed_markup_ends_the_scan_without_looping_or_panicking() {
    let accepting = limits(16, Dtd::Refuse);
    for text in [
        &b"<"[..],
        b"</",
        b"<!-- never closed",
        b"<![CDATA[ never closed",
        b"<?pi never closed",
        b"<a x=\"",
        b"<a",
        b"<a><b></a>",
        b"\xff\xfe\x00\x00",
        b"",
    ] {
        assert!(xml::check(text, &accepting).is_ok() || xml::check(text, &accepting).is_err());
    }
}

#[test]
fn entity_references_and_doctype_declarations_do_not_count_as_elements() {
    let text = b"<!DOCTYPE r [<!ENTITY e \"<a>\">]><r>&e;&#60;a&#62;</r>";
    assert!(xml::check(text, &limits(1, Dtd::InternalSubset { max_bytes: 1_000 })).is_ok());
}

#[test]
fn a_refused_doctype_is_refused_whether_or_not_it_has_a_subset() {
    let refusing = limits(16, Dtd::Refuse);
    assert_eq!(xml::check(b"<!DOCTYPE r><r/>", &refusing), Err(Error::DtdRefused));
    assert_eq!(xml::check(b"<!DOCTYPE r [<!ENTITY e \"x\">]><r>&e;</r>", &refusing), Err(Error::DtdRefused));
    assert_eq!(xml::check(b"<!doctype r><r/>", &refusing), Err(Error::DtdRefused), "the keyword is matched ignoring case");
    assert!(!b"<!DOCX r>".eq_ignore_ascii_case(b"<!doctype r>"), "the fixture is not a declaration");
    assert!(xml::check(b"<!DOCKTYPE r><r/>", &refusing).is_ok(), "another declaration is not a doctype");
    assert!(xml::check(b"<r/>", &refusing).is_ok());
}

#[test]
fn a_declaration_without_a_subset_is_allowed_but_a_subset_is_not() {
    let declaration_only = limits(16, Dtd::DeclarationOnly);
    assert!(xml::check(b"<!DOCTYPE r><r/>", &declaration_only).is_ok());
    assert!(xml::check(b"<!DOCTYPE r SYSTEM \"r.dtd\"><r/>", &declaration_only).is_ok());
    assert_eq!(xml::check(b"<!DOCTYPE r [<!ENTITY e \"x\">]><r/>", &declaration_only), Err(Error::InternalSubsetNotAllowed));
    // An empty subset is still a subset.
    assert_eq!(xml::check(b"<!DOCTYPE r []><r/>", &declaration_only), Err(Error::InternalSubsetNotAllowed));
}

#[test]
fn an_internal_subset_inside_its_byte_cap_is_allowed() {
    let subset = b"<!ENTITY e \"0123456789\">";
    let document = format!("<!DOCTYPE r [{}]><r>&e;</r>", String::from_utf8_lossy(subset));
    let inside = limits(16, Dtd::InternalSubset { max_bytes: u64::try_from(subset.len()).unwrap() });
    assert!(xml::check(document.as_bytes(), &inside).is_ok());

    let outside = limits(16, Dtd::InternalSubset { max_bytes: u64::try_from(subset.len()).unwrap() - 1 });
    assert_eq!(
        xml::check(document.as_bytes(), &outside),
        Err(Error::InternalSubsetTooLarge { actual: u64::try_from(subset.len()).unwrap(), limit: u64::try_from(subset.len()).unwrap() - 1 })
    );

    // A zero cap admits no subset at all.
    assert!(xml::check(b"<!DOCTYPE r><r/>", &limits(16, Dtd::InternalSubset { max_bytes: 0 })).is_ok());
    assert_eq!(
        xml::check(b"<!DOCTYPE r [x]><r/>", &limits(16, Dtd::InternalSubset { max_bytes: 0 })),
        Err(Error::InternalSubsetTooLarge { actual: 1, limit: 0 })
    );
}

#[test]
fn a_subset_containing_markup_is_charged_as_bytes_not_as_nesting() {
    // VectorCraft counts markup inside entity declarations; this scan does not, so the size cap is
    // what bounds a subset full of tags. The document itself is only one element deep.
    let subset = format!("<!ENTITY e \"{}\">", "<a>".repeat(1_000));
    let document = format!("<!DOCTYPE r [{subset}]><r/>");
    assert_eq!(xml::check(document.as_bytes(), &limits(1, Dtd::InternalSubset { max_bytes: 1_000_000 })).unwrap(), ());
    assert!(matches!(xml::check(document.as_bytes(), &limits(1, Dtd::InternalSubset { max_bytes: 100 })), Err(Error::InternalSubsetTooLarge { .. })));
}

#[test]
fn a_comment_inside_a_declaration_does_not_end_it() {
    // A `>`, a `[` or a `]` inside a comment or a quoted value must not close the declaration.
    let text = b"<!DOCTYPE r [<!-- ]]> --><!ENTITY e \"a>b\">]><r/>";
    assert!(xml::check(text, &limits(4, Dtd::InternalSubset { max_bytes: 1_000 })).is_ok());
    assert_eq!(xml::check(text, &limits(4, Dtd::DeclarationOnly)), Err(Error::InternalSubsetNotAllowed));
}

#[test]
fn only_the_doctype_is_looked_for_outside_markup() {
    // (kept from the original suite; see the element-name tests below for the boundary they pin)
    assert!(xml::check(b"<ADOCTYPE/>", &limits(1, Dtd::Refuse)).is_ok());
    // An element whose *name* contains the keyword is an element, never a declaration: the
    // scan must require `<!` before the keyword, or such elements could bypass the depth cap
    // under `DeclarationOnly` and be falsely refused under `Refuse`.
    let nested = b"<aDOCTYPE><aDOCTYPE><aDOCTYPE/></aDOCTYPE></aDOCTYPE>";
    assert_eq!(
        xml::check(nested, &limits(1, Dtd::DeclarationOnly)),
        Err(Error::TooDeep { depth: 2, limit: 1 }),
        "an element named `aDOCTYPE` is an element: depth 3 must be refused"
    );
    assert!(xml::check(nested, &limits(3, Dtd::DeclarationOnly)).is_ok());
    assert!(xml::check(nested, &limits(3, Dtd::Refuse)).is_ok(), "under Refuse the document is still ordinary elements, not a declaration");
}

#[test]
fn a_processing_instruction_inside_a_subset_cannot_end_its_scan_early() {
    // A `]` inside a DTD processing instruction used to close the apparent subset, ending its
    // measurement early so the rest of the subset was never charged against the cap.
    let text = b"<!DOCTYPE r [<?p ]?> <!ENTITY e \"xxxxxxxxxxxxxxxx\">]><r/>";
    assert_eq!(
        xml::check(text, &limits(8, Dtd::InternalSubset { max_bytes: 4 })),
        Err(Error::InternalSubsetTooLarge { actual: 38, limit: 4 }),
        "the PI is part of the subset; everything up to the real `]` is charged"
    );
    // With a cap that admits it, the same document passes.
    assert!(xml::check(text, &limits(8, Dtd::InternalSubset { max_bytes: 100 })).is_ok());
    // Brackets and quotes inside a DTD PI are ordinary subset bytes, not structure.
    let hostile = b"<!DOCTYPE r [<?p [ \" ?>]><r/>";
    assert_eq!(xml::check(hostile, &limits(8, Dtd::InternalSubset { max_bytes: 3 })), Err(Error::InternalSubsetTooLarge { actual: 10, limit: 3 }));
    // A comment inside the subset keeps its existing semantics (skipped for structure, but
    // part of the subset for the byte cap) — pin it beside the PI case.
    let with_comment = b"<!DOCTYPE r [<!-- ] -->]><r/>";
    assert!(xml::check(with_comment, &limits(8, Dtd::InternalSubset { max_bytes: 200 })).is_ok());
}

#[test]
fn a_utf16_document_is_refused_rather_than_silently_admitted() {
    // `check` reads raw bytes; a UTF-16 document's markup is not contiguous ASCII, so the scan
    // would see no tags at all and admit a DTD the policy refuses. The scan must detect the
    // BOM and refuse: the caller must decode to UTF-8 first.
    let text = "<!DOCTYPE r [<!ENTITY e 'x'>]><r>&e;</r>";
    let mut utf16le = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        utf16le.extend_from_slice(&unit.to_le_bytes());
    }
    assert_eq!(xml::check(&utf16le, &limits(10, Dtd::Refuse)), Err(Error::UnsupportedEncoding));
    // The same document as UTF-8 (no BOM) is checked normally.
    assert!(xml::check(text.as_bytes(), &limits(10, Dtd::Refuse)).is_err());
    assert!(xml::check(text.as_bytes(), &limits(10, Dtd::InternalSubset { max_bytes: 100 })).is_ok());
    // UTF-16BE with BOM is refused the same way.
    let mut utf16be = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        utf16be.extend_from_slice(&unit.to_be_bytes());
    }
    assert_eq!(xml::check(&utf16be, &limits(10, Dtd::Refuse)), Err(Error::UnsupportedEncoding));
    // UTF-8 with its BOM is the same bytes as without it.
    let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
    utf8_bom.extend_from_slice(text.as_bytes());
    assert!(xml::check(&utf8_bom, &limits(10, Dtd::InternalSubset { max_bytes: 100 })).is_ok());
}

#[test]
fn text_and_attribute_values_mentioning_doctype_are_not_declarations() {
    // Text and attribute values mentioning DOCTYPE are not declarations.
    let text = b"<r note=\"&lt;!DOCTYPE x&gt;\">a &lt;!DOCTYPE y&gt; b</r>";
    assert!(xml::check(text, &limits(4, Dtd::Refuse)).is_ok());
}

#[test]
fn a_document_nested_far_deeper_than_any_parser_could_take_returns_an_error() {
    // The point of the scan: a quarter of a million levels must come back as an error, not as a
    // stack overflow that takes the process with it.
    let depth = 250_000;
    let mut text = Vec::with_capacity(depth * 8);
    for index in 0..depth {
        text.extend_from_slice(format!("<e{index}>").as_bytes());
    }
    assert_eq!(xml::check(&text, &limits(1_024, Dtd::Refuse)), Err(Error::TooDeep { depth: 1_025, limit: 1_024 }));

    let deep_in_comments = format!("<r><!-- {} --></r>", "<a>".repeat(depth));
    assert!(xml::check(deep_in_comments.as_bytes(), &limits(8, Dtd::Refuse)).is_ok());
}

#[test]
fn the_depth_counter_matches_the_scan_and_refuses_one_level_past_its_limit() {
    let mut depth = Depth::new(3);
    assert_eq!(depth.limit(), 3);
    assert_eq!(depth.current(), 0);
    for expected in 1..=3 {
        assert_eq!(depth.enter(), Ok(()));
        assert_eq!(depth.current(), expected);
    }
    assert_eq!(depth.enter(), Err(Error::TooDeep { depth: 4, limit: 3 }));
    assert_eq!(depth.current(), 3, "a refused level must not be counted");
    depth.leave();
    depth.leave();
    depth.leave();
    assert_eq!(depth.current(), 0);
    depth.leave();
    assert_eq!(depth.current(), 0, "closing more than were opened stays at zero");

    // The scan and the counter agree at the boundary.
    let document = nested(3);
    assert!(xml::check(&document, &limits(3, Dtd::Refuse)).is_ok());
    let mut counter = Depth::new(3);
    for _ in 0..3 {
        assert_eq!(counter.enter(), Ok(()));
    }
    assert!(counter.enter().is_err());
}

#[test]
fn a_limit_of_zero_admits_no_element_for_the_counter_either() {
    let mut depth = Depth::new(0);
    assert_eq!(depth.enter(), Err(Error::TooDeep { depth: 1, limit: 0 }));
    assert_eq!(depth.current(), 0);
}

#[test]
fn every_prefix_of_a_valid_document_is_checked_without_panicking() {
    let document = br#"<?xml version="1.0"?>
        <!DOCTYPE r [<!ENTITY e "x">]>
        <r a='1'><b/><!-- c --><![CDATA[<d>]]>&e;</r>"#;
    for end in 0..=document.len() {
        for dtd in [Dtd::Refuse, Dtd::DeclarationOnly, Dtd::InternalSubset { max_bytes: 64 }] {
            let _ = xml::check(document.get(..end).unwrap_or_default(), &limits(4, dtd));
        }
    }
}

#[test]
fn errors_describe_themselves() {
    let cases = [
        (Error::TooDeep { depth: 5, limit: 4 }, "elements nested 5 deep; the limit is 4"),
        (Error::DtdRefused, "a document type declaration is not allowed"),
        (Error::InternalSubsetNotAllowed, "a document type declaration with an internal subset is not allowed"),
        (Error::InternalSubsetTooLarge { actual: 9, limit: 8 }, "the internal subset is 9 bytes; the limit is 8"),
    ];
    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}
