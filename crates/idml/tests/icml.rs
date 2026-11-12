//! ICML: one story and what it uses, as one XML document.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_idml::icml::{MAX_ICML_BYTES, icml_xml, is_icml, read_icml};
use craft_idml::{El, Error};

const ICML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<?aid style="50" type="snippet" readerVersion="6.0" featureSet="513" product="16.0(1)" ?>
<?aid SnippetType="InCopyInterchange"?>
<Document DOMVersion="16.0" Self="d">
	<Color Self="Color/Black" Model="Process" Space="CMYK" ColorValue="0 0 0 100" />
	<RootParagraphStyleGroup Self="u1"><ParagraphStyle Self="ParagraphStyle/Body" Name="Body" /></RootParagraphStyleGroup>
	<idPkg:Graphic src="Resources/Graphic.xml" />
	<Story Self="us1"><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body"><CharacterStyleRange><Content>Hello</Content></CharacterStyleRange></ParagraphStyleRange></Story>
	<Story Self="us2"><ParagraphStyleRange><CharacterStyleRange><Content>Anchored</Content></CharacterStyleRange></ParagraphStyleRange></Story>
	<Hyperlink Self="uh1" Source="uh9" />
</Document>
"#;

#[test]
fn an_icml_file_reads_as_its_resources_and_stories() {
    assert!(is_icml(ICML.as_bytes()));
    assert!(is_icml(format!("\u{FEFF}  {ICML}").as_bytes()));
    assert!(!is_icml(b"<Document/>"));
    assert!(!is_icml(b"SnippetType=\"InCopyInterchange\""));
    let icml = read_icml(ICML.as_bytes()).unwrap();
    assert_eq!(icml.top.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Color", "RootParagraphStyleGroup", "Story", "Story", "Hyperlink"]);
    assert_eq!(icml.warnings, ["Resources/Graphic.xml: an ICML file has no package parts; it was not read"]);
    let stories = icml.stories();
    assert_eq!(stories.iter().map(|s| s.text(|_| None)).collect::<Vec<_>>(), ["Hello", "Anchored"]);
}

#[test]
fn an_icml_file_written_reads_back() {
    let icml = read_icml(ICML.as_bytes()).unwrap();
    let mut root = El::new("Document").with_attr("DOMVersion", craft_idml::DOM_VERSION).with_attr("Self", "d");
    root.children.extend(icml.top.iter().cloned().map(craft_idml::Node::El));
    let bytes = icml_xml(&root);
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<?aid style=\"50\" type=\"snippet\" readerVersion=\"6.0\" featureSet=\"513\" product=\"16.0(1)\" ?>\n<?aid SnippetType=\"InCopyInterchange\"?>\n<Document "), "{text}");
    assert!(is_icml(&bytes));
    assert_eq!(read_icml(&bytes).unwrap().stories(), icml.stories());
}

#[test]
fn what_is_no_icml_story_is_refused() {
    assert!(matches!(read_icml(b"<Workbook/>"), Err(Error::NotIdml(m)) if m.contains("<Workbook>")));
    assert!(matches!(read_icml(b"<Document><Color/></Document>"), Err(Error::NotIdml(m)) if m.contains("no story")));
    assert!(matches!(read_icml(b"<Document><Story>"), Err(Error::Part { .. })), "cut off");
    let huge = vec![b' '; MAX_ICML_BYTES + 1];
    assert_eq!(read_icml(&huge), Err(Error::TooLarge { part: "ICML".into(), limit: MAX_ICML_BYTES as u64 }));
}
