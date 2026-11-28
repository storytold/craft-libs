//! Comment bodies carry a single `<w:annotationRef/>` mark in a `CommentReference`-styled run
//! at the start of their first paragraph (ECMA-376 Part 1 §17.13.4.1). Synthetic fixtures written
//! for these tests from the spec.

use std::io::{Cursor, Read};

use craft_wordmodel::StoryRef;

use crate::fixtures::zip;

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;
const W15_NS: &str = r#"xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

fn unzip_part(bytes: &[u8], path: &str) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut entry = archive.by_name(path).unwrap_or_else(|e| panic!("missing {path}: {e}"));
    let mut text = String::new();
    entry.read_to_string(&mut text).unwrap();
    text
}

/// Extract each `<w:comment ...>...</w:comment>` slice in order.
fn split_elements<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let open_space = format!("<{tag} ");
    let open_gt = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    loop {
        let start = match (rest.find(&open_space), rest.find(&open_gt)) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => break,
        };
        let from_open = &rest[start..];
        let end = from_open.find(&close).unwrap_or_else(|| panic!("unclosed <{tag}> in {xml}"));
        out.push(&from_open[..end + close.len()]);
        rest = &from_open[end + close.len()..];
    }
    out
}

/// Strip the outer `<w:p...>` tag and any leading `<w:pPr>...</w:pPr>` block.
fn para_body_after_ppr(p_xml: &str) -> &str {
    let after_open = &p_xml[p_xml.find('>').unwrap() + 1..];
    if let Some(after_ppr) = after_open.strip_prefix("<w:pPr>") {
        let close = after_ppr.find("</w:pPr>").unwrap();
        &after_ppr[close + "</w:pPr>".len()..]
    } else if let Some(after_ppr) = after_open.strip_prefix("<w:pPr/>") {
        after_ppr
    } else {
        after_open
    }
}

/// Assert every `<w:comment>` has the expected paragraph count and exactly one `<w:annotationRef/>`
/// in a run styled `expected_style` at the start of its first `<w:p>` only.
fn assert_comment_marks(comments_xml: &str, expected_para_counts: &[usize], expected_style: &str) {
    let expected_run = format!(r#"<w:r><w:rPr><w:rStyle w:val="{expected_style}"/></w:rPr><w:annotationRef/></w:r>"#);
    let comments = split_elements(comments_xml, "w:comment");
    assert_eq!(comments.len(), expected_para_counts.len(), "comment count mismatch: {comments_xml}");
    for (idx, (comment, &want_paras)) in comments.iter().zip(expected_para_counts).enumerate() {
        assert_eq!(comment.matches("<w:annotationRef/>").count(), 1, "comment {idx} must have exactly one <w:annotationRef/>: {comment}");
        let paras = split_elements(comment, "w:p");
        assert_eq!(paras.len(), want_paras, "comment {idx} paragraph count mismatch: {comment}");
        let first_body = para_body_after_ppr(paras[0]);
        assert!(first_body.starts_with(&expected_run), "comment {idx} first paragraph must start with {expected_run}, got: {first_body}");
        for (p_idx, p) in paras.iter().enumerate().skip(1) {
            assert_eq!(p.matches("annotationRef").count(), 0, "comment {idx} paragraph {p_idx} must not carry annotationRef: {p}");
        }
    }
}

/// Build a synthetic package containing:
/// - comment 0: single-paragraph comment
/// - comment 1: two-paragraph comment
/// - comment 2: reply to comment 1 (linked via `commentsExtended.xml`)
fn fixture_docx(styles_xml: Option<&str>) -> Vec<u8> {
    let doc = format!(
        r#"<w:document {W_NS}><w:body><w:p><w:commentRangeStart w:id="0"/><w:r><w:t>First clause. </w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="0"/></w:r><w:commentRangeStart w:id="1"/><w:commentRangeStart w:id="2"/><w:r><w:t>Second clause.</w:t></w:r><w:commentRangeEnd w:id="1"/><w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="1"/></w:r><w:commentRangeEnd w:id="2"/><w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="2"/></w:r></w:p></w:body></w:document>"#
    );
    let comments = format!(
        r#"<w:comments {W_NS}><w:comment w:id="0" w:author="Ada" w:initials="A"><w:p w14:paraId="11111111"><w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:annotationRef/></w:r><w:r><w:t>Single paragraph note.</w:t></w:r></w:p></w:comment><w:comment w:id="1" w:author="Ben" w:initials="B"><w:p w14:paraId="22222221"><w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:annotationRef/></w:r><w:r><w:t>First note paragraph.</w:t></w:r></w:p><w:p w14:paraId="22222222"><w:r><w:t>Second note paragraph.</w:t></w:r></w:p></w:comment><w:comment w:id="2" w:author="Cara" w:initials="C"><w:p w14:paraId="33333333"><w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:annotationRef/></w:r><w:r><w:t>Reply in thread.</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    let comments_ex = format!(
        r#"<w15:commentsEx {W15_NS}><w15:commentEx w15:paraId="11111111" w15:done="0"/><w15:commentEx w15:paraId="22222222" w15:done="0"/><w15:commentEx w15:paraId="33333333" w15:paraIdParent="22222222" w15:done="0"/></w15:commentsEx>"#
    );
    let mut doc_rels = String::from(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/><Relationship Id="rId2" Type="http://schemas.microsoft.com/office/2011/relationships/commentsExtended" Target="commentsExtended.xml"/>"#,
    );
    if styles_xml.is_some() {
        doc_rels.push_str(
            r#"<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
        );
    }
    doc_rels.push_str("</Relationships>");
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
        ("word/comments.xml", comments.as_bytes()),
        ("word/commentsExtended.xml", comments_ex.as_bytes()),
    ];
    if let Some(styles) = styles_xml {
        entries.push(("word/styles.xml", styles.as_bytes()));
    }
    zip(&entries)
}

