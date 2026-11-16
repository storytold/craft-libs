//! Justified paragraphs keep their kashida length (`ST_Jc` `lowKashida`, `mediumKashida`,
//! `highKashida`, and ordinary `both` / `justify`) through read and write.

use std::io::{Read, Write};

use std::sync::Arc;

use craft_wordmodel::props::{Align, Kashida, ParaProps};
use craft_wordmodel::styles::{Style, StyleKind, StyleSheet};
use craft_wordmodel::{Document, Paragraph, para_block};

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    zip(&[("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())])
}

fn paras(d: &Document) -> Vec<&Paragraph> {
    d.body.iter().filter_map(|b| b.as_para()).collect()
}

fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

fn jc_para(body: &str, val: &str) -> String {
    format!(r#"<w:p><w:pPr><w:jc w:val="{val}"/></w:pPr><w:r><w:t>{body}</w:t></w:r></w:p>"#)
}

#[test]
fn kashida_jc_values_are_read() {
    let body = [
        jc_para("both", "both"),
        jc_para("justify", "justify"),
        jc_para("low", "lowKashida"),
        jc_para("medium", "mediumKashida"),
        jc_para("high", "highKashida"),
        jc_para("left", "left"),
    ]
    .join("");
    let d = craft_docx::read(&docx(&body)).unwrap();
    let ps = paras(&d);
    let got: Vec<_> = ps.iter().map(|p| (p.props.align, p.props.kashida)).collect();
    assert_eq!(
        got,
        vec![
            (Some(Align::Justify), Some(Kashida::None)),
            (Some(Align::Justify), Some(Kashida::None)),
            (Some(Align::Justify), Some(Kashida::Short)),
            (Some(Align::Justify), Some(Kashida::Medium)),
            (Some(Align::Justify), Some(Kashida::Long)),
            (Some(Align::Left), None),
        ]
    );
}

#[test]
fn kashida_length_round_trips() {
    let cases = [
        (Some(Kashida::None), "w:val=\"both\""),
        (Some(Kashida::Short), "w:val=\"lowKashida\""),
        (Some(Kashida::Medium), "w:val=\"mediumKashida\""),
        (Some(Kashida::Long), "w:val=\"highKashida\""),
    ];
    for (kashida, token) in cases {
        let mut p = Paragraph::with_text("مثال", Default::default());
        p.props = ParaProps { align: Some(Align::Justify), kashida, ..Default::default() };
        let mut d = Document::new();
        d.body = vec![para_block(p.clone())];
        let bytes = craft_docx::write(&d).unwrap();
        let xml = document_xml(&bytes);
        assert!(xml.contains(token), "expected {token} in {xml}");
        let r = craft_docx::read(&bytes).unwrap();
        let q = paras(&r)[0];
        assert_eq!(q.props.align, Some(Align::Justify));
        assert_eq!(q.props.kashida, kashida, "round-trip of {token}");
    }
}

fn body_style(kashida: Kashida) -> StyleSheet {
    let mut sheet = StyleSheet::empty();
    sheet.styles.push(Style {
        id: "Body".into(),
        name: "Body".into(),
        kind: StyleKind::Paragraph,
        para: ParaProps { align: Some(Align::Justify), kashida: Some(kashida), ..ParaProps::default() },
        ..Style::default()
    });
    sheet
}

#[test]
fn style_kashida_length_survives_a_write_when_the_paragraph_has_no_direct_jc() {
    let mut p = Paragraph::with_text("مثال", Default::default());
    p.props = ParaProps { style: Some("Body".into()), ..ParaProps::default() };
    let mut d = Document::new();
    d.styles = Arc::new(body_style(Kashida::Short));
    d.body = vec![para_block(p)];
    assert_eq!(d.styles.resolve_para(&d.body[0].as_para().unwrap().props).kashida, Kashida::Short);
    let bytes = craft_docx::write(&d).unwrap();
    let r = craft_docx::read(&bytes).unwrap();
    let q = paras(&r)[0];
    assert_eq!(q.props.kashida, None, "no direct w:jc on the paragraph");
    assert_eq!(r.styles.resolve_para(&q.props).kashida, Kashida::Short);
}

#[test]
fn writing_justify_without_a_local_kashida_keeps_the_style_length() {
    let mut p = Paragraph::with_text("مثال", Default::default());
    p.props = ParaProps { style: Some("Body".into()), align: Some(Align::Justify), kashida: None, ..ParaProps::default() };
    let mut d = Document::new();
    d.styles = Arc::new(body_style(Kashida::Medium));
    d.body = vec![para_block(p)];
    let bytes = craft_docx::write(&d).unwrap();
    let xml = document_xml(&bytes);
    assert!(xml.contains("w:val=\"mediumKashida\""), "expected mediumKashida in {xml}");
    let r = craft_docx::read(&bytes).unwrap();
    let q = paras(&r)[0];
    assert_eq!(q.props.align, Some(Align::Justify));
    assert_eq!(q.props.kashida, Some(Kashida::Medium));
    assert_eq!(r.styles.resolve_para(&q.props).kashida, Kashida::Medium);
}
