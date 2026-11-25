//! Tests of the reader on HWPX written here by hand from the published element names.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use super::super::read as read_bytes;

/// A package of `section` (an `hs:sec` element) with an empty header.
pub(crate) fn hwpx(section: &str) -> Vec<u8> {
    hwpx_with(r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"/>"#, section)
}

/// A package of `header` (an `hh:head` element) and `section`.
pub(crate) fn hwpx_with(header: &str, section: &str) -> Vec<u8> {
    let mut z = craft_zip::Writer::new();
    let _ = z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored);
    let _ = z.add("Contents/header.xml", header.as_bytes(), craft_zip::Compression::Deflated);
    let _ = z.add("Contents/section0.xml", section.as_bytes(), craft_zip::Compression::Deflated);
    z.finish().unwrap_or_default()
}

pub(crate) const NS: &str = r#"xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section""#;

/// The text of a section's paragraphs reads into the body, one paragraph each.
#[test]
fn paragraphs_read_into_the_body() {
    let doc = read_bytes(&hwpx(&format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:t>첫째</hp:t></hp:run></hp:p><hp:p><hp:run><hp:t>둘째</hp:t></hp:run></hp:p></hs:sec>"#
    )))
    .unwrap();
    let texts: Vec<&str> = doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.as_str()).collect();
    assert_eq!(texts, ["첫째", "둘째"]);
}

/// How an object floats.
fn wrap_of(o: &craft_wordmodel::InlineObject) -> Option<craft_wordmodel::para::Wrap> {
    use craft_wordmodel::InlineObject as O;
    match o {
        O::Image { float, .. } | O::Shape { float, .. } | O::Group { float, .. } => Some(float.wrap),
        _ => None,
    }
}

/// The floating objects of a document's headers.
fn header_floats(doc: &craft_wordmodel::Document) -> Vec<craft_wordmodel::InlineObject> {
    doc.parts
        .values()
        .filter(|p| p.kind == craft_wordmodel::PartKind::Header)
        .flat_map(|p| p.blocks.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.clone()).collect::<Vec<_>>())
        .filter(|o| o.is_floating())
        .collect()
}

/// A page background colour goes behind the text of every page: the model has one background
/// for the whole document only, so it rides in the section's header.
#[test]
fn a_page_background_lies_behind_the_text_in_the_header() {
    let header = r##"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core"><hh:refList><hh:borderFills><hh:borderFill id="2"><hc:fillBrush><hc:winBrush faceColor="#FFEEDD"/></hc:fillBrush></hh:borderFill></hh:borderFills></hh:refList></hh:head>"##;
    let section = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr><hp:pageBorderFill type="BOTH" borderFillIDRef="2" textBorder="PAPER" fillArea="PAPER"/></hp:secPr><hp:t>본문</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let doc = read_bytes(&hwpx_with(header, &section)).unwrap();
    let floats = header_floats(&doc);
    assert!(!floats.is_empty(), "no background in a header");
    assert!(floats.iter().all(|o| wrap_of(o) == Some(craft_wordmodel::para::Wrap::BehindText)));
}

/// Objects on the master page (Hancom: 바탕쪽) repeat on every page behind the text, whatever wrapping they had.
#[test]
fn master_page_objects_lie_behind_the_text_in_the_header() {
    let section = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr><hp:masterPage type="BOTH"><hp:subList><hp:p><hp:run><hp:rect textWrap="IN_FRONT_OF_TEXT"><hp:sz width="7200" height="7200"/><hp:pos treatAsChar="0" vertRelTo="PAPER" horzRelTo="PAPER"/></hp:rect></hp:run></hp:p></hp:subList></hp:masterPage></hp:secPr><hp:t>본문</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let doc = read_bytes(&hwpx(&section)).unwrap();
    let floats = header_floats(&doc);
    assert_eq!(floats.len(), 1, "the master page's shape is not in a header");
    assert!(floats.iter().all(|o| wrap_of(o) == Some(craft_wordmodel::para::Wrap::BehindText)));
}

