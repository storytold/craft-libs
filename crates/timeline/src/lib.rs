#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Editorial timeline interchange: the parts every reader and writer of an edit shares.
//!
//! - [`time`]: exact media time. A [`time::Tick`] divides evenly into every broadcast frame and
//!   common audio sample duration, so frame counts, SMPTE timecode and rational seconds convert
//!   without drift. [`rate`] has the frame-rate conventions documents use (`timebase` + `ntsc`,
//!   floating rates).
//! - [`report`]: what an import or export could not represent exactly.
//! - [`path`]: media references (`file://` URLs, absolute and relative paths), lexically.
//! - [`model`]: a neutral timeline (bins, media, sequences, tracks, clips, transitions, markers,
//!   effects with keyframes) for the formats that describe a whole edit, with [`model::Catalog`]
//!   for the application's effects. [`fx`] has the effects the formats name themselves; [`build`]
//!   builds a [`model::Project`] while reading.
//! - [`ale`]: Avid Log Exchange clip logs.
//! - [`edl`]: CMX 3600 edit decision lists.
//! - [`fcp7`] (feature `xml`): Final Cut Pro 7 XML (`xmeml`).
//! - [`fcpxml`] (feature `xml`): FCPXML 1.9–1.11.
//! - [`otio`] (feature `otio`): OpenTimelineIO JSON.
//! - [`Format`], [`detect_text`], [`decode_text`]: which interchange format a document is.
//!
//! No file I/O: documents are text or bytes, and media references are strings. Each format module
//! gives the document as the format writes it; mapping it onto an application's own timeline is
//! the caller's.

pub mod ale;
pub mod build;
pub mod edl;
#[cfg(feature = "xml")]
pub mod fcp7;
#[cfg(feature = "xml")]
pub mod fcpxml;
pub mod fx;
pub mod model;
#[cfg(feature = "otio")]
pub mod otio;
pub mod path;
pub mod rate;
pub mod report;
pub mod time;
#[cfg(feature = "xml")]
mod xml;

pub use build::Imported;
pub use report::{Level, Report, ReportEntry};
pub use time::{FrameRate, Tick};

/// A timeline interchange format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Format {
    /// CMX 3600 edit decision list.
    Edl,
    /// Final Cut Pro 7 XML interchange (`xmeml`), as read/written by Premiere Pro.
    Fcp7Xml,
    /// Final Cut Pro X XML (1.9–1.11).
    Fcpxml,
    /// OpenTimelineIO JSON.
    Otio,
    /// Advanced Authoring Format (AAF Edit Protocol, structured storage).
    Aaf,
    /// OMF Interchange 2.0 (Bento container).
    Omf,
}

impl Format {
    pub const ALL: [Format; 6] = [Format::Edl, Format::Fcp7Xml, Format::Fcpxml, Format::Otio, Format::Aaf, Format::Omf];

    pub fn name(self) -> &'static str {
        match self {
            Format::Edl => "CMX 3600 EDL",
            Format::Fcp7Xml => "Final Cut Pro XML",
            Format::Fcpxml => "FCPXML",
            Format::Otio => "OpenTimelineIO",
            Format::Aaf => "AAF",
            Format::Omf => "OMF",
        }
    }

    /// Default file extension (without the dot).
    pub fn extension(self) -> &'static str {
        match self {
            Format::Edl => "edl",
            Format::Fcp7Xml => "xml",
            Format::Fcpxml => "fcpxml",
            Format::Otio => "otio",
            Format::Aaf => "aaf",
            Format::Omf => "omf",
        }
    }

    /// Format for a file extension (case-insensitive, with or without the dot). `.xml` is ambiguous
    /// between xmeml and FCPXML; use [`detect_text`] when the bytes are available.
    pub fn from_extension(ext: &str) -> Option<Format> {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "edl" => Some(Format::Edl),
            "xml" => Some(Format::Fcp7Xml),
            "fcpxml" | "fcpxmld" => Some(Format::Fcpxml),
            "otio" => Some(Format::Otio),
            "aaf" => Some(Format::Aaf),
            "omf" | "omfi" => Some(Format::Omf),
            _ => None,
        }
    }
}

/// Error from an importer or exporter. Unsupported *features* never error; they go in the [`Report`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The document cannot be read as its format.
    Parse {
        format: &'static str,
        message: String,
    },
    /// The document contains no timeline.
    Empty,
    /// The project has no sequence with this id.
    NoSequence(model::ItemId),
    Other(String),
}

