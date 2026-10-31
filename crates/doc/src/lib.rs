#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Read-only access to the Craft apps' native documents, without the apps: PhotoCraft
//! `.pcraft`, VectorCraft `.vectorcraft` (and `.drawcraft`) and EffectCraft `.ecproj`.
//!
//! - [`sniff`] tells which app wrote a file, from its first bytes.
//! - [`info`] reads the format version and the writer the file names.
//! - [`renditions`] returns the previews and the PDF a file carries.
//! - [`outline`] returns the canvas, the pages (artboards or compositions) and the layers.
//!
//! Bytes go in and plain data comes out: no file system access, no rendering and no app crate.
//! A file whose format version is newer than this crate reads gives [`Info::supported`] `false`
//! from [`info`] and [`Error::TooNew`] from the other calls.
//!
//! Nothing panics on input. Every length a file states is checked against [`Limits`] before
//! anything is allocated for it, and every decompressed byte is counted against them.
//!
//! ```
//! let file = br#"{"format":"vectorcraft","version":4,"generator":"VectorCraft 0.1.0","document":{"artboards":[],"layers":[]}}"#;
//! assert_eq!(craft_doc::sniff(file), Some(craft_doc::Kind::VectorCraft));
//! let info = craft_doc::info(file, &craft_doc::Limits::default()).unwrap();
//! assert_eq!(info.version, 4);
//! assert!(info.supported);
//! ```

mod b64;
mod ecproj;
mod pcraft;
mod vectorcraft;
mod zip;

use serde::de::DeserializeOwned;

/// The app that wrote a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kind {
    /// A PhotoCraft `.pcraft` bundle (a ZIP archive).
    PhotoCraft,
    /// A VectorCraft `.vectorcraft` document (JSON, plain or gzip), or one saved under the
    /// earlier name `.drawcraft`.
    VectorCraft,
    /// An EffectCraft `.ecproj` project (JSON).
    EffectCraft,
}

impl Kind {
    /// The file extension the app saves with, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Kind::PhotoCraft => "pcraft",
            Kind::VectorCraft => "vectorcraft",
            Kind::EffectCraft => "ecproj",
        }
    }

    /// The newest format version this crate reads: PhotoCraft's `format_version`, VectorCraft's
    /// `version` or EffectCraft's `schema`.
    pub fn supported_version(self) -> u32 {
        match self {
            Kind::PhotoCraft => pcraft::FORMAT_VERSION,
            Kind::VectorCraft => vectorcraft::VERSION,
            Kind::EffectCraft => ecproj::SCHEMA_VERSION,
        }
    }
}

/// What a document says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Info {
    pub kind: Kind,
    /// The format version the file declares.
    pub version: u32,
    /// The writer, as the file names it: VectorCraft's `generator` (`"VectorCraft 0.13.1"`),
    /// PhotoCraft's `generator` (`"photocraft-format 0.1.0"`) or EffectCraft's `savedBy` (the
    /// app version, `"0.12.0"`). `None` when the file has none.
    pub generator: Option<String>,
    /// `false` when [`Info::version`] is newer than [`Kind::supported_version`]; the other calls
    /// then return [`Error::TooNew`].
    pub supported: bool,
}

/// Why a document could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// [`sniff`] does not recognize the bytes as a Craft document.
    #[error("not a PhotoCraft, VectorCraft or EffectCraft document")]
    Unrecognized,
    /// The file is damaged or does not follow its format.
    #[error("malformed document: {0}")]
    Malformed(String),
    /// The file's format version is newer than this crate reads.
    #[error("{} version {version} is newer than this reader supports ({supported})", kind.extension())]
    TooNew { kind: Kind, version: u32, supported: u32 },
    /// A valid container feature this reader does not implement (ZIP64, encryption, a ZIP
    /// compression method other than STORE or DEFLATE).
    #[error("{0}")]
    Unsupported(String),
    /// Reading the file would exceed the caller's [`Limits`].
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
}

