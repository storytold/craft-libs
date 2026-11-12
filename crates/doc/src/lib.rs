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

mod cancel;
mod ecproj;
mod pcraft;
#[cfg(feature = "pcraft-pixels")]
mod pcraft_pixels;
mod vectorcraft;
mod zip;

#[cfg(feature = "pcraft-pixels")]
pub use pcraft_pixels::{LayerMask, LayerPixels, Samples, pcraft_icc_profile, pcraft_layer_rgba};

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use serde::Deserialize;
use serde::de::{self, DeserializeOwned, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};

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
    pub kind: RenditionKind,
    /// The MIME type: the one a VectorCraft file gives (a member without one gives no
    /// rendition), or `image/png` for PhotoCraft's previews.
    pub mime: String,
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

/// The bytes one call may still consume from gzip or ZIP entries, or return as renditions.
pub(crate) struct Budget<'a> {
    pub(crate) limits: &'a Limits,
    pub(crate) control: Option<&'a cancel::Control<'a>>,
    left: u64,
}

impl<'a> Budget<'a> {
    fn new(limits: &'a Limits) -> Self {
        Budget { limits, control: None, left: limits.max_total }
    }

    fn controlled(limits: &'a Limits, control: &'a cancel::Control<'a>) -> Self {
        Budget { limits, control: Some(control), left: limits.max_total }
    }

    pub(crate) fn check_cancelled(&self) -> Result<(), Error> {
        if let Some(control) = self.control {
            control.check()?;
        }
        Ok(())
    }

    /// Copy bytes whose size the caller has already checked against the budget.
    pub(crate) fn copy(&self, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        if self.control.is_none() {
            return Ok(bytes.to_vec());
        }
        self.check_cancelled()?;
        let mut out = Vec::with_capacity(bytes.len());
        for chunk in bytes.chunks(cancel::CHUNK) {
            self.check_cancelled()?;
            out.extend_from_slice(chunk);
        }
        Ok(out)
    }

    /// Bytes still allowed.
    pub(crate) fn left(&self) -> u64 {
        self.left
    }

    /// Count `n` bytes of `what` against the total.
    pub(crate) fn take(&mut self, n: u64, what: &str) -> Result<(), Error> {
        self.check_cancelled()?;
        self.left = self
            .left
            .checked_sub(n)
            .ok_or_else(|| Error::LimitExceeded(format!("{what}: more than max_total ({} bytes) in all", self.limits.max_total)))?;
        Ok(())
    }

    /// Check one rendition or ZIP entry of `n` bytes against `max_entry`.
    pub(crate) fn check_entry(&self, n: u64, what: &str) -> Result<(), Error> {
        self.check_cancelled()?;
        if n > self.limits.max_entry {
            return Err(Error::LimitExceeded(format!("{what} is {n} bytes, more than max_entry ({})", self.limits.max_entry)));
        }
        Ok(())
    }

    /// Check one rendition or ZIP entry of `n` bytes against `max_entry`, then count it.
    pub(crate) fn take_entry(&mut self, n: u64, what: &str) -> Result<(), Error> {
        self.check_entry(n, what)?;
        self.take(n, what)
    }

    /// Check `n` bytes of JSON against `max_json`.
    pub(crate) fn check_json(&self, n: u64, what: &str) -> Result<(), Error> {
        self.check_cancelled()?;
        if n > self.limits.max_json {
            return Err(Error::LimitExceeded(format!("{what} is more than max_json ({} bytes)", self.limits.max_json)));
        }
        Ok(())
    }
}