impl Error {
    #[cfg_attr(not(any(feature = "xml", feature = "otio")), allow(dead_code))]
    pub(crate) fn parse(format: Format, message: impl Into<String>) -> Error {
        Error::Parse { format: format.name(), message: message.into() }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Parse { format, message } => write!(f, "{format} parse error: {message}"),
            Error::Empty => f.write_str("the document contains no timeline"),
            Error::NoSequence(id) => write!(f, "no sequence {id:?} in the project"),
            Error::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// How to read a timeline document.
#[derive(Clone, Copy)]
pub struct ImportOptions<'a> {
    /// Directory relative media paths are resolved against (normally the document's directory).
    pub base_dir: Option<&'a str>,
    /// Name for a timeline the document leaves unnamed.
    pub name: Option<&'a str>,
    /// The application's effects and generators (see [`model::Catalog`]; [`fx::Standard`] knows
    /// only what the formats name themselves).
    pub catalog: &'a dyn model::Catalog,
    /// The application's name for what it writes into documents for itself: a generator's payload
    /// in xmeml is `<namespace>:<payload>`, and OpenTimelineIO metadata sits under this key.
    pub namespace: &'a str,
}

/// How to write a timeline document.
#[derive(Clone, Copy)]
pub struct ExportOptions<'a> {
    /// Write media paths relative to this directory where the format allows it.
    pub relative_to: Option<&'a str>,
    /// Document name (default: the sequence's).
    pub name: Option<&'a str>,
    pub catalog: &'a dyn model::Catalog,
    /// See [`ImportOptions::namespace`].
    pub namespace: &'a str,
    /// `xmeml` version attribute (4 or 5).
    pub xmeml_version: u32,
    pub fcpxml_version: FcpxmlVersion,
}

/// FCPXML document version to write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FcpxmlVersion {
    V1_9,
    #[default]
    V1_10,
    V1_11,
}

impl FcpxmlVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            FcpxmlVersion::V1_9 => "1.9",
            FcpxmlVersion::V1_10 => "1.10",
            FcpxmlVersion::V1_11 => "1.11",
        }
    }
}

/// Sniff which text format (EDL, xmeml, FCPXML, OTIO) a document is from its bytes, using the file
/// extension as a hint. The binary formats (AAF, OMF) have their own sniffers; try those first.
pub fn detect_text(bytes: &[u8], extension: Option<&str>) -> Option<Format> {
    let head = String::from_utf8_lossy(bytes.get(..bytes.len().min(4096)).unwrap_or_default());
    let head = head.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with('<') {
        if head.contains("<xmeml") {
            return Some(Format::Fcp7Xml);
        }
        if head.contains("<fcpxml") {
            return Some(Format::Fcpxml);
        }
        return extension.and_then(Format::from_extension).filter(|f| matches!(f, Format::Fcp7Xml | Format::Fcpxml));
    }
    if head.starts_with('{') {
        if head.contains("\"OTIO_SCHEMA\"") {
            return Some(Format::Otio);
        }
        return extension.and_then(Format::from_extension).filter(|f| *f == Format::Otio);
    }
    if edl::looks_like_edl(head) {
        return Some(Format::Edl);
    }
    extension.and_then(Format::from_extension).filter(|f| *f == Format::Edl)
}

/// The text of a document: UTF-8 (with or without a byte order mark), else Latin-1 (old EDLs).
pub fn decode_text(bytes: &[u8]) -> String {
    let b = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_formats_are_detected_from_bytes_then_extension() {
        assert_eq!(detect_text(b"\xef\xbb\xbf<?xml version=\"1.0\"?>\n<!DOCTYPE xmeml>\n<xmeml version=\"4\">", None), Some(Format::Fcp7Xml));
        assert_eq!(detect_text(b"<?xml version=\"1.0\"?><fcpxml version=\"1.10\">", Some("xml")), Some(Format::Fcpxml));
        assert_eq!(detect_text(b"<?xml version=\"1.0\"?><other/>", Some("xml")), Some(Format::Fcp7Xml));
        assert_eq!(detect_text(b"<?xml version=\"1.0\"?><other/>", Some("edl")), None);
        assert_eq!(detect_text(b"{ \"OTIO_SCHEMA\": \"Timeline.1\" }", None), Some(Format::Otio));
        assert_eq!(detect_text(b"{}", Some(".OTIO")), Some(Format::Otio));
        assert_eq!(detect_text(b"{}", None), None);
        assert_eq!(detect_text(b"TITLE: x\nFCM: NON-DROP FRAME\n", None), Some(Format::Edl));
        assert_eq!(detect_text(b"hello", Some("edl")), Some(Format::Edl));
        assert_eq!(detect_text(b"hello", Some("aaf")), None);
        assert_eq!(detect_text(b"", None), None);
        for f in Format::ALL {
            assert_eq!(Format::from_extension(f.extension()), Some(f), "{}", f.name());
        }
        assert_eq!(Format::from_extension("OMFI"), Some(Format::Omf));
    }

    #[test]
    fn text_is_utf8_or_latin1() {
        assert_eq!(decode_text(b"\xef\xbb\xbfTITLE: caf\xc3\xa9"), "TITLE: café");
        assert_eq!(decode_text(b"TITLE: caf\xe9"), "TITLE: café");
    }
}
