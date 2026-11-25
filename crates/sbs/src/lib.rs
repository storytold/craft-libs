#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
//! Bounded SBS package/schema XML parsing, sniffing, embedded resource codecs, and envelope writing.
//!
//! Applications interpret material graph semantics locally on top of the parsed XML tree.

pub mod embedded;
pub mod uri;
pub mod value;
pub mod xml;

use std::fmt::Write as _;

pub const SBS_EXT: &str = "sbs";
pub const MAX_SBS_BYTES: u64 = 256 << 20;
pub const MAX_XML_NODES: u32 = 8_000_000;
pub const MAX_RESOURCE_BYTES: u64 = 256 << 20;
pub const SNIFF_BYTES: usize = 4096;
pub const SELF_DEPENDENCY: &str = "?himself";
pub const FIRST_UID: u64 = 1_000_000_000;

/// Why an SBS package XML document could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("the SBS file is {actual} bytes, over the {limit} byte limit")]
    TooLarge { actual: u64, limit: u64 },
    #[error("the SBS file is not UTF-8 text: {0}")]
    InvalidUtf8(#[from] std::str::Utf8Error),
    #[error("the SBS file is not valid XML: {0}")]
    InvalidXml(#[from] roxmltree::Error),
    #[error("not an SBS package (the root element is not <package>)")]
    NotPackage,
}

/// Sniff whether `bytes` begins with an SBS `<package>` XML document.
pub fn is_sbs(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(SNIFF_BYTES)]);
    let mut rest = head.trim_start_matches('\u{feff}');

    // Skip the XML declaration and comments before the root element
    loop {
        rest = rest.trim_start();
        let (skip_open, skip_close) = if rest.starts_with("<?") {
            ("<?", "?>")
        } else if rest.starts_with("<!--") {
            ("<!--", "-->")
        } else {
            break;
        };

        match rest[skip_open.len()..].find(skip_close) {
            Some(end) => rest = &rest[skip_open.len() + end + skip_close.len()..],
            None => return false,
        }
    }

    rest.strip_prefix("<package").is_some_and(|after| after.starts_with(|c: char| c == '>' || c == '/' || c.is_whitespace()))
}

/// A validated SBS XML `<package>` document.
pub struct PackageDocument<'input> {
    document: roxmltree::Document<'input>,
}

impl<'input> PackageDocument<'input> {
    /// Parse borrowed input with the default byte, XML-node, and DTD limits.
    pub fn parse(bytes: &'input [u8]) -> Result<Self, Error> {
        Self::parse_with_limits(bytes, MAX_SBS_BYTES, MAX_XML_NODES)
    }

    /// Parse borrowed input with explicit byte and XML-node limits.
    pub fn parse_with_limits(bytes: &'input [u8], max_bytes: u64, max_xml_nodes: u32) -> Result<Self, Error> {
        let actual = bytes.len() as u64;
        if actual > max_bytes {
            return Err(Error::TooLarge { actual, limit: max_bytes });
        }

        let text = std::str::from_utf8(bytes)?;
        let text = text.trim_start_matches('\u{feff}');
        let options = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: max_xml_nodes, ..roxmltree::ParsingOptions::default() };
        let document = roxmltree::Document::parse_with_options(text, options)?;

        if document.root_element().tag_name().name() != "package" {
            return Err(Error::NotPackage);
        }