/// Is the JSON text nested at most `max` levels deep (arrays and objects), and at most
/// [`MAX_DEPTH`]?
pub(crate) fn check_depth(json: &[u8], max: u32, control: Option<&cancel::Control<'_>>) -> Result<(), Error> {
    let max = max.min(MAX_DEPTH);
    let (mut depth, mut in_str, mut escaped) = (0u32, false, false);
    for chunk in json.chunks(cancel::CHUNK) {
        if let Some(control) = control {
            control.check()?;
        }
        for &b in chunk {
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
    }
    Ok(())
}

thread_local! {
    /// The elements [`counted`] lists may still read in the current [`parse`], and whether a
    /// list went past them.
    static ITEMS: Cell<(u64, bool)> = const { Cell::new((u64::MAX, false)) };
}

/// Run `f` with `max` elements for [`counted`] lists; when `f` fails after a list went past
/// them, the error is [`Error::LimitExceeded`].
fn counting<T>(max: u64, f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    let saved = ITEMS.with(|c| c.replace((max, false)));
    let r = f();
    let (_, over) = ITEMS.with(|c| c.replace(saved));
    match r {
        Err(_) if over => Err(Error::LimitExceeded(format!("the file lists more than max_items ({max}) items, layers and pages"))),
        r => r,
    }
}

/// Count one list element against [`Limits::max_items`].
fn take_item<E: de::Error>() -> Result<(), E> {
    ITEMS.with(|c| match c.get().0.checked_sub(1) {
        Some(left) => {
            c.set((left, false));
            Ok(())
        }
        None => {
            c.set((0, true));
            Err(E::custom("more than max_items list elements"))
        }
    })
}

/// A `T` read after one element is counted with [`take_item`].
struct Item<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for Item<T> {
    type Value = T;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<T, D::Error> {
        take_item()?;
        T::deserialize(d)
    }
}

/// A list read by [`parse`] (as `#[serde(deserialize_with = "counted")]`), each element counted
/// against [`Limits::max_items`] before it is read.
pub(crate) fn counted<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
    struct List<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for List<T> {
        type Value = Vec<T>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a list")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            let mut out = Vec::new();
            while let Some(v) = seq.next_element_seed(Item(PhantomData))? {
                out.push(v);
            }
            Ok(out)
        }
    }
    d.deserialize_seq(List(PhantomData))
}

/// [`counted`] for a list that may be `null`.
pub(crate) fn counted_option<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Vec<T>>, D::Error> {
    struct Counted<T>(Vec<T>);
    impl<'de, T: Deserialize<'de>> Deserialize<'de> for Counted<T> {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            counted(d).map(Counted)
        }
    }
    Ok(Option::<Counted<T>>::deserialize(d)?.map(|c| c.0))
}

/// [`counted`] for a map, each value counted before it is read.
pub(crate) fn counted_map<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<BTreeMap<String, T>, D::Error> {
    struct Map<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Map<T> {
        type Value = BTreeMap<String, T>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a map")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<BTreeMap<String, T>, A::Error> {
            let mut out = BTreeMap::new();
            while let Some(k) = map.next_key::<String>()? {
                let v = map.next_value_seed(Item(PhantomData))?;
                out.insert(k, v);
            }
            Ok(out)
        }
    }
    d.deserialize_map(Map(PhantomData))
}

/// Parse one JSON value of type `T` from `json`, after checking its nesting against the limits.
pub(crate) fn parse<T: DeserializeOwned>(json: &[u8], budget: &Budget) -> Result<T, Error> {
    check_depth(json, budget.limits.max_depth, budget.control)?;
    counting(budget.limits.max_items, || parse_checked(json, budget).map_err(malformed))
}

/// Parse JSON whose nesting [`check_depth`] has accepted.
fn parse_checked<T: DeserializeOwned>(json: &[u8], budget: &Budget) -> Result<T, serde_json::Error> {
    fn read<'de, T: DeserializeOwned, R: serde_json::de::Read<'de>>(mut de: serde_json::Deserializer<R>) -> Result<T, serde_json::Error> {
        de.disable_recursion_limit();
        let v = T::deserialize(&mut de)?;
        de.end()?;
        Ok(v)
    }
    match budget.control {
        Some(control) => read(serde_json::Deserializer::from_reader(cancel::Reader::new(json, Some(control)))),
        None => read(serde_json::Deserializer::from_slice(json)),
    }
}

