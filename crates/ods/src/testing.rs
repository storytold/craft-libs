//! Spreadsheets built in code, for tests and examples.

/// The namespace declarations `content.xml` uses, for a test's own documents.
pub const NAMESPACES: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:calcext="urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0""#;

/// `content.xml` with `tables` (the `table:table` elements) as the spreadsheet body.
pub fn content(tables: &str) -> String {
    format!(
        r#"<office:document-content {NAMESPACES}><office:body><office:spreadsheet>{tables}</office:spreadsheet></office:body></office:document-content>"#
    )
}

/// An `.ods` package around [`content`]. A package that cannot be written returns the writer's
/// error.
pub fn spreadsheet(tables: &str) -> Result<Vec<u8>, craft_odf::Error> {
    package(content(tables).as_bytes())
}

/// An `.ods` package with this `content.xml`. A package that cannot be written returns the
/// writer's error.
pub fn package(content: &[u8]) -> Result<Vec<u8>, craft_odf::Error> {
    files(&[("content.xml", content)])
}

/// An `.ods` package with these XML files, in order.
fn files(files: &[(&str, &[u8])]) -> Result<Vec<u8>, craft_odf::Error> {
    let mut w = craft_odf::PackageWriter::new(craft_odf::mime::SPREADSHEET);
    for (path, data) in files {
        w.file(path, "text/xml", data);
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_package_that_cannot_be_written_returns_the_writer_error() {
        let xml = content("");
        let twice = files(&[("content.xml", xml.as_bytes()), ("content.xml", xml.as_bytes())]);
        match twice {
            Err(craft_odf::Error::Zip(message)) => assert!(message.contains("content.xml"), "{message}"),
            other => panic!("expected the writer's error, got {other:?}"),
        }
        assert!(crate::is_ods(&package(xml.as_bytes()).unwrap()));
    }
}