/// Each paragraph remembers the source paragraph it came from while it stays as read, so a
/// save back can tell edited paragraphs from untouched ones.
#[test]
fn paragraphs_remember_their_source_until_edited() {
    let bytes =
        hwpx(&format!(r#"<hs:sec {NS}><hp:p><hp:run><hp:t>첫째</hp:t></hp:run></hp:p><hp:p><hp:run><hp:t>둘째</hp:t></hp:run></hp:p></hs:sec>"#));
    let doc = read_bytes(&bytes).unwrap();
    let kept = doc.kept.of(crate::KEPT_FORMAT).expect("nothing kept");
    let paras: Vec<&craft_wordmodel::Paragraph> = doc.body.iter().filter_map(|b| b.as_para()).collect();
    let sources: Vec<_> = paras.iter().map(|p| kept.para(p).cloned()).collect();
    assert!(sources.iter().all(Option::is_some), "a paragraph as read has no source");
    assert_ne!(sources[0], sources[1], "two paragraphs name one source");
    let mut edited = paras[0].clone();
    edited.text.push('!');
    if let Some(r) = edited.runs.last_mut() {
        r.len += 1;
    }
    assert!(kept.para(&edited).is_none(), "an edited paragraph still names its source");
    assert_eq!(doc.passthrough.get(crate::PACKAGE_KEY).map(|b| b.as_slice()), Some(bytes.as_slice()), "the file read isn't kept");
}

// ==== moved from WordCraft's Hangul reader (its tests checked the DOCX it wrote; these check
// the model it now fills) ====================================================================

use craft_wordmodel::{Document, InlineObject, PartKind};

/// The section namespace with the paragraph namespace under a prefix of the file's own, `p`.
const NS_P: &str = r#"xmlns:p="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section""#;

fn package(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = craft_zip::Writer::new();
    let _ = z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored);
    for (name, bytes) in parts {
        let _ = z.add(name, bytes, craft_zip::Compression::Deflated);
    }
    z.finish().unwrap_or_default()
}

const EMPTY_HEAD: &[u8] = br#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"/>"#;

fn body_text(doc: &Document) -> String {
    doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n")
}

#[test]
fn names_are_normalised_and_paragraphs_numbered() {
    // A file using its own prefix ("p") for the paragraph namespace.
    let sec = format!(r#"<hs:sec {NS_P}><p:p><p:run><p:t>a&amp;b</p:t></p:run></p:p><p:p/></hs:sec>"#);
    let root = crate::xml::parse(sec.as_bytes(), true).unwrap();
    let ps: Vec<_> = root.iter("hp:p").collect();
    assert_eq!(ps.len(), 2);
    assert_eq!(ps[0].get(crate::xml::ORDINAL), Some("0"));
    assert_eq!(ps[1].get(crate::xml::ORDINAL), Some("1"));
    assert_eq!(root.iter("hp:t").next().unwrap().text, "a&b");
}

#[test]
fn switch_keeps_the_known_case() {
    let sec = format!(
        r#"<hs:sec {NS_P}><p:switch><p:case p:required-namespace="http://www.hancom.co.kr/hwpml/2016/HwpUnitChar"><p:p id="case"/></p:case><p:default><p:p id="default"/></p:default></p:switch></hs:sec>"#
    );
    let root = crate::xml::parse(sec.as_bytes(), true).unwrap();
    let ids: Vec<_> = root.iter("hp:p").map(|p| p.get("id").unwrap_or("")).collect();
    assert_eq!(ids, ["case"]);
    // numbered before the switch was resolved: the dropped default copy keeps its number
    assert_eq!(root.iter("hp:p").next().unwrap().get(crate::xml::ORDINAL), Some("0"));
}

/// A switch inside the branch read is resolved too, and the text after a switch is kept.
/// Before, the inner switch stayed (its text unread), and the text after a switch was lost.
#[test]
fn switches_in_branches_and_the_text_after_them_are_read() {
    let sec = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:switch><hp:default><hp:switch><hp:default><hp:t>안쪽</hp:t></hp:default></hp:switch></hp:default></hp:switch><hp:t>앞<hp:switch><hp:default><hp:tab/></hp:default></hp:switch>뒤</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let text = body_text(&read_bytes(&hwpx(&sec)).unwrap());
    assert!(text.contains("안쪽"), "the inner switch's branch: {text:?}");
    assert!(text.contains('뒤'), "the text after the switch: {text:?}");
}

/// A note whose tracked deletion never ends (a damaged file) doesn't take the body's text after
/// it into the deletion. Before, the deletion state came back with the body.
#[test]
fn a_deletion_left_open_in_a_note_stays_in_the_note() {
    let sec = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:t>앞</hp:t><hp:ctrl><hp:footNote><hp:subList><hp:p><hp:run><hp:t><hp:deleteBegin/>지운 글</hp:t></hp:run></hp:p></hp:subList></hp:footNote></hp:ctrl><hp:t>뒤</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let text = body_text(&read_bytes(&hwpx(&sec)).unwrap());
    assert!(text.contains('뒤'), "the body's text after the note: {text:?}");
}

/// A paragraph whose shape breaks the page before it, holding a table and then text, breaks
/// the page once, before the table. Before, the text after the table broke the page again.
#[test]
fn a_page_break_before_a_paragraph_holding_a_table_comes_once() {
    let head = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:paraProperties itemCnt="2"><hh:paraPr id="0"/><hh:paraPr id="1"><hh:breakSetting pageBreakBefore="1"/></hh:paraPr></hh:paraProperties></hh:refList></hh:head>"#;
    let tbl = r#"<hp:tbl rowCnt="1" colCnt="1"><hp:sz width="10000" height="2000"/><hp:pos treatAsChar="0"/><hp:tr><hp:tc><hp:subList><hp:p><hp:run><hp:t>칸</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:cellSz width="10000" height="2000"/></hp:tc></hp:tr></hp:tbl>"#;
    let sec = format!(
        r#"<hs:sec {NS}><hp:p paraPrIDRef="0"><hp:run><hp:t>첫 문단</hp:t></hp:run></hp:p><hp:p paraPrIDRef="1"><hp:run>{tbl}<hp:t>표 뒤 글</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let doc = read_bytes(&hwpx_with(head, &sec)).unwrap();
    let breaks = doc.body.iter().filter_map(|b| b.as_para()).filter(|p| p.props.page_break_before == Some(true)).count();
    assert!(doc.body.iter().any(|b| b.as_table().is_some()), "the table is a block");
    assert_eq!(breaks, 1, "one page break");
    let after = doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.text.contains("표 뒤 글")).expect("the text after the table");
    assert_ne!(after.props.page_break_before, Some(true), "not before the text after the table");
}

/// A section asking for no page number (`pos="NONE"`) stops the number of the footer it
/// inherits; the footer's text stays (viewer, 2026-11-25). Before, the inherited number went on.
#[test]
fn no_page_number_stops_an_inherited_one() {
    let first = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr/><hp:ctrl><hp:footer applyPageType="BOTH"><hp:subList><hp:p><hp:run><hp:t>꼬리말 글</hp:t></hp:run></hp:p></hp:subList></hp:footer></hp:ctrl><hp:ctrl><hp:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar=""/></hp:ctrl><hp:t>첫 구역</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let second = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr/><hp:ctrl><hp:pageNum pos="NONE" formatType="DIGIT" sideChar=""/></hp:ctrl><hp:t>둘째 구역</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let bytes =
        package(&[("Contents/header.xml", EMPTY_HEAD), ("Contents/section0.xml", first.as_bytes()), ("Contents/section1.xml", second.as_bytes())]);
    let doc = read_bytes(&bytes).unwrap();
    let secs = sections(&doc);
    assert!(!part_page_fields(&doc, secs[0].footers.default).is_empty(), "the first section numbers its pages");
    let footer = secs[1].footers.default.or(secs[0].footers.default);
    assert!(part_text(&doc, footer).contains("꼬리말 글"), "the footer's text goes on");
    assert!(part_page_fields(&doc, footer).is_empty(), "without the number");
}

/// A page number is on even pages too when they have a footer of their own (viewer,
/// 2026-11-25). Before, only the default footer took the number.
#[test]
fn the_page_number_is_on_an_even_pages_own_footer_too() {
    let sec = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr/><hp:ctrl><hp:footer applyPageType="BOTH"><hp:subList><hp:p><hp:run><hp:t>양쪽</hp:t></hp:run></hp:p></hp:subList></hp:footer></hp:ctrl><hp:ctrl><hp:footer applyPageType="EVEN"><hp:subList><hp:p><hp:run><hp:t>짝수</hp:t></hp:run></hp:p></hp:subList></hp:footer></hp:ctrl><hp:ctrl><hp:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar=""/></hp:ctrl><hp:t>본문</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let doc = read_bytes(&hwpx(&sec)).unwrap();
    let footers = &sections(&doc)[0].footers;
    assert!(!part_page_fields(&doc, footers.default).is_empty(), "odd pages");
    assert!(part_text(&doc, footers.even).contains("짝수"), "the even pages' own footer");
    assert!(!part_page_fields(&doc, footers.even).is_empty(), "with the number");
}

/// The binding margin (gutter) narrows the line: two tables that fit side by side without it go
/// one under the other with it (viewer, 2026-11-25). Before, the line was measured without it.
#[test]
fn the_binding_margin_narrows_the_line() {
    let table = |t: &str| {
        format!(
            r#"<hp:tbl rowCnt="1" colCnt="1"><hp:sz width="9000" height="1000"/><hp:pos treatAsChar="1"/><hp:tr><hp:tc><hp:subList><hp:p><hp:run><hp:t>{t}</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:cellSz width="9000" height="1000"/></hp:tc></hp:tr></hp:tbl>"#
        )
    };
    // 20,000 wide between the margins: two tables of 9,000 fit, in 16,000 beside a 4,000 gutter not
    let read = |gutter: u32| {
        let sec = format!(
            r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr><hp:pagePr width="24000" height="34000"><hp:margin left="2000" right="2000" top="2000" bottom="2000" header="0" footer="0" gutter="{gutter}"/></hp:pagePr></hp:secPr><hp:t>표 둘</hp:t>{}{}</hp:run></hp:p></hs:sec>"#,
            table("왼쪽"),
            table("오른쪽")
        );
        let doc = read_bytes(&hwpx(&sec)).unwrap();
        doc.body.iter().filter(|b| b.as_table().is_some()).count()
    };
    assert_eq!(read(0), 0, "side by side on the line without a binding margin");
    assert_eq!(read(4000), 2, "one under the other with it");
}

/// A picture mirrored by its drawing matrix as well as its `hp:flip` is read mirrored; `hp:flip`
/// alone isn't, as the viewer doesn't mirror it (2026-11-25). Before, pictures were never read
/// mirrored.
#[test]
fn a_picture_mirrored_by_its_matrix_reads_mirrored() {
    let png = {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend(1u32.to_be_bytes());
        b.extend(1u32.to_be_bytes());
        b.extend([8, 6, 0, 0, 0]);
        b
    };
    let read = |sca: &str| {
        let pic = format!(
            r#"<hp:pic id="1" zOrder="1" textWrap="IN_FRONT_OF_TEXT"><hp:offset x="0" y="0"/><hp:orgSz width="4000" height="2000"/><hp:curSz width="4000" height="2000"/><hp:flip horizontal="1" vertical="0"/><hp:renderingInfo><hc:transMatrix e1="1" e2="0" e3="4000" e4="0" e5="1" e6="0"/><hc:scaMatrix e1="{sca}" e2="0" e3="0" e4="0" e5="1" e6="0"/><hc:rotMatrix e1="1" e2="0" e3="0" e4="0" e5="1" e6="0"/></hp:renderingInfo><hc:img binaryItemIDRef="image1"/><hp:sz width="4000" height="2000"/><hp:pos treatAsChar="0" vertRelTo="PARA" horzRelTo="COLUMN" vertAlign="TOP" horzAlign="LEFT" vertOffset="0" horzOffset="0"/></hp:pic>"#
        );
        let sec =
            format!(r#"<hs:sec {NS} xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core"><hp:p><hp:run>{pic}<hp:t>글</hp:t></hp:run></hp:p></hs:sec>"#);
        let doc =
            read_bytes(&package(&[("Contents/header.xml", EMPTY_HEAD), ("Contents/section0.xml", sec.as_bytes()), ("BinData/image1.png", &png)]))
                .unwrap();
        doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).find_map(|o| match o {
            InlineObject::Image { float, .. } => Some(float.flip_h),
            _ => None,
        })
    };
    assert_eq!(read("-1"), Some(true), "mirrored by the matrix");
    assert_eq!(read("1"), Some(false), "hp:flip alone");
}

#[test]
fn package_finds_sections() {
    let bytes = hwpx(&format!(r#"<hs:sec {NS}><hp:p/></hs:sec>"#));
    assert!(crate::sniff(&bytes));
    let pkg = crate::package::Package::open(&bytes).unwrap();
    assert_eq!(pkg.section_names, ["Contents/section0.xml"]);
    assert_eq!(pkg.header_name.as_deref(), Some("Contents/header.xml"));
}

#[test]
fn text_scanners_match_their_patterns() {
    assert_eq!(super::props::level_placeholders("^1.^2)^x"), "%1.%2)^x");
    assert_eq!(super::text::unescape_backslash(r"a\;b\\c"), r"a;b\c");
}

#[test]
fn hostile_inputs_are_errors_not_panics() {
    assert!(!crate::sniff(b""));
    assert!(crate::package::Package::open(b"PK\x03\x04garbage").is_err());
    assert!(crate::xml::parse(b"", false).is_err());
    let deep = "<a>".repeat(10_000);
    assert!(crate::xml::parse(deep.as_bytes(), false).is_err());
    assert!(crate::xml::parse(b"<a><b>unclosed", false).is_ok());
}

/// A bullet that is a symbol font's private-use glyph can't be drawn as it is. U+F0FC is drawn
/// as a check mark in Hancom's viewer, so it becomes one; other private-use bullets stay "•".
#[test]
fn private_use_bullets_become_what_hancom_draws() {
    let bullet = |ch: char| {
        let head = format!(
            r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:bullets><hh:bullet id="1" char="{ch}"/></hh:bullets><hh:paraProperties><hh:paraPr id="1"><hh:heading type="BULLET" idRef="1" level="0"/></hh:paraPr></hh:paraProperties></hh:refList></hh:head>"#
        );
        let sec = format!(r#"<hs:sec {NS}><hp:p paraPrIDRef="1"><hp:run><hp:t>항목</hp:t></hp:run></hp:p></hs:sec>"#);
        let doc = read_bytes(&hwpx_with(&head, &sec)).unwrap();
        doc.numbering.abstracts.iter().flat_map(|a| a.levels.first()).map(|l| l.text.clone()).next().unwrap_or_default()
    };
    assert_eq!(bullet('\u{F0FC}'), "✓");
    assert_eq!(bullet('\u{F06C}'), "•");
    assert_eq!(bullet('▶'), "▶");
}

/// A table on a text line goes where its paragraph aligns it, and floating objects (with only
/// spaces) before it in the same paragraph are placed from the table's line, as in Hancom's
/// viewer: the table takes the right alignment of its paragraph, and the piece of paragraph
/// holding the objects takes (almost) no height instead of a line above the table.
#[test]
fn table_on_a_text_line_follows_its_paragraph() {
    let head = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:paraProperties><hh:paraPr id="1"><hh:align horizontal="RIGHT"/></hh:paraPr></hh:paraProperties></hh:refList></hh:head>"#;
    let rect = r#"<hp:rect id="1" zOrder="1" textWrap="IN_FRONT_OF_TEXT"><hp:sz width="2000" height="8000"/><hp:pos treatAsChar="0" vertRelTo="PARA" horzRelTo="COLUMN" vertAlign="TOP" horzAlign="LEFT" vertOffset="1800" horzOffset="600"/><hp:curSz width="2000" height="8000"/><hp:orgSz width="2000" height="8000"/></hp:rect>"#;
    let tbl = r#"<hp:tbl rowCnt="1" colCnt="1"><hp:sz width="20000" height="2000"/><hp:pos treatAsChar="1" vertRelTo="PARA" horzRelTo="PARA"/><hp:tr><hp:tc><hp:cellAddr rowAddr="0" colAddr="0"/><hp:cellSz width="20000" height="2000"/><hp:subList><hp:p><hp:run><hp:t>칸</hp:t></hp:run></hp:p></hp:subList></hp:tc></hp:tr></hp:tbl>"#;
    let sec = format!(
        r#"<hs:sec {NS}><hp:p paraPrIDRef="1"><hp:run>{rect}<hp:t>  </hp:t>{tbl}</hp:run></hp:p><hp:p><hp:run><hp:t>뒤</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let doc = read_bytes(&hwpx_with(head, &sec)).unwrap();
    let at = doc.body.iter().position(|b| b.as_table().is_some()).expect("no table");
    let table = doc.body[at].as_table().unwrap();
    assert_eq!(table.props.align, Some(craft_wordmodel::props::Align::Right));
    let before = doc.body[..at].iter().rev().find_map(|b| b.as_para()).expect("no paragraph before the table");
    assert!(before.objects.iter().any(InlineObject::is_floating), "the floating object is before the table");
    assert_eq!(before.props.line_spacing, Some(craft_wordmodel::props::LineSpacing::Exactly(1.0)), "the objects' piece takes no height");
}

/// A section part starting with a comment that holds tags is still found by its root element,
/// and one whose first 4 KB don't tell (a long comment) still counts when the manifest lists it.
#[test]
fn sections_starting_with_comments_are_read() {
    let sec = |lead: &str| format!(r#"<?xml version="1.0"?>{lead}<hs:sec {NS}><hp:p><hp:run><hp:t>본문</hp:t></hp:run></hp:p></hs:sec>"#);
    let read = |section: String| {
        let hpf = br#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:manifest><opf:item id="header" href="Contents/header.xml" media-type="application/xml"/><opf:item id="s" href="Contents/body.xml" media-type="application/xml"/></opf:manifest><opf:spine><opf:itemref idref="header"/><opf:itemref idref="s"/></opf:spine></opf:package>"#;
        let bytes = package(&[("Contents/content.hpf", hpf), ("Contents/header.xml", EMPTY_HEAD), ("Contents/body.xml", section.as_bytes())]);
        body_text(&read_bytes(&bytes).unwrap())
    };
    assert!(read(sec("<!-- <hh:head> -->")).contains("본문"));
    assert!(read(sec(&format!("<!-- {} -->", "x".repeat(5000)))).contains("본문"));
}

/// Offsets are signed 32-bit values written unsigned: 4294966596 is -700 HWPUNIT, a little left
/// of the column, not 40 km to the right.
#[test]
fn drawing_offsets_are_signed() {
    let rect = r#"<hp:rect id="1" zOrder="1" textWrap="IN_FRONT_OF_TEXT"><hp:sz width="2000" height="2000"/><hp:pos treatAsChar="0" vertRelTo="PARA" horzRelTo="COLUMN" vertAlign="TOP" horzAlign="LEFT" vertOffset="0" horzOffset="4294966596"/><hp:curSz width="2000" height="2000"/><hp:orgSz width="2000" height="2000"/></hp:rect>"#;
    let doc = read_bytes(&hwpx(&format!(r#"<hs:sec {NS}><hp:p><hp:run>{rect}<hp:t>글</hp:t></hp:run></hp:p></hs:sec>"#))).unwrap();
    let x = doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).find_map(|o| match o {
        InlineObject::Shape { float, .. } => Some(float.x),
        _ => None,
    });
    assert!(x.is_some_and(|x| x < 0.0), "{x:?}");
}

/// Where Hancom's viewer draws an object before its anchor (2026-11-25): an offset written with a
/// minus sign is 0; held in the body (flowWithText), one above its paragraph is at the
/// paragraph's top, and one left of the column stays there.
#[test]
fn offsets_before_the_anchor_go_where_hancom_draws_them() {
    let at = |flow: &str, v: &str, h: &str| {
        let rect = format!(
            r#"<hp:rect id="1" zOrder="1" textWrap="IN_FRONT_OF_TEXT"><hp:sz width="2000" height="2000"/><hp:pos treatAsChar="0" flowWithText="{flow}" vertRelTo="PARA" horzRelTo="COLUMN" vertAlign="TOP" horzAlign="LEFT" vertOffset="{v}" horzOffset="{h}"/></hp:rect>"#
        );
        let doc = read_bytes(&hwpx(&format!(r#"<hs:sec {NS}><hp:p><hp:run>{rect}<hp:t>글</hp:t></hp:run></hp:p></hs:sec>"#))).unwrap();
        doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).find_map(|o| match o {
            InlineObject::Shape { float, .. } => Some((float.x, float.y)),
            _ => None,
        })
    };
    let before = "4294964461"; // -2835
    assert_eq!(at("0", "1000", "-2835").map(|p| p.0), Some(0.0), "a minus sign reads as 0");
    assert!(at("1", "1000", before).is_some_and(|p| p.0 < 0.0), "held, it still goes left of the column");
    assert_eq!(at("1", before, "0").map(|p| p.1), Some(0.0), "held, not above its paragraph");
    assert!(at("0", before, "0").is_some_and(|p| p.1 < 0.0), "not held, above its paragraph");
}

/// A turned object's frame (hp:sz) is the box around it (viewer, 2026-11-25): a 100 × 40 shape
/// turned 90° in a 40 × 100 frame keeps its own size, centred in the frame, and is turned there.
#[test]
fn a_turned_object_keeps_its_own_size_in_its_frame() {
    let rect = r#"<hp:rect id="1" zOrder="1" textWrap="IN_FRONT_OF_TEXT"><hp:sz width="4000" height="10000"/><hp:pos treatAsChar="0" vertRelTo="PARA" horzRelTo="COLUMN" vertAlign="TOP" horzAlign="LEFT" vertOffset="0" horzOffset="1000"/><hp:rotationInfo angle="90"/><hp:curSz width="10000" height="4000"/><hp:orgSz width="10000" height="4000"/></hp:rect>"#;
    let doc = read_bytes(&hwpx(&format!(r#"<hs:sec {NS}><hp:p><hp:run>{rect}<hp:t>글</hp:t></hp:run></hp:p></hs:sec>"#))).unwrap();
    let (w, h, f) = doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .find_map(|o| match o {
            InlineObject::Shape { w, h, float, .. } => Some((*w, *h, float.clone())),
            _ => None,
        })
        .expect("no shape");
    assert!(w > h, "its own size, wide: {w} × {h}");
    assert!(f.rot != 0.0, "turned");
    let (frame_x, frame_y) = (10.0 + 20.0, 50.0); // the frame's centre, points
    assert!((f.x + w / 2.0 - frame_x).abs() < 0.5 && (f.y + h / 2.0 - frame_y).abs() < 0.5, "centred in the frame: {:?}", (f.x, f.y));
}

/// A field command's backslashes escape only separators and themselves: a Windows path written
/// as it is keeps its backslashes, and one written escaped reads the same.
#[test]
fn backslashes_in_paths_are_kept() {
    use super::text::unescape_backslash;
    assert_eq!(unescape_backslash(r"E:\Work\other.hwp"), r"E:\Work\other.hwp");
    assert_eq!(unescape_backslash(r"E:\\Work\\other.hwp"), r"E:\Work\other.hwp");
    assert_eq!(unescape_backslash(r"a\;b\/c"), "a;b/c");
}

/// A picture of a kind not known by its name is named by its bytes: WebP data is kept as .webp,
/// not called PNG.
#[test]
fn pictures_are_named_by_what_they_hold() {
    let webp = [b"RIFF".as_slice(), &[0, 0, 0, 0], b"WEBPVP8 "].concat();
    assert_eq!(super::media::image_ext("bin", &webp), "webp");
    assert_eq!(super::media::image_ext("", b"\x89PNG\r\n"), "png");
    assert_eq!(super::media::image_ext("xyz", b"???"), "png");
    assert_eq!(super::media::image_ext("JPG", b"???"), "jpg");
}

/// A header of 100,000 styles: a style is found by its id directly, and a later definition of an
/// id replaces the first in place. Large on purpose (about 1 s in a debug build): a lookup that
/// went through the styles before it would take minutes here.
#[test]
fn many_styles_are_found_by_id() {
    let n = 100_000;
    let mut head = String::from(r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:styles>"#);
    for k in 0..n {
        head.push_str(&format!(r#"<hh:style id="{k}" type="PARA" name="s{k}" nextStyleIDRef="{}"/>"#, (k + 1) % n));
    }
    head.push_str(r#"<hh:style id="7" type="PARA" name="again"/></hh:styles></hh:refList></hh:head>"#);
    let root = crate::xml::parse(head.as_bytes(), false).unwrap();
    let h = crate::header::Header::new(Some(&root));
    assert_eq!(h.styles.len(), n);
    assert!(h.styles.iter().all(|s| h.style(s.next).is_some_and(|t| t.id == s.next)));
    assert_eq!(h.style(7).map(|s| s.name.as_str()), Some("again"));
    assert!(h.style(-1).is_none());
}

/// A manifest of 100,000 items (all naming one picture), all in the spine: its section is found,
/// and the picture by its last id. Large on purpose (about 2 s in a debug build): a lookup that went
/// through the whole manifest for each entry would take minutes here.
#[test]
fn hwpx_many_manifest_items_open() {
    let n = 100_000;
    let mut items = String::from(
        r#"<opf:item id="header" href="Contents/header.xml" media-type="application/xml"/><opf:item id="s" href="Contents/section0.xml" media-type="application/xml"/>"#,
    );
    let mut spine = String::from(r#"<opf:itemref idref="header"/><opf:itemref idref="s"/>"#);
    for k in 0..n {
        items.push_str(&format!(r#"<opf:item id="img{k}" href="BinData/img.png" media-type="image/png"/>"#));
        spine.push_str(&format!(r#"<opf:itemref idref="img{k}"/>"#));
    }
    let hpf = format!(
        r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:manifest>{items}</opf:manifest><opf:spine>{spine}</opf:spine></opf:package>"#
    );
    let sec = format!(r#"<hs:sec {NS}><hp:p><hp:run><hp:t>본문</hp:t></hp:run></hp:p></hs:sec>"#);
    let bytes = package(&[
        ("Contents/content.hpf", hpf.as_bytes()),
        ("Contents/header.xml", EMPTY_HEAD),
        ("Contents/section0.xml", sec.as_bytes()),
        ("BinData/img.png", b"picture"),
    ]);
    let pkg = crate::package::Package::open(&bytes).unwrap();
    assert_eq!(pkg.section_names, ["Contents/section0.xml"]);
    assert_eq!(pkg.binary(&format!("img{}", n - 1)).map(|b| b.1), Some(&b"picture"[..]));
}

/// Pictures named by file stem only (no manifest item) are found by the stem directly: 30,000
/// of them, among as many entries. Large on purpose (several seconds in a debug build, most of it
/// building and opening the package): a search through every entry for each would take minutes.
#[test]
fn hwpx_pictures_named_by_stem_are_found_directly() {
    let n = 30_000;
    let sec = format!(r#"<hs:sec {NS}><hp:p><hp:run><hp:t>본문</hp:t></hp:run></hp:p></hs:sec>"#);
    let pictures: Vec<(String, Vec<u8>)> = (0..n).map(|k| (format!("BinData/p{k}.png"), format!("{k}").into_bytes())).collect();
    let mut parts: Vec<(&str, &[u8])> = vec![("Contents/header.xml", EMPTY_HEAD), ("Contents/section0.xml", sec.as_bytes())];
    parts.extend(pictures.iter().map(|(n, b)| (n.as_str(), b.as_slice())));
    let pkg = crate::package::Package::open(&package(&parts)).unwrap();
    for k in 0..n {
        assert_eq!(pkg.binary(&format!("p{k}")).map(|b| b.1.to_vec()), Some(format!("{k}").into_bytes()));
    }
    assert!(pkg.binary("missing").is_none());
}

/// A section asking for a page number at the bottom while it inherits a text-only footer gets a
/// footer of its own: the inherited text and the page number.
#[test]
fn an_inherited_footer_without_a_number_takes_one() {
    let first = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr/><hp:ctrl><hp:footer applyPageType="BOTH"><hp:subList><hp:p><hp:run><hp:t>꼬리말 글</hp:t></hp:run></hp:p></hp:subList></hp:footer></hp:ctrl><hp:t>첫 구역</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let second = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr/><hp:ctrl><hp:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar=""/></hp:ctrl><hp:t>둘째 구역</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let bytes =
        package(&[("Contents/header.xml", EMPTY_HEAD), ("Contents/section0.xml", first.as_bytes()), ("Contents/section1.xml", second.as_bytes())]);
    let doc = read_bytes(&bytes).unwrap();
    let footer = sections(&doc)[1].footers.default;
    assert!(part_text(&doc, footer).contains("꼬리말 글"), "the second section's footer has the text");
    assert!(!part_page_fields(&doc, footer).is_empty(), "and the number");
}

/// A section that continues an earlier section's numbered footer and hides the page number on
/// its first page shows that footer's text there without the number.
#[test]
fn a_hidden_first_page_number_is_taken_from_an_inherited_footer() {
    let first = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr/><hp:ctrl><hp:footer applyPageType="BOTH"><hp:subList><hp:p><hp:run><hp:t>꼬리말 글</hp:t></hp:run></hp:p></hp:subList></hp:footer></hp:ctrl><hp:ctrl><hp:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar=""/></hp:ctrl><hp:t>첫 구역</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let second = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr/><hp:ctrl><hp:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar=""/></hp:ctrl><hp:ctrl><hp:pageHiding hideHeader="0" hideFooter="0" hidePageNum="1"/></hp:ctrl><hp:t>둘째 구역</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let bytes =
        package(&[("Contents/header.xml", EMPTY_HEAD), ("Contents/section0.xml", first.as_bytes()), ("Contents/section1.xml", second.as_bytes())]);
    let doc = read_bytes(&bytes).unwrap();
    let footers = &sections(&doc)[1].footers;
    assert!(part_text(&doc, footers.first).contains("꼬리말 글"), "the first page's footer has the text");
    assert!(part_page_fields(&doc, footers.first).is_empty(), "without the number");
    // (no default footer of its own: it inherits the first section's)
    let default = footers.default.or(sections(&doc)[0].footers.default);
    assert!(!part_page_fields(&doc, default).is_empty(), "the other pages keep it");
}

/// Table cells a file declares but doesn't hold are made up to a budget for the whole document;
/// past it, declared rows without cells are left out. A table can declare 10,000 rows of 64
/// columns with one cell; that alone would make 640,000 cells. The budget is spent for real here
/// (under 1 s in a debug build).
#[test]
fn declared_but_missing_table_cells_are_bounded() {
    let tbl = |col: u32, text: &str| {
        format!(
            r#"<hp:tbl rowCnt="10000" colCnt="64"><hp:sz width="20000" height="2000"/><hp:pos treatAsChar="1"/><hp:tr><hp:tc><hp:cellAddr rowAddr="0" colAddr="{col}"/><hp:cellSz width="300" height="300"/><hp:subList><hp:p><hp:run><hp:t>{text}</hp:t></hp:run></hp:p></hp:subList></hp:tc></hp:tr></hp:tbl>"#
        )
    };
    // the first table spends the budget; the second holds its one cell after a gap (column 1)
    let sec =
        format!(r#"<hs:sec {NS}><hp:p><hp:run>{}</hp:run></hp:p><hp:p><hp:run>{}</hp:run></hp:p></hs:sec>"#, tbl(0, "첫째 칸"), tbl(1, "둘째 칸"));
    let doc = read_bytes(&package(&[("Contents/section0.xml", sec.as_bytes())])).unwrap();
    let tables: Vec<_> = doc.body.iter().filter_map(|b| b.as_table()).collect();
    let cells: i64 = tables.iter().map(|t| t.rows.iter().map(|r| r.cells.len() as i64).sum::<i64>()).sum();
    assert!(cells <= super::section::MAX_MADE_CELLS + 4, "{cells} cells");
    // every cell the file holds is kept, the one after a gap once the budget is spent too
    let text: String = tables
        .iter()
        .flat_map(|t| t.rows.iter().flat_map(|r| r.cells.iter().flat_map(|c| c.blocks.iter().filter_map(|b| b.as_para().map(|p| p.text.clone())))))
        .collect();
    assert!(text.contains("첫째 칸") && text.contains("둘째 칸"), "the cells the file holds are kept");
}

/// Page margins at the edge of the number range read without overflowing their sums (top and
/// header, bottom and footer), across and down the page.
#[test]
fn huge_page_margins_do_not_overflow() {
    for dir in ["HORIZONTAL", "VERTICAL"] {
        let big = i64::MAX;
        let sec = format!(
            r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr textDirection="{dir}"><hp:pagePr width="59528" height="84188"><hp:margin left="{big}" right="{big}" top="{big}" bottom="{big}" header="{big}" footer="{big}" gutter="{big}"/></hp:pagePr></hp:secPr><hp:t>가</hp:t></hp:run></hp:p></hs:sec>"#
        );
        let s = read_bytes(&hwpx(&sec)).unwrap().last_section;
        let all = [s.margin_top, s.margin_bottom, s.margin_left, s.margin_right, s.header, s.footer, s.gutter];
        assert!(all.iter().all(|m| m.is_finite() && *m >= 0.0), "{dir}: no wrapped margin: {all:?}");
    }
}

/// A page background tiled with a picture too small to lay tile by tile (1 x 1 pixel over an A4
/// page) is one picture covering the page.
#[test]
fn a_tiny_tiled_page_background_covers_the_page() {
    let header = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core"><hh:refList><hh:borderFills itemCnt="1"><hh:borderFill id="1"><hc:fillBrush><hc:imgBrush mode="TILE"><hc:img binaryItemIDRef="image1"/></hc:imgBrush></hc:fillBrush></hh:borderFill></hh:borderFills></hh:refList></hh:head>"#;
    let sec = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr><hp:pagePr width="59528" height="84188"/><hp:pageBorderFill type="BOTH" borderFillIDRef="1" fillArea="PAPER"/></hp:secPr><hp:t>가</hp:t></hp:run></hp:p></hs:sec>"#
    );
    // a PNG header: signature, then IHDR with a width and height of 1
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend(1u32.to_be_bytes());
    png.extend(1u32.to_be_bytes());
    png.extend([8, 6, 0, 0, 0]);
    let doc =
        read_bytes(&package(&[("Contents/header.xml", header.as_bytes()), ("Contents/section0.xml", sec.as_bytes()), ("BinData/image1.png", &png)]))
            .unwrap();
    let pictures: Vec<(f32, f32)> = header_floats(&doc)
        .iter()
        .filter_map(|o| match o {
            InlineObject::Image { w, h, .. } => Some((*w, *h)),
            _ => None,
        })
        .collect();
    assert_eq!(pictures.len(), 1, "one picture");
    let s = &doc.last_section;
    assert!(pictures.iter().all(|(w, h)| *w >= s.page_w - 1.0 && *h >= s.page_h - 1.0), "it covers the page: {pictures:?}");
}

/// Two tables set like characters in a section's first paragraph, with text: on a page too narrow
/// for both side by side they go one under the other. The section's page size, in that same
/// paragraph, is read before the tables are measured.
#[test]
fn inline_tables_are_measured_against_their_sections_page() {
    let table = |t: &str| {
        format!(
            r#"<hp:tbl rowCnt="1" colCnt="1"><hp:sz width="15000" height="1000"/><hp:pos treatAsChar="1"/><hp:tr><hp:tc><hp:subList><hp:p><hp:run><hp:t>{t}</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:cellSz width="15000" height="1000"/></hp:tc></hp:tr></hp:tbl>"#
        )
    };
    let sec = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr><hp:pagePr width="24000" height="34000"><hp:margin left="2000" right="2000" top="2000" bottom="2000" header="0" footer="0" gutter="0"/></hp:pagePr></hp:secPr><hp:t>표 둘</hp:t>{}{}</hp:run></hp:p></hs:sec>"#,
        table("왼쪽"),
        table("오른쪽")
    );
    let doc = read_bytes(&hwpx(&sec)).unwrap();
    assert_eq!(doc.body.iter().filter(|b| b.as_table().is_some()).count(), 2, "two tables of the body");
    assert!(!doc.parts.values().any(|p| p.kind == PartKind::TextBox), "not boxes on the line");
}

/// A paragraph that starts with the document's own bookmark still remembers its source.
#[test]
fn paragraphs_starting_with_a_bookmark_remember_their_source() {
    let doc = read_bytes(&hwpx(&format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:ctrl><hp:bookmark name="_heading"/></hp:ctrl><hp:t>책갈피 문단</hp:t></hp:run></hp:p></hs:sec>"#
    )))
    .unwrap();
    let kept = doc.kept.of(crate::KEPT_FORMAT).expect("nothing kept");
    let p = doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.text.contains("책갈피")).expect("no paragraph");
    assert!(kept.para(p).is_some());
    assert!(p.objects.iter().any(|o| matches!(o, InlineObject::BookmarkStart { name } if name == "_heading")), "the bookmark is kept");
}

/// The FILETIME a memo keeps (2013-06-26 09:14:43 UTC) reads as ISO 8601.
#[test]
fn memo_times_read_as_iso() {
    assert_eq!(super::text::iso_from_filetime((30_306_893u64 << 32) | 2_552_988_672).as_deref(), Some("2013-06-26T09:14:43Z"));
    assert_eq!(super::text::iso_from_filetime(0), None);
    // the Unix epoch itself is a time, not none
    assert_eq!(super::text::iso_from_filetime(11_644_473_600 * 10_000_000).as_deref(), Some("1970-01-01T00:00:00Z"));
}

/// A level line has no height: its shown width stays (the original size, 100 x 100, is only
/// the space its end points are given in), as level lines in Hancom's files have it.
#[test]
fn level_lines_keep_their_width() {
    let el = |tag: &str| {
        let xml = format!(r#"<hp:{tag} {NS}><hp:curSz width="22189" height="0"/><hp:orgSz width="100" height="100"/></hp:{tag}>"#);
        crate::xml::parse(xml.as_bytes(), false).unwrap()
    };
    assert_eq!(super::drawing::size_of(&el("line")), (22189, 0));
    assert_eq!(super::drawing::size_of(&el("rect")), (100, 100));
}

#[test]
fn spaces_split_off_their_text() {
    use super::text::split_spaces;
    assert_eq!(split_spaces("  가나 다라  끝"), ["  ", "가나", " ", "다라", "  ", "끝"]);
    assert_eq!(split_spaces("가나"), ["가나"]);
    assert!(split_spaces("").is_empty());
}

/// A footnote anchored inside a hyperlink is a story of its own: its text doesn't take the
/// link, and the text after the note in the body still does.
#[test]
fn a_note_inside_a_hyperlink_keeps_its_own_text_unlinked() {
    let sec = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:ctrl><hp:fieldBegin id="7" type="HYPERLINK"><hp:parameters><hp:stringParam name="Command">https://example.com;1;0;0;</hp:stringParam></hp:parameters></hp:fieldBegin></hp:ctrl><hp:t>앞</hp:t><hp:ctrl><hp:footNote><hp:subList><hp:p><hp:run><hp:t>각주 글</hp:t></hp:run></hp:p></hp:subList></hp:footNote></hp:ctrl><hp:t>뒤</hp:t><hp:ctrl><hp:fieldEnd beginIDRef="7"/></hp:ctrl></hp:run></hp:p></hs:sec>"#
    );
    let doc = read_bytes(&hwpx(&sec)).unwrap();
    let linked = |p: &craft_wordmodel::Paragraph, s: &str| {
        let at = p.text.find(s).expect("text missing");
        let mut pos = 0;
        p.runs
            .iter()
            .find(|r| {
                let hit = pos <= at && at < pos + r.len;
                pos += r.len;
                hit
            })
            .is_some_and(|r| r.props.link.is_some())
    };
    let body = doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.text.contains('뒤')).expect("no body paragraph");
    assert!(linked(body, "앞") && linked(body, "뒤"), "the body text keeps its link around the note");
    let note = doc
        .parts
        .values()
        .filter(|p| p.kind == PartKind::Footnote)
        .flat_map(|p| p.blocks.iter().filter_map(|b| b.as_para().cloned()).collect::<Vec<_>>())
        .find(|p| p.text.contains("각주"))
        .expect("no note paragraph");
    assert!(!linked(&note, "각주"), "the note's text isn't linked");
}

/// What the model can't hold yet is drawn as near as it can be and logged, never dropped
/// silently: an arc (drawn as its frame), a cell's diagonal lines, and a page background whose
/// picture the package doesn't hold.
#[test]
fn unread_arcs_diagonals_and_background_pictures_are_logged() {
    let header = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core"><hh:refList><hh:borderFills><hh:borderFill id="3"><hh:slash type="CENTER"/></hh:borderFill><hh:borderFill id="4"><hc:fillBrush><hc:imgBrush mode="TOTAL"><hc:img binaryItemIDRef="nowhere"/></hc:imgBrush></hc:fillBrush></hh:borderFill></hh:borderFills></hh:refList></hh:head>"#;
    let section = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr><hp:pageBorderFill type="BOTH" borderFillIDRef="4" textBorder="PAPER" fillArea="PAPER"/></hp:secPr><hp:arc><hp:sz width="4000" height="4000"/><hp:pos treatAsChar="1"/></hp:arc><hp:t>본문</hp:t></hp:run></hp:p><hp:p><hp:run><hp:tbl rowCnt="1" colCnt="1"><hp:tr><hp:tc borderFillIDRef="3"><hp:subList><hp:p><hp:run><hp:t>칸</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSz width="5000" height="1000"/></hp:tc></hp:tr></hp:tbl></hp:run></hp:p></hs:sec>"#
    );
    let (doc, warnings) = crate::test_log::warnings(|| read_bytes(&hwpx_with(header, &section)));
    assert!(doc.is_ok());
    for what in ["arcs", "cell diagonal lines", "page background picture"] {
        assert!(warnings.iter().any(|w| w.contains(what)), "{what} not logged: {warnings:?}");
    }
}

/// One small picture tiled over the page of many sections: the tiles laid one by one stop at
/// the document's budget, the backgrounds past it are one stretched picture each, and that is
/// logged. Before, every section laid its own thousand tiles.
#[test]
fn tiled_backgrounds_stop_at_the_documents_budget() {
    let header = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core"><hh:refList><hh:borderFills itemCnt="1"><hh:borderFill id="1"><hc:fillBrush><hc:imgBrush mode="TILE"><hc:img binaryItemIDRef="image1"/></hc:imgBrush></hc:fillBrush></hh:borderFill></hh:borderFills></hh:refList></hh:head>"#;
    let sec = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr><hp:pagePr width="59528" height="84188"/><hp:pageBorderFill type="BOTH" borderFillIDRef="1" fillArea="PAPER"/></hp:secPr><hp:t>가</hp:t></hp:run></hp:p></hs:sec>"#
    );
    // a PNG header of 30 x 30 pixels: about a thousand tiles on an A4 page
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend(30u32.to_be_bytes());
    png.extend(30u32.to_be_bytes());
    png.extend([8, 6, 0, 0, 0]);
    let read = |count: usize| {
        let names: Vec<String> = (0..count).map(|i| format!("Contents/section{i}.xml")).collect();
        let mut parts: Vec<(&str, &[u8])> = vec![("Contents/header.xml", header.as_bytes()), ("BinData/image1.png", &png)];
        parts.extend(names.iter().map(|n| (n.as_str(), sec.as_bytes())));
        let (doc, warnings) = crate::test_log::warnings(|| read_bytes(&package(&parts)));
        let doc = doc.unwrap();
        let pictures = |id: Option<u32>| {
            id.and_then(|id| doc.parts.get(&id)).map_or(0, |p| {
                p.blocks.iter().filter_map(|b| b.as_para()).flat_map(|q| q.objects.iter()).filter(|o| matches!(o, InlineObject::Image { .. })).count()
            })
        };
        let last = pictures(sections(&doc).last().and_then(|s| s.headers.default));
        (header_floats(&doc).iter().filter(|o| matches!(o, InlineObject::Image { .. })).count(), warnings, last)
    };
    let (one, _, _) = read(1);
    assert!(one > 1, "one section's page is tiled: {one}");
    let (many, warnings, last) = read(160);
    assert_eq!(last, 1, "a late section keeps its background, one stretched picture");
    assert!(many > one, "the first sections are still tiled: {many}");
    assert!(many < 160 * one / 2, "far fewer than every section's tiles: {many} of {}", 160 * one);
    assert!(warnings.iter().any(|w| w.contains("page background tiles")), "{warnings:?}");
}

/// A manifest href with a `.` segment (`BinData/./photo.png`) names the package entry it
/// points at; before, the item was dropped and a picture referring to its id had no image.
#[test]
fn manifest_hrefs_are_normalised() {
    let hpf = r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:manifest><opf:item id="pic" href="BinData/./photo.png" media-type="image/png"/><opf:item id="s" href="Contents/section0.xml" media-type="application/xml"/></opf:manifest><opf:spine><opf:itemref idref="s"/></opf:spine></opf:package>"#;
    let sec = format!(r#"<hs:sec {NS}><hp:p><hp:run><hp:t>본문</hp:t></hp:run></hp:p></hs:sec>"#);
    let bytes = package(&[
        ("Contents/content.hpf", hpf.as_bytes()),
        ("Contents/header.xml", EMPTY_HEAD),
        ("Contents/section0.xml", sec.as_bytes()),
        ("BinData/photo.png", b"picture"),
    ]);
    let pkg = crate::package::Package::open(&bytes).unwrap();
    assert_eq!(pkg.binary("pic").map(|b| b.1), Some(&b"picture"[..]));
    // relative to a manifest in a folder of its own, `..` steps out of that folder
    let container = br#"<ocf:container xmlns:ocf="urn:oasis:names:tc:opendocument:xmlns:container"><ocf:rootfiles><ocf:rootfile full-path="Docs/Sub/content.hpf"/></ocf:rootfiles></ocf:container>"#;
    let hpf = r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:manifest><opf:item id="pic" href="../Pics/photo.png" media-type="image/png"/><opf:item id="s" href="../../Contents/section0.xml" media-type="application/xml"/></opf:manifest><opf:spine><opf:itemref idref="s"/></opf:spine></opf:package>"#;
    let bytes = package(&[
        ("META-INF/container.xml", container),
        ("Docs/Sub/content.hpf", hpf.as_bytes()),
        ("Contents/header.xml", EMPTY_HEAD),
        ("Contents/section0.xml", sec.as_bytes()),
        ("Docs/Pics/photo.png", b"picture"),
    ]);
    let pkg = crate::package::Package::open(&bytes).unwrap();
    assert_eq!(pkg.binary("pic").map(|b| b.1), Some(&b"picture"[..]), "Docs/Pics/photo.png");
}

/// A section part whose root element's name is cut off where the reader stops looking (after
/// a long run of spaces) still counts as a section the manifest lists; before, it read as
/// another kind of part and the section was lost.
#[test]
fn a_section_root_cut_off_by_the_peek_still_counts() {
    let hpf = r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:manifest><opf:item id="s0" href="Contents/section0.xml" media-type="application/xml"/><opf:item id="s1" href="Contents/section1.xml" media-type="application/xml"/></opf:manifest><opf:spine><opf:itemref idref="s0"/><opf:itemref idref="s1"/></opf:spine></opf:package>"#;
    let body = |t: &str| format!(r#"<hs:sec {NS}><hp:p><hp:run><hp:t>{t}</hp:t></hp:run></hp:p></hs:sec>"#);
    let first = format!("{}{}", " ".repeat(4096 - "<hs:s".len()), body("첫째"));
    let second = body("둘째");
    let bytes = package(&[
        ("Contents/content.hpf", hpf.as_bytes()),
        ("Contents/header.xml", EMPTY_HEAD),
        ("Contents/section0.xml", first.as_bytes()),
        ("Contents/section1.xml", second.as_bytes()),
    ]);
    let pkg = crate::package::Package::open(&bytes).unwrap();
    assert_eq!(pkg.section_names, ["Contents/section0.xml", "Contents/section1.xml"]);
}

/// A memo field named by both its id and its field id, ended by both: the memo ends once.
/// Before, each alias ended it, and the comment had two ends.
#[test]
fn a_memo_ended_by_both_its_names_ends_once() {
    let section = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:ctrl><hp:fieldBegin id="5" fieldid="9" type="MEMO"><hp:parameters><hp:stringParam name="Command">MEMO/0/0/0/0/저자</hp:stringParam></hp:parameters><hp:subList><hp:p><hp:run><hp:t>메모</hp:t></hp:run></hp:p></hp:subList></hp:fieldBegin></hp:ctrl><hp:t>본문</hp:t><hp:ctrl><hp:fieldEnd beginIDRef="5" fieldid="9"/></hp:ctrl></hp:run></hp:p></hs:sec>"#
    );
    let doc = read_bytes(&hwpx(&section)).unwrap();
    let objects: Vec<&InlineObject> = doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).collect();
    let starts = objects.iter().filter(|o| matches!(o, InlineObject::CommentStart { .. })).count();
    let ends = objects.iter().filter(|o| matches!(o, InlineObject::CommentEnd { .. })).count();
    assert_eq!((starts, ends), (1, 1));
}

/// A hyperlink command whose target ends in an escaped backslash (`\\`) before the `;` that
/// ends it: the target stops there. Before, the pair read as escaping the `;`, and the field's
/// other settings went into the link.
#[test]
fn a_link_target_ends_at_a_semicolon_after_an_escaped_backslash() {
    let link = |cmd: &str| {
        let xml = format!(
            r#"<hp:fieldBegin {NS} type="HYPERLINK"><hp:parameters><hp:stringParam name="Command">{cmd}</hp:stringParam></hp:parameters></hp:fieldBegin>"#
        );
        super::text::hyperlink_url(&crate::xml::parse(xml.as_bytes(), false).unwrap())
    };
    let url = link(r"https://example.com/a\\;1;0;0;").expect("a link");
    assert!(!url.contains(';'), "{url}");
    // one backslash still escapes the semicolon
    assert!(link(r"https://example.com/a\;b;1;0;0;").expect("a link").contains("a;b"));
}

/// A JPEG whose frame marker follows fill bytes (0xFF 0xFF 0xC0) still tells its pixel size;
/// before, the second 0xFF read as the marker and the size was lost (a tiled background then
/// became one stretched picture).
#[test]
fn jpeg_sizes_are_read_past_fill_bytes() {
    let frame = |fill: usize| {
        let mut b = vec![0xFF, 0xD8];
        b.extend(std::iter::repeat_n(0xFF, fill + 1));
        // SOF0: length 17, precision 8, height 32, width 48
        b.extend([0xC0, 0x00, 0x11, 0x08, 0x00, 0x20, 0x00, 0x30]);
        b.extend([0u8; 16]);
        b
    };
    assert_eq!(super::media::pixel_size(&frame(0)), Some((48, 32)));
    assert_eq!(super::media::pixel_size(&frame(2)), Some((48, 32)));
}

/// A picture stored without a file extension is kept as what its bytes are (a JPEG as .jpg);
/// before, it was always called a PNG.
#[test]
fn pictures_without_an_extension_are_named_by_their_bytes() {
    let hpf = r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:manifest><opf:item id="pic" href="BinData/photo" media-type="image/jpeg"/><opf:item id="s" href="Contents/section0.xml" media-type="application/xml"/></opf:manifest><opf:spine><opf:itemref idref="s"/></opf:spine></opf:package>"#;
    let sec = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:pic><hp:sz width="7200" height="7200"/><hp:pos treatAsChar="1"/><hc:img xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core" binaryItemIDRef="pic"/></hp:pic></hp:run></hp:p></hs:sec>"#
    );
    let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0];
    let doc = read_bytes(&package(&[
        ("Contents/content.hpf", hpf.as_bytes()),
        ("Contents/header.xml", EMPTY_HEAD),
        ("Contents/section0.xml", sec.as_bytes()),
        ("BinData/photo", &jpeg),
    ]))
    .unwrap();
    let keys: Vec<&String> = doc.media.keys().collect();
    assert!(keys.len() == 1 && keys[0].ends_with(".jpg"), "{keys:?}");
}

/// With a bold, italic default character shape, a run whose own shape is neither reads as
/// neither, and one whose shape is reads as both. Before, the plain run took the default's
/// bold and italic.
#[test]
fn plain_runs_stay_plain_under_a_bold_default() {
    let header = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:charProperties><hh:charPr id="0" height="1000"><hh:bold/><hh:italic/></hh:charPr><hh:charPr id="1" height="1000"/></hh:charProperties><hh:styles><hh:style id="0" type="PARA" name="바탕글" paraPrIDRef="0" charPrIDRef="0"/></hh:styles></hh:refList></hh:head>"#;
    let section = format!(
        r#"<hs:sec {NS}><hp:p styleIDRef="0"><hp:run charPrIDRef="1"><hp:t>보통</hp:t></hp:run><hp:run charPrIDRef="0"><hp:t>굵게</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let doc = read_bytes(&hwpx_with(header, &section)).unwrap();
    let p = doc.body.iter().find_map(|b| b.as_para()).unwrap();
    let style = p.props.style.as_deref();
    let shown: Vec<(bool, bool)> = p
        .runs
        .iter()
        .filter(|r| r.len > 0)
        .map(|r| {
            let rc = doc.styles.resolve_char(style, &r.props);
            (rc.bold, rc.italic)
        })
        .collect();
    assert_eq!(shown.first(), Some(&(false, false)), "{shown:?}");
    assert_eq!(shown.last(), Some(&(true, true)), "{shown:?}");
}

/// A table in the text flow placed left of its column (a negative offset, which HWPX writes as
/// its unsigned 32-bit form) is indented the same way a floating table is placed: to the left.
#[test]
fn an_in_flow_table_keeps_a_negative_offset() {
    let sec = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:tbl rowCnt="1" colCnt="1"><hp:sz width="10000" height="1000"/><hp:pos treatAsChar="0" vertRelTo="PARA" horzRelTo="COLUMN" horzAlign="LEFT" horzOffset="4294966596"/><hp:tr><hp:tc><hp:subList><hp:p><hp:run><hp:t>칸</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:cellSz width="10000" height="1000"/></hp:tc></hp:tr></hp:tbl></hp:run></hp:p></hs:sec>"#
    );
    let doc = read_bytes(&hwpx(&sec)).unwrap();
    let t = doc.body.iter().find_map(|b| b.as_table()).expect("no table");
    assert!(t.props.indent.is_some_and(|i| i < 0.0), "indented to the left: {:?}", t.props.indent);
}

/// A section asking for more columns than a section's properties hold breaks its columns by the
/// count it is written with: the column breaks of a page are one fewer than its columns.
#[test]
fn column_breaks_follow_the_written_column_count() {
    // every paragraph's cached lines start again at the top: a new column, or a new page
    let para = |k: usize| {
        format!(
            r#"<hp:p><hp:run><hp:t>단 {k}</hp:t></hp:run><hp:linesegarray><hp:lineseg vertpos="0"/><hp:lineseg vertpos="1000"/></hp:linesegarray></hp:p>"#
        )
    };
    let first = r#"<hp:p><hp:run><hp:secPr/><hp:ctrl><hp:colPr type="NEWSPAPER" colCount="200" sameGap="0"/></hp:ctrl><hp:t>단 0</hp:t></hp:run><hp:linesegarray><hp:lineseg vertpos="0"/><hp:lineseg vertpos="1000"/></hp:linesegarray></hp:p>"#;
    let body: String = (1..120).map(para).collect();
    let doc = read_bytes(&hwpx(&format!(r#"<hs:sec {NS}>{first}{body}</hs:sec>"#))).unwrap();
    let paras: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).collect();
    let breaks = |p: &&craft_wordmodel::Paragraph| p.text.starts_with(craft_wordmodel::para::COLUMN_BREAK);
    // the breaks before the first paragraph that starts a page without one
    let run = paras.iter().skip(1).take_while(|p| breaks(p)).count();
    let count = paras.iter().find_map(|p| p.section.as_ref()).map(|s| s.columns.count).unwrap_or(doc.last_section.columns.count);
    // 200 asked for, 45 the most a section's properties hold
    assert_eq!(count, 45, "the columns a section's properties hold");
    assert_eq!(run + 1, 45, "a page's column breaks are one fewer than its columns");
}

/// An explicit page break starts its page in the first column: in two columns, the next page's
/// second column is still a column, not a page. Before, the break counted as one more column
/// restart, and columns and pages after it alternated the wrong way.
#[test]
fn a_page_break_starts_its_page_in_the_first_column() {
    // cached lines start again at the top in every paragraph; the fourth asks for a page break
    let para = |k: usize| {
        let pb = if k == 3 { r#" pageBreak="1""# } else { "" };
        format!(
            r#"<hp:p{pb}><hp:run><hp:t>단 {k}</hp:t></hp:run><hp:linesegarray><hp:lineseg vertpos="0"/><hp:lineseg vertpos="1000"/></hp:linesegarray></hp:p>"#
        )
    };
    let first = r#"<hp:p><hp:run><hp:secPr/><hp:ctrl><hp:colPr type="NEWSPAPER" colCount="2" sameGap="0"/></hp:ctrl><hp:t>단 0</hp:t></hp:run><hp:linesegarray><hp:lineseg vertpos="0"/><hp:lineseg vertpos="1000"/></hp:linesegarray></hp:p>"#;
    let body: String = (1..6).map(para).collect();
    let doc = read_bytes(&hwpx(&format!(r#"<hs:sec {NS}>{first}{body}</hs:sec>"#))).unwrap();
    let column = |k: usize| {
        doc.body
            .iter()
            .filter_map(|b| b.as_para())
            .find(|p| p.text.contains(&format!("단 {k}")))
            .is_some_and(|p| p.text.starts_with(craft_wordmodel::para::COLUMN_BREAK))
    };
    // page 1: 0 | 1, page 2: 2 | (page break) page 3: 3 | 4, page 4: 5
    assert!(column(1) && !column(2), "two columns a page");
    assert!(!column(3), "the page break starts a page");
    assert!(column(4), "the page after the break has its second column");
    assert!(!column(5), "and then a new page");
}

/// A cell whose span runs past the widest grid a table can have spans only the columns left.
#[test]
fn a_span_past_the_grid_ends_at_its_edge() {
    let cell = |col: u32, span: u32| {
        format!(
            r#"<hp:tc><hp:subList><hp:p><hp:run><hp:t>칸</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="{col}" rowAddr="0"/><hp:cellSpan colSpan="{span}" rowSpan="1"/><hp:cellSz width="300" height="300"/></hp:tc>"#
        )
    };
    let cells: String = (0..62).map(|c| cell(c, 1)).collect::<String>() + &cell(62, 5).replace("칸", "끝 칸");
    let sec = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:tbl rowCnt="1" colCnt="70"><hp:sz width="20000" height="300"/><hp:pos treatAsChar="1"/><hp:tr>{cells}</hp:tr></hp:tbl></hp:run></hp:p></hs:sec>"#
    );
    let doc = read_bytes(&hwpx(&sec)).unwrap();
    let t = doc.body.iter().find_map(|b| b.as_table()).expect("no table");
    for row in &t.rows {
        let spanned: usize = row.cells.iter().map(|c| c.props.span.max(1) as usize).sum();
        assert!(spanned <= t.grid.len(), "the row spans {spanned} of {} columns", t.grid.len());
    }
    let last = t.rows.iter().flat_map(|r| r.cells.iter()).find(|c| c.blocks.iter().filter_map(|b| b.as_para()).any(|p| p.text == "끝 칸"));
    let last = last.expect("the clipped cell stays");
    assert_eq!(last.props.span.max(1) as usize, t.grid.len() - 62, "it spans the columns left");
}

/// An explicit page break starts a new page even where the cache starts its lines at the same
/// place as the page before: a page-hiding control on that second page doesn't hide the
/// section's first-page header.
#[test]
fn an_explicit_page_break_counts_where_the_cache_starts_alike() {
    let sec = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr/><hp:ctrl><hp:header applyPageType="BOTH"><hp:subList><hp:p><hp:run><hp:t>머리말</hp:t></hp:run></hp:p></hp:subList></hp:header></hp:ctrl><hp:t>첫 쪽</hp:t></hp:run><hp:linesegarray><hp:lineseg vertpos="0"/></hp:linesegarray></hp:p><hp:p pageBreak="1"><hp:run><hp:ctrl><hp:pageHiding hideHeader="1" hideFooter="0" hidePageNum="0"/></hp:ctrl><hp:t>둘째 쪽</hp:t></hp:run><hp:linesegarray><hp:lineseg vertpos="0"/></hp:linesegarray></hp:p></hs:sec>"#
    );
    let doc = read_bytes(&hwpx(&sec)).unwrap();
    let title = doc.body.iter().filter_map(|b| b.as_para()).filter_map(|p| p.section.as_ref()).any(|s| s.title_page) || doc.last_section.title_page;
    assert!(!title, "the first page keeps its header");
}

/// An object inside a table cell placed at the right of its column sits at the cell's right
/// edge, inside the cell (viewer, 2026-11-25): the cell is the column there. Before, the section's
/// body width was taken, and the object landed far outside its cell.
#[test]
fn an_object_in_a_cell_aligns_to_the_cells_column() {
    let rect = r#"<hp:rect textWrap="IN_FRONT_OF_TEXT"><hp:sz width="2000" height="2000"/><hp:pos treatAsChar="0" vertRelTo="PARA" horzRelTo="COLUMN" horzAlign="RIGHT" horzOffset="0"/></hp:rect>"#;
    let sec = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr><hp:pagePr width="59528" height="84188"><hp:margin left="8504" right="8504" top="5668" bottom="4252" header="0" footer="0" gutter="0"/></hp:pagePr></hp:secPr><hp:tbl rowCnt="1" colCnt="1"><hp:sz width="10000" height="3000"/><hp:pos treatAsChar="1"/><hp:tr><hp:tc><hp:subList><hp:p><hp:run>{rect}<hp:t>칸</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:cellSz width="10000" height="3000"/><hp:cellMargin left="0" right="0" top="0" bottom="0"/></hp:tc></hp:tr></hp:tbl></hp:run></hp:p></hs:sec>"#
    );
    let doc = read_bytes(&hwpx(&sec)).unwrap();
    let t = doc.body.iter().find_map(|b| b.as_table()).expect("no table");
    let cell = t.rows.first().and_then(|r| r.cells.first()).expect("no cell");
    let o = cell.blocks.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).find(|o| o.is_floating()).expect("no floating object");
    let x = o.clone().float_mut().map(|f| f.x).expect("not floating");
    let cell_w = cell.props.width.unwrap_or(100.0);
    assert!(x > 0.0 && x < cell_w, "the object starts inside its {cell_w} pt cell: {x}");
}

/// The section properties of a document, in order (each section's last paragraph, then the
/// document's last section).
fn sections(doc: &Document) -> Vec<craft_wordmodel::SectionProps> {
    let mut out: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).filter_map(|p| p.section.as_deref().cloned()).collect();
    out.push(doc.last_section.clone());
    out
}

/// The floating objects of part `id` (a master page or a page background drawn behind the text).
fn part_floats(doc: &Document, id: Option<u32>) -> usize {
    let Some(p) = id.and_then(|id| doc.parts.get(&id)) else { return 0 };
    p.blocks.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).filter(|o| o.is_floating()).count()
}

/// The text of part `id`'s paragraphs, one line each.
fn part_text(doc: &Document, id: Option<u32>) -> String {
    let Some(p) = id.and_then(|id| doc.parts.get(&id)) else { return String::new() };
    p.blocks.iter().filter_map(|b| b.as_para()).map(|q| q.text.as_str()).collect::<Vec<_>>().join("\n")
}

/// The PAGE fields of part `id`.
fn part_page_fields(doc: &Document, id: Option<u32>) -> Vec<String> {
    let Some(p) = id.and_then(|id| doc.parts.get(&id)) else { return vec![] };
    p.blocks
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Field { instr, .. } if instr.starts_with("PAGE") => Some(instr.clone()),
            _ => None,
        })
        .collect()
}

const FILL_HEAD: &str = r##"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core"><hh:refList><hh:borderFills><hh:borderFill id="2"><hc:fillBrush><hc:winBrush faceColor="#FFEEDD"/></hc:fillBrush></hh:borderFill></hh:borderFills></hh:refList></hh:head>"##;

/// A page background is drawn in its own section only (viewer, 2026-11-25): a later section
/// without one, which shows the earlier section's header text, shows no background. Before, it
/// inherited the header the background rode in, and the background with it.
#[test]
fn a_page_background_stays_in_its_section() {
    let first = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr><hp:pageBorderFill type="BOTH" borderFillIDRef="2" textBorder="PAPER" fillArea="PAPER"/></hp:secPr><hp:ctrl><hp:header applyPageType="BOTH"><hp:subList><hp:p><hp:run><hp:t>머리말</hp:t></hp:run></hp:p></hp:subList></hp:header></hp:ctrl><hp:t>첫 구역</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let second = format!(r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr/><hp:t>둘째 구역</hp:t></hp:run></hp:p></hs:sec>"#);
    let bytes = package(&[
        ("Contents/header.xml", FILL_HEAD.as_bytes()),
        ("Contents/section0.xml", first.as_bytes()),
        ("Contents/section1.xml", second.as_bytes()),
    ]);
    let doc = read_bytes(&bytes).unwrap();
    let secs = sections(&doc);
    assert!(part_floats(&doc, secs[0].headers.default) > 0, "the first section shows its background");
    let later = secs[1].headers.default;
    assert!(later.is_some() && later != secs[0].headers.default, "the second section has a header of its own: {secs:?}");
    assert_eq!(part_floats(&doc, later), 0, "without the background");
    assert!(
        doc.parts.get(&later.unwrap_or(0)).is_some_and(|p| p.blocks.iter().filter_map(|b| b.as_para()).any(|p| p.text == "머리말")),
        "but with the header text"
    );
}

/// A master page is drawn on every page of its section, whichever header the page shows
/// (viewer, 2026-11-25): even pages with a header of their own, and a first page whose header
/// is hidden. A first page that hides the master page goes without it.
#[test]
fn the_master_page_is_behind_every_header_of_its_section() {
    let master = r#"<hp:masterPage type="BOTH"><hp:subList><hp:p><hp:run><hp:rect textWrap="IN_FRONT_OF_TEXT"><hp:sz width="7200" height="7200"/><hp:pos treatAsChar="0" vertRelTo="PAPER" horzRelTo="PAPER"/></hp:rect></hp:run></hp:p></hp:subList></hp:masterPage>"#;
    let headers = r#"<hp:ctrl><hp:header applyPageType="BOTH"><hp:subList><hp:p><hp:run><hp:t>양쪽</hp:t></hp:run></hp:p></hp:subList></hp:header></hp:ctrl><hp:ctrl><hp:header applyPageType="EVEN"><hp:subList><hp:p><hp:run><hp:t>짝수</hp:t></hp:run></hp:p></hp:subList></hp:header></hp:ctrl>"#;
    let read = |hide: &str| {
        let sec = format!(
            r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr>{master}</hp:secPr>{headers}<hp:ctrl><hp:pageHiding hideHeader="1" hideFooter="0" hidePageNum="0" hideMasterPage="{hide}"/></hp:ctrl><hp:t>본문</hp:t></hp:run></hp:p></hs:sec>"#
        );
        let doc = read_bytes(&hwpx(&sec)).unwrap();
        let s = doc.last_section.clone();
        (part_floats(&doc, s.headers.default), part_floats(&doc, s.headers.even), part_floats(&doc, s.headers.first))
    };
    let (default, even, first) = read("0");
    assert!(default > 0 && even > 0 && first > 0, "default {default}, even {even}, first {first}");
    let (default, even, first) = read("1");
    assert!(default > 0 && even > 0 && first == 0, "hidden on the first page: default {default}, even {even}, first {first}");
}

/// A master page kept as a part of its own, which the section names by its manifest id (the
/// form Hancom writes and its viewer opens), is read. Before, only a master page inside the
/// section definition was, which Hancom's viewer doesn't open.
#[test]
fn a_master_page_part_is_read() {
    let hpf = br#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:manifest><opf:item id="header" href="Contents/header.xml" media-type="application/xml"/><opf:item id="section0" href="Contents/section0.xml" media-type="application/xml"/><opf:item id="masterpage0" href="Contents/masterpage0.xml" media-type="application/xml"/></opf:manifest><opf:spine><opf:itemref idref="header"/><opf:itemref idref="section0"/></opf:spine></opf:package>"#;
    let master = format!(
        r#"<masterPage {NS} id="masterpage0" type="BOTH"><hp:subList><hp:p><hp:run><hp:rect textWrap="IN_FRONT_OF_TEXT"><hp:sz width="7200" height="7200"/><hp:pos treatAsChar="0" vertRelTo="PAPER" horzRelTo="PAPER"/></hp:rect></hp:run></hp:p></hp:subList></masterPage>"#
    );
    let sec = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr masterPageCnt="1"><hp:masterPage idRef="masterpage0"/></hp:secPr><hp:t>본문</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let bytes = package(&[
        ("Contents/content.hpf", hpf),
        ("Contents/header.xml", EMPTY_HEAD),
        ("Contents/section0.xml", sec.as_bytes()),
        ("Contents/masterpage0.xml", master.as_bytes()),
    ]);
    let doc = read_bytes(&bytes).unwrap();
    assert!(part_floats(&doc, doc.last_section.headers.default) > 0, "the master page's shape is behind the text");
}

/// A section asking for another page number than the footer it inherits shows its own number
/// in that footer's place (viewer, 2026-11-25: roman at the right, the earlier centre number
/// gone, the footer's text kept). Before, the inherited footer kept its number.
#[test]
fn an_inherited_footer_shows_the_sections_own_page_number() {
    let first = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr/><hp:ctrl><hp:footer applyPageType="BOTH"><hp:subList><hp:p><hp:run><hp:t>꼬리말 글</hp:t></hp:run></hp:p></hp:subList></hp:footer></hp:ctrl><hp:ctrl><hp:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar=""/></hp:ctrl><hp:t>첫 구역</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let second = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr/><hp:ctrl><hp:pageNum pos="BOTTOM_RIGHT" formatType="ROMAN_SMALL" sideChar=""/></hp:ctrl><hp:t>둘째 구역</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let bytes =
        package(&[("Contents/header.xml", EMPTY_HEAD), ("Contents/section0.xml", first.as_bytes()), ("Contents/section1.xml", second.as_bytes())]);
    let doc = read_bytes(&bytes).unwrap();
    let secs = sections(&doc);
    assert_eq!(part_page_fields(&doc, secs[0].footers.default), ["PAGE"]);
    let footer = secs[1].footers.default;
    assert_eq!(part_page_fields(&doc, footer), ["PAGE \\* roman"], "one number, the section's own");
    assert!(
        part_text(&doc, secs[0].footers.default).contains("꼬리말 글") && part_text(&doc, footer).contains("꼬리말 글"),
        "the footer's text in both"
    );
}

/// A section that hides the page number on its first page, inheriting a numbered footer without
/// asking for a number of its own, shows that footer there without the number (viewer,
/// 2026-11-25). Before, the number was hidden only when the section asked for one itself.
#[test]
fn a_hidden_first_page_number_hides_an_inherited_one() {
    let first = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr/><hp:ctrl><hp:footer applyPageType="BOTH"><hp:subList><hp:p><hp:run><hp:t>꼬리말 글</hp:t></hp:run></hp:p></hp:subList></hp:footer></hp:ctrl><hp:ctrl><hp:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar=""/></hp:ctrl><hp:t>첫 구역</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let second = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:secPr/><hp:ctrl><hp:pageHiding hideHeader="0" hideFooter="0" hidePageNum="1"/></hp:ctrl><hp:t>둘째 구역</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let bytes =
        package(&[("Contents/header.xml", EMPTY_HEAD), ("Contents/section0.xml", first.as_bytes()), ("Contents/section1.xml", second.as_bytes())]);
    let doc = read_bytes(&bytes).unwrap();
    let secs = sections(&doc);
    assert!(secs[1].title_page, "the first page is set apart");
    let first_page = secs[1].footers.first;
    assert!(part_page_fields(&doc, first_page).is_empty(), "no number on the first page");
    // (no default footer of its own: it inherits the first section's)
    let default = secs[1].footers.default.or(secs[0].footers.default);
    assert!(!part_page_fields(&doc, default).is_empty(), "the other pages keep it");
    assert!(
        doc.parts.get(&first_page.unwrap_or(0)).is_some_and(|p| p.blocks.iter().filter_map(|b| b.as_para()).any(|p| p.text == "꼬리말 글")),
        "the text stays"
    );
}