/// The top-level member that holds a document's version number: VectorCraft's `version`,
/// PhotoCraft's `format_version` or EffectCraft's `schema`.
fn version_member(kind: Kind) -> &'static str {
    match kind {
        Kind::PhotoCraft => "format_version",
        Kind::VectorCraft => "version",
        Kind::EffectCraft => "schema",
    }
}

/// The version number in the member a [`Visitor`] is given, read from the top-level JSON object;
/// the other members are skipped, and a repeated version member is refused.
struct VersionIn(&'static str);

impl<'de> Visitor<'de> for VersionIn {
    type Value = Option<u32>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Option<u32>, A::Error> {
        let mut version = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == self.0 {
                if version.is_some() {
                    return Err(de::Error::duplicate_field(self.0));
                }
                version = Some(map.next_value()?);
            } else {
                map.next_value::<de::IgnoredAny>()?;
            }
        }
        Ok(version)
    }
}

/// Parse `T` from `json`; when that fails, report [`Error::TooNew`] instead if the file's version
/// (its top-level member that [`version_member`] names) is newer than `kind` supports, since a
/// newer format may change what `T` expects.
pub(crate) fn parse_versioned<T: DeserializeOwned>(json: &[u8], budget: &Budget, kind: Kind) -> Result<T, Error> {
    fn version<'de, R: serde_json::de::Read<'de>>(mut de: serde_json::Deserializer<R>, kind: Kind) -> Result<Option<u32>, serde_json::Error> {
        de.disable_recursion_limit();
        let version = (&mut de).deserialize_map(VersionIn(version_member(kind)))?;
        de.end()?;
        Ok(version)
    }
    check_depth(json, budget.limits.max_depth, budget.control)?;
    counting(budget.limits.max_items, || {
        parse_checked(json, budget).map_err(|e| {
            if let Err(cancelled) = budget.check_cancelled() {
                return cancelled;
            }
            let version = match budget.control {
                Some(control) => version(serde_json::Deserializer::from_reader(cancel::Reader::new(json, Some(control))), kind),
                None => version(serde_json::Deserializer::from_slice(json), kind),
            };
            match version {
                Ok(Some(version)) if version > kind.supported_version() => too_new(kind, version),
                _ => malformed(e),
            }
        })
    })
}

/// A member read as `None` when its value has the wrong shape, as the format crates read a
/// file's optional members. The value is read as it is parsed: the elements of an array or
/// object of the wrong shape are skipped one at a time with [`de::IgnoredAny`], and none of the
/// value is kept.
pub(crate) struct Lenient<T>(pub(crate) Option<T>);

/// A type [`Lenient`] reads.
pub(crate) trait Shaped: Sized {
    /// The value, or `None` when it has the wrong shape.
    fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Self>, D::Error>;
}

impl<'de, T: Shaped> Deserialize<'de> for Lenient<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        T::read(d).map(Lenient)
    }
}

impl Shaped for String {
    fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        Ok(Scalar::deserialize(d)?.string())
    }
}

/// [`Lenient`] as a `deserialize_with` function.
pub(crate) fn lenient<'de, D: Deserializer<'de>, T: Shaped>(d: D) -> Result<Option<T>, D::Error> {
    Lenient::deserialize(d).map(|l| l.0)
}

/// One JSON value: a string, a non-negative integer or a boolean is kept, and an array or object
/// is skipped one element at a time with [`de::IgnoredAny`].
pub(crate) enum Scalar {
    Str(String),
    Unsigned(u64),
    Bool(bool),
    Null,
    Other,
}

