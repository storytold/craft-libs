//! Annotations of different kinds keep distinct `w:id`s through a save (#3307). All markup here is
//! synthetic, written from ECMA-376 Part 1 §17.13.4–§17.13.6.

use std::collections::BTreeMap;
use std::io::Read;

use craft_wordmodel::{Block, Document, InlineObject};

use crate::fixtures::zip;

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const DOC_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/></Relationships>"#;

/// The annotation start elements whose `w:id`s share one number space.
const STARTS: &[&str] = &[
    "w:bookmarkStart",
    "w:permStart",
    "w:commentRangeStart",
    "w:ins",
    "w:del",
    "w:moveFromRangeStart",
    "w:moveToRangeStart",
    "w:pPrChange",
    "w:rPrChange",
];

/// One paragraph with a bookmark, an editable range, a comment and an insertion, ids as given.
fn docx(bookmark: u32, perm: u32, comment: u32, ins: u32) -> Vec<u8> {
    let doc = format!(
        r#"<w:document {W_NS}><w:body><w:p><w:bookmarkStart w:id="{bookmark}" w:name="Mark"/><w:permStart w:id="{perm}" w:edGrp="everyone"/><w:commentRangeStart w:id="{comment}"/><w:r><w:t xml:space="preserve">Kept </w:t></w:r><w:ins w:id="{ins}" w:author="Ann" w:date="2026-01-02T03:04:05Z"><w:r><w:t>added</w:t></w:r></w:ins><w:commentRangeEnd w:id="{comment}"/><w:r><w:commentReference w:id="{comment}"/></w:r><w:permEnd w:id="{perm}"/><w:bookmarkEnd w:id="{bookmark}"/></w:p></w:body></w:document>"#
    );
    let comments = format!(
        r#"<w:comments {W_NS}><w:comment w:id="{comment}" w:author="Bob" w:initials="B"><w:p><w:r><w:t>A note</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    zip(&[
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", DOC_RELS.as_bytes()),
        ("word/comments.xml", comments.as_bytes()),
    ])
}

/// The `w:id` of each annotation start element, by element name.
fn ids(xml: &str) -> BTreeMap<&'static str, Vec<String>> {
    let mut out = BTreeMap::new();
    for el in STARTS {
        let found: Vec<String> = xml
            .split(&format!("<{el} "))
            .skip(1)
            .filter_map(|s| format!(" {}", s.split('>').next()?).split(" w:id=\"").nth(1)?.split('"').next().map(str::to_string))
            .collect();
        if !found.is_empty() {
            out.insert(*el, found);
        }
    }
    out
}

/// Ids that elements of two different kinds share.
fn shared_across_kinds(ids: &BTreeMap<&'static str, Vec<String>>) -> Vec<String> {
    let mut owner: BTreeMap<&str, &str> = BTreeMap::new();
    let mut shared = Vec::new();
    for (el, list) in ids {
        for id in list {
            match owner.get(id.as_str()) {
                Some(other) if other != el => shared.push(format!("{id} ({other}, {el})")),
                _ => {
                    owner.insert(id, el);
                }
            }
        }
    }
    shared
}

fn saved_part(d: &Document, name: &str) -> String {
    let saved = craft_docx::write(d).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(saved)).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

/// Saves `d` and checks every annotation is written with an id no other kind shares, and that the
/// comment range still names its comment. Returns the written ids.
fn check_distinct(d: &Document) -> BTreeMap<&'static str, Vec<String>> {
    let body = saved_part(d, "word/document.xml");
    let written = ids(&body);
    for el in ["w:bookmarkStart", "w:permStart", "w:commentRangeStart", "w:ins"] {
        assert!(written.contains_key(el), "{el} is written back: {body}");
    }
    assert_eq!(shared_across_kinds(&written), Vec::<String>::new(), "ids shared across kinds: {body}");
    let comments = saved_part(d, "word/comments.xml");
    let comment = &written["w:commentRangeStart"][0];
    assert!(comments.contains(&format!(r#"<w:comment w:id="{comment}""#)), "the comment range still names its comment: {comments}");
    written
}

#[test]
fn annotations_of_different_kinds_keep_distinct_ids_through_a_save() {
    // The ids #3307 reports, and ids at both ends of the number space.
    for (bookmark, perm, comment, ins) in [(10, 20, 1, 30), (0, u32::MAX, 1, 2), (u32::MAX, 0, u32::MAX - 1, 1)] {
        check_distinct(&craft_docx::read(&docx(bookmark, perm, comment, ins)).unwrap());
    }
}

#[test]
fn an_editable_range_at_the_last_id_keeps_it_and_nothing_wraps_onto_it() {
    let mut d = craft_docx::read(&docx(10, 20, 1, 30)).unwrap();
    let Some(Block::Para(p)) = d.body.first_mut().map(std::sync::Arc::make_mut) else { panic!("the paragraph is read") };
    for o in &mut p.objects {
        if let InlineObject::PermStart { id, .. } | InlineObject::PermEnd { id } = o {
            *id = u32::MAX;
        }
    }
    let written = check_distinct(&d);
    assert_eq!(written["w:permStart"], [u32::MAX.to_string()], "the editable range keeps its id");
}
