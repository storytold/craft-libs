//! Original XML and model fixtures for explicit no-fill over a colored paragraph style.
//! No document or oracle from another application is used (ODF paragraph background semantics).

use crate::text::tests::{NS_T, content_of, export_model, package, styles_of};
use crate::text::import;
use craft_wordmodel::props::ParaProps;
use craft_wordmodel::styles::{Style, StyleKind};
use craft_wordmodel::{Document, Paragraph, Rgb, para_block};

fn input() -> Vec<u8> {
    let styles = format!(
        r##"<office:document-styles {NS_T}><office:styles>
<style:style style:name="Base" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:background-color="#f1cc45"/></style:style>
<style:style style:name="Inherited" style:family="paragraph" style:parent-style-name="Base"/>
<style:style style:name="Cleared" style:family="paragraph" style:parent-style-name="Base"><style:paragraph-properties fo:background-color="transparent"/></style:style>
<style:style style:name="Colored" style:family="paragraph" style:parent-style-name="Cleared"><style:paragraph-properties fo:background-color="#248ab6"/></style:style>
</office:styles></office:document-styles>"##
    );
    let content = format!(
        r##"<office:document-content {NS_T}><office:automatic-styles>
<style:style style:name="AutoInherit" style:family="paragraph" style:parent-style-name="Base"/>
<style:style style:name="AutoClear" style:family="paragraph" style:parent-style-name="Base"><style:paragraph-properties fo:background-color="transparent"/></style:style>
<style:style style:name="AutoClearChild" style:family="paragraph" style:parent-style-name="AutoClear"/>
<style:style style:name="AutoColor" style:family="paragraph" style:parent-style-name="Cleared"><style:paragraph-properties fo:background-color="#248ab6"/></style:style>
</office:automatic-styles><office:body><office:text>
<text:p text:style-name="Base">base</text:p><text:p text:style-name="Inherited">inherited</text:p><text:p text:style-name="AutoInherit">automatic inherit</text:p>
<text:p text:style-name="Colored">named color</text:p><text:p text:style-name="AutoColor">automatic color</text:p>
<text:p text:style-name="Cleared">named clear</text:p><text:p text:style-name="AutoClear">automatic clear</text:p><text:p text:style-name="AutoClearChild">inherited automatic clear</text:p>
</office:text></office:body></office:document-content>"##
    );
    package(&content, Some(&styles))
}

fn fills(doc: &Document) -> Vec<(Option<Rgb>, bool)> {
    doc.body
        .iter()
        .map(|b| {
            let props = &b.as_para().unwrap().props;
            let resolved = doc.styles.resolve_para(props);
            (resolved.shading, resolved.shading_auto)
        })
        .collect()
}

#[test]
fn transparent_paragraph_shading_overrides_its_named_style() {
    let doc = import(&input()).unwrap();
    let yellow = (Some(Rgb(0xf1, 0xcc, 0x45)), false);
    let blue = (Some(Rgb(0x24, 0x8a, 0xb6)), false);
    assert_eq!(fills(&doc), [yellow, yellow, yellow, blue, blue, (None, true), (None, true), (None, true)]);
    let inherited = &doc.body[1].as_para().unwrap().props;
    assert!(inherited.shading.is_none() && !inherited.shading_auto, "absence remains inheritance");
    let cleared = &doc.body[5].as_para().unwrap().props;
    let style = doc.styles.get(cleared.style.as_deref().unwrap()).unwrap();
    assert_eq!(style.name, "Cleared");
    assert!(style.para.shading.is_none() && style.para.shading_auto, "the common style keeps its own clear");
    assert!(doc.body[6].as_para().unwrap().props.shading_auto, "automatic shading remains direct formatting");
}

#[test]
fn paragraph_no_fill_round_trips_with_a_colored_parent() {
    let doc = import(&input()).unwrap();
    let bytes = export_model(&doc).unwrap();
    let styles = styles_of(&bytes);
    let content = content_of(&bytes);
    assert!(
        styles.split("<style:style ").any(|s| {
            let s = s.split("</style:style>").next().unwrap();
            s.contains("style:name=\"Cleared\"")
                && s.contains("style:parent-style-name=\"Base\"")
                && s.contains("fo:background-color=\"transparent\"")
        }),
        "a named style must override its colored parent: {styles}"
    );
    assert!(
        content.split("<style:style ").any(|s| {
            let s = s.split("</style:style>").next().unwrap();
            s.contains("style:parent-style-name=\"Base\"") && s.contains("fo:background-color=\"transparent\"")
        }),
        "a direct clear must keep its colored parent and override it: {content}"
    );
    let back = import(&bytes).unwrap();
    assert_eq!(fills(&back), fills(&doc), "saving must not bring the style's color back");
    for (a, b) in doc.body.iter().zip(&back.body) {
        let a = a.as_para().unwrap();
        let b = b.as_para().unwrap();
        assert_eq!(a.text, b.text);
        assert_eq!(a.props.style, b.props.style, "paragraphs keep their named style");
    }
}

