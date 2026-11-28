//! A saved comment reference run names a character style that `styles.xml` defines (#3309).
//! All markup here is synthetic, written from ECMA-376 §17.13.4 and §17.7.

use std::io::Read;

use crate::fixtures::zip;

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const RT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";

/// A document with one comment, and a `styles.xml` holding `styles` when given.
fn commented(styles: Option<&str>) -> Vec<u8> {
    let doc = format!(
        r#"<w:document {W_NS}><w:body><w:p><w:commentRangeStart w:id="1"/><w:r><w:t>Text</w:t></w:r><w:commentRangeEnd w:id="1"/><w:r><w:commentReference w:id="1"/></w:r></w:p></w:body></w:document>"#
    );
    let comments = format!(
        r#"<w:comments {W_NS}><w:comment w:id="1" w:author="Ann" w:initials="A"><w:p><w:r><w:t>Note</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    let styles = styles.map(|s| format!(r#"<w:styles {W_NS}>{s}</w:styles>"#));
    let mut rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RT}comments" Target="comments.xml"/>"#
    );
    if styles.is_some() {
        rels.push_str(&format!(r#"<Relationship Id="rId2" Type="{RT}styles" Target="styles.xml"/>"#));
    }
    rels.push_str("</Relationships>");
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/comments.xml", comments.as_bytes()),
    ];
    if let Some(s) = &styles {
        entries.push(("word/styles.xml", s.as_bytes()));
    }
    zip(&entries)
}

/// Opens and saves `bytes`, then returns the saved (document.xml, styles.xml).
fn resave(bytes: &[u8]) -> (String, String) {
    let saved = craft_docx::write(&craft_docx::read(bytes).unwrap()).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(saved)).unwrap();
    let mut part = |name: &str| {
        let mut s = String::new();
        archive.by_name(name).unwrap().read_to_string(&mut s).unwrap();
        s
    };
    (part("word/document.xml"), part("word/styles.xml"))
}

/// The `w:rStyle` of the run holding `w:commentReference`.
fn reference_style(document: &str) -> String {
    let at = document.find("<w:commentReference").expect("a comment reference is saved");
    let run = &document[document[..at].rfind("<w:r>").expect("the reference sits in a run")..at];
    let val = run.split(r#"<w:rStyle w:val=""#).nth(1).expect("the reference run names a style");
    val[..val.find('"').unwrap()].to_string()
}

/// How many styles in `styles` have the id `id`.
fn defines(styles: &str, id: &str) -> usize {
    styles.matches(&format!(r#"w:styleId="{id}""#)).count()
}

#[test]
fn comment_reference_style_is_defined_when_the_document_has_none() {
    let normal = r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>"#;
    for styles in [None, Some(normal)] {
        let (document, saved_styles) = resave(&commented(styles));
        let id = reference_style(&document);
        assert_eq!(defines(&saved_styles, &id), 1, "the reference run's style {id:?} is defined once (styles.xml given: {})", styles.is_some());
    }
}

#[test]
fn a_documents_own_comment_reference_style_is_kept_alone() {
    // A localized document gives Word's built-in "annotation reference" style another id.
    let own =
        r#"<w:style w:type="character" w:styleId="Odwolanie"><w:name w:val="annotation reference"/><w:rPr><w:sz w:val="16"/></w:rPr></w:style>"#;
    let (document, saved_styles) = resave(&commented(Some(own)));
    assert_eq!(reference_style(&document), "Odwolanie");
    assert_eq!(defines(&saved_styles, "Odwolanie"), 1);
    assert_eq!(defines(&saved_styles, "CommentReference"), 0, "no second comment reference style is added");
}
