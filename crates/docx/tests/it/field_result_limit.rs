//! A field's cached result is buffered while it is read, up to 100,000 items (text runs and
//! inline objects together); items past that are dropped (`emit_text` and `emit_obj` in
//! `read/story/runs.rs`). All markup here is synthetic, written from ECMA-376.

use std::io::Write;

use craft_docx::read;
use craft_wordmodel::{InlineObject, Path, StoryRef};

const LIMIT: usize = 100_000;
const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
/// A positional tab: an inline object without a frame, whose plain text is a tab.
const PTAB: &str = r#"<w:r><w:ptab w:alignment="left" w:relativeTo="margin" w:leader="none"/></w:r>"#;

fn package(body: &str) -> Vec<u8> {
    let doc = format!(r#"<w:document {W_NS}><w:body>{body}</w:body></w:document>"#);
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [("_rels/.rels", ROOT_RELS.as_bytes()), ("word/document.xml", doc.as_bytes())] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn fld(kind: &str) -> String {
    format!(r#"<w:r><w:fldChar w:fldCharType="{kind}"/></w:r>"#)
}

/// The `n` letters `a`, `b`, … `z`, `a`, … that `text_runs(n)` writes, so order is checked too.
fn letters(n: usize) -> String {
    (0..n).map(|i| char::from(b'a' + (i % 26) as u8)).collect()
}

/// `n` runs of one letter each.
fn text_runs(n: usize) -> String {
    letters(n).chars().map(|c| format!("<w:r><w:t>{c}</w:t></w:r>")).collect()
}

/// Read a paragraph holding a `MERGEFIELD` with the runs `before` between its code and
/// `separate`, and `result` runs after it, then the text `after`. Returns the field's result
/// and the paragraph's text.
fn read_field(before: &str, result: &str) -> (String, String) {
    let p = format!(
        r#"<w:p>{}<w:r><w:instrText xml:space="preserve"> MERGEFIELD name </w:instrText></w:r>{before}{}{result}{}<w:r><w:t>after</w:t></w:r></w:p>"#,
        fld("begin"),
        fld("separate"),
        fld("end"),
    );
    let d = read(&package(&p)).unwrap();
    let para = d.para(StoryRef::Body, &Path::top(0)).unwrap();
    let result = para
        .objects
        .iter()
        .find_map(|o| match o {
            InlineObject::Field { instr, result, .. } if instr == "MERGEFIELD name" => Some(result.clone()),
            _ => None,
        })
        .expect("the MERGEFIELD is read as a field");
    (result, para.text.clone())
}

/// Compare long results without printing them.
fn check(got: &str, want: &str) {
    assert_eq!(got.len(), want.len(), "result length");
    let at = got.bytes().zip(want.bytes()).position(|(a, b)| a != b);
    assert!(at.is_none(), "results differ from byte {at:?}");
}

/// Text past the limit is dropped. Ordinary runs between `begin` and `separate` (`w:t` text and
/// an object, not field code, which is `w:instrText`) are not part of the result and don't count
/// towards it. Reading goes on after the field.
#[test]
fn text_items_past_the_limit_are_dropped() {
    let before = format!("{}{PTAB}", "<w:r><w:t>i</w:t></w:r>".repeat(10));
    let (result, text) = read_field(&before, &text_runs(LIMIT + 1));
    check(&result, &letters(LIMIT));
    assert!(text.ends_with("after"), "the paragraph goes on after the field");
}

/// The object that fills the last place is kept; the one after it is dropped.
#[test]
fn an_object_in_the_last_place_is_kept_and_the_next_dropped() {
    let (result, _) = read_field("", &format!("{}{PTAB}{PTAB}", text_runs(LIMIT - 1)));
    check(&result, &format!("{}\t", letters(LIMIT - 1)));
}

/// Objects and text share the limit: an object first leaves room for one text run fewer.
#[test]
fn objects_and_text_share_the_limit() {
    let (result, _) = read_field("", &format!("{PTAB}{}", text_runs(LIMIT)));
    check(&result, &format!("\t{}", letters(LIMIT - 1)));
}
