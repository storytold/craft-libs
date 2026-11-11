#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Read-only access to the Craft apps' native documents, without the apps: PhotoCraft
//! `.pcraft`, VectorCraft `.vectorcraft` (and `.drawcraft`) and EffectCraft `.ecproj`.
//!
//! - [`sniff`] tells which app wrote a file, from its first bytes.
//! - [`info`] reads the format version and the writer the file names.
//! - [`renditions`] returns the previews and the PDF a file carries.
//! - [`rendition`] reads just one kind of preview or PDF.
//! - [`outline`] returns the canvas, the pages (artboards or compositions) and the layers.
//! - [`pcraft_document`] reads a caller-defined PhotoCraft document head without its payloads.
//! - `pcraft_layer_rgba`, with the feature `pcraft-pixels`, decodes a PhotoCraft layer's pixels
//!   and its layer mask.
//! - `pcraft_icc_profile`, with the same feature, returns a document's raw declared ICC payload.
//!
//! Bytes go in and plain data comes out: no file system access, no rendering and no app crate.
//! Each format's envelope is read through its craft-libs crate: `craft-pcraft-format`,
//! `craft-vectorcraft-format` and `craft-ecproj-format`.
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

mod budget;
mod cancel;
mod ecproj;
mod lenient;
mod pcraft;
#[cfg(feature = "pcraft-pixels")]
mod pcraft_pixels;
mod vectorcraft;
mod zip;

use budget::{Budget, check_depth, counted, counted_map, counted_option, counting, parse, parse_checked};
use lenient::{Lenient, Scalar, Shaped, lenient, parse_versioned, too_new};

use serde::de::DeserializeOwned;

#[cfg(feature = "pcraft-pixels")]
pub use pcraft_pixels::{LayerMask, LayerPixels, Samples, pcraft_icc_profile, pcraft_layer_rgba};

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
            Kind::PhotoCraft => craft_pcraft_format::FORMAT_VERSION,
            Kind::VectorCraft => craft_vectorcraft_format::VERSION,
            Kind::EffectCraft => craft_ecproj_format::SCHEMA_VERSION,
        }
    }
}

/// What a document says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Info {
    #[allow(missing_docs)]
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
    /// A cancellation-aware read observed the caller's cancellation request.
    #[error("document read canceled")]
    Cancelled,
    /// [`sniff`] does not recognize the bytes as a Craft document, or a ZIP archive's
    /// `manifest.json` is a JSON object without `format_version`, so the archive is not a
    /// PhotoCraft bundle.
    #[error("not a PhotoCraft, VectorCraft or EffectCraft document")]
    Unrecognized,
    /// The file is damaged or does not follow its format.
    #[error("malformed document: {0}")]
    Malformed(String),
    /// The file's format version is newer than this crate reads.
    #[error("{} version {version} is newer than this reader supports ({supported})", kind.extension())]
    TooNew {
        #[allow(missing_docs)]
        kind: Kind,
        #[allow(missing_docs)]
        version: u32,
        #[allow(missing_docs)]
        supported: u32,
    },
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
    /// Most bytes one call consumes from gzip, reads out of ZIP entries and returns as
    /// renditions, together; a rendition decompressed from gzip is counted once, and a second
    /// rendition of the same bytes as another copy. Unused lookahead from initial gzip reads
    /// (chunks of up to 64 KiB) is not counted. Default 1 GiB.
    pub max_total: u64,
    /// Deepest nesting of JSON arrays and objects, at most [`MAX_DEPTH`] (a larger value counts
    /// as [`MAX_DEPTH`]). Default [`MAX_DEPTH`]. [`renditions`] and [`outline`] also refuse a
    /// PhotoCraft manifest nested deeper than PhotoCraft reads, 367 levels
    /// (`craft_pcraft_format::manifest_depth(MAX_GROUP_DEPTH)`).
    pub max_depth: u32,
    /// Most list elements one call reads, together: EffectCraft project items and composition
    /// layers, VectorCraft artboards and layers, and PhotoCraft layers at every level. Each is
    /// counted before it is read. Default 1,048,576.
    /// `pcraft_icc_profile` instead caps structural JSON items throughout the manifest,
    /// including skipped fields, before reading its profile reference.
    pub max_items: u64,
}

/// The deepest JSON nesting any call reads: 400 levels, which holds PhotoCraft's 100 nested
/// groups of layers at three levels each.
pub const MAX_DEPTH: u32 = 400;

