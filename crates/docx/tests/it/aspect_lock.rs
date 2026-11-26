//! An inline picture's graphic-frame aspect lock (ECMA-376 Part 1 §20.4.2.4
//! `wp:cNvGraphicFramePr`, §20.1.2.2.19 `a:graphicFrameLocks`).
//!
//! The package is original markup written in this test. The PNG is generated here.
//! No file from another program is embedded.

use craft_wordmodel::InlineObject;
use std::io::{Read, Write};

fn png() -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([20, 90, 200, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// An inline picture. `aspect` is the raw `noChangeAspect` value, spaces included; `None` omits
/// the lock. Every picture also carries `a:picLocks noChangeAspect="1"`, a different element.
/// `cropped` sets `a:srcRect`.
fn picture(id: u32, descr: &str, aspect: Option<&str>, cropped: bool) -> String {
    let frame = match aspect {
        Some(v) => format!(r#"<wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="{v}"/></wp:cNvGraphicFramePr>"#),
        None => "<wp:cNvGraphicFramePr/>".to_string(),
    };
    let src = if cropped { r#"<a:srcRect l="25000"/>"# } else { "" };
    format!(
        r#"<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="914400" cy="457200"/><wp:docPr id="{id}" name="Picture {id}" descr="{descr}"/>{frame}<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="square.png"/><pic:cNvPicPr><a:picLocks noChangeAspect="1"/></pic:cNvPicPr></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdPic"/>{src}<a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
    )
}

fn package(body: &str) -> Vec<u8> {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body>{body}<w:sectPr/></w:body></w:document>"#
    );
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdPic" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/square.png"/></Relationships>"#;
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let png = png();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("word/document.xml", xml.as_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
        ("word/media/square.png", png.as_slice()),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn document_xml(bytes: &[u8]) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    xml
}

/// Whether the drawing described by `descr` carries the graphic-frame aspect lock.
/// Looks only inside `wp:cNvGraphicFramePr`, so `a:picLocks` does not count.
fn frame_locks_aspect(xml: &str, descr: &str) -> bool {
    let needle = format!("descr=\"{descr}\"");
    let drawing = xml.split("<w:drawing").find(|d| d.contains(&needle)).unwrap_or_else(|| panic!("no drawing {descr}"));
    let start = drawing.find("<wp:cNvGraphicFramePr").unwrap_or_else(|| panic!("no frame props for {descr}"));
    let rest = &drawing[start..];
    let end = rest
        .find("</wp:cNvGraphicFramePr>")
        .map(|i| i + "</wp:cNvGraphicFramePr>".len())
        .unwrap_or_else(|| rest.find("/>").map(|i| i + 2).unwrap_or_else(|| panic!("unclosed frame props for {descr}")));
    let frame = &rest[..end];
    frame.contains("<a:graphicFrameLocks") && frame.contains("noChangeAspect=\"1\"")
}

/// `(aspect lock, cropped)` for the picture with this description.
fn picture_state(doc: &craft_wordmodel::Document, descr: &str) -> (bool, bool) {
    let pic = doc.body[0].as_para().unwrap().objects.iter().find(|o| o.alt_text() == descr);
    let Some(InlineObject::Image { lock_aspect, crop, .. }) = pic else { panic!("no picture {descr}") };
    (*lock_aspect, crop.iter().any(|c| *c > 0.0))
}

#[test]
fn an_inline_picture_keeps_its_graphic_frame_aspect_lock() {
    // Plain and cropped pictures that lock the frame, and one that does not (it still has
    // `a:picLocks`). A save must keep the lock only where the file had it.
    let body =
        format!("<w:p>{}{}{}</w:p>", picture(1, "plain", Some("1"), false), picture(2, "cropped", Some("1"), true), picture(3, "free", None, false),);
    let doc = craft_docx::read(&package(&body)).unwrap();
    assert_eq!(picture_state(&doc, "plain"), (true, false));
    assert_eq!(picture_state(&doc, "cropped"), (true, true));
    assert_eq!(picture_state(&doc, "free"), (false, false));

    let saved = craft_docx::write(&doc).unwrap();
    let xml = document_xml(&saved);
    assert!(frame_locks_aspect(&xml, "plain"));
    assert!(frame_locks_aspect(&xml, "cropped"));
    assert!(!frame_locks_aspect(&xml, "free"), "{xml}");

    let back = craft_docx::read(&saved).unwrap();
    assert_eq!(picture_state(&back, "plain"), (true, false));
    assert_eq!(picture_state(&back, "cropped"), (true, true));
    assert_eq!(picture_state(&back, "free"), (false, false));
}

/// Space around the token is not part of it: ` false ` stays unlocked and is not saved as
/// `noChangeAspect="1"`, and ` 1 ` stays locked.
#[test]
fn surrounding_space_on_an_aspect_lock_does_not_change_its_value() {
    let body = format!("<w:p>{}{}</w:p>", picture(1, "off", Some(" false "), false), picture(2, "on", Some(" 1 "), false),);
    let doc = craft_docx::read(&package(&body)).unwrap();
    assert_eq!(picture_state(&doc, "off"), (false, false));
    assert_eq!(picture_state(&doc, "on"), (true, false));

    let xml = document_xml(&craft_docx::write(&doc).unwrap());
    assert!(!frame_locks_aspect(&xml, "off"), "{xml}");
    assert!(frame_locks_aspect(&xml, "on"));
}
