//! HWPX, the XML format of Hancom Hangul documents (OWPML, KS X 6101): a ZIP package of a
//! header part and section parts, read into the `craft-wordmodel` document model.
//!
//! Written from Hancom's public format documents only; see the README for the documents and
//! their editions.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

mod error;
mod header;
mod package;
mod read;
#[cfg(test)]
mod test_log;
mod units;
mod xml;

pub use error::HwpxError;

/// Whether `bytes` look like an HWPX document: a ZIP archive whose `mimetype` names HWP, or
/// which holds HWPX section parts.
pub fn sniff(bytes: &[u8]) -> bool {
    package::is_hwpx(bytes)
}

/// Read an HWPX document into the model. A damaged or hostile file gives an error, never a
/// panic.
pub fn read(bytes: &[u8]) -> Result<craft_wordmodel::Document, HwpxError> {
    let pkg = package::Package::open(bytes)?;
    let mut doc = read::read(&pkg)?;
    doc.passthrough.insert(PACKAGE_KEY.into(), std::sync::Arc::new(bytes.to_vec()));
    Ok(doc)
}

/// `Document::passthrough` key holding the file a document was read from, as read, for a writer
/// that saves back into it: what changed written, the rest kept.
pub const PACKAGE_KEY: &str = "hwpx:package";

/// The format paragraphs read from a file are kept under (`craft_wordmodel::kept::KeptXml::format`).
/// Each kept paragraph's entry names the source paragraph it was made of, `section:ordinal:piece`
/// (the ordinal counts every `hp:p` of the section part in document order; a source paragraph
/// split around its tables makes several pieces), so a writer can tell the paragraphs nothing
/// changed and write their source back as it was.
pub const KEPT_FORMAT: &str = "hwpx";