/// Bounds on the work and memory one call may use. Each is checked before the bytes it covers
/// are allocated.
///
/// ```
/// let mut limits = craft_doc::Limits::default();
/// limits.max_entry = 16 << 20;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Most bytes of JSON read: the VectorCraft file's JSON, the PhotoCraft manifest or the
    /// EffectCraft project. The names and other strings returned are parts of it, so this also
    /// bounds them. Default 256 MiB.
    pub max_json: u64,
    /// Most bytes of one rendition or one ZIP entry. Default 256 MiB.
    pub max_entry: u64,
    /// Most bytes one call decompresses from gzip, reads out of ZIP entries and returns as
    /// renditions, together. Default 1 GiB.
    pub max_total: u64,
    /// Deepest nesting of JSON arrays and objects, at most [`MAX_DEPTH`] (a larger value counts
    /// as [`MAX_DEPTH`]). Default [`MAX_DEPTH`].
    pub max_depth: u32,
}

/// The deepest JSON nesting any call reads: 400 levels, which holds PhotoCraft's 100 nested
/// groups of layers at three levels each.
pub const MAX_DEPTH: u32 = 400;

impl Default for Limits {
    fn default() -> Self {
        Limits { max_json: 256 << 20, max_entry: 256 << 20, max_total: 1 << 30, max_depth: MAX_DEPTH }
    }
}

/// What a rendition shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RenditionKind {
    /// A PDF of the document (VectorCraft's `pdf`, saved with Create PDF-Compatible File).
    Pdf,
    /// A small preview image: VectorCraft's `preview` (the first artboard) or PhotoCraft's
    /// `thumbnail`.
    Thumbnail,
    /// PhotoCraft's flattened `composite` preview.
    Composite,
}

/// An image or PDF the document carries.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rendition {
    pub kind: RenditionKind,
    /// The MIME type: the one the file gives, else `image/png` or `application/pdf`.
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// The structure of a document, without its content.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct Outline {
    /// PhotoCraft's canvas; `None` for the other formats.
    pub canvas: Option<Canvas>,
    /// VectorCraft's artboards (in points) or EffectCraft's compositions (in pixels), in file
    /// order; empty for PhotoCraft.
    pub pages: Vec<Page>,
    /// VectorCraft's top-level layers or PhotoCraft's layer tree, bottom first; empty for
    /// EffectCraft, whose layers are in each [`Page`].
    pub layers: Vec<Layer>,
}

/// A PhotoCraft canvas.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub resolution_dpi: f32,
    /// The color mode as the manifest writes it: `Rgb`, `Cmyk`, `Grayscale`, `Lab`, `Indexed`,
    /// `Bitmap`, `Duotone` or `Multichannel`.
    pub mode: String,
    /// The sample type as the manifest writes it: `U8`, `U16` or `F32`.
    pub depth: String,
}

/// A VectorCraft artboard or an EffectCraft composition.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Page {
    pub name: String,
    /// The left edge (0 for a composition).
    pub x: f64,
    /// The top edge (0 for a composition).
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// A composition's frame rate and duration; `None` for an artboard.
    pub timing: Option<Timing>,
    /// A composition's layers, bottom first; empty for an artboard.
    pub layers: Vec<Layer>,
}

/// A composition's timing.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Timing {
    /// Frames per second as `frame_rate_num / frame_rate_den`.
    pub frame_rate_num: i64,
    pub frame_rate_den: i64,
    pub duration_seconds: f64,
}

/// One layer.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Layer {
    pub name: String,
    /// What the layer holds, as the file names it: `layer` for VectorCraft; `raster`, `group`,
    /// `adjustment`, `fill`, `text`, `shape` or `smart` for PhotoCraft; the source type for
    /// EffectCraft (`Footage`, `Comp`, `Solid`, `Text`, `Shape`, `Null`, `Camera`, `Light`,
    /// `Model`, `Primitive`).
    pub kind: String,
    pub visible: bool,
    /// Opacity from 0 to 1; `None` for EffectCraft, where opacity is an animated property.
    pub opacity: Option<f32>,
    /// The blend mode as the file names it (`Normal`, `Multiply`…).
    pub blend: Option<String>,
    /// A PhotoCraft group's layers, bottom first.
    pub children: Vec<Layer>,
}

