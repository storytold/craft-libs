//! The TTML reader used to sit on `roxmltree` and now sits on `craft-xml`. Its logic is the same;
//! what changed is the tree it is handed. This compares the two trees on TTML documents: element
//! names by namespace, attributes by namespace, and character data, in document order.

use craft_captions::{Cue, Document, Format, FrameRate, WriteOptions, write};

const NAMESPACES: [(&str, &str); 11] = [
    ("tt", "http://www.w3.org/ns/ttml"),
    ("tt10", "http://www.w3.org/2006/10/ttaf1"),
    ("tt04", "http://www.w3.org/2006/04/ttaf1"),
    ("ttm", "http://www.w3.org/ns/ttml#metadata"),
    ("ttm10", "http://www.w3.org/2006/10/ttaf1#metadata"),
    ("ttm04", "http://www.w3.org/2006/04/ttaf1#metadata"),
    ("tts", "http://www.w3.org/ns/ttml#styling"),
    ("tts10", "http://www.w3.org/2006/10/ttaf1#style"),
    ("tts10s", "http://www.w3.org/2006/10/ttaf1#styling"),
    ("tts04", "http://www.w3.org/2006/04/ttaf1#style"),
    ("tts04s", "http://www.w3.org/2006/04/ttaf1#styling"),
];

/// A name the way the reader's canonical names spell it.
fn name(namespace: Option<&str>, local: &str) -> String {
    match namespace {
        None => local.to_string(),
        Some("http://www.w3.org/XML/1998/namespace") => format!("xml:{local}"),
        Some(uri) => format!("{}:{local}", NAMESPACES.iter().find(|(_, u)| *u == uri).map_or("?", |(p, _)| p)),
    }
}

fn dump_rox(n: roxmltree::Node, out: &mut Vec<String>) {
    let mut attrs: Vec<String> = n.attributes().map(|a| format!("@{}={}", name(a.namespace(), a.name()), a.value())).collect();
    attrs.sort();
    out.push(format!("<{} {}", name(n.tag_name().namespace(), n.tag_name().name()), attrs.join(" ")));
    let mut text = String::new();
    for c in n.children() {
        if c.is_text() {
            text.push_str(c.text().unwrap_or(""));
        } else if c.is_element() {
            out.push(format!("#{}", std::mem::take(&mut text)));
            dump_rox(c, out);
        }
    }
    out.push(format!("#{text}"));
    out.push(">".into());
}

fn dump_craft(n: &craft_xml::El, out: &mut Vec<String>) {
    let mut attrs: Vec<String> = n.attrs.iter().map(|(k, v)| format!("@{k}={v}")).collect();
    attrs.sort();
    out.push(format!("<{} {}", n.name, attrs.join(" ")));
    let mut text = String::new();
    for c in &n.children {
        match c {
            craft_xml::Node::Text(t) | craft_xml::Node::CData(t) => text.push_str(t),
            craft_xml::Node::El(e) => {
                out.push(format!("#{}", std::mem::take(&mut text)));
                dump_craft(e, out);
            }
            _ => {}
        }
    }
    out.push(format!("#{text}"));
    out.push(">".into());
}

