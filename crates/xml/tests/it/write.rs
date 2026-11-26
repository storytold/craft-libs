//! Escaping, serialising, the writer, and what a round trip keeps.

use craft_xml::{El, Node, Options, Writer, is_xml_char, parse};

/// Reads keeping everything the tree can hold.
fn full() -> Options<'static> {
    Options { cdata: true, pis: true, comments: true, ..Options::default() }
}

#[test]
fn every_character_survives_a_round_trip_or_is_dropped() {
    let all: String = (0..0x3000u32).chain([0xD7FF, 0xE000, 0xFFFD, 0xFFFE, 0xFFFF, 0x1_0000, 0x10_FFFF]).filter_map(char::from_u32).collect();
    let kept: String = all.chars().filter(|c| is_xml_char(*c)).collect();
    let root = El::new("a").with_attr("v", &all).with_text(&all);
    let back = parse(root.to_xml().as_bytes()).unwrap();
    assert_eq!(back.attr("v"), Some(kept.as_str()));
    assert_eq!(back.text(), kept);
    let mut w = Writer::new();
    w.tag("a").attr("v", &all).text(&all);
    assert_eq!(parse(w.finish().as_bytes()).unwrap(), back);
}

/// A name from outside cannot become markup: the element or attribute is left out.
#[test]
fn a_name_that_is_not_a_name_is_not_written() {
    let root = El::new("r")
        .with_attr("ok", 1)
        .with_attr("x onload=\"evil()\"", 2)
        .with_attr("", 3)
        .with_child(El::new("a><script").with_text("gone").with_child(El::new("inner")))
        .with_child(El::new("b").with_text("kept"))
        .with_node(Node::Pi { target: "bad target".into(), data: "x".into() })
        .with_node(Node::Pi { target: "xml".into(), data: "version='1.0'".into() })
        .with_node(Node::Pi { target: "p".into(), data: "a?>b".into() })
        .with_node(Node::Comment("a--b".into()))
        .with_node(Node::Comment("ends-".into()));
    assert_eq!(root.to_xml(), "<r ok=\"1\"><b>kept</b></r>");
    assert_eq!(root.to_xml_pretty("  "), "<r ok=\"1\"><b>kept</b></r>\n");
    assert_eq!(El::new("not a name").to_xml(), "");
    // An element whose content was all left out is written the way it reads back.
    let hollow = El::new("r").with_child(El::new("a").with_child(El::new("1")).with_text("\u{1}")).with_child(El::new("b").with_child(El::new("2")));
    assert_eq!(hollow.to_xml(), "<r><a/><b/></r>");
    assert_eq!(hollow.to_xml_pretty(" "), "<r>\n <a/>\n <b/>\n</r>\n");
}

#[test]
fn compact_round_trip_keeps_what_the_tree_holds() {
    let docs = [
        "<a/>",
        "<p:sp a=\"1\" xmlns:p=\"urn:p\" xmlns=\"urn:d\"><p:t>x &amp; y</p:t><e/></p:sp>",
        "<a> <b> leading</b>\n\t<b>trailing </b> mixed <c/> text </a>",
        "<a k=\"&quot;&amp;&lt;&gt;&#9;&#10;&#13;'\">&amp;&lt;&gt;&#13;\n\t</a>",
        "<a><![CDATA[<raw> & ]]]]><![CDATA[> end]]><?ACE 18?><?empty?><!-- note -->t<![CDATA[]]></a>",
        "<a x=\"1\" x=\"2\"><b><c><d>deep</d></c></b></a>",
        "<é:日本 ü=\"ö\">😀</é:日本>",
    ];
    for doc in docs {
        let tree = full().parse(doc.as_bytes()).unwrap();
        assert_eq!(tree.to_xml(), doc, "these are already in the form the writer uses");
        assert_eq!(full().parse(tree.to_xml().as_bytes()).unwrap(), tree);
    }
}

#[test]
fn pretty_output() {
    let tree = parse(b"<a x='1'><b><c/><d>text</d><e>mixed <f/> content</e></b><g/><h> </h></a>").unwrap();
    let pretty = tree.to_xml_pretty("  ");
    assert_eq!(pretty, "<a x=\"1\">\n  <b>\n    <c/>\n    <d>text</d>\n    <e>mixed <f/> content</e>\n  </b>\n  <g/>\n  <h> </h>\n</a>\n");
    // Character data is unchanged: reading it back without the indentation gives the tree.
    let back = Options { drop_blank_text: true, ..Options::default() }.parse(pretty.as_bytes()).unwrap();
    assert_eq!(back, tree);
    // Printing what was printed changes nothing.
    assert_eq!(parse(pretty.as_bytes()).unwrap().to_xml_pretty("  "), pretty);
    assert_eq!(tree.to_xml_pretty("\t").lines().nth(2), Some("\t\t<c/>"));
    // An element with a comment or an instruction among its children is written on one line.
    let noted = full().parse(b"<a><!-- n --><b/></a>").unwrap();
    assert_eq!(noted.to_xml_pretty("  "), "<a><!-- n --><b/></a>\n");
}

#[test]
fn writer_closes_names_in_order_and_what_is_left_open() {
    let mut w = Writer::new();
    for name in ["a", "w:body", "日本", "d"] {
        w.open(name, &[]);
    }
    w.close();
    w.empty("e", &[]);
    assert_eq!(w.finish(), "<a><w:body><日本><d></d><e/></日本></w:body></a>");
}
