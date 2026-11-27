//! Original synthetic cases for ECMA-376 Part 1 §§17.3.3.1 (br), 17.3.3.30 (sym)
//! and 17.16.18 (fldChar). No external documents or font files are used.
use craft_wordmodel::{InlineObject, Path, StoryRef};

fn read_result(result: &str) -> craft_wordmodel::Document {
    let xml = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:fldSimple w:instr="QUOTE x"><w:r>{result}</w:r></w:fldSimple></w:p><w:p><w:r><w:t>Outside.</w:t></w:r></w:p></w:body></w:document>"#
    );
    craft_docx::read(&super::fixtures::zip(&[("word/document.xml", xml.as_bytes())])).unwrap()
}

#[test]
fn cached_field_breaks_survive_edited_saves() {
    let mut doc = read_result(
        r#"<w:t>A</w:t><w:br w:type="page"/><w:t>B</w:t><w:br w:type="column"/><w:t>C</w:t><w:tab/><w:br/><w:noBreakHyphen/><w:softHyphen/>"#,
    );
    let expected = doc.body[0].as_para().unwrap().plain_text();
    assert!(expected.contains('\u{c}') && expected.contains('\u{e}'));
    for _ in 0..2 {
        doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().insert_text(0, "Edit. ", &Default::default()).unwrap();
        doc = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
        assert_eq!(doc.body[0].as_para().unwrap().plain_text(), expected);
    }
}

#[test]
fn cached_field_symbols_keep_their_own_font_through_edited_saves() {
    let mut doc = read_result(r#"<w:sym w:font="Synthetic Symbols" w:char="F041"/>"#);
    let p = doc.body[0].as_para().unwrap();
    let text = p.plain_text();
    let font = p.props_of_char(0).sym_font.clone();
    assert!(font.is_some());
    for _ in 0..2 {
        doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().insert_text(0, "Edit. ", &Default::default()).unwrap();
        doc = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
        let p = doc.body[0].as_para().unwrap();
        assert_eq!(p.plain_text(), text);
        assert!(matches!(p.objects.as_slice(), [InlineObject::Field { .. }]));
        assert_eq!(p.props_of_char(0).sym_font, font);
    }
}