impl Default for Limits {
    fn default() -> Self {
        Limits { max_json: 256 << 20, max_entry: 256 << 20, max_total: 1 << 30, max_depth: MAX_DEPTH, max_items: 1 << 20 }
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
    /// PhotoCraft's full-canvas flattened image, or its composite preview when absent.
    Composite,
}

/// An image or PDF the document carries.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rendition {
    #[allow(missing_docs)]
    pub kind: RenditionKind,
    /// The MIME type: the one a VectorCraft file gives (a member without one gives no
    /// rendition), or `image/png` for PhotoCraft's previews.
    pub mime: String,
    /// The encoded image data.
    pub bytes: Vec<u8>,
    /// True only for a PhotoCraft [`RenditionKind::Composite`] selected from the manifest's
    /// explicit `full_composite` member. False for the legacy composite preview and all other
    /// renditions. This records the selected source; image bytes and canvas dimensions are
    /// not decoded or validated. Equality and debug output include this provenance.
    pub is_full_composite: bool,
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
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Resolution in dots per inch.
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
    #[allow(missing_docs)]
    pub name: String,
    /// The left edge (0 for a composition).
    pub x: f64,
    /// The top edge (0 for a composition).
    pub y: f64,
    /// Width, in the same units as `x`.
    pub width: f64,
    /// Height, in the same units as `y`.
    pub height: f64,
    /// A composition's frame rate and duration; `None` for an artboard, and for a composition
    /// that gives either one without the other.
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
    #[allow(missing_docs)]
    pub frame_rate_den: i64,
    #[allow(missing_docs)]
    pub duration_seconds: f64,
}

/// One layer.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Layer {
    #[allow(missing_docs)]
    pub name: String,
    /// What the layer holds, as the file names it: `layer` for VectorCraft; `raster`, `group`,
    /// `adjustment`, `fill`, `text`, `shape` or `smart` for PhotoCraft; the source type for
    /// EffectCraft (`Footage`, `Comp`, `Solid`, `Text`, `Shape`, `Null`, `Camera`, `Light`,
    /// `Model`, `Primitive`).
    pub kind: String,
    #[allow(missing_docs)]
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

fn read_kind(bytes: &[u8], budget: &Budget) -> Result<Kind, Error> {
    if pcraft::sniff_checked(bytes, budget.control)? {
        Ok(Kind::PhotoCraft)
    } else if vectorcraft::sniff_checked(bytes, budget.control)? {
        Ok(Kind::VectorCraft)
    } else if ecproj::sniff_checked(bytes, budget.control)? {
        Ok(Kind::EffectCraft)
    } else {
        Err(Error::Unrecognized)
    }
}

/// The document's kind, version and writer. A version newer than this crate reads is not an
/// error here: [`Info::supported`] is `false`.
pub fn info(bytes: &[u8], limits: &Limits) -> Result<Info, Error> {
    let mut budget = Budget::new(limits);
    read_info(bytes, &mut budget)
}

/// [`info`] with cooperative cancellation. The callback runs synchronously on this thread;
/// returning true stops the read at a checkpoint with [`Error::Cancelled`]. The limits and
/// returned data are the same as [`info`].
///
/// ```
/// use std::sync::atomic::{AtomicBool, Ordering};
/// let stopped = AtomicBool::new(true);
/// let result = craft_doc::info_with_cancel(b"", &craft_doc::Limits::default(), &|| stopped.load(Ordering::Relaxed));
/// assert_eq!(result, Err(craft_doc::Error::Cancelled));
/// ```
pub fn info_with_cancel(bytes: &[u8], limits: &Limits, cancelled: &dyn Fn() -> bool) -> Result<Info, Error> {
    cancel::run(cancelled, |control| read_info(bytes, &mut Budget::controlled(limits, control)))
}

fn read_info(bytes: &[u8], budget: &mut Budget) -> Result<Info, Error> {
    let kind = read_kind(bytes, budget)?;
    let (version, generator) = match kind {
        Kind::PhotoCraft => pcraft::info(bytes, budget)?,
        Kind::VectorCraft => vectorcraft::info(bytes, budget)?,
        Kind::EffectCraft => ecproj::info(bytes, budget)?,
    };
    Ok(Info { kind, version, generator, supported: version <= kind.supported_version() })
}

/// The previews and the PDF the document carries, the thumbnail first. A
/// `.vectorcraft` file has a `preview` only when saved with one and a `pdf` only when saved with
/// Create PDF-Compatible File; a `.pcraft` bundle has the `thumbnail` and flattened composite its
/// manifest names; an `.ecproj` project has none.
pub fn renditions(bytes: &[u8], limits: &Limits) -> Result<Vec<Rendition>, Error> {
    read_renditions(bytes, None, &mut Budget::new(limits))
}

