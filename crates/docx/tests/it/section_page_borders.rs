//! A section's page borders and which of its pages they are drawn on (`w:pgBorders/@w:display`,
//! ECMA-376 Part 1 §17.6.10, values from ST_PageBorderDisplay). Fixtures are hand-written from
//! the spec.

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

/// A `w:sectPr` whose page borders show on the pages `display` names (empty: no attribute).
fn section(display: &str) -> String {
    let attr = if display.is_empty() { String::new() } else { format!(r#" w:display="{display}""#) };
    format!(
        r#"<w:sectPr><w:pgBorders{attr}><w:top w:val="single" w:sz="8" w:space="24" w:color="C00000"/><w:left w:val="single" w:sz="8" w:space="24" w:color="C00000"/><w:bottom w:val="single" w:sz="8" w:space="24" w:color="C00000"/><w:right w:val="single" w:sz="8" w:space="24" w:color="C00000"/></w:pgBorders></w:sectPr>"#
    )
}

/// storytold/wordcraft#3076: `w:display` is read and written back, so a cover-page-only border
/// stays one. The borders used to be read without it and saved as `allPages` on every section.
#[test]
fn page_border_display_round_trips() {
    for display in ["firstPage", "notFirstPage", "allPages"] {
        let body = format!(r#"<w:p><w:r><w:t>Cover</w:t></w:r></w:p>{}"#, section(display));
        let d = craft_docx::read(&docx(&body)).unwrap();
        assert_eq!(d.last_section.page_borders_display.as_deref(), Some(display));
        assert!(d.last_section.page_borders.is_some(), "{display}");
        let out = craft_docx::write(&d).unwrap();
        let xml = document_xml(&out);
        assert_eq!(xml.matches(&format!(r#"w:display="{display}""#)).count(), 1, "{xml}");
        assert_eq!(craft_docx::read(&out).unwrap().last_section.page_borders_display.as_deref(), Some(display));
    }
}

/// Borders with no `w:display`, and ones whose value is not in ST_PageBorderDisplay, go on every
/// page: nothing is kept and `w:pgBorders` is written as it always was, `w:offsetFrom` included.
#[test]
fn a_missing_or_malformed_display_is_every_page() {
    for display in ["", "oddPages", "FirstPage", "1"] {
        let body = format!(r#"<w:p><w:r><w:t>Body</w:t></w:r></w:p>{}"#, section(display));
        let d = craft_docx::read(&docx(&body)).unwrap();
        assert_eq!(d.last_section.page_borders_display, None, "{display:?}");
        let xml = document_xml(&craft_docx::write(&d).unwrap());
        assert!(xml.contains(r#"<w:pgBorders w:offsetFrom="text">"#), "{display:?}: {xml}");
        assert!(!xml.contains("w:display"), "{display:?}: {xml}");
    }
}

/// Each section keeps its own setting: a cover section bordered on its first page only, then a
/// body section bordered on every page but its first.
#[test]
fn each_section_keeps_its_own_display() {
    let body = format!(
        r#"<w:p><w:pPr>{}</w:pPr><w:r><w:t>Cover</w:t></w:r></w:p><w:p><w:r><w:t>Body</w:t></w:r></w:p>{}"#,
        section("firstPage"),
        section("notFirstPage")
    );
    let d = craft_docx::read(&docx(&body)).unwrap();
    let got: Vec<(Option<String>, bool)> = d.sections().iter().map(|(_, s)| (s.page_borders_display.clone(), s.page_borders.is_some())).collect();
    assert_eq!(got, vec![(Some("firstPage".to_string()), true), (Some("notFirstPage".to_string()), true)]);
    let xml = document_xml(&craft_docx::write(&d).unwrap());
    assert_eq!(xml.matches(r#"w:display="firstPage""#).count(), 1, "{xml}");
    assert_eq!(xml.matches(r#"w:display="notFirstPage""#).count(), 1, "{xml}");
}
