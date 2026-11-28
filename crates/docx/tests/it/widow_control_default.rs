//! widow/orphan control a file never sets is off.

use std::io::{Read, Write};

fn docx(styles: Option<&str>) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let doc = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Text</w:t></w:r></w:p></w:body></w:document>"#;
    let mut parts = vec![("_rels/.rels", root.to_string()), ("word/document.xml", doc.to_string())];
    if let Some(s) = styles {
        parts.push(("word/_rels/document.xml.rels", doc_rels.to_string()));
        parts.push((
            "word/styles.xml",
            format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{s}</w:styles>"#),
        ));
    }
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, xml) in &parts {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(xml.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn styles_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/styles.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

fn widow_control(d: &craft_wordmodel::Document) -> bool {
    let p = d.body[0].as_para().unwrap();
    d.styles.resolve_para(&p.props).widow_control
}

const NORMAL: &str = r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>"#;

#[test]
fn a_docx_that_never_sets_widow_control_has_it_off() {
    for styles in [Some(NORMAL.to_string()), Some(format!("<w:docDefaults><w:pPrDefault><w:pPr/></w:pPrDefault></w:docDefaults>{NORMAL}"))] {
        let d = craft_docx::read(&docx(styles.as_deref())).unwrap();
        assert!(!widow_control(&d), "off when nothing sets it: {styles:?}");
        let bytes = craft_docx::write(&d).unwrap();
        assert!(!styles_xml(&bytes).contains("widowControl"), "written back by omission: {styles:?}");
        assert!(!widow_control(&craft_docx::read(&bytes).unwrap()), "still off after a save: {styles:?}");
    }
}

#[test]
fn widow_control_set_in_the_file_stays_on() {
    for styles in [
        format!("<w:docDefaults><w:pPrDefault><w:pPr><w:widowControl/></w:pPr></w:pPrDefault></w:docDefaults>{NORMAL}"),
        r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:pPr><w:widowControl/></w:pPr></w:style>"#
            .to_string(),
    ] {
        let d = craft_docx::read(&docx(Some(&styles))).unwrap();
        assert!(widow_control(&d), "on: {styles}");
        assert!(widow_control(&craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap()), "on after a save: {styles}");
    }
}

#[test]
fn a_new_document_keeps_widow_control_through_a_save() {
    let mut d = craft_wordmodel::Document::new();
    d.body = vec![craft_wordmodel::para_block(craft_wordmodel::Paragraph::with_text("Text", Default::default()))];
    assert!(widow_control(&d), "the app's default is on");
    let reopened = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
    assert!(widow_control(&reopened), "and stays on in its .docx");
}
