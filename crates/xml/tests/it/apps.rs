//! The tests the Craft apps had for their own XML helpers, against this crate: each app's
//! expectations, read the way that app reads (see the migration table in the README).

use craft_xml::{Canonical, Doctype, El, Error, Event, Limits, Local, Node, NsName, Options, Scope, Writer, esc, esc_attr, parse, unescape};

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
fn word_deep_text_and_find() {
    let x = format!(r#"<w:p xmlns:w="{W}">a<w:r><w:t>b</w:t><w:r><w:t>c</w:t></w:r></w:r>d</w:p>"#);
    let p = word(x.as_bytes()).unwrap();
    assert_eq!(p.text(), "ad");
    assert_eq!(p.deep_text(), "abcd");
    assert_eq!(p.find("w:t").map(|t| t.text().into_owned()).as_deref(), Some("b"));
    assert_eq!(p.find_all("w:t").count(), 2);
    assert!(p.find("w:p").is_none(), "the element itself is not its own descendant");
}

// ------------------------------------------------------------------ GridCraft: local names, streaming

fn grid(bytes: &[u8]) -> Result<El, Error> {
    Options { unknown_entity: craft_xml::UnknownEntity::Literal, ..Options::local() }.parse(bytes)
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

// ------------------------------------------------------------------ AniCraft: strict, indentation dropped, mixed content

fn ani() -> Options<'static> {
    Options { strict: true, drop_blank_text: true, unknown_entity: craft_xml::UnknownEntity::Literal, ..Options::default() }
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
