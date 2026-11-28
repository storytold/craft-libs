//! Tests of writing back into the file read: HWPX written here by hand, read, edited in the
//! model, written back.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::sync::Arc;

use craft_wordmodel::{Block, Blocks, CharProps, Document, Paragraph};

use super::SaveReport;
use super::tests::zip_part;
use crate::read::tests::{NS, hwpx};

/// `doc` written back into `original`, and what was written.
fn write_back(original: &[u8], doc: &Document) -> (Vec<u8>, SaveReport) {
    super::back::write_back(original, doc, None).unwrap()
}

/// The paragraphs of a story, those in its tables' cells too, for editing.
fn paras_mut(blocks: &mut Blocks, f: &mut dyn FnMut(&mut Paragraph) -> bool) -> bool {
    for b in blocks.iter_mut() {
        let done = match Arc::make_mut(b) {
            Block::Para(p) => f(p),
            Block::Table(t) => t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()).any(|c| paras_mut(&mut c.blocks, f)),
        };
        if done {
            return true;
        }
    }
    false
}

/// The character properties at byte `off` of `p`.
fn props_at(p: &Paragraph, off: usize) -> CharProps {
    let mut pos = 0;
    for r in &p.runs {
        if off < pos + r.len || pos + r.len == p.text.len() {
            return r.props.clone();
        }
        pos += r.len;
    }
    CharProps::default()
}

/// `original` read, with the first `from` in the text of a paragraph (any story) replaced by
/// `to` (typed in that text's properties).
fn edited(original: &[u8], from: &str, to: &str) -> Document {
    let mut doc = crate::read(original).unwrap();
    let mut edit = |p: &mut Paragraph| {
        let Some(at) = p.text.find(from) else { return false };
        let props = props_at(p, at);
        p.delete(at, at + from.len()).unwrap();
        p.insert_text(at, to, &props).unwrap();
        true
    };
    let mut done = paras_mut(&mut doc.body, &mut edit);
    for part in doc.parts.values_mut() {
        done = done || paras_mut(&mut part.blocks, &mut edit);
    }
    assert!(done, "no {from:?} in the document");
    doc
}

/// The texts of a document's body paragraphs.
fn body_texts(doc: &Document) -> Vec<String> {
    doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect()
}