/// Which app wrote the document whose first bytes are `head`, or `None`. The first 64 KiB are
/// enough for every file the apps write. A re-zipped `.pcraft` whose `manifest.json` entry
/// starts past `head` is recognized only when `head` holds the whole file.
pub fn sniff(head: &[u8]) -> Option<Kind> {
    if pcraft::sniff(head) {
        Some(Kind::PhotoCraft)
    } else if vectorcraft::sniff(head) {
        Some(Kind::VectorCraft)
    } else if ecproj::sniff(head) {
        Some(Kind::EffectCraft)
    } else {
        None
    }
}

/// The document's kind, version and writer. A version newer than this crate reads is not an
/// error here: [`Info::supported`] is `false`.
pub fn info(bytes: &[u8], limits: &Limits) -> Result<Info, Error> {
    let mut budget = Budget::new(limits);
    let kind = sniff(bytes).ok_or(Error::Unrecognized)?;
    let (version, generator) = match kind {
        Kind::PhotoCraft => pcraft::info(bytes, &mut budget)?,
        Kind::VectorCraft => vectorcraft::info(bytes, &mut budget)?,
        Kind::EffectCraft => ecproj::info(bytes, &mut budget)?,
    };
    Ok(Info { kind, version, generator, supported: version <= kind.supported_version() })
}

/// The previews and the PDF the document carries, the thumbnail first. A
/// `.vectorcraft` file has a `preview` only when saved with one and a `pdf` only when saved with
/// Create PDF-Compatible File; a `.pcraft` bundle has the `thumbnail` and `composite` its
/// manifest names; an `.ecproj` project has none.
pub fn renditions(bytes: &[u8], limits: &Limits) -> Result<Vec<Rendition>, Error> {
    let mut budget = Budget::new(limits);
    match sniff(bytes).ok_or(Error::Unrecognized)? {
        Kind::PhotoCraft => pcraft::renditions(bytes, &mut budget),
        Kind::VectorCraft => vectorcraft::renditions(bytes, &mut budget),
        Kind::EffectCraft => ecproj::renditions(bytes, &mut budget),
    }
}

/// The document's canvas, pages and layers (see [`Outline`]).
pub fn outline(bytes: &[u8], limits: &Limits) -> Result<Outline, Error> {
    let mut budget = Budget::new(limits);
    match sniff(bytes).ok_or(Error::Unrecognized)? {
        Kind::PhotoCraft => pcraft::outline(bytes, &mut budget),
        Kind::VectorCraft => vectorcraft::outline(bytes, &mut budget),
        Kind::EffectCraft => ecproj::outline(bytes, &mut budget),
    }
}

fn malformed(m: impl std::fmt::Display) -> Error {
    Error::Malformed(m.to_string())
}

/// The bytes one call may still decompress, read out of ZIP entries or return as renditions.
pub(crate) struct Budget<'a> {
    pub(crate) limits: &'a Limits,
    left: u64,
}

impl<'a> Budget<'a> {
    fn new(limits: &'a Limits) -> Self {
        Budget { limits, left: limits.max_total }
    }

    /// Bytes still allowed.
    pub(crate) fn left(&self) -> u64 {
        self.left
    }

    /// Count `n` bytes of `what` against the total.
    pub(crate) fn take(&mut self, n: u64, what: &str) -> Result<(), Error> {
        self.left = self
            .left
            .checked_sub(n)
            .ok_or_else(|| Error::LimitExceeded(format!("{what}: more than max_total ({} bytes) in all", self.limits.max_total)))?;
        Ok(())
    }

