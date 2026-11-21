//! Tests of the reader on HWPX written here by hand from the published element names.

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
