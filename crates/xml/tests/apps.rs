//! The tests the Craft apps had for their own XML helpers, against this crate: each app's
//! expectations, read the way that app reads (see the migration table in the README).

use craft_xml::{Canonical, Doctype, El, Error, Event, Limits, Local, Node, NsName, Options, Scope, Writer, esc, esc_attr, parse, to_utf8, unescape};

// ------------------------------------------------------------------ WordCraft: canonical prefixes

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const WORD_NS: &[(&str, &str)] = &[
    ("w", W),
    ("r", "http://schemas.openxmlformats.org/officeDocument/2006/relationships"),
    ("wp", "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"),
    ("a", "http://schemas.openxmlformats.org/drawingml/2006/main"),
    ("mc", "http://schemas.openxmlformats.org/markup-compatibility/2006"),
    ("wpi", "http://schemas.microsoft.com/office/word/2010/wordprocessingInk"),
    ("wps", "http://schemas.microsoft.com/office/word/2010/wordprocessingShape"),
    // ISO/IEC 29500 Strict namespaces map onto the same prefixes.
    ("w", "http://purl.oclc.org/ooxml/wordprocessingml/main"),
    ("wp", "http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing"),
    ("a", "http://purl.oclc.org/ooxml/drawingml/main"),
];
const WORD: Canonical<'static> = Canonical::new(WORD_NS);

fn word(bytes: &[u8]) -> Result<El, Error> {
    Options::canonical(&WORD).with_limits(Limits { max_nodes: 8_000_000, ..Limits::DEFAULT }).parse(bytes)
}

#[test]
fn word_parses_with_namespaces() {
    let x = br#"<?xml version="1.0"?><x:document xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:rr="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><x:p rr:id="r1" x:val="a&amp;b"><x:t xml:space="preserve"> hi &#65;&lt; </x:t></x:p></x:document>"#;
    let e = word(x).unwrap();
    assert_eq!(e.name, "w:document");
    assert!(e.attrs.is_empty(), "namespace declarations are not attributes of this tree");
    let p = e.child("w:p").unwrap();
    assert_eq!(p.attr("r:id"), Some("r1"));
    assert_eq!(p.attr("w:val"), Some("a&b"));
    let t = p.child("w:t").unwrap();
    assert_eq!(t.text(), " hi A< ");
    assert_eq!(t.attr("xml:space"), Some("preserve"));
    assert_eq!((t.local(), t.prefix()), ("t", "w"));
    assert_eq!(e.child_attr("w:p", "w:val"), Some("a&b"));
}

