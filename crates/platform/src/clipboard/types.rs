//! Clipboard flavors, images, limits and errors.

use crate::PlatformText;
use std::borrow::Cow;

/// `text/plain`, UTF-8.
pub const TEXT: &str = "text/plain";
/// `text/html`, a rich-text fragment.
pub const HTML: &str = "text/html";
/// `text/rtf`.
pub const RTF: &str = "text/rtf";
/// `image/png`.
pub const PNG: &str = "image/png";
/// `image/svg+xml`.
pub const SVG: &str = "image/svg+xml";
/// `application/pdf`.
pub const PDF: &str = "application/pdf";
/// `text/uri-list`: copied files, one `file://` URI per line (CRLF line ends, RFC 2483).
pub const URI_LIST: &str = "text/uri-list";
/// Any bitmap the clipboard holds (`image/png`, `image/bmp`, a TIFF on macOS…). A read returns it
/// as an [`RGBA8`] flavor (never PNG: this crate has no PNG encoder); [`ClipboardBackend::read_image`]
/// returns it as [`ClipboardImage`]. Also for [`ClipboardBackend::has`] and [`Clipboard::has`].
pub const BITMAP: &str = "image/*";
/// A raw bitmap as [`EventClipboard`] carries it: width and height as little-endian `u32`s, then
/// the straight RGBA8 pixels. The form an `RGBA8` flavour carries.
pub const RGBA8: &str = "image/x-rgba8";

/// One representation of the clipboard's contents: its MIME type and bytes. A copy offers
/// several, best first (VectorCraft: SVG, PDF, PNG, then text).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flavor {
    /// The MIME type, such as `text/plain` or [`RGBA8`].
    pub mime: Cow<'static, str>,
    /// The flavor's bytes, in the encoding the MIME type names.
    pub data: Vec<u8>,
}

impl Flavor {
    /// A flavor of `data` with the given MIME type.
    pub fn new(mime: impl Into<Cow<'static, str>>, data: Vec<u8>) -> Self {
        Self { mime: mime.into(), data }
    }

    /// `text/plain` with `text`.
    pub fn text(text: &str) -> Self {
        Self::new(TEXT, text.as_bytes().to_vec())
    }
}

/// A bitmap on the clipboard: straight (not premultiplied) RGBA8, rows top to bottom,
/// `rgba.len() == width * height * 4`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Straight (non-premultiplied) RGBA8 samples, row by row, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl ClipboardImage {
    /// True when the size is not zero and `rgba` holds exactly `width * height * 4` bytes.
    /// [`Clipboard::read_image`] drops images that fail this (PhotoCraft
    /// `import_os_clipboard`'s filter).
    // pc: crates/ui-egui/src/app/clipboard.rs:60 image.filter
    pub fn is_whole(&self) -> bool {
        let bytes = u64::from(self.width) * u64::from(self.height) * 4;
        bytes != 0 && self.rgba.len() as u64 == bytes
    }
}

/// The most a paste reads, so a broken or hostile clipboard source can't make the app allocate
/// without end. Over a limit, the read fails with [`ClipboardError::TooLarge`] and nothing is
/// pasted (owner question Q-S5S-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipboardLimits {
    /// Bytes of one image's pixels or encoded data: 1 GiB, a 16K × 16K RGBA bitmap
    /// (PhotoCraft's `MAX_CLIPBOARD_IMAGE_BYTES = 4 * 16384 * 16384`), so no real screenshot is
    /// turned away.
    pub image_bytes: u64,
    /// Bytes of text, rich text and SVG markup: 128 MiB (PhotoCraft's `MAX_CLIPBOARD_SVG_BYTES`,
    /// the size an `.svgz` may inflate to).
    pub text_bytes: u64,
    /// Paths in one list of copied files: 10 000.
    pub files: usize,
}

impl ClipboardLimits {
    /// The limits used when none are set: 1 GiB of image data, 128 MiB of text, 10 000 files.
    pub const DEFAULT: ClipboardLimits = ClipboardLimits { image_bytes: 4 * 16384 * 16384, text_bytes: 128 << 20, files: 10_000 };
}

