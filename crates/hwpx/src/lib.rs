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
mod write;
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

/// Write `doc` as HWPX. A document [`read`] from an HWPX (its file kept in `Document::passthrough`
/// under [`PACKAGE_KEY`]) is written back into a copy of that file: the paragraphs, tables and
/// pictures that changed are written, everything else stays as it was in the file. Any other
/// document becomes a new package. What the format or this writer can't hold is left out and
/// logged (`log::warn!`).
pub fn write(doc: &craft_wordmodel::Document) -> Result<Vec<u8>, HwpxError> {
    write_with(doc, &WriteOptions::default())
}

/// [`write`] with options.
pub fn write_with(doc: &craft_wordmodel::Document, opts: &WriteOptions) -> Result<Vec<u8>, HwpxError> {
    let (bytes, report) = match doc.passthrough.get(PACKAGE_KEY) {
        Some(original) => write::back::write_back(original, doc, opts.hangul_font)?,
        None => write::new::from_document(doc, opts.hangul_font)?,
    };
    report.log();
    Ok(bytes)
}

/// Options for [`write_with`].
#[derive(Clone, Copy, Debug, Default)]
pub struct WriteOptions {
    /// The font to write Hangul in for a run that names no East Asian font, given the run's
    /// font: `None` when that font has Hangul itself. Hangul documents name a font per script,
    /// and a font without Hangul glyphs leaves Hancom drawing Hangul in a fallback of its own;
    /// an app answers from the fonts it has. Without it, the run's font is named for every
    /// script.
    pub hangul_font: Option<fn(&str) -> Option<String>>,
}

static APPLICATION_NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Names the application written to `version.xml`, for an app that embeds this writer under its
/// own name; "WordCraft" until set. Only the first call counts.
pub fn set_application_name(name: &str) {
    let _ = APPLICATION_NAME.set(name.to_owned());
}

/// The application `version.xml` names (see [`set_application_name`]).
pub fn application_name() -> &'static str {
    APPLICATION_NAME.get().map_or("WordCraft", String::as_str)
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
