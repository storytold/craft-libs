//! Tests of the reader on HWPX written here by hand from the published element names.

use super::super::read as read_bytes;

/// A package of `section` (an `hs:sec` element) with an empty header.
pub(crate) fn hwpx(section: &str) -> Vec<u8> {
    let mut z = craft_zip::Writer::new();
    let _ = z.add("mimetype", b"application/hwp+zip", craft_zip::Compression::Stored);
    let _ = z.add("Contents/header.xml", br#"<hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"/>"#, craft_zip::Compression::Deflated);
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
