//! The package: reading parts within the limits, lenient names, designmap includes, writing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Cursor, Read, Write};

use craft_idml::{
    El, Error, Limits, Local, MIMETYPE, Node, Package, Props, designmap_xml, is_idml, parse_part, part_root, part_xml, prop, with_props,
    write_package,
};

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in entries {
        z.start_file(*name, o).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn designmap() -> El {
    El::new("Document")
        .with_attr("xmlns:idPkg", craft_idml::PACKAGING_NS)
        .with_attr("DOMVersion", craft_idml::DOM_VERSION)
        .with_attr("Self", "d")
        .with_child(El::new("Language").with_attr("Self", "Language/$ID/English%3a USA"))
        .with_child(El::new("idPkg:Styles").with_attr("src", "Resources/Styles.xml"))
        .with_child(El::new("Layer").with_attr("Self", "ua"))
        .with_child(El::new("idPkg:Spread").with_attr("src", "Spreads/Spread_u1.xml"))
        .with_child(El::new("idPkg:Story").with_attr("src", "Stories/Story_u2.xml"))
}

fn story() -> El {
    let mut content = El::new("Content").with_text("Page ");
    content.push(Node::Pi { target: "ACE".into(), data: "18".into() });
    let range = with_props(
        El::new("CharacterStyleRange").with_attr("PointSize", "9").with_child(content).with_child(El::new("Br")),
        vec![prop("Leading", "unit", "14")],
    );
    part_root("Story").with_child(El::new("Story").with_attr("Self", "u2").with_child(El::new("ParagraphStyleRange").with_child(range)))
}

fn sample() -> Vec<u8> {
    let styles =
        part_root("Styles").with_child(El::new("RootParagraphStyleGroup").with_attr("Self", "g")).with_child(El::new("RootCharacterStyleGroup"));
    let spread = part_root("Spread").with_child(El::new("Spread").with_attr("Self", "u1"));
    write_package(&[
        ("META-INF/container.xml", craft_idml::CONTAINER_XML.as_bytes().to_vec()),
        ("META-INF/metadata.xml", b"<x:xmpmeta><xmp:CreateDate>2026-01-02T03:04:05Z</xmp:CreateDate></x:xmpmeta>".to_vec()),
        ("designmap.xml", designmap_xml(&designmap())),
        ("Resources/Styles.xml", part_xml(&styles)),
        ("Spreads/Spread_u1.xml", part_xml(&spread)),
        ("Stories/Story_u2.xml", part_xml(&story())),
    ])
    .unwrap()
}

#[test]
fn a_written_package_starts_with_the_stored_mimetype() {
    let bytes = sample();
    assert!(is_idml(&bytes));
    assert!(
        !is_idml(b"PK\x03\x04") && !is_idml(b"") && !is_idml(&zip_of(&[("mimetype", MIMETYPE.as_bytes())])),
        "a compressed mimetype is not the signature"
    );
    let mut z = zip::ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
    assert_eq!(z.name_for_index(0).unwrap().unwrap(), "mimetype");
    let mut first = z.by_index(0).unwrap();
    assert_eq!(first.compression(), zip::CompressionMethod::Stored);
    let mut s = String::new();
    first.read_to_string(&mut s).unwrap();
    assert_eq!(s, MIMETYPE);
}

#[test]
fn writing_two_parts_with_one_name_is_an_error() {
    assert!(matches!(write_package(&[("a.xml", b"1"), ("a.xml", b"2")]), Err(Error::Write(_))));
    assert!(matches!(write_package(&[("mimetype", b"1")]), Err(Error::Write(_))));
}

#[test]
fn the_document_has_its_includes_in_place() {
    let bytes = sample();
    let doc = Package::open(&bytes).unwrap().document().unwrap();
    assert_eq!(doc.root.attr("Self"), Some("d"));
    assert_eq!(doc.root.elements().filter(|e| e.name.starts_with("idPkg:")).count(), 3, "the root keeps its includes as written");
    let names: Vec<&str> = doc.top.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Language", "RootParagraphStyleGroup", "RootCharacterStyleGroup", "Layer", "Spread", "Story"]);
    assert!(doc.warnings.is_empty());
    assert_eq!(doc.metadata.as_deref().and_then(|m| craft_idml::value::xmp_value(m, "xmp:CreateDate")), Some("2026-01-02T03:04:05Z"));
}

#[test]
fn story_content_survives_writing_and_reading() {
    let bytes = sample();
    let doc = Package::open(&bytes).unwrap().document().unwrap();
    let story = doc.top.iter().find(|e| e.name == "Story").unwrap();
    let range = story.find(Local("CharacterStyleRange")).unwrap();
    assert_eq!(range.num("PointSize"), Some(9.0));
    assert_eq!(range.num("Leading"), Some(14.0), "the Properties form");
    let content = range.child("Content").unwrap();
    assert_eq!(content.children, [Node::Text("Page ".into()), Node::Pi { target: "ACE".into(), data: "18".into() }]);
    assert_eq!(content.deep_text(), "Page ");
}

#[test]
fn a_listed_part_that_is_missing_is_a_warning() {
    let bytes = write_package(&[("designmap.xml", designmap_xml(&designmap()))]).unwrap();
    let doc = Package::open(&bytes).unwrap().document().unwrap();
    assert_eq!(doc.top.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Language", "Layer"]);
    assert_eq!(doc.warnings.len(), 3);
    assert!(doc.warnings[0].starts_with("Resources/Styles.xml: listed in designmap.xml but missing"), "{:?}", doc.warnings);
    assert_eq!(doc.metadata, None);
}

#[test]
fn a_listed_part_that_cannot_be_read_is_an_error() {
    let bytes =
        write_package(&[("designmap.xml", designmap_xml(&designmap())), ("Resources/Styles.xml", b"<idPkg:Styles><a></idPkg:Styles>".to_vec())])
            .unwrap();
    match Package::open(&bytes).unwrap().document() {
        Err(Error::Part { part, .. }) => assert_eq!(part, "Resources/Styles.xml"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn what_is_not_an_idml_package_is_refused() {
    assert!(matches!(Package::open(b"not a zip").err(), Some(Error::NotIdml(_))));
    let no_designmap = zip_of(&[("mimetype", MIMETYPE.as_bytes())]);
    assert!(matches!(Package::open(&no_designmap).unwrap().document(), Err(Error::NotIdml(m)) if m.contains("designmap")));
    let other_type = zip_of(&[("mimetype", b"application/epub+zip"), ("designmap.xml", b"<Document/>")]);
    assert!(matches!(Package::open(&other_type).unwrap().document(), Err(Error::NotIdml(m)) if m.contains("mimetype")));
    let other_root = zip_of(&[("designmap.xml", b"<Workbook/>")]);
    assert!(matches!(Package::open(&other_root).unwrap().document(), Err(Error::NotIdml(m)) if m.contains("<Workbook>")));
    // A package without a mimetype part, with trailing whitespace in it, is read.
    let padded = zip_of(&[("mimetype", format!("{MIMETYPE}\n").as_bytes()), ("designmap.xml", b"<Document/>")]);
    assert!(Package::open(&padded).unwrap().document().is_ok());
    assert!(Package::open(&zip_of(&[("designmap.xml", b"<Document/>")])).unwrap().document().is_ok());
}

/// Writers differ in percent-encoding, case, path separators and a leading `./` or `/`.
#[test]
fn part_names_are_matched_leniently() {
    let bytes = zip_of(&[("./Stories\\Story%20One.xml", b"first"), ("stories/story one.XML", b"second"), ("dir/", b"")]);
    let mut pkg = Package::open(&bytes).unwrap();
    for name in ["Stories/Story One.xml", "Stories/Story%20One.xml", "/stories/STORY one.xml"] {
        assert_eq!(pkg.part(name).unwrap().as_deref(), Some(&b"first"[..]), "{name}: the first entry wins");
    }
    assert_eq!(pkg.part("stories/story one.XML").unwrap().as_deref(), Some(&b"second"[..]), "an exact name is its own entry");
    assert_eq!(pkg.part("Stories/Other.xml").unwrap(), None);
    assert_eq!(pkg.part("dir/").unwrap(), None);
}

// ---------- limits ----------

#[test]
fn a_part_past_the_part_limit_is_refused() {
    let bytes = zip_of(&[("small", &[b'a'; 100]), ("big", &[b'a'; 101])]);
    let mut pkg = Package::open_with(&bytes, Limits { max_part_bytes: 100, ..Limits::DEFAULT }).unwrap();
    assert_eq!(pkg.part("small").unwrap().map(|p| p.len()), Some(100));
    assert_eq!(pkg.part("big"), Err(Error::TooLarge { part: "big".into(), limit: 100 }));
}

#[test]
fn parts_cannot_add_up_past_the_total_budget() {
    let bytes = zip_of(&[("a", &[b'a'; 100]), ("b", &[b'a'; 100]), ("c", &[b'a'; 100])]);
    let mut pkg = Package::open_with(&bytes, Limits { max_total_bytes: 250, ..Limits::DEFAULT }).unwrap();
    assert!(pkg.part("a").unwrap().is_some());
    assert!(pkg.part("b").unwrap().is_some());
    assert_eq!(pkg.budget_left(), 50);
    assert_eq!(pkg.part("c"), Err(Error::TooLarge { part: "c".into(), limit: 50 }));
    // Reading a part again spends the budget again: it is a budget of work, not of distinct bytes.
    assert!(pkg.part("a").is_err());
}

#[test]
fn a_package_with_too_many_entries_is_refused() {
    let bytes = zip_of(&[("a", b"1"), ("b", b"2"), ("c", b"3")]);
    assert_eq!(Package::open_with(&bytes, Limits { max_entries: 2, ..Limits::DEFAULT }).err(), Some(Error::TooManyEntries { found: 3, limit: 2 }));
    assert!(Package::open_with(&bytes, Limits { max_entries: 3, ..Limits::DEFAULT }).is_ok());
}

/// Nesting past the budget is an error from the parser, not a stack overflow later.
#[test]
fn nesting_past_the_budget_is_refused() {
    let nested = |n: usize| format!("{}{}", "<A>".repeat(n), "</A>".repeat(n)).into_bytes();
    let depth = craft_idml::xml_limits().max_depth;
    assert!(parse_part(&nested(depth)).is_ok(), "the budget itself parses");
    assert_eq!(parse_part(&nested(depth + 1)), Err(craft_xml::Error::TooDeep(depth)));
    assert!(parse_part(&nested(100_000)).is_err());
}

#[test]
fn a_part_with_more_nodes_than_the_limit_is_refused() {
    let part = format!("<Document>{}</Document>", "<A/>".repeat(50));
    let bytes = zip_of(&[("designmap.xml", part.as_bytes())]);
    let limits = |max_nodes| Limits { xml: craft_xml::Limits { max_nodes, ..craft_idml::xml_limits() }, ..Limits::DEFAULT };
    assert!(Package::open_with(&bytes, limits(51)).unwrap().document().is_ok());
    assert!(matches!(Package::open_with(&bytes, limits(50)).unwrap().document(), Err(Error::Part { msg, .. }) if msg.contains("50 nodes")));
}

// ---------- the XML of a part ----------

#[test]
fn parts_are_read_strictly_but_as_indesign_writes_them() {
    let x = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<?aid style=\"50\" type=\"document\" ?>\n<Document A=\"x&quot;&lt;&amp;&#x9;\" A2='a\tb'>\n\t<Content>a\tb &amp; &lt;c&gt;<![CDATA[ <raw> ]]></Content>\n</Document>\n";
    let root = parse_part(x).unwrap();
    assert_eq!(root.attr("A"), Some("x\"<&\t"));
    assert_eq!(root.attr("A2"), Some("a b"), "a literal tab in an attribute is a space");
    assert_eq!(root.child("Content").unwrap().deep_text(), "a\tb & <c> <raw> ");
    for cut in ["<Document><A>", "<Document><A></B></Document>", "<Document>", ""] {
        assert!(parse_part(cut.as_bytes()).is_err(), "{cut}");
    }
}

#[test]
fn properties_form() {
    let e = parse_part(br#"<A PointSize="9" On="true" Off=" false " W="NaN" X="inf" Blank="  "><Properties><Leading type="unit">14</Leading><Y type="unit">-inf</Y><Z>2.5</Z><Font type="string"> Minion </Font></Properties><Leading>99</Leading></A>"#).unwrap();
    assert_eq!(e.num("PointSize"), Some(9.0));
    assert_eq!(e.num("Leading"), Some(14.0), "only a child of Properties is a property");
    assert_eq!(e.prop("Missing"), None);
    assert_eq!(e.prop_el("Leading").and_then(|l| l.attr("type")), Some("unit"));
    // `NaN` and infinities parse as `f64` but are not usable numbers.
    assert_eq!((e.num("W"), e.num("X"), e.num("Y"), e.num("Z")), (None, None, None, Some(2.5)));
    assert_eq!((e.boolean("On"), e.boolean("Off"), e.boolean("PointSize"), e.boolean("Missing")), (Some(true), Some(false), None, None));
    assert_eq!(e.prop("Font").as_deref(), Some(" Minion "));
    assert_eq!(e.value("Font").as_deref(), Some("Minion"));
    assert_eq!(e.value("Blank"), None);
}

#[test]
fn parts_are_written_one_element_per_line_without_touching_text() {
    let bytes = part_xml(&story());
    let s = String::from_utf8(bytes.clone()).unwrap();
    assert!(s.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<idPkg:Story xmlns:idPkg=\"http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging\" DOMVersion=\"16.0\">\n\t<Story Self=\"u2\">\n"), "{s}");
    assert!(s.contains("\t\t\t\t<Properties>\n\t\t\t\t\t<Leading type=\"unit\">14</Leading>\n\t\t\t\t</Properties>\n"), "{s}");
    assert!(s.contains("<Content>Page <?ACE 18?></Content>"), "{s}");
    assert_eq!(parse_part(&bytes).unwrap().find(Local("Leading")).map(El::deep_text).as_deref(), Some("14"));
    let dm = String::from_utf8(designmap_xml(&designmap())).unwrap();
    assert!(
        dm.contains("?>\n<?aid style=\"50\" type=\"document\" readerVersion=\"6.0\" featureSet=\"257\" product=\"16.0(1)\" ?>\n<Document "),
        "{dm}"
    );
    assert!(with_props(El::new("A"), Vec::new()).children.is_empty());
}

/// A ZIP64 end record can declare millions of entries in a small file; the count is refused on
/// the raw bytes, before the archive is indexed.
#[test]
fn a_package_declaring_too_many_entries_is_refused_before_it_is_indexed() {
    let bomb = craft_zip::zip64_entry_count_sample(100_001);
    assert_eq!(Package::open(&bomb).err(), Some(Error::TooManyEntries { found: 100_001, limit: 100_000 }));
}

#[test]
fn equal_parts_give_equal_bytes() {
    assert_eq!(sample(), sample());
}