fn documents() -> Vec<String> {
    let mut docs: Vec<String> = [
        r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div><p begin="1s" end="2s">Hi</p></div></body></tt>"#,
        // prefixed content, styling and metadata; xml:lang, xml:id, xml:space
        r#"<?xml version="1.0" encoding="UTF-8"?>
<x:tt xmlns:x="http://www.w3.org/ns/ttml" xmlns:s="http://www.w3.org/ns/ttml#styling" xmlns:m="http://www.w3.org/ns/ttml#metadata"
      xmlns:p="http://www.w3.org/ns/ttml#parameter" xml:lang="pt-BR" p:frameRate="30" p:frameRateMultiplier="1000 1001">
  <x:head><x:metadata><m:agent xml:id="a1" type="person"><m:name type="full" xml:space="preserve"> Ann  Lee </m:name></m:agent></x:metadata>
    <x:styling><x:style xml:id="it" s:fontStyle="italic"/></x:styling></x:head>
  <x:body begin="10s"><x:div end="20s"><x:p xml:id="c1" begin="1s" dur="2s" m:agent="a1" style="it">one<x:br/>two <x:span s:fontWeight="bold">three</x:span> four</x:p></x:div></x:body>
</x:tt>"#,
        // the 2006 drafts
        r#"<tt xmlns="http://www.w3.org/2006/10/ttaf1" xmlns:ttm="http://www.w3.org/2006/10/ttaf1#metadata" xmlns:tts="http://www.w3.org/2006/10/ttaf1#style">
<head><ttm:agent xml:id="b"><ttm:name>Bo</ttm:name></ttm:agent></head><body><div><p begin="00:00:01:12" end="00:00:02.5" ttm:agent="b" tts:textDecoration="underline">x</p></div></body></tt>"#,
        r#"<tt xmlns="http://www.w3.org/2006/04/ttaf1" xmlns:ttm="http://www.w3.org/2006/04/ttaf1#metadata"><head><ttm:agent id="b"><ttm:name type="full">Bo</ttm:name></ttm:agent></head>
<body><div><p begin="1s" end="2s" ttm:agent="b">x</p></div></body></tt>"#,
        // no namespace at all, an unknown namespace, and a foreign element inside
        r#"<tt><body timeContainer="seq"><div><p begin="1s" end="2s" agent="_Some_Name">x</p></div></body></tt>"#,
        r#"<tt xmlns="urn:example:other" xmlns:o="urn:example:more"><head><o:thing o:k="v" k="w"/></head><body><div><p begin="1s" end="2s">x</p></div></body></tt>"#,
        // character data: references, CDATA, comments, processing instructions, mixed whitespace
        "<tt xmlns=\"http://www.w3.org/ns/ttml\"><body><div><p begin=\"1s\" end=\"2s\" title=\"a &amp; b &#x41;&#10;\">\n  a &lt;b&gt; &amp; &#169; &#x1F600;<![CDATA[ <raw> & ]]>tail<!-- note -->\u{a0}more<?pi data?> end\r\n</p></div></body></tt>",
        // a namespace redeclared in an inner scope
        r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:a="http://www.w3.org/ns/ttml#metadata"><head><a:agent xml:id="x" xmlns:a="urn:example:shadow"><a:name>no</a:name></a:agent></head><body/></tt>"#,
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    // and what the writer produces, in both flavours
    let rate = FrameRate::FPS_25;
    let cue = |f: i64, text: &str, speaker: Option<&str>| Cue {
        start: rate.tick_of(f),
        end: rate.tick_of(f + 25),
        text: text.into(),
        speaker: speaker.map(str::to_string),
        ..Default::default()
    };
    let doc = Document {
        cues: vec![cue(25, "<i>Olá</i> & <b>tchau</b>\n1 < 2", Some("Ana \"A\" <x>")), cue(75, "<u>second</u>\u{a0}line", None)],
        language: Some("pt".into()),
        ..Default::default()
    };
    for format in [Format::Ttml, Format::Dfxp] {
        docs.push(String::from_utf8(write(&doc, format, WriteOptions { rate: Some(rate), ..Default::default() })).unwrap());
    }
    docs
}

#[test]
fn craft_xml_hands_the_ttml_reader_the_tree_roxmltree_did() {
    let canonical = craft_xml::Canonical::new(&NAMESPACES);
    for text in documents() {
        let rox = roxmltree::Document::parse(&text).unwrap();
        let craft = craft_xml::Options::canonical(&canonical).strict().parse(text.as_bytes()).unwrap();
        let (mut a, mut b) = (Vec::new(), Vec::new());
        dump_rox(rox.root_element(), &mut a);
        dump_craft(&craft, &mut b);
        assert_eq!(a, b, "{text}");
    }
}