    /// Check one rendition or ZIP entry of `n` bytes against `max_entry`, then count it.
    pub(crate) fn take_entry(&mut self, n: u64, what: &str) -> Result<(), Error> {
        if n > self.limits.max_entry {
            return Err(Error::LimitExceeded(format!("{what} is {n} bytes, more than max_entry ({})", self.limits.max_entry)));
        }
        self.take(n, what)
    }

    /// Check `n` bytes of JSON against `max_json`.
    pub(crate) fn check_json(&self, n: u64, what: &str) -> Result<(), Error> {
        if n > self.limits.max_json {
            return Err(Error::LimitExceeded(format!("{what} is more than max_json ({} bytes)", self.limits.max_json)));
        }
        Ok(())
    }
}

/// Is the JSON text nested at most `max` levels deep (arrays and objects), and at most
/// [`MAX_DEPTH`]?
fn check_depth(json: &[u8], max: u32) -> Result<(), Error> {
    let max = max.min(MAX_DEPTH);
    let (mut depth, mut in_str, mut escaped) = (0u32, false, false);
    for &b in json {
        if in_str {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > max {
                    return Err(Error::LimitExceeded(format!("JSON nested deeper than max_depth ({max} levels)")));
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

/// Parse one JSON value of type `T` from `json`, after checking its nesting against the limits.
pub(crate) fn parse<T: DeserializeOwned>(json: &[u8], limits: &Limits) -> Result<T, Error> {
    check_depth(json, limits.max_depth)?;
    parse_checked(json).map_err(malformed)
}

/// Parse JSON whose nesting [`check_depth`] has accepted.
fn parse_checked<T: DeserializeOwned>(json: &[u8]) -> Result<T, serde_json::Error> {
    let mut de = serde_json::Deserializer::from_slice(json);
    de.disable_recursion_limit();
    let v = T::deserialize(&mut de)?;
    de.end()?;
    Ok(v)
}

/// A document's version number: VectorCraft's `version`, PhotoCraft's `format_version` or
/// EffectCraft's `schema`.
#[derive(serde::Deserialize)]
struct Version {
    #[serde(alias = "format_version", alias = "schema")]
    version: u32,
}

/// Parse `T` from `json`; when that fails, report [`Error::TooNew`] instead if the file's version
/// (its top-level `version`, `format_version` or `schema`) is newer than `kind` supports, since a
/// newer format may change what `T` expects.
pub(crate) fn parse_versioned<T: DeserializeOwned>(json: &[u8], limits: &Limits, kind: Kind) -> Result<T, Error> {
    check_depth(json, limits.max_depth)?;
    parse_checked(json).map_err(|e| match parse_checked::<Version>(json) {
        Ok(v) if v.version > kind.supported_version() => too_new(kind, v.version),
        _ => malformed(e),
    })
}

/// Refuse a version newer than `kind` supports.
pub(crate) fn check_version(kind: Kind, version: u32) -> Result<(), Error> {
    if version > kind.supported_version() { Err(too_new(kind, version)) } else { Ok(()) }
}

fn too_new(kind: Kind, version: u32) -> Error {
    Error::TooNew { kind, version, supported: kind.supported_version() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_counts_brackets_outside_strings() {
        assert!(check_depth(br#"[[["[[[[[[[["]]]"#, 3).is_ok());
        assert!(check_depth(br#"[[[["\"[["]]]]"#, 3).is_err());
        assert!(check_depth(br#"{"a":{"b":[1]}}"#, 3).is_ok());
    }

    #[test]
    fn the_budget_refuses_past_its_limits() {
        let limits = Limits { max_json: 10, max_entry: 5, max_total: 8, max_depth: 4 };
        let mut b = Budget::new(&limits);
        assert!(b.take_entry(6, "x").is_err());
        assert!(b.take_entry(5, "x").is_ok());
        assert!(matches!(b.take_entry(4, "x"), Err(Error::LimitExceeded(_))));
        assert_eq!(b.left(), 3);
        assert!(b.check_json(11, "x").is_err() && b.check_json(10, "x").is_ok());
    }
}
