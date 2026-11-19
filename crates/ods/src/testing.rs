//! Spreadsheets built in code, for tests and examples.

/// The namespace declarations `content.xml` uses, for a test's own documents.
pub const NAMESPACES: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:calcext="urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0""#;

/// `content.xml` with `tables` (the `table:table` elements) as the spreadsheet body.
pub fn content(tables: &str) -> String {
    format!(
        r#"<office:document-content {NAMESPACES}><office:body><office:spreadsheet>{tables}</office:spreadsheet></office:body></office:document-content>"#
    )
}

/// An `.ods` package around [`content`].
pub fn spreadsheet(tables: &str) -> Vec<u8> {
    package(content(tables).as_bytes())
}

/// An `.ods` package with this `content.xml`.
pub fn package(content: &[u8]) -> Vec<u8> {
    let mut w = craft_odf::PackageWriter::new(craft_odf::mime::SPREADSHEET);
    w.file("content.xml", "text/xml", content);
    w.finish().unwrap_or_default()
}
