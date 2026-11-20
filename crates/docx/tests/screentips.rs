//! Original OOXML fixtures. ScreenTips are hyperlink data, not run formatting.
use std::io::{Read, Write};

use craft_wordmodel::{CharProps, Pos};

fn package() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, xml) in [
        (
            "word/document.xml",
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:t>Before. </w:t></w:r><w:hyperlink r:id="external" w:tooltip="Original café &amp; &quot;Ω&quot;"><w:r><w:t>First</w:t></w:r></w:hyperlink><w:hyperlink r:id="external" w:tooltip="Different hint"><w:r><w:t>Second</w:t></w:r></w:hyperlink><w:hyperlink w:anchor="target" w:tooltip="Internal hint"><w:r><w:t>Internal</w:t></w:r></w:hyperlink></w:p><w:p><w:bookmarkStart w:id="0" w:name="target"/><w:r><w:t>Target.</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p><w:sectPr/></w:body></w:document>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        (
            "word/_rels/document.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="external" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.invalid/reference" TargetMode="External"/></Relationships>"#,
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn hyperlink_screentips_survive_edited_saves_without_merging_different_hints() {
    let mut doc = craft_docx::read(&package()).unwrap();
    for _ in 0..2 {
        doc.insert_text(&Pos::body(0, 0), "Edit. ", &CharProps::default()).unwrap();
        let bytes = craft_docx::write(&doc).unwrap();
        let mut xml = String::new();
        zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap().by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
        assert_eq!(xml.matches("w:tooltip=").count(), 3, "{xml}");
        for tip in ["Original café &amp; &quot;Ω&quot;", "Different hint", "Internal hint"] {
            assert!(xml.contains(&format!("w:tooltip=\"{tip}\"")), "{xml}");
        }
        assert!(xml.contains("w:anchor=\"target\""));
        doc = craft_docx::read(&bytes).unwrap();
        let paragraph = doc.body[0].as_para().unwrap();
        for (text, tip) in [("First", "Original café & \"Ω\""), ("Second", "Different hint"), ("Internal", "Internal hint")] {
            let offset = paragraph.text.find(text).unwrap();
            assert_eq!(paragraph.props_of_char(offset).link_tooltip.as_deref(), Some(tip), "{text}");
        }
        assert!(doc.body[0].as_para().unwrap().plain_text().ends_with("FirstSecondInternal"));
    }
}

#[test]
fn oversized_screentips_share_one_utf8_bounded_value_and_save_at_the_same_size() {
    use std::sync::Arc;

    let kept = "a".repeat(4095);
    let tip = format!("{kept}Ω trailing text");
    let runs: String = (0..32).map(|i| format!("<w:r><w:rPr><w:b w:val=\"{}\"/></w:rPr><w:t>word </w:t></w:r>", i % 2)).collect();
    let xml = format!(
        r##"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:hyperlink w:anchor="target" w:tooltip="{tip}">{runs}</w:hyperlink></w:p></w:body></w:document>"##
    );
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(xml.as_bytes()).unwrap();
    let mut doc = craft_docx::read(&zip.finish().unwrap().into_inner()).unwrap();
    let p = doc.body[0].as_para().unwrap();
    assert_eq!(p.runs.len(), 32);
    let first = p.runs[0].props.link_tooltip.as_ref().unwrap();
    assert_eq!(first.as_ref(), kept);
    assert!(p.runs.iter().all(|run| Arc::ptr_eq(first, run.props.link_tooltip.as_ref().unwrap())));

    // Native model data can arrive without going through the XML reader.
    let props = CharProps { link: Some("#target".into()), link_tooltip: Some(Arc::from(tip)), ..CharProps::default() };
    doc.insert_text(&Pos::body(0, 0), "Native ", &props).unwrap();
    let saved = craft_docx::write(&doc).unwrap();
    let mut xml = String::new();
    zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap().by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    assert_eq!(xml.matches("w:tooltip=").count(), 1);
    assert!(xml.contains(&format!("w:tooltip=\"{kept}\"")));
    assert!(!xml.contains("trailing text"));
}