#[test]
fn word_attr_falls_back_to_the_unqualified_name() {
    let x = format!(r#"<w:p xmlns:w="{W}"><w:a w:val="1"/><w:b val="2"/><w:c other="3"/></w:p>"#);
    let p = word(x.as_bytes()).unwrap();
    let val = |name: &str| p.child(name).and_then(|c| c.attr_or_plain("w:val"));
    assert_eq!((val("w:a"), val("w:b"), val("w:c")), (Some("1"), Some("2"), None));
    assert_eq!(p.child("w:b").and_then(|c| c.attr("w:val")), None, "the exact lookup does not fall back");
}

#[test]
fn word_unknown_namespaces_read_behind_a_question_mark_and_are_not_written() {
    let x = format!(
        r#"<w:body xmlns:w="{W}" xmlns:z="urn:future" xmlns:u="urn:other"><w:p z:new="1" w:old="2"><z:thing><w:r/></z:thing><w:r>t</w:r></w:p><undeclared:x/><plain/></w:body>"#
    );
    let body = word(x.as_bytes()).unwrap();
    let p = body.child("w:p").unwrap();
    assert_eq!(p.attr("?:new"), Some("1"));
    assert!(p.child("?:thing").is_some());
    assert!(body.child("x").is_some(), "a prefix nothing declares is no namespace");
    assert!(body.child("plain").is_some());
    assert_eq!(body.to_xml(), r#"<w:body><w:p w:old="2"><w:r>t</w:r></w:p><x/><plain/></w:body>"#);
}

/// A vocabulary's own namespace listed with the empty prefix reads unprefixed (SpreadsheetML
/// keeps its main namespace as the default one), whatever prefix the document used.
#[test]
fn an_empty_canonical_prefix_reads_unprefixed() {
    const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    let canon = Canonical::new(&[("", MAIN), ("x14", "urn:x14")]);
    let x = format!(r#"<x:worksheet xmlns:x="{MAIN}" xmlns:e="urn:x14"><x:sheetData x:a="1" e:b="2"/><e:id/><row xmlns="{MAIN}"/></x:worksheet>"#);
    let root = Options::canonical(&canon).parse(x.as_bytes()).unwrap();
    assert_eq!(root.to_xml(), r#"<worksheet><sheetData a="1" x14:b="2"/><x14:id/><row/></worksheet>"#);
}

/// WordCraft reads the one Strict WordprocessingDrawing namespace as Word 2010's several, by
/// element and parent: the element hook.
#[test]
fn word_strict_drawing_names_read_as_word_2010_names() {
    const WNE: &str = "http://schemas.microsoft.com/office/word/2006/wordml";
    fn drawing(n: &NsName<'_>) -> Option<&'static str> {
        match (n.prefix, n.parent, n.local) {
            ("wp", _, "wsp") => Some("wps"),
            ("wp", Some("wps:wsp"), "spPr" | "txbx" | "bodyPr") => Some("wps"),
            ("wp", Some("wps:txbx"), "txbxContent") => Some("w"),
            ("?", Some("wps:txbx"), "txbxContent") if n.uri == WNE => Some("w"),
            _ => None,
        }
    }
    let canon = Canonical { element: Some(&drawing), ..WORD };
    let x = br#"<w:body xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main" xmlns:d="http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing" xmlns:wne="http://schemas.microsoft.com/office/word/2006/wordml">
<d:anchor><d:docPr/><d:wsp><d:spPr/><d:txbx><wne:txbxContent><w:p/></wne:txbxContent></d:txbx><d:bodyPr/></d:wsp><d:wsp><d:txbx><d:txbxContent/></d:txbx></d:wsp></d:anchor>
<wne:txbxContent/><d:inline><d:bodyPr/></d:inline></w:body>"#;
    let root = Options::canonical(&canon).parse(x).unwrap();
    let names: Vec<&str> = std::iter::once(&root).chain(root.descendants()).map(|e| e.name.as_str()).collect();
    assert_eq!(
        names.join(" "),
        "w:body wp:anchor wp:docPr wps:wsp wps:spPr wps:txbx w:txbxContent w:p wps:bodyPr wps:wsp wps:txbx w:txbxContent ?:txbxContent wp:inline wp:bodyPr"
    );
}

/// `mc:Choice/@Requires` holds prefixes, not names: WordCraft resolves them while the scope is
/// known. The attribute-value hook.
#[test]
fn word_choice_requirements_resolve_namespace_aliases_and_rebindings() {
    fn requires(el: &str, attr: &str, value: &str, scope: &Scope<'_>) -> Option<String> {
        (el == "mc:Choice" && attr == "Requires").then(|| {
            let prefixes = value.split_whitespace().map(|p| scope.uri(p).map_or("?", |uri| WORD.prefix(uri)));
            prefixes.collect::<Vec<_>>().join(" ")
        })
    }
    let canon = Canonical { attr_value: Some(&requires), ..WORD };
    let x = br#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:pen="http://schemas.microsoft.com/office/word/2010/wordprocessingInk"><mc:Choice Requires="pen"/><mc:Choice xmlns:pen="urn:future" Requires="pen"/><mc:Choice Requires="undeclared"/><mc:Choice Requires="pen mc"/></mc:AlternateContent>"#;
    let e = Options::canonical(&canon).parse(x).unwrap();
    let values: Vec<_> = e.children("mc:Choice").map(|c| c.attr("Requires")).collect();
    assert_eq!(values, [Some("wpi"), Some("?"), Some("?"), Some("wpi mc")]);
}

#[test]
fn word_strict_parsing_rejects_a_dropped_entity_and_trailing_markup() {
    let strict = Options::canonical(&WORD).strict();
    let bad = [&b"<w:t>before &unknown; after</w:t>"[..], b"<a></a><b></b>", b"<a></a><b>", b"<a><b></a></b>", b"<a>", b"</a>", b"<a/>text"];
    for xml in bad {
        assert!(strict.parse(xml).is_err(), "{}", String::from_utf8_lossy(xml));
    }
    assert!(strict.parse(b"<a>x &amp; y</a>\n").is_ok());
    assert!(strict.parse(b"<?xml version='1.0'?>\n<!-- c -->\n<a/>\n<!-- d -->\n").is_ok());
    assert!(word(b"<a></a><b></b>").is_ok(), "lenient parsing still stops at the root");
    assert_eq!(word(b"<w:t>before &unknown; after</w:t>").unwrap().text(), "before  after", "and drops an undefined entity");
}

#[test]
fn word_utf16_parts_parse_and_odd_ones_never_panic() {
    let src = r#"<?xml version="1.0" encoding="UTF-16"?><r:Relationships xmlns:r="urn:x"><r:Relationship Id="rId1" Target="café😀.xml"/></r:Relationships>"#;
    for le in [true, false] {
        let mut b = if le { vec![0xFF, 0xFE] } else { vec![0xFE, 0xFF] };
        b.extend(src.encode_utf16().flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() }));
        let root = word(&b).unwrap();
        assert_eq!(root.elements().next().and_then(|e| e.attr("Target")), Some("café😀.xml"), "le {le}");
    }
    // Odd length, unpaired surrogates, a bare BOM: an error or a tree, never a panic.
    for b in [&[0xFF, 0xFE][..], &[0xFE, 0xFF, 0x00], &[0xFF, 0xFE, b'<', 0, 0x00, 0xD8, b'a', 0, b'/', 0, b'>', 0], &[0xFF, 0xFE, 0x00, 0xDC, 0x00]]
    {
        let _ = word(b);
    }
    assert_eq!(to_utf8(&[0xFF, 0xFE, b'a', 0, 0x00, 0xD8, b'b']).as_ref(), "a\u{FFFD}".as_bytes());
    assert_eq!(to_utf8(b"<a/>").as_ref(), b"<a/>");
}

/// XML 1.0 §2.11: CR LF and a lone CR in text and CDATA are read as LF; `&#13;` stays a CR.
#[test]
fn word_line_ends_are_normalised_but_a_cr_reference_stays() {
    let x = format!("<x:document xmlns:x=\"{W}\"><x:t>a\r\nb\rc&#13;d<![CDATA[e\r\nf]]></x:t></x:document>");
    assert_eq!(word(x.as_bytes()).unwrap().child("w:t").unwrap().text(), "a\nb\nc\rde\nf");
    // The writer escapes a CR, so a CR in the text survives a save.
    let x = format!("<x:document xmlns:x=\"{W}\"><x:t>{}</x:t></x:document>", esc("a\r\nb\rc"));
    assert_eq!(word(x.as_bytes()).unwrap().child("w:t").unwrap().text(), "a\r\nb\rc");
    // CDATA that isn't UTF-8 is decoded lossily, and its line ends are still normalised.
    let mut x = format!("<x:document xmlns:x=\"{W}\"><x:t><![CDATA[a").into_bytes();
    x.extend_from_slice(b"\xff\r\nb\rc]]></x:t></x:document>");
    assert_eq!(word(&x).unwrap().child("w:t").unwrap().text(), "a\u{FFFD}\nb\nc");
}

#[test]
fn word_deep_text_and_find() {
    let x = format!(r#"<w:p xmlns:w="{W}">a<w:r><w:t>b</w:t><w:r><w:t>c</w:t></w:r></w:r>d</w:p>"#);
    let p = word(x.as_bytes()).unwrap();
    assert_eq!(p.text(), "ad");
    assert_eq!(p.deep_text(), "abcd");
    assert_eq!(p.find("w:t").map(|t| t.text().into_owned()).as_deref(), Some("b"));
    assert_eq!(p.find_all("w:t").count(), 2);
    assert!(p.find("w:p").is_none(), "the element itself is not its own descendant");
}

#[test]
fn word_hostile_parts_are_errors() {
    assert_eq!(word("<a>".repeat(1000).as_bytes()), Err(Error::TooDeep(256)));
    // Thousands of `xmlns:` declarations on one tag are refused, not bound (RUSTSEC-2026-0195).
    let decls: String = (0..10_000).map(|i| format!(" xmlns:a{i}=\"urn:a{i}\"")).collect();
    assert_eq!(word(format!("<a{decls}><b/></a>").as_bytes()), Err(Error::TooManyAttrs(1024)));
    // One start tag with 200,000 attributes, distinct or duplicate, is refused (wordcraft#3702).
    let distinct: String = (0..200_000).map(|i| format!(" a{i}=\"\"")).collect();
    let duplicate = " a=\"\"".repeat(200_000);
    for attrs in [distinct, duplicate] {
        assert_eq!(word(format!("<w:p xmlns:w=\"{W}\"{attrs}/>").as_bytes()), Err(Error::TooManyAttrs(1024)));
    }
    // Far more than any real producer writes still reads.
    let many: String = (0..256).map(|i| format!(" a{i}=\"{i}\"")).collect();
    let p = word(format!("<p{many}/>").as_bytes()).unwrap();
    assert_eq!((p.attrs.len(), p.attr("a255")), (256, Some("255")));
    assert!(word(b"").is_err());
    assert!(word(b"\xff\xfe<").is_err());
    let _ = word(b"<a><b></a>");
    assert_eq!(word(b"<a x='1' x='2'/>").unwrap().attrs.len(), 2, "a repeated attribute is kept each time");
}

#[test]
fn word_writer() {
    assert_eq!(esc_attr("a<b&\"\u{1}"), "a&lt;b&amp;&quot;");
    let mut w = Writer::document();
    w.open("w:p", &[("w:rsidR", "00A1")]);
    w.empty("w:jc", &[("w:val", "center")]);
    w.leaf("w:t", &[], "x & y");
    w.raw("<w:br/>");
    w.text("tail\r");
    w.close();
    let expect = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:p w:rsidR=\"00A1\"><w:jc w:val=\"center\"/><w:t>x &amp; y</w:t><w:br/>tail&#13;</w:p>";
    assert_eq!(String::from_utf8(w.into_bytes()).unwrap(), expect);
}

// ------------------------------------------------------------------ GridCraft: local names, streaming

fn grid(bytes: &[u8]) -> Result<El, Error> {
    Options { unknown_entity: craft_xml::UnknownEntity::Literal, ..Options::local() }.parse(bytes)
}

#[test]
fn grid_parse_basic() {
    let x = br#"<?xml version="1.0"?><a:root xmlns:a="x" k="v &amp; w" r:id="rId1"><b>t &lt; u&#65;</b><c/><![CDATA[<raw>]]></a:root>"#;
    let e = grid(x).unwrap();
    assert_eq!(e.name, "root");
    assert_eq!(e.attr("k"), Some("v & w"));
    assert_eq!(e.attr("id"), Some("rId1"), "attributes are named by their local name");
    assert_eq!(e.attrs.len(), 2, "namespace declarations carry no data");
    assert_eq!(e.child("b").unwrap().text(), "t < uA");
    assert!(e.child("c").is_some());
    assert_eq!(e.text(), "<raw>");
    assert_eq!(grid(b"<a>&unknown; &amp;</a>").unwrap().text(), "&unknown; &", "an undefined entity stays as written");
}

#[test]
fn grid_typed_attributes_paths_and_values() {
    let x = br#"<sheet n=" 7 " neg="-3" f="1.5" inf="inf" on="on" t="t" no="false" odd="yes"><a><b><c val="deep"/></b></a><v val="top"/></sheet>"#;
    let e = grid(x).unwrap();
    assert_eq!((e.attr_u32("n"), e.attr_u32("neg"), e.attr_u32("f"), e.attr_u32("missing")), (Some(7), None, None, None));
    assert_eq!((e.attr_i64("neg"), e.attr_i32("neg"), e.attr_f64("f"), e.attr_f64("inf")), (Some(-3), Some(-3), Some(1.5), None));
    assert_eq!((e.attr_bool("on"), e.attr_bool("t"), e.attr_bool("no"), e.attr_bool("odd")), (Some(true), Some(true), Some(false), None));
    assert!(e.flag("odd", true) && !e.flag("no", true) && !e.flag("missing", false));
    assert_eq!(e.path(&["a", "b", "c"]).and_then(|c| c.attr("val")), Some("deep"));
    assert!(e.path(&["a", "c"]).is_none());
    assert_eq!(e.child_attr("v", "val"), Some("top"));
    assert_eq!(e.children("v").count(), 1);
}

#[test]
fn grid_streaming_split() {
    let x = b"<r><row n='1'/><row n='2'><c/></row><other/><g><row n='3'/><h><row n='4'/></h></g></r>";
    let mut seen = Vec::new();
    let root = Options::local()
        .parse_streaming(x, &["row"], |e| {
            assert_eq!(e.name, "row");
            seen.push(e.attr_u32("n"));
            Ok::<(), Error>(())
        })
        .unwrap();
    assert_eq!(seen, [Some(1), Some(2), Some(3)], "children and grandchildren of the root are handed out");
    assert_eq!(root.elements().count(), 2);
    assert_eq!(root.find("row").and_then(|r| r.attr("n")), Some("4"), "a deeper element of the same name stays in the tree");
}

#[test]
fn grid_a_streaming_error_stops_the_read() {
    #[derive(Debug, PartialEq)]
    enum AppError {
        Xml(Error),
        Row(u32),
    }
    impl From<Error> for AppError {
        fn from(e: Error) -> Self {
            AppError::Xml(e)
        }
    }
    let mut rows = 0;
    let read = |xml: &[u8], rows: &mut u32| {
        Options::local().parse_streaming(xml, &["row"], |_| {
            *rows += 1;
            if *rows == 2 { Err(AppError::Row(2)) } else { Ok(()) }
        })
    };
    assert_eq!(read(b"<r><row/><row/><row/></r>", &mut rows), Err(AppError::Row(2)));
    assert_eq!(rows, 2);
    assert_eq!(read(b"", &mut rows), Err(AppError::Xml(Error::NoRoot)));
}

#[test]
fn grid_a_streamed_part_may_hold_more_elements_than_the_cap() {
    // 501 rows of 10 cells is 10,020 `c` and `v` elements, more than this cap, but the rows are
    // streamed out one by one and never pile up in the tree. Kept in a tree, the part is refused.
    let opts = Options::local().with_limits(Limits { max_nodes: 10_000, ..Limits::DEFAULT });
    let row = format!("<row>{}</row>", "<c><v>1</v></c>".repeat(10));
    let xml = format!("<worksheet><sheetData>{}</sheetData></worksheet>", row.repeat(501));
    let mut rows = 0usize;
    let root = opts
        .parse_streaming(xml.as_bytes(), &["row"], |r| {
            assert_eq!(r.elements().count(), 10);
            rows += 1;
            Ok::<(), Error>(())
        })
        .unwrap();
    assert_eq!(rows, 501);
    assert_eq!(root.child("sheetData").map(|d| d.children.len()), Some(0));
    assert_eq!(opts.parse(xml.as_bytes()), Err(Error::TooManyNodes(10_000)));
    // One row that is itself over the cap is refused while it is being read.
    let huge = format!("<sheetData><row>{}</row></sheetData>", "<c/>".repeat(10_001));
    assert_eq!(opts.parse_streaming(huge.as_bytes(), &["row"], |_| Ok::<(), Error>(())), Err(Error::TooManyNodes(10_000)));
}

#[test]
fn grid_first_start_reads_one_tag_and_nothing_after_it() {
    let vml = b"<xml xmlns:v='urn:v'><v:shape id='a'><div>one<br>two<br></div><x:ClientData ObjectType=\"Note\" r:id='7'><broken";
    let found = Options::local().first_start(vml, "ClientData").unwrap();
    assert_eq!((found.name.as_str(), found.attr("ObjectType"), found.attr("id")), ("ClientData", Some("Note"), Some("7")));
    assert!(found.children.is_empty());
    assert_eq!(Options::default().first_start(vml, Local("ClientData")).map(|e| e.name), Some("x:ClientData".to_string()));
    assert!(Options::local().first_start(vml, "missing").is_none());
    // Tags that are never closed are not nesting.
    let unclosed = format!("{}<hit/>", "<br>".repeat(1000));
    assert!(Options::local().first_start(unclosed.as_bytes(), "hit").is_some());
}

#[test]
fn grid_utf16_without_a_byte_order_mark() {
    let src = "<?xml version=\"1.0\" encoding=\"UTF-16\"?><a k=\"ü\"/>";
    for le in [true, false] {
        let b: Vec<u8> = src.encode_utf16().flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() }).collect();
        assert_eq!(grid(&b).unwrap().attr("k"), Some("ü"), "le {le}");
    }
    assert_eq!(grid(b"\xEF\xBB\xBF<a/>").unwrap().name, "a");
}

#[test]
fn grid_garbage_and_depth() {
    assert!(grid("<a>".repeat(10_000).as_bytes()).is_err());
    assert!(grid(b"").is_err());
    let _ = grid(b"\x00\x01\x02 not xml");
    let _ = grid(b"<a><b></a>");
    let _ = grid(b"<a attr='&bogus;'>&unknown;</a>");
}

/// The appending escape writes what the character loop GridCraft checked it against writes.
#[test]
fn grid_escaping() {
    assert_eq!(esc("a\rb"), "a&#13;b");
    assert_eq!(esc_attr("a\rb"), "a&#13;b");
    for t in ["plain", "", "a & <b> > c \"q\"", "tab\tnew\nline\rcr", "\u{1}\u{8}ctl", "日本語 ü", "\u{FFFE}\u{FFFF}x", "~\u{7F}"] {
        let mut reference = String::new();
        for c in t.chars() {
            match c {
                '&' => reference.push_str("&amp;"),
                '<' => reference.push_str("&lt;"),
                '>' => reference.push_str("&gt;"),
                '\r' => reference.push_str("&#13;"),
                c if craft_xml::is_xml_char(c) => reference.push(c),
                _ => {}
            }
        }
        let mut s = String::from("x");
        craft_xml::push_esc(&mut s, t);
        assert_eq!(s, format!("x{reference}"), "{t:?}");
        assert_eq!(esc(t), reference);
    }
}

// ------------------------------------------------------------------ DeckCraft: as written, matched by local name

#[test]
fn deck_parse_basic_and_entities() {
    let root =
        parse(b"<?xml version=\"1.0\"?><a:p xmlns:a=\"x\"><a:r><a:t>A &amp; B &#x41; &lt;</a:t></a:r><a:x r:id=\"rId1\" id=\"5\"/></a:p>").unwrap();
    assert_eq!((root.name.as_str(), root.local(), root.prefix()), ("a:p", "p", "a"));
    assert!(root.is(Local("p")) && root.is("a:p") && !root.is("p"));
    assert_eq!(root.find(Local("t")).map(El::deep_text), Some("A & B A <".to_string()));
    let x = root.child(Local("x")).unwrap();
    assert_eq!(x.attr_local("id"), Some("5"));
    assert_eq!(x.attr_local("r:id"), Some("rId1"));
    assert_eq!(x.attr_local("q:id"), Some("rId1"), "a qualified lookup takes the local name under any prefix");
    assert_eq!(root.ns_decls().collect::<Vec<_>>(), [("a", "x")]);
    assert_eq!(root.attr_local("a"), None, "a namespace declaration is not an attribute by local name");
    // Without a plain `id`, the unqualified lookup takes the prefixed one.
    let only = parse(b"<a r:id='rId9' xmlns='urn:d'/>").unwrap();
    assert_eq!(only.attr_local("id"), Some("rId9"));
    assert_eq!(only.ns_decls().collect::<Vec<_>>(), [("", "urn:d")]);
}

#[test]
fn deck_lossy_integers_and_booleans() {
    use craft_xml::value::{to_bool, to_i32_lossy, to_i64_lossy, to_u32_lossy};
    let e = parse(b"<a:off x='12.7' y='-3' big='1e40' on='on' t='t'/>").unwrap();
    assert_eq!(e.attr_local("x").and_then(to_i64_lossy), Some(12));
    assert_eq!(e.attr_local("y").and_then(to_u32_lossy), Some(0));
    assert_eq!(e.attr_local("big").and_then(to_i32_lossy), Some(i32::MAX));
    assert_eq!(e.attr_local("on").and_then(to_bool), Some(true));
    assert_eq!(e.attr_i64("x"), None, "the exact getter does not accept a fraction");
}

#[test]
fn deck_line_ends_and_carriage_returns() {
    let root = parse(b"<a:p><a:t>one\r\ntwo\rthree&#13;four</a:t><a:t><![CDATA[x\r\ny]]></a:t></a:p>").unwrap();
    let t: Vec<String> = root.children(Local("t")).map(El::deep_text).collect();
    assert_eq!(t, ["one\ntwo\nthree\rfour", "x\ny"]);
    let mut w = Writer::new();
    w.tag("a:t").text("one\rtwo\r\nthree");
    assert_eq!(parse(w.as_str().as_bytes()).unwrap().deep_text(), "one\rtwo\r\nthree");
}

#[test]
fn deck_truncated_and_garbage() {
    let kept = parse(b"<a><b><c>text").unwrap();
    assert_eq!(kept.path(&["b", "c"]).map(|c| c.text().into_owned()).as_deref(), Some("text"), "a truncated document keeps what it had");
    assert!(parse(b"").is_err());
    assert!(parse("<a>".repeat(256 + 10).as_bytes()).is_err());
}

#[test]
fn deck_escape_drops_control_chars() {
    assert_eq!(esc("a\u{b}b<\u{0}"), "ab&lt;");
    assert_eq!(esc_attr("\"x\"\n"), "&quot;x&quot;&#10;");
}

#[test]
fn deck_roundtrip_and_mutation() {
    let mut root = parse(b"<p:sp a=\"1\" xmlns:p='urn:p'><p:t>x &amp; y</p:t><e/></p:sp>").unwrap();
    assert_eq!(root.to_xml(), "<p:sp a=\"1\" xmlns:p=\"urn:p\"><p:t>x &amp; y</p:t><e/></p:sp>");
    root.set_attr("a", "2");
    root.set_attr("b", 3);
    for e in root.elements_mut() {
        e.set_attr("seen", true);
    }
    if let Some(e) = root.child_mut(Local("e")) {
        e.push("text");
        e.push(El::new("f"));
    }
    assert_eq!(root.remove_attr("xmlns:p").as_deref(), Some("urn:p"));
    assert_eq!(root.remove_attr("xmlns:p"), None);
    assert!(root.has_attr("b") && !root.has_attr("c"));
    assert_eq!(root.to_xml(), "<p:sp a=\"2\" b=\"3\"><p:t seen=\"true\">x &amp; y</p:t><e seen=\"true\">text<f/></e></p:sp>");
}

#[test]
fn deck_writer_attributes() {
    let name: Option<&str> = None;
    let mut w = Writer::new();
    w.tag("p:sp").attr("id", 5).attr_opt("name", name).attr_opt("bold", Some(u8::from(true))).attr("t", "a\"b").open();
    w.tag("a:off").attr("x", -1.5).attr("y", 2).empty();
    w.tag("a:t").text("hi");
    assert_eq!(w.finish(), "<p:sp id=\"5\" bold=\"1\" t=\"a&quot;b\"><a:off x=\"-1.5\" y=\"2\"/><a:t>hi</a:t></p:sp>");
}

// ------------------------------------------------------------------ DesignCraft: builder, PIs, pretty documents

fn design() -> Options<'static> {
    Options { strict: true, pis: true, ..Options::default() }
}

/// designcraft#712: nesting past the budget is an error from the reader, not a stack overflow.
#[test]
fn design_nesting_past_the_budget_is_refused() {
    let nested = |n: usize| format!("{}{}", "<A>".repeat(n), "</A>".repeat(n)).into_bytes();
    assert!(design().parse(&nested(256)).is_ok(), "the budget itself parses");
    assert_eq!(design().parse(&nested(257)), Err(Error::TooDeep(256)));
    assert!(design().parse(&nested(100_000)).is_err());
    // An empty element is a level too.
    let leaf = |n: usize| format!("{}<B/>{}", "<A>".repeat(n), "</A>".repeat(n)).into_bytes();
    assert!(design().parse(&leaf(255)).is_ok());
    assert_eq!(design().parse(&leaf(256)), Err(Error::TooDeep(256)));
}

#[test]
fn design_roundtrip_text_pi_and_entities() {
    let page = El::new("Content").with_text("Page ").with_node(Node::Pi { target: "ACE".into(), data: "18".into() });
    let root = El::new("Root").with_attr("A", "x\"<&\t").with_attr("N", 9).with_child(El::new("Content").with_text("a\tb & <c>")).with_child(page);
    let mut w = Writer::pretty("\t");
    w.decl(true);
    w.pi("aid", "style=\"50\" type=\"document\"");
    w.tree(&root);
    let text = w.finish();
    assert_eq!(
        text,
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<?aid style=\"50\" type=\"document\"?>\n<Root A=\"x&quot;&lt;&amp;&#9;\" N=\"9\">\n\t<Content>a\tb &amp; &lt;c&gt;</Content>\n\t<Content>Page <?ACE 18?></Content>\n</Root>\n"
    );
    let back = design().parse(text.as_bytes()).unwrap();
    assert_eq!(back.attr("A"), Some("x\"<&\t"));
    let contents: Vec<&El> = back.children(Local("Content")).collect();
    assert_eq!(contents[0].deep_text(), "a\tb & <c>");
    assert_eq!(contents[1].children[1], Node::Pi { target: "ACE".into(), data: "18".into() });
    assert_eq!(Options { drop_blank_text: true, ..design() }.parse(text.as_bytes()).unwrap(), root);
}

#[test]
fn design_unbalanced_documents_are_errors() {
    for bad in [&b"<A><B></A>"[..], b"<A>", b"</A>", b"", b"<A></A></A>"] {
        assert!(design().parse(bad).is_err(), "{}", String::from_utf8_lossy(bad));
    }
    assert_eq!(design().parse(b"<A>x&nbsp;y</A>").unwrap().text(), "xy", "an undefined entity is dropped");
}

// ------------------------------------------------------------------ AniCraft: strict, indentation dropped, mixed content

fn ani() -> Options<'static> {
    Options { strict: true, drop_blank_text: true, unknown_entity: craft_xml::UnknownEntity::Literal, ..Options::default() }
}

