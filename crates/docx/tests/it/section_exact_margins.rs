//! Exact top and bottom margins: `w:pgMar` `w:top` and `w:bottom` are ST_SignedTwipsMeasure, and a
//! negative value places the text at that distance from the page edge even when the header
//! (footer) is taller, so the two overlap (ECMA-376 Part 1 §17.6.11). Word writes them when the
//! margin is typed as a negative number. Fixtures are hand-written.

use std::io::{Read, Write};

fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

fn margins(top: &str, bottom: &str) -> (f32, f32, f32, f32) {
    let body = format!(
        r#"<w:p><w:r><w:t>A</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="{top}" w:right="1134" w:bottom="{bottom}" w:left="-1134" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>"#
    );
    let d = craft_docx::read(&docx(&body)).unwrap();
    let s = &d.last_section;
    (s.margin_top, s.margin_bottom, s.margin_left, s.text_height())
}

/// A negative top or bottom keeps its sign (the exact margin); left, right and the distances stay
/// unsigned as before.
#[test]
fn negative_top_and_bottom_margins_keep_their_sign() {
    let (top, bottom, left, height) = margins("-798", "-1200");
    assert!((top + 39.9).abs() < 0.01, "top {top}");
    assert!((bottom + 60.0).abs() < 0.01, "bottom {bottom}");
    assert!((left - 56.7).abs() < 0.01, "left stays unsigned: {left}");
    assert!((height - (841.9 - 39.9 - 60.0)).abs() < 0.01, "the text height counts the margins' size: {height}");
    let (top, bottom, ..) = margins("1417", "1134");
    assert!((top - 70.85).abs() < 0.01 && (bottom - 56.7).abs() < 0.01, "{top} {bottom}");
}

/// Saving writes the negative values back, so Word reads the same exact margins.
#[test]
fn exact_margins_round_trip() {
    let body = r#"<w:p><w:r><w:t>A</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="-798" w:right="1134" w:bottom="-1200" w:left="1134" w:header="708" w:footer="1389" w:gutter="0"/></w:sectPr>"#;
    let d = craft_docx::read(&docx(body)).unwrap();
    let xml = document_xml(&craft_docx::write(&d).unwrap());
    assert!(xml.contains(r#"w:top="-798""#) && xml.contains(r#"w:bottom="-1200""#), "{xml}");
}
