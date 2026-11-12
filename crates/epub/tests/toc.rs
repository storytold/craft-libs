//! Tables of contents and `encryption.xml`, read.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Cursor, Write};

use craft_epub::{Encrypted, Epub, MAX_TOC_ENTRIES, TocEntry, parse_encryption, parse_nav, parse_ncx};

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    for (name, data) in entries {
        z.start_file(*name, o).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

const CONTAINER: &[u8] = br#"<container><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>"#;

fn e(label: &str, path: Option<&str>, fragment: Option<&str>, level: u8) -> TocEntry {
    TocEntry { label: label.into(), path: path.map(str::to_string), fragment: fragment.map(str::to_string), level }
}

const NAV: &[u8] = br##"<?xml version="1.0"?><!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body>
<nav epub:type="landmarks"><ol><li><a href="cover.xhtml">Cover</a></li></ol></nav>
<nav epub:type="toc  other" id="toc"><h1>Contents</h1><ol>
  <li><a href="../text/c1.xhtml#start">Chapter&#160;<em>One</em>
     &amp; more</a>
    <ol><li><a href="c%201.xhtml">Section</a></li><li><span>Not a link</span><ol><li><a href="#local">Deep</a></li></ol></li></ol></li>
  <li><a href="/abs.xhtml">Two</a></li><li>bare text</li>
</ol></nav></body></html>"##;

#[test]
fn a_navigation_document_is_flattened_with_levels() {
    let toc = parse_nav(NAV, "OPS/nav/nav.xhtml").unwrap();
    assert_eq!(
        toc,
        [
            e("Chapter One & more", Some("OPS/text/c1.xhtml"), Some("start"), 1),
            e("Section", Some("OPS/nav/c 1.xhtml"), None, 2),
            e("Not a link", None, None, 2),
            e("Deep", Some("OPS/nav/nav.xhtml"), Some("local"), 3),
            e("Two", Some("abs.xhtml"), None, 1),
        ]
    );
    // Without a `toc` the first `nav` is used; without any there is no table of contents.
    assert_eq!(parse_nav(br#"<html><nav><ol><li><a href="a">A</a></li></ol></nav></html>"#, "nav.xhtml").unwrap().len(), 1);
    assert_eq!(parse_nav(b"<html><body/></html>", "nav.xhtml").unwrap(), []);
    assert_eq!(parse_nav(b"<html><nav/></html>", "nav.xhtml").unwrap(), []);
    assert!(parse_nav(b"", "nav.xhtml").is_err());
    // An HTML entity is not XML: it is dropped, not a reason to refuse the table of contents.
    assert_eq!(parse_nav(br#"<nav><ol><li><a href="a">A&nbsp;B</a></li></ol></nav>"#, "nav.xhtml").unwrap()[0].label, "AB");
}

const NCX: &[u8] = br#"<?xml version="1.0"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><head/><docTitle><text>T</text></docTitle>
<navMap>
 <navPoint id="a" playOrder="1"><navLabel><text>One</text></navLabel><content src="text/c1.html"/>
  <navPoint id="b" playOrder="2"><navLabel><text> One  one </text></navLabel><content src="text/c1.html#s"/></navPoint>
 </navPoint>
 <navPoint id="c" playOrder="3"><navLabel><text>Two</text></navLabel></navPoint>
</navMap></ncx>"#;

#[test]
fn an_ncx_is_flattened_with_levels() {
    let toc = parse_ncx(NCX, "OPS/toc.ncx").unwrap();
    assert_eq!(toc, [e("One", Some("OPS/text/c1.html"), None, 1), e("One one", Some("OPS/text/c1.html"), Some("s"), 2), e("Two", None, None, 1)]);
    assert_eq!(parse_ncx(b"<ncx/>", "toc.ncx").unwrap(), []);
}

#[test]
fn the_package_gives_its_navigation_or_else_its_ncx() {
    let opf3 = br#"<package version="3.0"><manifest><item id="n" href="nav/nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest><spine toc="ncx"/></package>"#;
    let bytes = zip_of(&[("META-INF/container.xml", CONTAINER), ("OPS/book.opf", opf3), ("OPS/nav/nav.xhtml", NAV), ("OPS/toc.ncx", NCX)]);
    assert_eq!(Epub::open(&bytes).unwrap().toc().unwrap().len(), 5, "the navigation document comes first");
    // The navigation document is missing: the NCX is used.
    let bytes = zip_of(&[("META-INF/container.xml", CONTAINER), ("OPS/book.opf", opf3), ("OPS/toc.ncx", NCX)]);
    assert_eq!(Epub::open(&bytes).unwrap().toc().unwrap().len(), 3);
    // Neither is there.
    let bytes = zip_of(&[("META-INF/container.xml", CONTAINER), ("OPS/book.opf", opf3)]);
    assert_eq!(Epub::open(&bytes).unwrap().toc().unwrap(), []);
    let bytes = zip_of(&[("META-INF/container.xml", CONTAINER), ("OPS/book.opf", b"<package><manifest/><spine/></package>")]);
    assert_eq!(Epub::open(&bytes).unwrap().toc().unwrap(), []);
}

/// Nesting as deep as the XML reader allows is walked without recursion, and a table of contents
/// with more entries than the limit is cut there.
#[test]
fn deep_and_huge_tables_of_contents_are_bounded() {
    let deep = format!("<nav>{}{}</nav>", "<ol><li><a href=\"a\">x</a>".repeat(120), "</li></ol>".repeat(120));
    let toc = parse_nav(deep.as_bytes(), "nav.xhtml").unwrap();
    assert_eq!((toc.len(), toc.last().map(|t| t.level)), (120, Some(120)));
    let too_deep = format!("<nav>{}{}</nav>", "<ol><li>".repeat(200), "</li></ol>".repeat(200));
    assert!(parse_nav(too_deep.as_bytes(), "nav.xhtml").is_err(), "past the XML depth limit");
    let many = format!("<ncx><navMap>{}</navMap></ncx>", "<navPoint><navLabel><text>x</text></navLabel></navPoint>".repeat(MAX_TOC_ENTRIES + 10));
    assert_eq!(parse_ncx(many.as_bytes(), "toc.ncx").unwrap().len(), MAX_TOC_ENTRIES);
}

const ENCRYPTION: &[u8] = br#"<?xml version="1.0"?>
<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container" xmlns:enc="http://www.w3.org/2001/04/xmlenc#">
 <enc:EncryptedData><enc:EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/><enc:CipherData><enc:CipherReference URI="OPS/fonts/a%20b.otf"/></enc:CipherData></enc:EncryptedData>
 <enc:EncryptedData><enc:EncryptionMethod Algorithm="http://ns.adobe.com/pdf/enc#RC"/><enc:CipherData><enc:CipherReference URI="OPS/fonts/c.otf"/></enc:CipherData></enc:EncryptedData>
 <enc:EncryptedData><enc:EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/><enc:CipherData><enc:CipherReference URI="OPS/c1.xhtml"/></enc:CipherData></enc:EncryptedData>
 <enc:EncryptedData><enc:CipherData/></enc:EncryptedData>
</encryption>"#;

#[test]
fn encrypted_resources_are_listed_and_font_obfuscation_is_told_from_drm() {
    let list = parse_encryption(ENCRYPTION).unwrap();
    assert_eq!(
        list.iter().map(|x| (x.path.as_str(), x.is_font_obfuscation())).collect::<Vec<_>>(),
        [("OPS/fonts/a b.otf", true), ("OPS/fonts/c.otf", true), ("OPS/c1.xhtml", false)]
    );
    assert_eq!(list[2], Encrypted { path: "OPS/c1.xhtml".into(), algorithm: "http://www.w3.org/2001/04/xmlenc#aes128-cbc".into() });
    assert!(parse_encryption(b"<encryption><EncryptedData>").is_err(), "a cut-off file is an error, not an unencrypted book");
    let opf = b"<package><manifest/><spine/></package>";
    let bytes = zip_of(&[("META-INF/container.xml", CONTAINER), ("OPS/book.opf", opf), ("META-INF/encryption.xml", ENCRYPTION)]);
    assert_eq!(Epub::open(&bytes).unwrap().encryption().unwrap().len(), 3);
    let bytes = zip_of(&[("META-INF/container.xml", CONTAINER), ("OPS/book.opf", opf)]);
    assert_eq!(Epub::open(&bytes).unwrap().encryption().unwrap(), []);
}
