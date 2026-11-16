//! Word 2010 shapes (`wps:wsp`) are written inside `mc:AlternateContent` with a VML fallback
//! (ECMA-376 Part 3, Markup Compatibility; Part 4 §14.1, VML), so a reader that doesn't know
//! `wps` — Word 2007, or Word 2000-2003 with the Office Compatibility Pack — takes the fallback
//! instead of refusing the whole file. A shape with no VML form is left bare.

use std::io::Read;

use craft_wordmodel::para::{Anchor, FloatAlign, ShapeKind, Wrap};
use craft_wordmodel::props::TextVert;
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
    let mut o = shape(ShapeKind::TextBox, Some(id));
    if let InlineObject::Shape { float, .. } = &mut o {
        (float.dist, float.dist_top, float.dist_bottom) = (18.0, 12.0, 6.0);
    }
    d.insert_object(&craft_wordmodel::Pos::body(0, 1), o, &Default::default()).unwrap();
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
        "mso-wrap-distance-left:18.00pt;mso-wrap-distance-right:18.00pt;mso-wrap-distance-top:12.00pt;mso-wrap-distance-bottom:6.00pt",
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

/// The fallback keeps the box's own stacking order (not that of a floating object inside it),
/// its hidden flag and its text direction.
#[test]
fn the_fallback_keeps_order_visibility_and_direction() {
    let mut d = Document::from_text("a");
    let inner = d.add_part(PartKind::TextBox, vec![para_block(craft_wordmodel::Paragraph::with_text("inner", Default::default()))]);
    let mut nested = shape(ShapeKind::TextBox, Some(inner));
    if let InlineObject::Shape { float, .. } = &mut nested {
        float.z = 100;
    }
    let mut p = craft_wordmodel::Paragraph::with_text("outer", Default::default());
    p.insert_object(0, nested, &Default::default()).unwrap();
    let outer = d.add_part(PartKind::TextBox, vec![para_block(p)]);
    d.parts.get_mut(&outer).unwrap().body = TextBody { vert: TextVert::Vert270, ..Default::default() };
    let mut o = shape(ShapeKind::TextBox, Some(outer));
    if let InlineObject::Shape { float, .. } = &mut o {
        float.z = 10;
        float.hidden = true;
    }
    d.insert_object(&craft_wordmodel::Pos::body(0, 1), o, &Default::default()).unwrap();
    let xml = document_xml(&craft_docx::write(&d).unwrap());
    // The outer box's VML: the `v:rect` holding the vertical text box (the inner box's fallback
    // is copied into the outer one's text, so it comes after it).
    let tb = xml.find(r#"<v:textbox style="layout-flow:vertical;mso-layout-flow-alt:bottom-to-top">"#).expect("the outer box's text, turned");
    let rect = &xml[xml[..tb].rfind("<v:rect ").expect("its v:rect")..tb];
    assert!(rect.contains("z-index:10;"), "{rect}");
    assert!(rect.contains("visibility:hidden"), "{rect}");
}

/// A shape VML has no simple form for stays a bare `wps:wsp`: guarded without a fallback, an old
/// reader would open the file without it and drop it on its next save.
#[test]
fn a_shape_without_a_vml_form_is_not_guarded() {
    let mut d = Document::from_text("a");
    d.insert_object(&craft_wordmodel::Pos::body(0, 1), shape(ShapeKind::Star, None), &Default::default()).unwrap();
    let bytes = craft_docx::write(&d).unwrap();
    let xml = document_xml(&bytes);
    assert!(xml.contains("<w:r><w:drawing>") && xml.contains(r#"<a:prstGeom prst="star5">"#), "{xml}");
    assert!(!xml.contains("<mc:AlternateContent>"), "{xml}");
    let r = craft_docx::read(&bytes).unwrap();
    let mut kinds = Vec::new();
    r.objects_in_reading_order(&mut |o| {
        if let InlineObject::Shape { kind, .. } = o {
            kinds.push(*kind);
        }
    });
    assert_eq!(kinds, vec![ShapeKind::Star]);
}
