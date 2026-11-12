//! The story model.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_idml::model::{Ace, ContentPart, ItemKind, Story, TextNode, stories};
use craft_idml::{parse_part, part_root, part_xml};

/// A story as InDesign writes one: two paragraphs; overrides; a page number and a tab in the
/// content; a hyperlink and a tracked change around text; a footnote; a table; an anchored
/// rectangle; elements the model does not type.
const STORY: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="16.0">
	<Story Self="us1" AppliedTOCStyle="n" TrackChanges="false">
		<StoryPreference OpticalMarginAlignment="true" OpticalMarginSize="12" StoryDirection="LeftToRightDirection" />
		<InCopyExportOption IncludeGraphicProxies="true" />
		<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Head">
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" PointSize="18">
				<Properties><AppliedFont type="string">Minion Pro</AppliedFont></Properties>
				<Content>Page <?ACE 18?>	of  two</Content>
				<Br />
			</CharacterStyleRange>
		</ParagraphStyleRange>
		<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body">
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Em">
				<HyperlinkTextSource Self="uh1" Name="link"><Content>a link</Content></HyperlinkTextSource>
				<Change ChangeType="InsertedText"><Content> added</Content></Change>
				<Content> &amp; <![CDATA[<raw>]]></Content>
				<Footnote><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Note"><CharacterStyleRange><Content><?ACE 4?> A note.</Content></CharacterStyleRange></ParagraphStyleRange></Footnote>
				<Table Self="ut1" BodyRowCount="1" ColumnCount="2">
					<Row Self="ut1Row0" Name="0" SingleRowHeight="20" />
					<Column Self="ut1Column0" Name="0" SingleColumnWidth="100" />
					<Column Self="ut1Column1" Name="1" SingleColumnWidth="50" />
					<Cell Self="ut1i0" Name="0:0" ColumnSpan="2" RowSpan="1"><ParagraphStyleRange><CharacterStyleRange><Content>cell</Content></CharacterStyleRange></ParagraphStyleRange></Cell>
				</Table>
				<Rectangle Self="ur1" ItemTransform="1 0 0 1 0 0"><AnchoredObjectSetting AnchoredPosition="InlinePosition" /></Rectangle>
				<HyperlinkTextDestination Self="ud1" Name="here" />
				<Content>end</Content>
			</CharacterStyleRange>
		</ParagraphStyleRange>
	</Story>
</idPkg:Story>
"#;

fn story() -> Story {
    stories(&parse_part(STORY.as_bytes()).unwrap().elements().cloned().collect::<Vec<_>>()).remove(0)
}

fn ace(a: Ace) -> Option<char> {
    (a == Ace::PageNumber).then_some('#')
}

#[test]
fn a_story_reads_as_ranges_and_content() {
    let s = story();
    assert_eq!(s.values.self_id(), Some("us1"));
    let pref = s.preference().unwrap();
    assert_eq!((pref.boolean("OpticalMarginAlignment"), pref.num("OpticalMarginSize")), (Some(true), Some(12.0)));
    let paras = s.paragraphs();
    assert_eq!(paras.iter().map(|p| p.paragraph_style().unwrap()).collect::<Vec<_>>(), ["ParagraphStyle/Head", "ParagraphStyle/Body"]);
    let head = paras[0].characters()[0];
    assert_eq!((head.values.num("PointSize"), head.values.value("AppliedFont").as_deref()), (Some(18.0), Some("Minion Pro")));
    // Content keeps its whitespace and its special characters in place.
    let TextNode::Content(parts) = &head.children[0] else { panic!() };
    assert_eq!(parts, &[ContentPart::Text("Page ".into()), ContentPart::Ace(Ace::PageNumber), ContentPart::Text("\tof  two".into())]);
    assert_eq!(head.children[1], TextNode::Break);
    // The text of the story: hyperlink and change text belong to it, the footnote and the table
    // do not.
    assert_eq!(s.text(ace), "Page #\tof  two\na link added & <raw>end");
    assert_eq!(s.text(|_| None), "Page \tof  two\na link added & <raw>end");
    // The body paragraph's character range holds every kind of node.
    let body = paras[1].characters();
    assert_eq!(body.len(), 1);
    let kinds: Vec<String> = body[0]
        .children
        .iter()
        .map(|n| match n {
            TextNode::Container(c) => c.name.clone(),
            TextNode::Content(_) => "Content".into(),
            TextNode::Table(_) => "Table".into(),
            TextNode::Item(i) => format!("item {}", i.kind.name()),
            TextNode::Other(e) => format!("other {}", e.name),
            n => format!("{n:?}"),
        })
        .collect();
    assert_eq!(
        kinds,
        ["HyperlinkTextSource", "Change", "Content", "Footnote", "Table", "item Rectangle", "other HyperlinkTextDestination", "Content"]
    );
}