#[test]
fn ani_elements_attributes_and_entities() {
    let x = br#"<?xml version="1.0" encoding="UTF-8"?>
<!-- a comment -->
<DOMDocument xmlns="http://ns.adobe.com/xfl/2008/" width="640" frameRate='30'>
  <timelines><DOMTimeline name="Scene &amp; &quot;1&quot; &#65;&#x42;"><layers/></DOMTimeline></timelines>
  <![CDATA[ <ignored/> ]]>
  text is kept
</DOMDocument>"#;
    let root = ani().parse(x).unwrap();
    assert_eq!(root.name, "DOMDocument");
    assert_eq!((root.attr_f64("width"), root.attr_u32("frameRate")), (Some(640.0), Some(30)));
    let tl = root.child("timelines").and_then(|t| t.elements().next()).unwrap();
    assert_eq!(tl.attr("name"), Some("Scene & \"1\" AB"));
    assert_eq!(tl.child("layers").map(|l| l.children.len()), Some(0));
    assert_eq!(root.elements().count(), 1, "CDATA and text are not elements");
    assert!(root.text().contains(" <ignored/> ") && root.text().contains("text is kept"), "{:?}", root.text());
}

#[test]
fn ani_character_data_is_kept_and_indentation_dropped() {
    let x = b"<t>\n  <characters>a &lt;b&gt;&#xD;line 2</characters>\n  <characters> </characters><c><![CDATA[x<y]]>z</c>\n</t>";
    let root = ani().parse(x).unwrap();
    assert_eq!(root.text(), "", "whitespace between elements");
    assert_eq!(root.children.len(), 3);
    let texts: Vec<String> = root.elements().map(|c| c.text().into_owned()).collect();
    assert_eq!(texts, ["a <b>\rline 2", " ", "x<yz"]);
}

