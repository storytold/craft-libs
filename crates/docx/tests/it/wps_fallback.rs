//! Word 2010 shapes (`wps:wsp`) are written inside `mc:AlternateContent` with a VML fallback
//! (ECMA-376 Part 3, Markup Compatibility; Part 4 §14.1, VML), so a reader that doesn't know
//! `wps` — Word 2007, or Word 2000-2003 with the Office Compatibility Pack — takes the fallback
//! or skips the shape instead of refusing the whole file.

use std::io::Read;

use craft_wordmodel::para::{Anchor, FloatAlign, ShapeKind, Wrap};
use craft_wordmodel::{Document, InlineObject, PartKind, TextBody, para_block};

fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

fn shape(kind: ShapeKind, story: Option<u32>) -> InlineObject {
    let float = craft_wordmodel::para::Float {
        wrap: Wrap::Square,
        h_rel: Anchor::Margin,
        h_align: Some(FloatAlign::End),
        v_rel: Anchor::Paragraph,
        y: 0.05,
        ..Default::default()
    };
    InlineObject::Shape {
        kind,
        w: 10.7,
        h: 12.2,
        fill: None,
        stroke: None,
        stroke_width: 0.75,
        float,
        story,
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    }
}

/// A page number in a floating text box, as in a footer: the shape that made the
/// Compatibility Pack refuse the file.
#[test]
fn a_text_box_gets_a_vml_fallback_with_its_text() {
    let mut d = Document::from_text("a");
    let id = d.add_part(PartKind::TextBox, vec![para_block(craft_wordmodel::Paragraph::with_text("29", Default::default()))]);
    d.parts.get_mut(&id).unwrap().body = TextBody { auto_fit: true, inset: Some([0; 4]), ..Default::default() };
    d.insert_object(&craft_wordmodel::Pos::body(0, 1), shape(ShapeKind::TextBox, Some(id)), &Default::default()).unwrap();
    let bytes = craft_docx::write(&d).unwrap();
    let xml = document_xml(&bytes);
    assert!(xml.contains(r#"<mc:AlternateContent><mc:Choice Requires="wps"><w:drawing>"#), "{xml}");
    assert!(!xml.contains("<w:r><w:drawing>"), "no bare Word 2010 shape: {xml}");
    let fallback = &xml[xml.find("<mc:Fallback>").expect("a fallback")..];
    assert!(fallback.starts_with("<mc:Fallback><w:pict><v:rect "), "{fallback}");
    for want in [
        "position:absolute",
        "mso-position-horizontal:right",
        "mso-position-horizontal-relative:margin",
        "mso-position-vertical-relative:text",
        r#"filled="f""#,
        r#"stroked="f""#,
        r#"<v:textbox inset="0.00pt,0.00pt,0.00pt,0.00pt" style="mso-fit-shape-to-text:t"><w:txbxContent>"#,
        r#"<w10:wrap type="square"/></v:rect></w:pict></mc:Fallback></mc:AlternateContent>"#,
    ] {
        assert!(fallback.contains(want), "{want} in {fallback}");
    }
    assert_eq!(xml.matches(">29</w:t>").count(), 2, "the text in the choice and again in the fallback");

    // Read back: the choice, once.
    let r = craft_docx::read(&bytes).unwrap();
    let mut boxes = Vec::new();
    r.objects_in_reading_order(&mut |o| boxes.extend(o.text_boxes()));
    boxes.retain(|id| r.parts.get(id).is_some_and(|p| p.kind == PartKind::TextBox));
    assert_eq!(boxes.len(), 1, "one text box, not the fallback as well");
    assert_eq!(r.plain_text(craft_wordmodel::StoryRef::Part(boxes[0])), "29");
}

/// A shape VML has no simple form for is still guarded, with no fallback: an old reader skips it.
#[test]
fn a_shape_without_a_vml_form_is_guarded_without_a_fallback() {
    let mut d = Document::from_text("a");
    d.insert_object(&craft_wordmodel::Pos::body(0, 1), shape(ShapeKind::Star, None), &Default::default()).unwrap();
    let xml = document_xml(&craft_docx::write(&d).unwrap());
    assert!(xml.contains(r#"<mc:AlternateContent><mc:Choice Requires="wps"><w:drawing>"#), "{xml}");
    assert!(xml.contains("</w:drawing></mc:Choice></mc:AlternateContent>"), "{xml}");
    assert!(!xml.contains("<mc:Fallback>"), "{xml}");
    let r = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
    let mut kinds = Vec::new();
    r.objects_in_reading_order(&mut |o| {
        if let InlineObject::Shape { kind, .. } = o {
            kinds.push(*kind);
        }
    });
    assert_eq!(kinds, vec![ShapeKind::Star]);
}