#[test]
fn single_multi_paragraph_and_reply_comments_emit_one_annotation_ref_in_first_paragraph() {
    let input = fixture_docx(None);
    let doc = craft_docx::read(&input).unwrap();
    assert_eq!(doc.comments.len(), 3);
    assert_eq!(doc.comments.get(&2).unwrap().parent, Some(1));

    let saved = craft_docx::write(&doc).unwrap();
    let comments_xml = unzip_part(&saved, "word/comments.xml");
    assert_comment_marks(&comments_xml, &[1, 2, 1], "CommentReference");

    let reread = craft_docx::read(&saved).unwrap();
    assert_eq!(reread.comments.len(), 3);
    assert_eq!(reread.comments.get(&2).unwrap().parent, Some(1));
    assert_eq!(reread.plain_text(StoryRef::Part(reread.comments[&0].part)), "Single paragraph note.");
    assert_eq!(reread.plain_text(StoryRef::Part(reread.comments[&1].part)), "First note paragraph.\nSecond note paragraph.");
    assert_eq!(reread.plain_text(StoryRef::Part(reread.comments[&2].part)), "Reply in thread.");
}

#[test]
fn second_save_keeps_single_annotation_ref_without_duplication() {
    let input = fixture_docx(None);
    let doc1 = craft_docx::read(&input).unwrap();
    let save1 = craft_docx::write(&doc1).unwrap();
    assert_comment_marks(&unzip_part(&save1, "word/comments.xml"), &[1, 2, 1], "CommentReference");

    let doc2 = craft_docx::read(&save1).unwrap();
    let save2 = craft_docx::write(&doc2).unwrap();
    assert_comment_marks(&unzip_part(&save2, "word/comments.xml"), &[1, 2, 1], "CommentReference");

    let doc3 = craft_docx::read(&save2).unwrap();
    for id in [0, 1, 2] {
        assert_eq!(
            doc3.plain_text(StoryRef::Part(doc3.comments[&id].part)),
            doc1.plain_text(StoryRef::Part(doc1.comments[&id].part)),
            "comment {id} text drifted across second save"
        );
    }
    assert_eq!(doc3.comments[&2].parent, Some(1));
}

#[test]
fn comment_annotation_ref_uses_localized_annotation_reference_style() {
    let styles = format!(
        r#"<w:styles {W_NS}><w:style w:type="character" w:styleId="Odwoaniedokomentarza"><w:name w:val="annotation reference"/></w:style></w:styles>"#
    );
    let input = fixture_docx(Some(&styles));
    let doc = craft_docx::read(&input).unwrap();
    let saved = craft_docx::write(&doc).unwrap();
    let comments_xml = unzip_part(&saved, "word/comments.xml");
    assert_comment_marks(&comments_xml, &[1, 2, 1], "Odwoaniedokomentarza");
}

#[test]
fn comment_without_source_mark_and_empty_comment_each_get_one_annotation_ref() {
    let doc = format!(
        r#"<w:document {W_NS}><w:body><w:p><w:commentRangeStart w:id="0"/><w:r><w:t>First. </w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r><w:commentRangeStart w:id="1"/><w:r><w:t>Second.</w:t></w:r><w:commentRangeEnd w:id="1"/><w:r><w:commentReference w:id="1"/></w:r></w:p></w:body></w:document>"#
    );
    let comments = format!(
        r#"<w:comments {W_NS}><w:comment w:id="0" w:author="Ada" w:initials="A"><w:p><w:r><w:t>No source mark.</w:t></w:r></w:p></w:comment><w:comment w:id="1" w:author="Ben" w:initials="B"/></w:comments>"#
    );
    let doc_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/></Relationships>"#;
    let input = zip(&[
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
        ("word/comments.xml", comments.as_bytes()),
    ]);

    let doc1 = craft_docx::read(&input).unwrap();
    assert_eq!(doc1.comments.len(), 2);
    let save1 = craft_docx::write(&doc1).unwrap();
    assert_comment_marks(&unzip_part(&save1, "word/comments.xml"), &[1, 1], "CommentReference");

    let doc2 = craft_docx::read(&save1).unwrap();
    assert_eq!(doc2.plain_text(StoryRef::Part(doc2.comments[&0].part)), "No source mark.");
    assert_eq!(doc2.plain_text(StoryRef::Part(doc2.comments[&1].part)), "");
    let save2 = craft_docx::write(&doc2).unwrap();
    assert_comment_marks(&unzip_part(&save2, "word/comments.xml"), &[1, 1], "CommentReference");
}
