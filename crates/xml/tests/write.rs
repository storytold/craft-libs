//! Escaping, serialising, the writer, and what a round trip keeps.

use craft_xml::{El, Local, Node, Options, Writer, esc, esc_attr, is_name, is_xml_char, parse, push_esc_attr};

/// Reads keeping everything the tree can hold.
fn full() -> Options<'static> {
    Options { cdata: true, pis: true, comments: true, ..Options::default() }
}

#[test]
fn escaping_text_and_attributes() {
    assert_eq!(esc("a & b < c > d \"q\" 'r'"), "a &amp; b &lt; c &gt; d \"q\" 'r'");
    assert_eq!(esc("tab\tlf\ncr\r]]>"), "tab\tlf\ncr&#13;]]&gt;");
    assert_eq!(esc_attr("a & b < c > d \"q\" 'r'"), "a &amp; b &lt; c &gt; d &quot;q&quot; 'r'");
    assert_eq!(esc_attr("tab\tlf\ncr\r"), "tab&#9;lf&#10;cr&#13;");
    // Characters XML 1.0 cannot carry, even as references, are dropped.
    let forbidden = "a\u{0}\u{1}\u{8}\u{B}\u{C}\u{E}\u{1F}\u{FFFE}\u{FFFF}b";
    assert_eq!((esc(forbidden), esc_attr(forbidden)), ("ab".to_string(), "ab".to_string()));
    assert_eq!(esc("\u{7F}\u{85}é日😀\u{FFFD}"), "\u{7F}\u{85}é日😀\u{FFFD}");
    let mut s = String::from("k=");
    push_esc_attr(&mut s, "\"");
    assert_eq!(s, "k=&quot;");
    assert!(is_xml_char('\t') && is_xml_char('\u{10FFFF}') && !is_xml_char('\u{0}') && !is_xml_char('\u{FFFE}'));
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

#[test]
fn names() {
    for good in ["a", "A1", "_x", "w:p", ":odd", "a-b.c", "é", "日本", "a\u{B7}"] {
        assert!(is_name(good), "{good}");
    }
    for bad in ["", "1a", "-a", ".a", "a b", "a>", "a=\"", "?:x", "a/", "<a", "a\u{0}", "\u{B7}a", "a\n"] {
        assert!(!is_name(bad), "{bad:?}");
    }
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
fn what_a_round_trip_does_not_keep() {
    let doc = "<?xml version='1.0'?>\n<!DOCTYPE a>\n<?before x?>\n<!-- before -->\n<a  b = '1'\n c=\"&#x32;\"><b></b><![CDATA[c]]>&#100;<!-- gone --><?gone too?>\r\n</a>\n<!-- after -->";
    let tree = parse(doc.as_bytes()).unwrap();
    // The declaration, the DOCTYPE and everything outside the root are not part of the tree.
    // Inside it: quoting and spacing in tags, how a character was written, `<b></b>` against
    // `<b/>`, CDATA sections as sections, comments and processing instructions (unless the
    // options keep them) and a literal CR are not kept.
    assert_eq!(tree.to_xml(), "<a b=\"1\" c=\"2\"><b/>cd\n</a>");
    let kept = full().parse(doc.as_bytes()).unwrap();
    assert_eq!(kept.to_xml(), "<a b=\"1\" c=\"2\"><b/><![CDATA[c]]>d<!-- gone --><?gone too?>\n</a>");
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
fn builder_and_accessors() {
    let mut root = El::new("root").with_attr("id", 7).with_child(El::new("p:item").with_attr("n", "1").with_text("one")).with_text("tail");
    root.push(El::new("q:item").with_node(Node::CData("two".into())));
    root.push(Node::Comment("c".into()));
    assert_eq!(root.to_xml(), "<root id=\"7\"><p:item n=\"1\">one</p:item>tail<q:item><![CDATA[two]]></q:item><!--c--></root>");
    assert_eq!(root.children(Local("item")).count(), 2);
    assert_eq!(root.children("p:item").count(), 1);
    assert_eq!(root.child_text(Local("item")).as_deref(), Some("one"));
    assert_eq!(root.child("q:item").map(|e| e.text().into_owned()).as_deref(), Some("two"), "CDATA is character data");
    assert_eq!((root.text().as_ref(), root.deep_text().as_str()), ("tail", "onetailtwo"));
    assert_eq!(root.path(&[Local("item")]).and_then(|e| e.attr("n")), Some("1"));
    assert_eq!(root.path::<&str>(&[]).map(|e| e.name.as_str()), Some("root"));
    let name = String::from("p:item");
    assert!(root.child(&name).is_some());
}

#[test]
fn writer_compact() {
    let mut w = Writer::new();
    w.open("a", &[("k", "v\"")]);
    w.open("b", &[]);
    w.text("t & u");
    w.cdata("x]]>y");
    w.close();
    w.pi("p", "d");
    w.tree(&El::new("c").with_text("<"));
    w.raw("<raw/>");
    assert_eq!(w.as_str(), "<a k=\"v&quot;\"><b>t &amp; u<![CDATA[x]]]]><![CDATA[>y]]></b><?p d?><c>&lt;</c><raw/>");
    w.close();
    w.close();
    w.close();
    assert_eq!(w.finish(), "<a k=\"v&quot;\"><b>t &amp; u<![CDATA[x]]]]><![CDATA[>y]]></b><?p d?><c>&lt;</c><raw/></a>");
    assert_eq!(Writer::document().into_bytes(), craft_xml::DECL_STANDALONE.as_bytes());
    let mut plain = Writer::new();
    plain.decl(false);
    plain.doctype("not a name");
    plain.pi("xml", "x");
    assert_eq!(plain.finish(), craft_xml::DECL);
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

#[test]
fn writer_pretty_nests_a_tree_at_the_current_depth() {
    let mut w = Writer::pretty("  ");
    w.open("a", &[]);
    w.tree(&El::new("b").with_child(El::new("c")));
    assert_eq!(w.finish(), "<a>\n  <b>\n    <c/>\n  </b>\n</a>\n");
}