#[test]
fn paragraph_shading_prefers_a_fixed_color_to_auto() {
    let blue = Rgb(0x24, 0x8a, 0xb6);
    let red = Rgb(0xa4, 0x2b, 0x35);
    let mut doc = Document::new();
    std::sync::Arc::make_mut(&mut doc.styles).upsert(Style {
        id: "Fixed".into(),
        name: "Fixed".into(),
        kind: StyleKind::Paragraph,
        based_on: Some("Normal".into()),
        para: ParaProps { shading: Some(blue), shading_auto: true, ..Default::default() },
        ..Default::default()
    });
    let mut named = Paragraph::with_text("named", Default::default());
    named.props.style = Some("Fixed".into());
    let mut direct = Paragraph::with_text("direct", Default::default());
    direct.props = ParaProps { style: Some("Fixed".into()), shading: Some(red), shading_auto: true, ..Default::default() };
    doc.body = vec![para_block(named), para_block(direct)];
    let bytes = export_model(&doc).unwrap();
    assert!(!styles_of(&bytes).contains("fo:background-color=\"transparent\""));
    assert!(!content_of(&bytes).contains("fo:background-color=\"transparent\""));
    assert_eq!(fills(&import(&bytes).unwrap()), [(Some(blue), false), (Some(red), false)]);
}

#[test]
fn default_and_standard_paragraph_shading_round_trip() {
    let yellow = (Some(Rgb(0xf1, 0xcc, 0x45)), false);
    let blue = (Some(Rgb(0x24, 0x8a, 0xb6)), false);
    for (standard, expected) in [("", yellow), ("transparent", (None, true)), ("#248ab6", blue)] {
        let background = if standard.is_empty() { String::new() } else { format!(" fo:background-color=\"{standard}\"") };
        let styles = format!(
            r##"<office:document-styles {NS_T}><office:styles>
<style:default-style style:family="paragraph"><style:paragraph-properties fo:background-color="#f1cc45"/></style:default-style>
<style:style style:name="Standard" style:family="paragraph"><style:paragraph-properties{background}/></style:style>
<style:style style:name="NoParent" style:family="paragraph"/>
<style:style style:name="Based" style:family="paragraph" style:parent-style-name="Standard"/>
<style:style style:name="Clear" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:background-color="transparent"/></style:style>
<style:style style:name="Color" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:background-color="#248ab6"/></style:style>
<style:style style:name="Title" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:background-color="transparent"/></style:style>
<style:style style:name="Quotations" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:background-color="#248ab6"/></style:style>
</office:styles></office:document-styles>"##
        );
        let content = format!(
            r##"<office:document-content {NS_T}><office:automatic-styles>
<style:style style:name="AutoNoParent" style:family="paragraph"/>
<style:style style:name="DirectClear" style:family="paragraph" style:parent-style-name="Based"><style:paragraph-properties fo:background-color="transparent"/></style:style>
<style:style style:name="DirectColor" style:family="paragraph" style:parent-style-name="Clear"><style:paragraph-properties fo:background-color="#248ab6"/></style:style>
</office:automatic-styles><office:body><office:text>
<text:p>plain</text:p><text:p text:style-name="Standard">standard</text:p>
<text:p text:style-name="NoParent">parentless common</text:p><text:p text:style-name="AutoNoParent">parentless automatic</text:p>
<text:p text:style-name="Based">inherited</text:p><text:p text:style-name="Clear">custom clear</text:p><text:p text:style-name="Color">custom color</text:p>
<text:p text:style-name="DirectClear">direct clear</text:p><text:p text:style-name="DirectColor">direct color</text:p>
<text:p text:style-name="Title">kind clear</text:p><text:p text:style-name="Quotations">kind color</text:p>
</office:text></office:body></office:document-content>"##
        );
        let doc = import(&package(&content, Some(&styles))).unwrap();
        let want = [expected, expected, yellow, yellow, expected, (None, true), blue, (None, true), blue, (None, true), blue];
        assert_eq!(fills(&doc), want, "Standard background {standard:?}");
        let inherited = &doc.body[4].as_para().unwrap().props;
        assert!(inherited.shading.is_none() && !inherited.shading_auto, "absence must remain inheritance");
        assert_eq!((doc.styles.default_para.shading, doc.styles.default_para.shading_auto), expected);
        let bytes = export_model(&doc).unwrap();
        let written = styles_of(&bytes);
        let default = written.split("<style:default-style style:family=\"paragraph\">").nth(1).unwrap().split("</style:default-style>").next().unwrap();
        let value = if standard.is_empty() { "#f1cc45" } else { standard };
        assert!(default.contains(&format!("fo:background-color=\"{value}\"")), "document background must be written: {default}");
        let back = import(&bytes).unwrap();
        assert_eq!(fills(&back), want, "saved Standard background {standard:?}");
        let texts = |d: &Document| d.body.iter().map(|b| b.as_para().unwrap().text.clone()).collect::<Vec<_>>();
        assert_eq!(texts(&back), texts(&doc));
    }
}