impl Scalar {
    pub(crate) fn string(self) -> Option<String> {
        match self {
            Scalar::Str(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn unsigned(self) -> Option<u64> {
        match self {
            Scalar::Unsigned(n) => Some(n),
            _ => None,
        }
    }

    pub(crate) fn bool(self) -> Option<bool> {
        match self {
            Scalar::Bool(b) => Some(b),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for Scalar {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct ScalarVisitor;
        impl<'de> Visitor<'de> for ScalarVisitor {
            type Value = Scalar;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON value")
            }

            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Scalar, E> {
                Ok(Scalar::Bool(v))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Scalar, E> {
                Ok(u64::try_from(v).map_or(Scalar::Other, Scalar::Unsigned))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Scalar, E> {
                Ok(Scalar::Unsigned(v))
            }

            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Scalar, E> {
                Ok(Scalar::Other)
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Scalar, E> {
                Ok(Scalar::Str(v.to_owned()))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Scalar, E> {
                Ok(Scalar::Str(v))
            }

            fn visit_unit<E: de::Error>(self) -> Result<Scalar, E> {
                Ok(Scalar::Null)
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Scalar, A::Error> {
                while seq.next_element::<de::IgnoredAny>()?.is_some() {}
                Ok(Scalar::Other)
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Scalar, A::Error> {
                while map.next_entry::<de::IgnoredAny, de::IgnoredAny>()?.is_some() {}
                Ok(Scalar::Other)
            }
        }
        d.deserialize_any(ScalarVisitor)
    }
}

pub(crate) fn too_new(kind: Kind, version: u32) -> Error {
    Error::TooNew { kind, version, supported: kind.supported_version() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_counts_brackets_outside_strings() {
        assert!(check_depth(br#"[[["[[[[[[[["]]]"#, 3, None).is_ok());
        assert!(check_depth(br#"[[[["\"[["]]]]"#, 3, None).is_err());
        assert!(check_depth(br#"{"a":{"b":[1]}}"#, 3, None).is_ok());
    }

    #[test]
    fn the_budget_refuses_past_its_limits() {
        let limits = Limits { max_json: 10, max_entry: 5, max_total: 8, max_depth: 4, max_items: 2 };
        let mut b = Budget::new(&limits);
        assert!(b.take_entry(6, "x").is_err());
        assert!(b.take_entry(5, "x").is_ok());
        assert!(matches!(b.take_entry(4, "x"), Err(Error::LimitExceeded(_))));
        assert_eq!(b.left(), 3);
        assert!(b.check_json(11, "x").is_err() && b.check_json(10, "x").is_ok());
    }

    /// A value that can be skipped but not read: reading it whole, as a buffer of the value
    /// would, fails.
    #[derive(Clone)]
    pub(crate) struct Unread;

    impl<'de> Deserializer<'de> for Unread {
        type Error = de::value::Error;

        fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
            Err(de::Error::custom("the value was read, not skipped"))
        }

        fn deserialize_ignored_any<V: Visitor<'de>>(self, v: V) -> Result<V::Value, Self::Error> {
            v.visit_unit()
        }

        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf option unit
            unit_struct newtype_struct seq tuple tuple_struct map struct enum identifier
        }
    }

    impl<'de> de::IntoDeserializer<'de> for Unread {
        type Deserializer = Unread;

        fn into_deserializer(self) -> Unread {
            self
        }
    }

    /// An array of `n` [`Unread`] elements.
    pub(crate) fn unread_array(n: usize) -> de::value::SeqDeserializer<std::iter::RepeatN<Unread>, de::value::Error> {
        de::value::SeqDeserializer::new(std::iter::repeat_n(Unread, n))
    }

    /// An object whose `n` members hold [`Unread`] values.
    pub(crate) fn unread_object(n: usize) -> de::value::MapDeserializer<'static, std::vec::IntoIter<(String, Unread)>, de::value::Error> {
        de::value::MapDeserializer::new((0..n).map(|i| (i.to_string(), Unread)).collect::<Vec<_>>().into_iter())
    }

    #[test]
    fn a_lenient_string_skips_an_array_or_object_without_reading_it_whole() {
        assert!(Lenient::<String>::deserialize(unread_array(1000)).unwrap().0.is_none());
        assert!(Lenient::<String>::deserialize(unread_object(1000)).unwrap().0.is_none());
        let s: Lenient<String> = Lenient::deserialize(de::value::StrDeserializer::<de::value::Error>::new("VectorCraft 0.1.0")).unwrap();
        assert_eq!(s.0.as_deref(), Some("VectorCraft 0.1.0"));
    }
}