        Ok(Self { document })
    }

    pub fn document(&self) -> &roxmltree::Document<'input> {
        &self.document
    }

    pub fn root(&self) -> roxmltree::Node<'_, 'input> {
        self.document.root_element()
    }

    pub fn text(&self) -> &'input str {
        self.document.input_text()
    }

    pub fn identifier(&self) -> Option<&str> {
        xml::child_value(self.root(), "identifier")
    }

    pub fn format_version(&self) -> Option<&str> {
        xml::child_value(self.root(), "formatVersion")
    }

    pub fn content(&self) -> Option<roxmltree::Node<'_, 'input>> {
        xml::child_element(self.root(), "content")
    }

    /// Resolve the display package name from `<identifier>` or `fallback_name`.
    pub fn package_name(&self, fallback_name: &str) -> String {
        let file_stem = fallback_name.rsplit(['/', '\\']).next().unwrap_or(fallback_name);
        let file_stem = file_stem.rsplit_once('.').map_or(file_stem, |(stem, _)| stem).trim();
        match self.identifier().map(str::trim) {
            Some(id) if !id.is_empty() && id != "Unsaved Package" => id.to_string(),
            _ if !file_stem.is_empty() => file_stem.to_string(),
            _ => "Untitled package".to_string(),
        }
    }

    /// Iterate `(uid, filename)` pairs from `<dependencies><dependency>`.
    pub fn dependencies(&self) -> impl Iterator<Item = (&str, &str)> {
        xml::child_element(self.root(), "dependencies")
            .into_iter()
            .flat_map(|d| xml::children_named(d, "dependency"))
            .filter_map(|dep| Some((xml::child_value(dep, "uid")?, xml::child_value(dep, "filename")?)))
    }
}

/// Write an SBS `<package>` XML envelope around serialized `<content>...</content>` XML.
pub fn write_package<'a>(
    identifier: &str,
    format_version: &str,
    updater_version: &str,
    dependencies: impl IntoIterator<Item = (&'a str, u64)>,
    content_xml: &str,
) -> Vec<u8> {
    let mut out = String::with_capacity(content_xml.len() + 4096);
    let _ = write!(
        out,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><package><identifier v=\"{}\"/><formatVersion v=\"{}\"/><updaterVersion v=\"{}\"/><fileUID v=\"0\"/><versionUID v=\"0\"/><dependencies>",
        xml::xml_escape(identifier),
        xml::xml_escape(format_version),
        xml::xml_escape(updater_version),
    );
    for (file, uid) in dependencies {
        let _ = write!(
            out,
            "<dependency><filename v=\"{}\"/><uid v=\"{uid}\"/><type v=\"package\"/><fileUID v=\"0\"/><versionUID v=\"0\"/></dependency>",
            xml::xml_escape(file),
        );
    }
    out.push_str("</dependencies>");
    out.push_str(content_xml);
    out.push_str("</package>\n");
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::{Error, FIRST_UID, PackageDocument, SELF_DEPENDENCY, is_sbs, write_package};

    #[test]
    fn sniffs_sbs_with_bom_declaration_and_comments() {
        assert!(is_sbs(b"\xef\xbb\xbf<?xml version=\"1.0\"?><!-- note --><package><content/></package>"));
        assert!(is_sbs(b"<package/>"));
        assert!(!is_sbs(b"<packaged/>"));
        assert!(!is_sbs(b"<other/>"));
        assert!(!is_sbs(b"<!-- unclosed <package>"));
    }

    #[test]
    fn parses_package_metadata_and_dependencies() {
        let xml = write_package("Wood", "1.1", "0.4.1", [(SELF_DEPENDENCY, FIRST_UID + 1), ("sbs://noise.sbs", FIRST_UID + 2)], "<content/>");
        let doc = PackageDocument::parse(&xml).expect("valid package");
        assert_eq!(doc.package_name("fallback.sbs"), "Wood");
        assert_eq!(doc.format_version(), Some("1.1"));
        let deps: Vec<_> = doc.dependencies().collect();
        assert_eq!(deps, vec![("1000000001", "?himself"), ("1000000002", "sbs://noise.sbs")]);
    }

    #[test]
    fn rejects_malformed_or_non_package_xml() {
        for bytes in [b"<package>".as_slice(), b"<other/>", b"\xff", b"<!DOCTYPE package [<!ENTITY x 'x'>]><package/>"] {
            assert!(PackageDocument::parse(bytes).is_err());
        }
        assert!(matches!(PackageDocument::parse_with_limits(b"<package/>", 4, 100), Err(Error::TooLarge { actual: 10, limit: 4 })));
    }
}