impl Default for ClipboardLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Why a clipboard read or write failed. "Nothing in that format" is not an error: reads
/// answer `Ok(None)` then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardError {
    /// No clipboard can be reached at all: a Flatpak on Wayland, where the sandbox hides the
    /// data-control protocol (storytold/photocraft#5031), or a headless session. Paste explains it with
    /// [`CLIPBOARD_UNAVAILABLE`].
    Unavailable,
    /// Another read or write holds the clipboard handle (a re-entrant call).
    Busy,
    /// The contents are larger than the limit for their kind, in bytes.
    TooLarge {
        /// The limit that was exceeded, in bytes.
        limit: u64,
    },
    /// None of the flavours written can be put on this clipboard (only SVG or PDF on the system
    /// clipboard, or an image the platform can't take).
    NoFormat,
    /// A bitmap whose pixels don't match its size ([`ClipboardImage::is_whole`]).
    BadImage,
    /// What the operating system said (a broken pipe on Wayland, a clipboard owner that never
    /// answered): free text, in the system's language, not a [`PlatformText`].
    Other(String),
}

impl ClipboardError {
    /// The library's own reason as a [`PlatformText`] the app looks up in its catalog, for the
    /// `{error}` of [`CLIPBOARD_FAILED`]. `None` for [`Other`](Self::Other), whose text is the
    /// system's. [`TooLarge`](Self::TooLarge) is [`CLIPBOARD_TOO_LARGE`] with its `{limit}`.
    pub fn text(&self) -> Option<PlatformText> {
        match self {
            Self::Unavailable => Some(CLIPBOARD_UNREACHABLE),
            Self::Busy => Some(CLIPBOARD_BUSY),
            Self::TooLarge { .. } => Some(CLIPBOARD_TOO_LARGE),
            Self::NoFormat => Some(CLIPBOARD_NO_FORMAT),
            Self::BadImage => Some(CLIPBOARD_BAD_IMAGE),
            Self::Other(_) => None,
        }
    }
}

impl std::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Other(message) => f.write_str(message),
            Self::TooLarge { limit } => f.write_str(&CLIPBOARD_TOO_LARGE.english.replace("{limit}", &format!("{limit} bytes"))),
            other => f.write_str(other.text().map_or("", |text| text.english)),
        }
    }
}

impl std::error::Error for ClipboardError {}

/// "The clipboard can't be read here", for [`ClipboardError::Unavailable`] on paste.
pub const CLIPBOARD_UNAVAILABLE: PlatformText =
    PlatformText { id: "clipboard.unavailable", english: "The system clipboard can't be reached. Paste works with what was copied in this app." };
/// A paste over [`ClipboardLimits`]; `{limit}` is the limit as a file size ("1 GB").
pub const CLIPBOARD_TOO_LARGE: PlatformText =
    PlatformText { id: "clipboard.tooLarge", english: "What's on the clipboard is too large to paste (more than {limit})." };
/// [`ClipboardError::Unavailable`] as the `{error}` of [`CLIPBOARD_FAILED`].
pub const CLIPBOARD_UNREACHABLE: PlatformText = PlatformText { id: "clipboard.unreachable", english: "the system clipboard can't be reached" };
/// [`ClipboardError::Busy`] as the `{error}` of [`CLIPBOARD_FAILED`].
pub const CLIPBOARD_BUSY: PlatformText = PlatformText { id: "clipboard.busy", english: "the clipboard is busy" };
/// [`ClipboardError::NoFormat`] as the `{error}` of [`CLIPBOARD_FAILED`].
pub const CLIPBOARD_NO_FORMAT: PlatformText = PlatformText { id: "clipboard.noFormat", english: "none of the formats can be put on the clipboard" };
/// [`ClipboardError::BadImage`] as the `{error}` of [`CLIPBOARD_FAILED`].
pub const CLIPBOARD_BAD_IMAGE: PlatformText = PlatformText { id: "clipboard.badImage", english: "the image's pixels don't match its size" };
/// Any other failure; `{error}` is the reason: [`ClipboardError::text`] looked up in the catalog,
/// or the system's own text of [`ClipboardError::Other`].
pub const CLIPBOARD_FAILED: PlatformText = PlatformText { id: "clipboard.failed", english: "Couldn't use the clipboard: {error}" };
/// Every [`PlatformText`] of this module, for the missing-translation report ([`crate::texts`]).
pub const TEXTS: &[PlatformText] =
    &[CLIPBOARD_UNAVAILABLE, CLIPBOARD_TOO_LARGE, CLIPBOARD_UNREACHABLE, CLIPBOARD_BUSY, CLIPBOARD_NO_FORMAT, CLIPBOARD_BAD_IMAGE, CLIPBOARD_FAILED];
