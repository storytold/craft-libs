use craft_captions::{Cue, Document, Format, TICKS_PER_SECOND, Tick, WriteOptions, parse, write};

fn cue() -> Cue {
    Cue { start: Tick(TICKS_PER_SECOND), end: Tick(2 * TICKS_PER_SECOND), text: "Olá".into(), ..Default::default() }
}

/// TTML and DFXP exports write the document's language as `xml:lang`.
#[test]
fn ttml_and_dfxp_write_the_document_language() {
    let doc = Document { cues: vec![cue()], language: Some("pt-BR".into()), ..Default::default() };
    for format in [Format::Ttml, Format::Dfxp] {
        let xml = String::from_utf8(write(&doc, format, WriteOptions::default())).unwrap();
        assert!(xml.contains(r#"xml:lang="pt-BR""#), "{format:?}: {xml}");
        // and the language comes back on import
        let back = parse(xml.as_bytes(), format).unwrap();
        assert_eq!(back.language.as_deref(), Some("pt-BR"), "{format:?}");
    }
    // a document with no language still writes a valid tag
    let doc = Document { cues: vec![cue()], ..Default::default() };
    let xml = String::from_utf8(write(&doc, Format::Ttml, WriteOptions::default())).unwrap();
    assert!(xml.contains(r#"xml:lang="en""#), "{xml}");
}

/// An `xml:lang` that is empty or not a language tag leaves the language unknown.
#[test]
fn ttml_import_ignores_a_bad_xml_lang() {
    let tt =
        |lang: &str| format!(r#"<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="{lang}"><body><div><p begin="1s" end="2s">Hi</p></div></body></tt>"#);
    let doc = parse(tt(" fr-CA ").as_bytes(), Format::Ttml).unwrap();
    assert_eq!((doc.language.as_deref(), doc.warnings.len()), (Some("fr-CA"), 0));
    let doc = parse(tt("").as_bytes(), Format::Ttml).unwrap();
    assert_eq!((doc.language, doc.warnings.len()), (None, 0));
    for bad in ["pt BR", "&lt;x&gt;", &"a".repeat(36)] {
        let doc = parse(tt(bad).as_bytes(), Format::Ttml).unwrap();
        assert_eq!(doc.language, None, "{bad}");
        assert_eq!(doc.warnings, ["ignored xml:lang: not a language tag"], "{bad}");
        assert_eq!(doc.cues.len(), 1);
    }
}
