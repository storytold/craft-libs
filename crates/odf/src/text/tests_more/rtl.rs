//! Right-to-left paragraphs (ODF 1.3 §20.404 `style:writing-mode` on paragraph properties: `rl-tb`
//! and `rl`), read as WordCraft's bidirectional paragraphs and written back.

use crate::text::tests::{NS_T, package};
use crate::text::{export, import};
use craft_wordmodel::Document;

fn open(mode: &str) -> Document {
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:paragraph-properties style:writing-mode="{mode}"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">שלום עולם</text:p><text:p>Left to right</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

fn bidi(d: &Document) -> Vec<Option<bool>> {
    d.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.bidi).collect()
}

fn resolved_directions(d: &Document) -> Vec<bool> {
    d.body.iter().filter_map(|b| b.as_para()).map(|p| d.styles.resolve_para(&p.props).bidi).collect()
}

#[test]
fn right_to_left_paragraphs_are_read() {
    assert_eq!(bidi(&open("rl-tb")), [Some(true), None]);
    assert_eq!(bidi(&open("rl")), [Some(true), None]);
    assert_eq!(bidi(&open("lr-tb")), [None, None]);
}

#[test]
fn right_to_left_paragraphs_round_trip() {
    let d = open("rl-tb");
    assert_eq!(bidi(&import(&export(&d).unwrap()).unwrap()), [Some(true), None]);
}

/// A named left-to-right style based on a right-to-left one stays left to right, and so does a
/// paragraph whose own style says left to right over a right-to-left named style: through import
/// and through export and reimport.
#[test]
fn left_to_right_over_a_right_to_left_style_stays_left_to_right() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Rtl" style:family="paragraph"><style:paragraph-properties style:writing-mode="rl-tb"/></style:style><style:style style:name="Ltr" style:family="paragraph" style:parent-style-name="Rtl"><style:paragraph-properties style:writing-mode="lr-tb"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:parent-style-name="Rtl"><style:paragraph-properties style:writing-mode="lr-tb"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="Rtl">שלום</text:p><text:p text:style-name="Ltr">child</text:p><text:p text:style-name="P1">own</text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    assert_eq!(resolved_directions(&d), [true, false, false], "import");
    // The child style and the paragraph each keep their own left to right.
    assert_eq!(d.styles.get("Ltr").and_then(|s| s.para.bidi), Some(false));
    assert_eq!(bidi(&d), [None, None, Some(false)]);
    d.kept.clear();
    assert_eq!(resolved_directions(&import(&export(&d).unwrap()).unwrap()), [true, false, false], "export and reimport");
}

#[test]
fn right_to_left_heading_style_survives_import_and_reexport() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles>
        <style:style style:name="Heading_20_1" style:family="paragraph"><style:paragraph-properties style:writing-mode="rl-tb"/></style:style>
        <style:style style:name="Heading_20_2" style:family="paragraph" style:parent-style-name="Heading_20_1"/>
        <style:style style:name="Custom" style:family="paragraph" style:parent-style-name="Heading_20_1"/>
        </office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text>
        <text:h text:outline-level="1" text:style-name="Heading_20_1">First heading</text:h>
        <text:h text:outline-level="2" text:style-name="Heading_20_2">Second heading</text:h>
        <text:p text:style-name="Custom">Named child</text:p>
        <text:p>Body paragraph</text:p>
        </office:text></office:body></office:document-content>"#
    );
    let mut doc = import(&package(&content, Some(&styles))).unwrap();
    assert_eq!(resolved_directions(&doc), [true, true, true, false]);
    doc.kept.clear();
    assert_eq!(resolved_directions(&import(&export(&doc).unwrap()).unwrap()), [true, true, true, false]);
}

#[test]
fn right_to_left_default_style_preserves_inheritance_and_left_to_right_overrides() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles>
        <style:default-style style:family="paragraph"><style:paragraph-properties style:writing-mode="rl-tb"/></style:default-style>
        <style:style style:name="NamedLtr" style:family="paragraph"><style:paragraph-properties style:writing-mode="lr-tb"/></style:style>
        </office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles>
        <style:style style:name="P1" style:family="paragraph"><style:paragraph-properties style:writing-mode="lr-tb"/></style:style>
        </office:automatic-styles><office:body><office:text>
        <text:p>inherited</text:p><text:p text:style-name="NamedLtr">named override</text:p><text:p text:style-name="P1">direct override</text:p>
        </office:text></office:body></office:document-content>"#
    );
    let mut doc = import(&package(&content, Some(&styles))).unwrap();
    assert_eq!(doc.styles.default_para.bidi, Some(true));
    assert_eq!(bidi(&doc), [None, None, Some(false)]);
    assert_eq!(resolved_directions(&doc), [true, false, false]);
    // Verify generated styles rather than the imported XML, which already has the direction.
    doc.kept.clear();
    assert_eq!(resolved_directions(&import(&export(&doc).unwrap()).unwrap()), [true, false, false]);
}
