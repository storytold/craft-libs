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

/// The texts of a document's parts of `kind`, each with whether it shows a PAGE field.
fn stories(doc: &Document, kind: PartKind) -> Vec<(String, bool)> {
    doc.parts
        .values()
        .filter(|p| p.kind == kind)
        .map(|p| {
            let paras: Vec<_> = p.blocks.iter().filter_map(|b| b.as_para()).collect();
            let text = paras.iter().map(|q| q.text.as_str()).collect::<Vec<_>>().join("\n");
            let page =
                paras.iter().flat_map(|q| q.objects.iter()).any(|o| matches!(o, InlineObject::Field { instr, .. } if instr.starts_with("PAGE")));
            (text, page)
        })
        .collect()
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
    let footers = stories(&read_bytes(&bytes).unwrap(), PartKind::Footer);
    assert!(footers.iter().any(|(t, page)| t.contains("꼬리말 글") && *page), "a footer with the text and the number: {footers:?}");
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
    let footers = stories(&read_bytes(&bytes).unwrap(), PartKind::Footer);
    assert!(footers.iter().any(|(t, page)| t.contains("꼬리말 글") && !*page), "a first-page footer without the number: {footers:?}");
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

/// A header and sections given as element trees (as `craft-hwp` rebuilds an HWP file) read as
/// the same XML does from a package: the same paragraphs, each remembering the same source, so a
/// writer finds them in either.
#[test]
fn trees_read_as_their_xml_does() {
    let head = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"/>"#;
    let sec = format!(
        r#"<hs:sec {NS}><hp:p><hp:run><hp:t>첫째</hp:t><hp:ctrl><hp:footNote><hp:subList><hp:p><hp:run><hp:t>각주</hp:t></hp:run></hp:p></hp:subList></hp:footNote></hp:ctrl></hp:run></hp:p><hp:p><hp:run><hp:tbl rowCnt="1" colCnt="1"><hp:sz width="10000" height="1000"/><hp:pos treatAsChar="1"/><hp:tr><hp:tc><hp:subList><hp:p><hp:run><hp:t>칸</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:cellSz width="10000" height="1000"/></hp:tc></hp:tr></hp:tbl></hp:run></hp:p><hp:p><hp:run><hp:t>셋째</hp:t></hp:run></hp:p></hs:sec>"#
    );
    let from_xml = read_bytes(&hwpx_with(head, &sec)).unwrap();
    let trees =
        crate::owpml::read(crate::xml::parse(head.as_bytes(), false).unwrap(), vec![crate::xml::parse(sec.as_bytes(), false).unwrap()], vec![])
            .unwrap();
    // every paragraph of the body, the cells and the parts, with the source it remembers
    fn walk(blocks: &craft_wordmodel::Blocks, kept: Option<&craft_wordmodel::kept::KeptXml>, out: &mut Vec<(String, Option<String>)>) {
        for b in blocks.iter() {
            if let Some(p) = b.as_para() {
                out.push((p.text.clone(), kept.and_then(|k| k.para(p)).map(|s| s.to_string())));
            }
            for c in b.as_table().iter().flat_map(|t| t.rows.iter()).flat_map(|r| r.cells.iter()) {
                walk(&c.blocks, kept, out);
            }
        }
    }
    let sources = |doc: &Document| {
        let kept = doc.kept.of(crate::KEPT_FORMAT);
        let mut out = Vec::new();
        walk(&doc.body, kept, &mut out);
        for part in doc.parts.values() {
            walk(&part.blocks, kept, &mut out);
        }
        out
    };
    let (a, b) = (sources(&from_xml), sources(&trees));
    assert!(a.iter().filter(|(_, s)| s.is_some()).count() >= 3, "paragraphs remember their source: {a:?}");
    assert_eq!(a, b);
    assert!(!trees.passthrough.contains_key(crate::PACKAGE_KEY), "no HWPX file to write back into");
}

/// A tab stop past any page (a position no page holds) is kept within the longest length the
/// model takes, as the DOCX reader keeps one, instead of millions of points.
#[test]
fn tab_stops_past_any_page_are_bounded() {
    let head = r#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:tabProperties><hh:tabPr id="1"><hh:tabItem pos="2147483000" type="LEFT" leader="NONE"/></hh:tabPr></hh:tabProperties><hh:paraProperties><hh:paraPr id="0" tabPrIDRef="1"/></hh:paraProperties></hh:refList></hh:head>"#;
    let doc =
        read_bytes(&hwpx_with(head, &format!(r#"<hs:sec {NS}><hp:p paraPrIDRef="0"><hp:run><hp:t>탭</hp:t></hp:run></hp:p></hs:sec>"#))).unwrap();
    let p = doc.body.iter().filter_map(|b| b.as_para()).next().expect("no paragraph");
    let tabs = p.props.tabs.clone().unwrap_or_default();
    assert!(!tabs.is_empty(), "the tab stop is read");
    assert!(tabs.iter().all(|t| t.pos <= super::drawing::MAX_LEN_PT), "{tabs:?}");
}