#[test]
fn footnotes_tables_and_anchored_items_are_text_and_items_of_their_own() {
    let s = story();
    let body = s.paragraphs()[1].characters()[0];
    let TextNode::Container(note) = &body.children[3] else { panic!() };
    assert_eq!(note.name, "Footnote");
    assert_eq!(note.paragraphs()[0].paragraph_style(), Some("ParagraphStyle/Note"));
    assert_eq!(note.text(|a| (a == Ace::FootnoteNumber).then_some('1')), "1 A note.");
    let TextNode::Table(table) = &body.children[4] else { panic!() };
    assert_eq!(table.rows().len(), 1);
    assert_eq!(table.columns().iter().map(|c| c.num("SingleColumnWidth").unwrap()).collect::<Vec<_>>(), [100.0, 50.0]);
    let cell = table.cells().next().unwrap();
    assert_eq!((cell.position(), cell.span()), (Some((0, 0)), (2, 1)));
    assert_eq!(cell.text(|_| None), "cell");
    let TextNode::Item(item) = &body.children[5] else { panic!() };
    assert_eq!(item.kind, ItemKind::Rectangle);
    assert_eq!(item.other("AnchoredObjectSetting").and_then(|a| a.attr("AnchoredPosition")), Some("InlinePosition"));
    let TextNode::Container(link) = &body.children[0] else { panic!() };
    assert_eq!((link.values.name(), link.text(|_| None).as_str()), (Some("link"), "a link"));
}

#[test]
fn a_story_written_and_read_again_is_the_same() {
    let s = story();
    let bytes = part_xml(&part_root("Story").with_child(s.to_el()));
    let again = stories(&parse_part(&bytes).unwrap().elements().cloned().collect::<Vec<_>>()).remove(0);
    assert_eq!(again, s);
    let xml = String::from_utf8(bytes).unwrap();
    // Content is written on one line, its whitespace and instructions as they were.
    assert!(xml.contains("<Content>Page <?ACE 18?>\tof  two</Content>"), "{xml}");
    assert!(xml.contains("<Content> &amp; &lt;raw&gt;</Content>"), "{xml}");
}

#[test]
fn unknown_ace_codes_and_instructions_are_kept() {
    let el = parse_part(b"<Story><ParagraphStyleRange><CharacterStyleRange><Content>a<?ACE 99?>b<?ACE x?><?other data?></Content></CharacterStyleRange></ParagraphStyleRange></Story>").unwrap();
    let s = Story::from_el(&el);
    let TextNode::Content(parts) = &s.paragraphs()[0].characters()[0].children[0] else { panic!() };
    assert_eq!(
        parts,
        &[
            ContentPart::Text("a".into()),
            ContentPart::Ace(Ace::Other(99)),
            ContentPart::Text("b".into()),
            ContentPart::Pi { target: "ACE".into(), data: "x".into() },
            ContentPart::Pi { target: "other".into(), data: "data".into() },
        ]
    );
    assert_eq!(Story::from_el(&parse_part(&part_xml(&s.to_el())).unwrap()), s);
    for code in [4, 7, 8, 18, 19, 3] {
        assert_eq!(Ace::from_code(code).code(), code);
    }
}

/// Text containers nest as deep as the XML reader allows; reading the text does not recurse.
#[test]
fn deeply_nested_containers_are_read() {
    let depth = 200;
    let xml = format!("<Story>{}<Content>x</Content>{}</Story>", "<XMLElement>".repeat(depth), "</XMLElement>".repeat(depth));
    let s = Story::from_el(&parse_part(xml.as_bytes()).unwrap());
    assert_eq!(s.text(|_| None), "x");
}
