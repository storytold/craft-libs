//! HWP 5.0, the binary format of Hancom Hangul documents: a compound file of record streams,
//! read into the `craft-wordmodel` document model.
//!
//! The records are rebuilt as the elements an HWPX holds and read by `craft-hwpx`
//! ([`craft_hwpx::owpml`]), so both formats read the same way. Written from Hancom's public format
//! documents only; see the README for the documents and their editions.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

mod error;
mod read;
#[cfg(test)]
mod tests;

pub use error::HwpError;

/// Where [`read`] keeps the file it read in `Document::passthrough`.
pub const FILE_KEY: &str = "hwp:file";

/// Whether `bytes` look like an HWP 5.0 document: a compound file whose `FileHeader` stream
/// names it.
pub fn sniff(bytes: &[u8]) -> bool {
    read::is_hwp(bytes)
}

/// Read an HWP 5.0 document into the model. A damaged or hostile file gives an error, never a
/// panic. The file read is kept in `Document::passthrough` under [`FILE_KEY`], and each
/// paragraph's source paragraph in `Document::kept` (`craft_hwpx::KEPT_FORMAT`).
pub fn read(bytes: &[u8]) -> Result<craft_wordmodel::Document, HwpError> {
    let rebuilt = read::open(bytes)?;
    let mut doc = craft_hwpx::owpml::read(rebuilt.header, rebuilt.sections, rebuilt.binaries)?;
    doc.passthrough.insert(FILE_KEY.into(), std::sync::Arc::new(bytes.to_vec()));
    Ok(doc)
}