/// [`renditions`] with cooperative cancellation, as [`info_with_cancel`]. This interrupts
/// extraction; it does not parse or render the PDF or decode the image a rendition contains.
pub fn renditions_with_cancel(bytes: &[u8], limits: &Limits, cancelled: &dyn Fn() -> bool) -> Result<Vec<Rendition>, Error> {
    cancel::run(cancelled, |control| read_renditions(bytes, None, &mut Budget::controlled(limits, control)))
}

fn read_renditions(bytes: &[u8], wanted: Option<RenditionKind>, budget: &mut Budget) -> Result<Vec<Rendition>, Error> {
    match read_kind(bytes, budget)? {
        Kind::PhotoCraft => pcraft::renditions(bytes, wanted, budget),
        Kind::VectorCraft => vectorcraft::renditions(bytes, wanted, budget),
        Kind::EffectCraft => ecproj::renditions(bytes, budget),
    }
}

/// Read the requested rendition, or `None` when this format or document carries none of that
/// kind. Container metadata is still validated. Other rendition payloads are not decoded,
/// copied or charged against the entry budget; JSON and bytes traversed to reach the selected
/// compressed member still count against the usual limits.
pub fn rendition(bytes: &[u8], kind: RenditionKind, limits: &Limits) -> Result<Option<Rendition>, Error> {
    Ok(read_renditions(bytes, Some(kind), &mut Budget::new(limits))?.pop())
}

/// The document's canvas, pages and layers (see [`Outline`]).
pub fn outline(bytes: &[u8], limits: &Limits) -> Result<Outline, Error> {
    let mut budget = Budget::new(limits);
    read_outline(bytes, &mut budget)
}

/// [`outline`] with cooperative cancellation, as [`info_with_cancel`].
pub fn outline_with_cancel(bytes: &[u8], limits: &Limits, cancelled: &dyn Fn() -> bool) -> Result<Outline, Error> {
    cancel::run(cancelled, |control| read_outline(bytes, &mut Budget::controlled(limits, control)))
}

fn read_outline(bytes: &[u8], budget: &mut Budget) -> Result<Outline, Error> {
    match read_kind(bytes, budget)? {
        Kind::PhotoCraft => pcraft::outline(bytes, budget),
        Kind::VectorCraft => vectorcraft::outline(bytes, budget),
        Kind::EffectCraft => ecproj::outline(bytes, budget),
    }
}

/// A PhotoCraft manifest's `document` read as the caller's `D`, without reading its previews,
/// tiles or blobs. `D` may describe only the metadata the caller needs; other members are
/// skipped by serde. This does not validate the document against PhotoCraft's complete model
/// or check that its referenced payload entries exist.
///
/// The manifest and envelope checks are the same as [`outline`]. Before `D` is deserialized,
/// [`Limits::max_items`] additionally bounds structural JSON items throughout the manifest,
/// including members `D` skips, as [`craft_pcraft_format::check_manifest`] counts them; the
/// count is opening array/object brackets and commas outside strings. The shared format's
/// own item ceiling still applies. Byte and nesting limits apply as usual.
/// `D`'s deserializer is caller code: its own allocations and behavior remain the caller's
/// responsibility. No pixel feature is required.
///
/// A recognized document of another kind gives [`Error::Unsupported`].
///
/// ```
/// #[derive(serde::Deserialize)]
/// struct Head { mode: String }
/// # fn read(bytes: &[u8]) -> Result<(), craft_doc::Error> {
/// let head: Head = craft_doc::pcraft_document(bytes, &craft_doc::Limits::default())?;
/// assert_eq!(head.mode, "Rgb");
/// # Ok(())
/// # }
/// ```
pub fn pcraft_document<D: DeserializeOwned>(bytes: &[u8], limits: &Limits) -> Result<D, Error> {
    match sniff(bytes).ok_or(Error::Unrecognized)? {
        Kind::PhotoCraft => pcraft::head(bytes, &mut Budget::new(limits)),
        kind => Err(Error::Unsupported(format!("document metadata is read from .pcraft bundles, not .{}", kind.extension()))),
    }
}

fn malformed(m: impl std::fmt::Display) -> Error {
    Error::Malformed(m.to_string())
}

#[cfg(test)]
extern crate self as craft_doc;

#[cfg(test)]
#[path = "../tests/main.rs"]
mod integration;