/// Mixed content needs no special mode: text nodes keep their place among the elements.
#[test]
fn ani_mixed_content_keeps_text_in_order() {
    let x = b"<svg><text x='1'>Hello <tspan>big</tspan> world&amp;<!-- c --> more</text>\n  <g>\n</g></svg>";
    let root = ani().parse(x).unwrap();
    let t = root.child("text").unwrap();
    let parts: Vec<Node> = t.children.clone();
    assert_eq!(parts, [Node::Text("Hello ".into()), Node::El(El::new("tspan").with_text("big")), Node::Text(" world& more".into())]);
    assert_eq!(t.text(), "Hello  world& more");
    assert_eq!(root.children.len(), 2, "indentation between elements is dropped");
    assert_eq!(root.child("g").map(|g| g.children.len()), Some(1), "an element without child elements keeps its whitespace");
}

#[test]
fn ani_malformed_input_is_an_error_not_a_panic() {
    for bad in [&b""[..], b"<", b"<a", b"<a b>", b"<a b='1>", b"<a></b>", b"<a><b></a>", b"<!-- open", b"<a>\xff\xfe", b"<a>\xff</a>"] {
        assert!(ani().parse(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
    }
    let deep = "<a>".repeat(400) + &"</a>".repeat(400);
    assert_eq!(ani().parse(deep.as_bytes()), Err(Error::TooDeep(256)));
    let big = Options::default().with_limits(Limits { max_input: 16, ..Limits::DEFAULT });
    assert_eq!(big.parse(b"<a>0123456789abc</a>"), Err(Error::TooLarge(16)));
    assert!(big.parse(b"<a>012345678</a>").is_ok());
}

// ------------------------------------------------------------------ LightCraft, EffectCraft: references

#[test]
fn light_references_are_resolved_once_and_strictly() {
    // 200 000 `&` then a `;`: each `&` is looked at once, not searched from for the `;`.
    let hostile = format!("{};&amp;", "&".repeat(200_000));
    assert_eq!(unescape(&hostile), format!("{};&", "&".repeat(200_000)));
    assert_eq!(unescape("&lt;a&gt; &#x10FFFF; &unknownreference;"), "<a> \u{10FFFF} &unknownreference;");
    assert_eq!(unescape("C&#0000000097;non &#x0000041; &#000000065; &#X42;"), "Canon A A B");
    assert_eq!(unescape("&abc&#xyz;&;"), "&abc&#xyz;&;");
    assert_eq!(unescape("a &unknown; b &amp c"), "a &unknown; b &amp c");
    use craft_xml::entity;
    assert_eq!((entity("#+65"), entity("#x+41"), entity("#"), entity("#x"), entity("#-1")), (None, None, None, None, None));
    assert_eq!((entity("#65"), entity("#x41"), entity("apos"), entity("nbsp")), (Some('A'), Some('A'), Some('\''), None));
    assert_eq!((entity("#0"), entity("#1"), entity("#xD800"), entity("#x110000"), entity("#99999999999999999999")), (None, None, None, None, None));
    // The same holds in a document: attribute values and text.
    let e = parse(format!("<lens maker='{hostile}' model='C&#0000000097;non'>&#x0000041;</lens>").as_bytes()).unwrap();
    assert_eq!(e.attr("maker").map(str::len), Some(200_002));
    assert_eq!((e.attr("model"), e.text().as_ref()), (Some("Canon"), "A"));
    assert_eq!(e.child_text("missing"), None);
}

// ------------------------------------------------------------------ FilmCraft: pretty writer, DOCTYPE

#[test]
fn film_writer_roundtrip() {
    let mut w = Writer::pretty("  ");
    w.decl(false);
    w.doctype("xmeml");
    w.open("a", &[("x", "1 & 2")]);
    w.leaf("b", &[], "<hi>");
    w.empty("c", &[]);
    w.open("d", &[]);
    w.tag("e").attr("n", 1.5).text("t");
    let s = w.finish();
    assert_eq!(
        s,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n<a x=\"1 &amp; 2\">\n  <b>&lt;hi&gt;</b>\n  <c/>\n  <d>\n    <e n=\"1.5\">t</e>\n  </d>\n</a>\n"
    );
    let film = Options { doctype: Doctype::RejectInternalSubset, drop_blank_text: true, ..Options::default() };
    let a = film.parse(s.as_bytes()).unwrap();
    assert_eq!(a.attr("x"), Some("1 & 2"));
    assert_eq!(a.child_text("b").as_deref(), Some("<hi>"));
    assert!(a.child("c").is_some());
}

#[test]
fn film_attr_whitespace_roundtrip() {
    let mut w = Writer::pretty("  ");
    w.open("a", &[("name", "x\ny\tz\r")]);
    w.leaf("b", &[], "a\nb");
    let a = parse(w.finish().as_bytes()).unwrap();
    assert_eq!(a.attr("name"), Some("x\ny\tz\r"));
    assert_eq!(a.child_text("b").as_deref(), Some("a\nb"));
    // Literal whitespace in an attribute value is normalised to spaces, as XML says.
    assert_eq!(parse(b"<a v='x\ny\tz\r\nw'/>").unwrap().attr("v"), Some("x y z w"));
}

#[test]
fn film_rejects_internal_dtd_entity_expansion() {
    let film = Options { doctype: Doctype::RejectInternalSubset, ..Options::default() };
    assert_eq!(film.parse(b"<!DOCTYPE root [<!ENTITY word 'expanded'>]><root>&word;</root>"), Err(Error::Doctype));
    assert!(film.parse(b"<!DOCTYPE xmeml><xmeml/>").is_ok());
    assert!(film.parse(b"<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\"><svg/>").is_ok());
}

// ------------------------------------------------------------------ MailCraft: no DOCTYPE at all

#[test]
fn mail_a_document_with_a_doctype_is_not_read() {
    let mail = Options { doctype: Doctype::Reject, ..Options::default() }.strict();
    assert_eq!(mail.parse(b"<!DOCTYPE s><s/>"), Err(Error::Doctype));
    assert_eq!(
        mail.parse(b"<?xml version=\"1.0\"?><!DOCTYPE s [<!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;&a;&a;&a;&a;\">]><s>&b;</s>"),
        Err(Error::Doctype)
    );
    assert!(matches!(mail.parse(b"<s>&b;</s>"), Err(Error::UnknownEntity { name, .. }) if name == "b"));
    assert!(mail.parse(b"<d:multistatus xmlns:d='DAV:'><d:href>/a&amp;b</d:href></d:multistatus>").is_ok());
}

// ------------------------------------------------------------------ WordCraft ODT: events with byte ranges

#[test]
fn odt_events_with_byte_ranges() {
    let xml = b"<office:text><text:p text:style-name=\"P1\">a &amp; b<text:s/></text:p></office:text>";
    let opts = Options::default();
    let mut events = opts.events(xml);
    let mut seen = Vec::new();
    while let Some(ev) = events.next().unwrap() {
        seen.push((ev, events.span()));
    }
    let start = |name: &str, attrs: &[(&str, &str)]| Event::Start {
        name: name.into(),
        attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
    };
    let end = |name: &str| Event::End { name: name.into() };
    let expect = vec![
        (start("office:text", &[]), 0..13),
        (start("text:p", &[("text:style-name", "P1")]), 13..42),
        (Event::Text("a ".into()), 42..44),
        (Event::Text("&".into()), 44..49),
        (Event::Text(" b".into()), 49..51),
        (start("text:s", &[]), 51..60),
        (end("text:s"), 51..60),
        (end("text:p"), 60..69),
        (end("office:text"), 69..83),
    ];
    assert_eq!(seen, expect);
    assert_eq!((&xml[51..60], xml.len()), (&b"<text:s/>"[..], 83));
    // Malformed markup ends the stream with an error; what came before it was delivered.
    let mut events = opts.events(b"<a><b x='1'>t</b><");
    let mut count = 0;
    let err = loop {
        match events.next() {
            Ok(Some(_)) => count += 1,
            Ok(None) => break None,
            Err(e) => break Some(e),
        }
    };
    assert_eq!(count, 4);
    assert!(matches!(err, Some(Error::Syntax { .. })), "{err:?}");
}