/// Read, then written back unedited: the same package, nothing written.
#[test]
fn write_back_of_an_unedited_document_keeps_the_file() {
    let original =
        hwpx(&format!(r#"<hs:sec {NS}><hp:p><hp:run><hp:t>첫째</hp:t></hp:run></hp:p><hp:p><hp:run><hp:t>둘째</hp:t></hp:run></hp:p></hs:sec>"#));
    let doc = crate::read(&original).unwrap();
    let (out, report) = write_back(&original, &doc);
    assert_eq!(report.modified + report.inserted + report.deleted, 0, "{report:?}");
    assert!(report.skipped.is_empty(), "{report:?}");
    assert_eq!(zip_part(&out, "Contents/section0.xml"), zip_part(&original, "Contents/section0.xml"));
}

/// The body's only paragraph replaced by a new one: it is that paragraph, edited, so the new text
/// is written in its place; emptied, its text goes.
#[test]
fn write_back_writes_a_body_whose_paragraph_was_replaced() {
    let original = hwpx(&format!(r#"<hs:sec {NS}><hp:p><hp:run><hp:t>원래 글</hp:t></hp:run></hp:p></hs:sec>"#));
    let mut doc = crate::read(&original).unwrap();
    doc.body = vec![Arc::new(Block::Para(Paragraph::with_text("새 글", CharProps::default())))];
    let (out, report) = write_back(&original, &doc);
    assert!(report.skipped.is_empty(), "{report:?}");
    let back = crate::read(&out).unwrap();
    assert_eq!(body_texts(&back), ["새 글"]);
    doc.body = vec![Arc::new(Block::Para(Paragraph::new()))];
    let (out, _) = write_back(&original, &doc);
    assert!(!zip_part(&out, "Contents/section0.xml").contains("원래 글"), "emptied");
}

/// The second paragraph's text appended to the first and the second removed.
fn merge_first_two(original: &[u8]) -> Document {
    let mut doc = crate::read(original).unwrap();
    let second = doc.body.remove(1);
    let second = second.as_para().cloned().unwrap();
    if let Some(Block::Para(first)) = doc.body.first_mut().map(Arc::make_mut) {
        first.append(second);
    }
    doc
}

/// Two paragraphs merged into one whose text can't be rewritten (it holds a hyphen control the
/// writer doesn't rewrite): both are kept and the save says so. Before, the second was deleted
/// while the merge was reported as not written, so its text was lost.
#[test]
fn write_back_keeps_merged_paragraphs_when_the_merge_isnt_written() {
    let original = hwpx(&format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:t>첫째<hp:hypen/>문단</hp:t></hp:run></hp:p><hp:p><hp:run><hp:t>둘째 문단</hp:t></hp:run></hp:p></hs:sec>"#
    ));
    let (out, report) = write_back(&original, &merge_first_two(&original));
    let sec = zip_part(&out, "Contents/section0.xml");
    assert!(sec.contains("둘째 문단"), "the second paragraph is kept: {report:?}");
    assert!(report.skipped.iter().any(|s| s.contains("merged into it are kept")), "{report:?}");
}

/// A paragraph merged into a plain one, the merged one holding a hyphen control the writer can't
/// clear: the merge isn't written, both stay as they were, and the save says so. Before, the
/// first took both texts while the second couldn't be removed: its text was there twice.
#[test]
fn write_back_writes_no_merge_it_cant_finish() {
    let original = hwpx(&format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:t>첫째 문단</hp:t></hp:run></hp:p><hp:p><hp:run><hp:t>둘째<hp:hypen/>문단</hp:t></hp:run></hp:p></hs:sec>"#
    ));
    let (out, report) = write_back(&original, &merge_first_two(&original));
    assert!(report.skipped.iter().any(|s| s.contains("the merge is not written")), "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    assert_eq!(sec.matches("둘째").count(), 1, "the second text once: {sec}");
    assert!(sec.contains("<hp:t>첫째 문단</hp:t>"), "the first as it was: {sec}");
}

/// Two plain paragraphs merged: the first takes both texts and the second goes.
#[test]
fn write_back_writes_a_merge() {
    let original = hwpx(&format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:t>첫째 문단</hp:t></hp:run></hp:p><hp:p><hp:run><hp:t>둘째 문단</hp:t></hp:run></hp:p></hs:sec>"#
    ));
    let (out, report) = write_back(&original, &merge_first_two(&original));
    assert!(report.skipped.is_empty(), "{report:?}");
    assert_eq!(body_texts(&crate::read(&out).unwrap()), ["첫째 문단둘째 문단"]);
}

/// A section naming the paragraph namespace with two prefixes (`p`, declared first, and `hp`):
/// the writer counts paragraphs as the reader does, so an edit to the first paragraph changes the
/// first. Before, it indexed only `p:p` and the edit went to the second paragraph of the same text.
#[test]
fn write_back_counts_paragraphs_of_every_prefix() {
    let original = hwpx(
        r#"<hs:sec xmlns:p="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section"><hp:p><hp:run><hp:t>같은 글</hp:t></hp:run></hp:p><p:p><p:run><p:t>같은 글</p:t></p:run></p:p></hs:sec>"#,
    );
    let (out, report) = write_back(&original, &edited(&original, "같은 글", "같은 글 첫째"));
    assert_eq!(report.modified, 1, "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    let first = sec.find("<hp:p>").unwrap();
    let second = sec.find("<p:p>").unwrap();
    assert!(sec[first..second].contains("같은 글 첫째"), "the first paragraph is edited: {sec}");
    assert!(!sec[second..].contains("첫째"), "the second is not: {sec}");
}

/// A section using the paragraph namespace as its default (no prefix): an edit writes `hp:`
/// runs, and the section root binds `hp`, so the saved part reads with the edit. Before, the
/// runs named a prefix the part never declared.
#[test]
fn write_back_binds_hp_in_default_namespace_sections() {
    let original = hwpx(
        r#"<hs:sec xmlns="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section"><p><run><t>original text</t></run></p></hs:sec>"#,
    );
    let (out, report) = write_back(&original, &edited(&original, "original text", "original text, edited"));
    assert_eq!(report.modified, 1, "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    let root = &sec[sec.find("<hs:sec").unwrap()..];
    assert!(root[..root.find('>').unwrap()].contains(r#"xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph""#), "{sec}");
    assert_eq!(body_texts(&crate::read(&out).unwrap()), ["original text, edited"], "the edit reads back");
}

/// A paragraph added after an `hp:p` in a part that declares `p` first for the same namespace is
/// closed by the name its copied start tag opens. Before, it opened `<hp:p` and closed `</p:p>`,
/// which no XML reader accepts.
#[test]
fn write_back_closes_new_paragraphs_by_their_own_name() {
    let original = hwpx(
        r#"<hs:sec xmlns:p="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section"><hp:p><hp:run><hp:t>첫째</hp:t></hp:run></hp:p><p:p><p:run><p:t>둘째</p:t></p:run></p:p></hs:sec>"#,
    );
    let mut doc = crate::read(&original).unwrap();
    doc.body.insert(1, Arc::new(Block::Para(Paragraph::with_text("새 문단", CharProps::default()))));
    let (out, report) = write_back(&original, &doc);
    assert_eq!(report.inserted, 1, "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    assert!(sec.contains("새 문단"), "{sec}");
    for pfx in ["hp", "p"] {
        let opens = sec.matches(&format!("<{pfx}:p>")).count() + sec.matches(&format!("<{pfx}:p ")).count();
        assert_eq!(opens, sec.matches(&format!("</{pfx}:p>")).count(), "{pfx}:p opened and closed alike: {sec}");
    }
    assert_eq!(body_texts(&crate::read(&out).unwrap()), ["첫째", "새 문단", "둘째"]);
}

/// A section root binding `hp` in single quotes keeps its one binding when edited, and one
/// binding `hp` to another namespace keeps its text (the writer's `hp:` would mean something
/// else there) and the save says so. Before, a second `xmlns:hp` made the root ill-formed.
#[test]
fn write_back_reads_the_roots_hp_binding() {
    let edit = |section: &str| {
        let original = hwpx(section);
        let (out, report) = write_back(&original, &edited(&original, "본문", "본문 고침"));
        (zip_part(&out, "Contents/section0.xml"), report)
    };
    let (sec, report) = edit(
        r#"<hs:sec xmlns:hp='http://www.hancom.co.kr/hwpml/2011/paragraph' xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section"><hp:p><hp:run><hp:t>본문</hp:t></hp:run></hp:p></hs:sec>"#,
    );
    assert_eq!(report.modified, 1, "{report:?}");
    let root = &sec[..sec.find('>').unwrap()];
    assert_eq!(root.matches("xmlns:hp").count(), 1, "{root}");
    let (sec, report) = edit(
        r#"<hs:sec xmlns:hp="urn:other" xmlns:p="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section"><p:p><p:run><p:t>본문</p:t></p:run></p:p></hs:sec>"#,
    );
    assert!(report.skipped.iter().any(|s| s.contains("another namespace")), "{report:?}");
    assert!(!sec.contains("고침"), "kept as it was: {sec}");
}

/// A paragraph that starts with a document's own bookmark (named like WordCraft's old source
/// marks) is edited like any other.
#[test]
fn paragraphs_starting_with_a_bookmark_are_written_back() {
    let original = hwpx(&format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:ctrl><hp:bookmark name="_hp_0_0"/></hp:ctrl><hp:t>책갈피 문단</hp:t></hp:run></hp:p></hs:sec>"#
    ));
    let (out, report) = write_back(&original, &edited(&original, "책갈피 문단", "책갈피 문단 고침"));
    assert_eq!(report.modified, 1, "{report:?}");
    assert!(zip_part(&out, "Contents/section0.xml").contains("고침"));
}

/// Source paragraphs moved in the model can't be moved in the original file (they keep their
/// places): the save says so. Before, both texts read unchanged and nothing was reported.
#[test]
fn write_back_reports_moved_paragraphs() {
    let original =
        hwpx(&format!(r#"<hs:sec {NS}><hp:p><hp:run><hp:t>첫째</hp:t></hp:run></hp:p><hp:p><hp:run><hp:t>둘째</hp:t></hp:run></hp:p></hs:sec>"#));
    let mut doc = crate::read(&original).unwrap();
    doc.body.swap(0, 1);
    let (_, report) = write_back(&original, &doc);
    assert!(report.skipped.iter().any(|s| s.contains("moved")), "{report:?}");
    let (_, report) = write_back(&original, &crate::read(&original).unwrap());
    assert!(report.skipped.is_empty(), "{report:?}");
}

/// Attributes in single quotes, or with spaces around `=`, are read and rewritten as XML allows.
/// A run whose shape is written `charPrIDRef='7'` keeps it when text is added to it. Before,
/// the shape read as 0 and the run's existing text took shape 0 too.
#[test]
fn single_quoted_attributes_are_read() {
    let attrs = crate::write::back::tag_attrs(r#"<hp:run charPrIDRef='7' a = "1" b='x"y'>"#);
    assert_eq!(attrs.get("charPrIDRef").map(String::as_str), Some("7"));
    assert_eq!(attrs.get("a").map(String::as_str), Some("1"));
    assert_eq!(attrs.get("b").map(String::as_str), Some(r#"x"y"#));
    let original = hwpx(&format!(r#"<hs:sec {NS}><hp:p paraPrIDRef='0'><hp:run charPrIDRef='7'><hp:t>원래</hp:t></hp:run></hp:p></hs:sec>"#));
    let (out, report) = write_back(&original, &edited(&original, "원래", "원래 글"));
    assert_eq!(report.modified, 1, "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    assert!(sec.contains("원래 글"), "{sec}");
    let runs: Vec<String> = sec.match_indices("<hp:run").map(|(i, _)| sec[i..i + sec[i..].find('>').unwrap()].to_string()).collect();
    assert!(runs.iter().all(|r| r.contains("charPrIDRef=\"7\"") || r.contains("charPrIDRef='7'")), "{runs:?}");
}

/// A WebP or TIFF picture in a new HWPX (or one added on a save) is named and listed as what it
/// is. Before, every unknown kind became a .png said to be image/png.
#[test]
fn hwpx_pictures_are_named_by_what_they_hold() {
    let webp = [b"RIFF".as_slice(), &[0, 0, 0, 0], b"WEBPVP8 "].concat();
    assert_eq!(crate::write::new::image_kind(&webp), ("webp", "image/webp"));
    assert_eq!(crate::write::new::image_kind(b"II*\0rest"), ("tif", "image/tiff"));
    assert_eq!(crate::write::new::image_kind(b"\x89PNG\r\n"), ("png", "image/png"));
}

#[test]
fn write_back_finds_the_fewest_edits_and_keeps_shapes() {
    use crate::write::back::tests_api::{opcodes, rewrite_text};
    assert_eq!(
        opcodes("abcd", "acbd"),
        [("equal", 0, 1, 0, 1), ("delete", 1, 2, 1, 1), ("equal", 2, 3, 1, 2), ("insert", 3, 3, 2, 3), ("equal", 3, 4, 3, 4)]
    );
    assert_eq!(opcodes("", "x"), [("insert", 0, 0, 0, 1)]);
    assert_eq!(opcodes("x", ""), [("delete", 0, 1, 0, 0)]);
    assert_eq!(opcodes("", ""), []);
    assert_eq!(opcodes("가나다", "가나다"), [("equal", 0, 3, 0, 3)]);
    assert_eq!(opcodes("가나다", "가X다"), [("equal", 0, 1, 0, 1), ("replace", 1, 2, 1, 2), ("equal", 2, 3, 2, 3)]);
    // Every pair of short texts over a small alphabet: the list rebuilds `b` with no more
    // edits than the fewest possible.
    let texts: Vec<String> = (0..4)
        .flat_map(|len| {
            (0..3usize.pow(len)).map(move |mut v| {
                (0..len)
                    .map(|_| {
                        let c = ['a', 'b', '가'][v % 3];
                        v /= 3;
                        c
                    })
                    .collect()
            })
        })
        .collect();
    for a in &texts {
        for b in &texts {
            let (out, edits) = apply_opcodes(a, b);
            assert_eq!(&out, b);
            assert_eq!(edits, fewest_edits(a, b), "{a:?} -> {b:?}");
        }
    }
    for (a, b) in [("the quick brown fox", "the quack brown box"), ("문단의 글자를 고쳤다", "문단 글자를 많이 고쳤다"), ("abcabba", "cbabac")]
    {
        let (out, edits) = apply_opcodes(a, b);
        assert_eq!((out.as_str(), edits), (b, fewest_edits(a, b)), "{a:?} -> {b:?}");
    }
    // typed text takes the shape of the character before it
    let shapes = |after: &str| rewrite_text(&[("가", "1"), ("나", "2")], after).into_iter().map(|(c, k)| format!("{c}{k}")).collect::<Vec<_>>();
    assert_eq!(shapes("가X나"), ["가1", "X1", "나2"]);
    assert_eq!(shapes("Y가나"), ["Y1", "가1", "나2"]);
}

/// Long or very different texts stay within the edit budget: a few edits in a long paragraph
/// are still found one by one; past the budget the middle is one replaced block.
#[test]
fn write_back_compares_long_texts_within_bounds() {
    use crate::write::back::tests_api::opcodes;
    let long: String = "가나다라마바사 ".repeat(20_000);
    let mut edited = long.clone();
    edited.insert_str("가나다라마바사 ".len() * 3_636, "새 글");
    edited.insert(3, 'X');
    let (out, edits) = apply_opcodes(&long, &edited);
    assert_eq!((out, edits), (edited.clone(), 4));
    let (a, b) = ("a".repeat(3_000), "b".repeat(3_000));
    assert_eq!(opcodes(&a, &b), [("replace", 0, 3_000, 0, 3_000)]);
    let (a, b) = (format!("<{}>", "ab".repeat(2_000)), format!("<{}>", "ba".repeat(2_000).replace('a', "c")));
    let ops = opcodes(&a, &b);
    assert_eq!(ops.first(), Some(&("equal", 0, 1, 0, 1)));
    assert_eq!(apply_opcodes(&a, &b).0, b);
}

/// Text split by an object (two groups, "ab" and "cd"): an edit inside one group stays in it,
/// text typed where the object is goes with the group before, an edit across it is refused.
#[test]
fn write_back_edits_text_around_objects() {
    use crate::write::back::tests_api::segment_spans;
    let groups = [(0, 2), (2, 4)];
    assert_eq!(segment_spans("abcd", "aXbcd", &groups), Some(vec![(0, 3), (3, 5)]));
    assert_eq!(segment_spans("abcd", "abcYd", &groups), Some(vec![(0, 2), (2, 5)]));
    assert_eq!(segment_spans("abcd", "abZcd", &groups), Some(vec![(0, 3), (3, 5)]));
    assert_eq!(segment_spans("abcd", "ad", &groups), None);
}

/// A page control in the run of a paragraph's text (Hancom writes page hiding after the text
/// it is typed in): the edit is written and the control kept, once, in a run after the text.
/// Without this the paragraph was skipped as "text and objects in one run".
#[test]
fn write_back_edits_text_beside_page_controls() {
    let hiding =
        r#"<hp:ctrl><hp:pageHiding hideHeader="1" hideFooter="0" hideMasterPage="1" hideBorder="0" hideFill="0" hidePageNum="0"/></hp:ctrl>"#;
    let original = hwpx(&format!(
        r#"<hs:sec {NS}><hp:p><hp:run charPrIDRef="2"><hp:t> 2. 지도</hp:t>{hiding}<hp:t/></hp:run></hp:p><hp:p><hp:run><hp:t>둘째</hp:t></hp:run></hp:p></hs:sec>"#
    ));
    let (out, report) = write_back(&original, &edited(&original, "2. 지도", "새 2. 지도"));
    assert!(report.skipped.is_empty(), "{report:?}");
    assert_eq!(report.modified, 1, "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    assert_eq!(sec.matches("<hp:pageHiding ").count(), 1, "{sec}");
    let (text, ctrl) = (sec.find("새 2. 지도").unwrap(), sec.find("<hp:pageHiding ").unwrap());
    assert!(text < ctrl, "the control follows the text: {sec}");
    assert!(sec.contains(&format!(r#"<hp:run charPrIDRef="2">{hiding}</hp:run>"#)), "{sec}");
    assert!(body_texts(&crate::read(&out).unwrap()).iter().any(|t| t.contains("새 2. 지도")));
}

/// A paragraph holding two tables reads as several paragraphs, each a piece of the source
/// paragraph. Each is compared with the piece at the same place, not with the first: saved
/// unchanged, nothing reads as edited.
#[test]
fn write_back_compares_each_piece_of_a_paragraph() {
    let tbl = |t: &str| {
        format!(
            r#"<hp:tbl rowCnt="1" colCnt="1"><hp:tr><hp:tc><hp:cellAddr rowAddr="0" colAddr="0"/><hp:cellSz width="5000" height="1000"/><hp:subList><hp:p><hp:run><hp:t>{t}</hp:t></hp:run></hp:p></hp:subList></hp:tc></hp:tr></hp:tbl>"#
        )
    };
    let original = hwpx(&format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:t>앞</hp:t>{}<hp:t>가운데</hp:t>{}<hp:t>뒤</hp:t></hp:run></hp:p></hs:sec>"#,
        tbl("칸1"),
        tbl("칸2")
    ));
    let doc = crate::read(&original).unwrap();
    assert!(body_texts(&doc).len() >= 3, "the paragraph reads in pieces");
    let (_, report) = write_back(&original, &doc);
    assert!(report.skipped.is_empty(), "{report:?}");
    assert_eq!(report.modified, 0, "{report:?}");
}

#[test]
fn write_back_hostile_inputs_are_errors() {
    let original = hwpx(&format!(r#"<hs:sec {NS}><hp:p><hp:run><hp:t>a</hp:t></hp:run></hp:p></hs:sec>"#));
    let doc = crate::read(&original).unwrap();
    assert!(super::back::write_back(b"garbage", &doc, None).is_err());
    assert!(super::back::write_back(b"PK\x03\x04 not a zip", &doc, None).is_err());
}

/// Shape ids are remapped in tags only: a cell whose text spells an id reference keeps its text.
/// Before, the text `charPrIDRef="0"` became the header's new id.
#[test]
fn remapped_table_ids_leave_cell_text_alone() {
    let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let xml = r#"<hp:tc borderFillIDRef="1"><hp:p paraPrIDRef="0"><hp:run charPrIDRef="0"><hp:t>charPrIDRef="0" paraPrIDRef="0"</hp:t></hp:run></hp:p></hp:tc>"#;
    let out = crate::write::restyle::remap_ids(xml, &ids(&["4"]), &ids(&["7"]), &ids(&["9"]));
    assert_eq!(
        out,
        r#"<hp:tc borderFillIDRef="9"><hp:p paraPrIDRef="7"><hp:run charPrIDRef="4"><hp:t>charPrIDRef="0" paraPrIDRef="0"</hp:t></hp:run></hp:p></hp:tc>"#
    );
}

/// Text typed into a source paragraph written as an empty tag (`<hp:p/>`) goes inside it: the
/// tag is opened. Before, the new run went after the tag, outside the paragraph, and reopening
/// the file showed the paragraph empty.
#[test]
fn write_back_opens_an_empty_paragraph_tag() {
    let original = hwpx(&format!(r#"<hs:sec {NS}><hp:p><hp:run><hp:t>첫째</hp:t></hp:run></hp:p><hp:p/></hs:sec>"#));
    let mut doc = crate::read(&original).unwrap();
    if let Some(Block::Para(p)) = doc.body.get_mut(1).map(Arc::make_mut) {
        p.insert_text(0, "새 글", &CharProps::default()).unwrap();
    }
    let (out, report) = write_back(&original, &doc);
    assert_eq!(report.modified, 1, "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    let root = crate::xml::parse(sec.as_bytes(), false).unwrap();
    let paras: Vec<_> = root.children.iter().filter(|c| c.tag.ends_with(":p")).collect();
    assert_eq!(paras.len(), 2, "{sec}");
    assert!(paras[1].iter("hp:t").any(|t| t.text.contains("새 글")), "the text is in the paragraph: {sec}");
}

/// In a section whose paragraph namespace is the default one, editing the text of a run that
/// holds settings before its text closes that run with its own name (`</run>`). Before, it was
/// closed as `</hp:run>`, which doesn't match `<run>`.
#[test]
fn write_back_closes_an_unprefixed_run_by_its_name() {
    let sec = r#"<hs:sec xmlns="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section"><p><run><secPr/><t>원래 글</t></run></p></hs:sec>"#;
    let original = hwpx(sec);
    let (out, report) = write_back(&original, &edited(&original, "원래 글", "바뀐 글"));
    assert_eq!(report.modified, 1, "{report:?}");
    let sec = zip_part(&out, "Contents/section0.xml");
    assert!(!sec.contains("</hp:run>"), "{sec}");
    assert!(body_texts(&crate::read(&out).unwrap()).iter().any(|t| t.contains("바뀐 글")), "{sec}");
}

fn apply_opcodes(a: &str, b: &str) -> (String, usize) {
    use crate::write::back::tests_api::opcodes;
    let (ac, bc): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (mut out, mut edits, mut i, mut j) = (String::new(), 0, 0, 0);
    for (op, a1, a2, b1, b2) in opcodes(a, b) {
        assert_eq!((a1, b1), (i, j), "{a:?} -> {b:?}: in order");
        // each kind does what the writer does with it: equal spans come from the source, a
        // delete takes only source characters and writes none, an insert only target ones
        match op {
            "equal" => {
                assert_eq!(ac[a1..a2], bc[b1..b2], "{a:?} -> {b:?}");
                out.extend(&ac[a1..a2]);
            }
            "delete" => {
                assert_eq!(b1, b2, "{a:?} -> {b:?}: a delete takes no target characters");
                edits += a2 - a1;
            }
            "insert" => {
                assert_eq!(a1, a2, "{a:?} -> {b:?}: an insert takes no source characters");
                out.extend(&bc[b1..b2]);
                edits += b2 - b1;
            }
            "replace" => {
                out.extend(&bc[b1..b2]);
                edits += (a2 - a1) + (b2 - b1);
            }
            other => panic!("{a:?} -> {b:?}: unknown edit {other}"),
        }
        (i, j) = (a2, b2);
    }
    assert_eq!((i, j), (ac.len(), bc.len()), "{a:?} -> {b:?}: covers both");
    (out, edits)
}

/// The fewest characters deleted plus inserted to turn `a` into `b` (by the longest common
/// subsequence, worked out cell by cell).
fn fewest_edits(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row = vec![0usize; b.len() + 1];
    for x in &a {
        let mut diag = 0;
        for (j, y) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = if x == y { diag + 1 } else { up.max(row[j]) };
            diag = up;
        }
    }
    a.len() + b.len() - 2 * row[b.len()]
}

/// A paragraph typed in after an edited one: the edited one is still the paragraph it was (the
/// more alike of the two), so it keeps its own paragraph shape, and the new one copies it.
#[test]
fn an_edited_paragraph_beside_a_new_one_keeps_its_own_shape() {
    let original = hwpx(&format!(
        r#"<hs:sec {NS}><hp:p paraPrIDRef="0"><hp:run><hp:t>머리 문단</hp:t></hp:run></hp:p><hp:p paraPrIDRef="5"><hp:run><hp:t>고칠 문단의 글</hp:t></hp:run></hp:p></hs:sec>"#
    ));
    let mut doc = edited(&original, "고칠 문단의 글", "고친 문단의 글");
    doc.body.push(Arc::new(Block::Para(Paragraph::with_text("새 문단", CharProps::default()))));
    let (out, report) = write_back(&original, &doc);
    assert_eq!((report.modified, report.inserted), (1, 1), "{report:?}");
    let sec = crate::xml::parse(zip_part(&out, "Contents/section0.xml").as_bytes(), false).unwrap();
    let shape_of = |text: &str| {
        sec.children.iter().find(|p| p.iter("hp:t").any(|t| t.text.contains(text))).and_then(|p| p.get("paraPrIDRef")).map(str::to_string)
    };
    assert_eq!(shape_of("고친 문단의 글").as_deref(), Some("5"));
    assert_eq!(shape_of("새 문단").as_deref(), Some("5"));
    assert_eq!(shape_of("머리 문단").as_deref(), Some("0"));
}

/// A turned picture resized is written again around its new size: hp:sz stays the box around
/// the turned picture, not its own size. Before, the resize set both to the same size and Hancom
/// drew the turned picture squeezed into it.
#[test]
fn write_back_keeps_a_resized_turned_picture_in_the_box_around_it() {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend(1u32.to_be_bytes());
    png.extend(1u32.to_be_bytes());
    png.extend([8, 6, 0, 0, 0]);
    let pic = r#"<hp:pic id="1" zOrder="1" textWrap="IN_FRONT_OF_TEXT"><hp:offset x="0" y="0"/><hp:orgSz width="10000" height="4000"/><hp:curSz width="10000" height="4000"/><hp:rotationInfo angle="90"/><hc:img binaryItemIDRef="image1"/><hp:sz width="4000" height="10000"/><hp:pos treatAsChar="0" vertRelTo="PARA" horzRelTo="COLUMN" vertAlign="TOP" horzAlign="LEFT" vertOffset="0" horzOffset="0"/></hp:pic>"#;
    let sec =
        format!(r#"<hs:sec {NS} xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core"><hp:p><hp:run>{pic}<hp:t>글</hp:t></hp:run></hp:p></hs:sec>"#);
    let mut z = craft_zip::Writer::new();
    let _ = z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored);
    let _ = z.add("Contents/header.xml", br#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"/>"#, craft_zip::Compression::Deflated);
    let _ = z.add("Contents/section0.xml", sec.as_bytes(), craft_zip::Compression::Deflated);
    let _ = z.add("BinData/image1.png", &png, craft_zip::Compression::Deflated);
    // a manifest to list the picture's new image item in
    let hpf = br#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:manifest><opf:item id="header" href="Contents/header.xml" media-type="application/xml"/><opf:item id="s" href="Contents/section0.xml" media-type="application/xml"/><opf:item id="image1" href="BinData/image1.png" media-type="image/png" isEmbeded="1"/></opf:manifest><opf:spine><opf:itemref idref="header"/><opf:itemref idref="s"/></opf:spine></opf:package>"#;
    let _ = z.add("Contents/content.hpf", hpf, craft_zip::Compression::Deflated);
    let original = z.finish().unwrap();
    let mut doc = crate::read(&original).unwrap();
    let resized = paras_mut(&mut doc.body, &mut |p| {
        let Some(o) = p.objects.iter_mut().find(|o| matches!(o, craft_wordmodel::InlineObject::Image { .. })) else { return false };
        if let craft_wordmodel::InlineObject::Image { w, h, .. } = o {
            (*w, *h) = (*w / 2.0, *h / 2.0);
        }
        true
    });
    assert!(resized, "the picture is read");
    let (bytes, _) = write_back(&original, &doc);
    let out = crate::xml::parse(zip_part(&bytes, "Contents/section0.xml").as_bytes(), false).unwrap();
    let pic = out.iter("hp:pic").next().expect("the picture");
    let size = |tag: &str| -> (i64, i64) {
        let e = pic.find(tag).expect(tag);
        (e.get("width").unwrap().parse().unwrap(), e.get("height").unwrap().parse().unwrap())
    };
    let ((fw, fh), (cw, ch)) = (size("hp:sz"), size("hp:curSz"));
    assert!(fw < 4000 || fh < 10000, "resized: {:?}", (fw, fh));
    assert!((fw - ch).abs() <= 1 && (fh - cw).abs() <= 1, "the box around the turned picture: {:?}", ((fw, fh), (cw, ch)));
}
