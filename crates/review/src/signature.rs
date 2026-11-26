//! Saved signatures and initials, and the signature pad's draft (N5.04).

use std::hash::{Hash as _, Hasher as _};
use std::io::Cursor;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::ImageEncoder as _;

/// The largest image file the Image tab takes: 4 MiB (PdfCraft's "Choose a PNG or JPEG image
/// (up to 4 MiB)").
pub const MAX_IMAGE_BYTES: usize = 4 << 20;

/// The largest decoded image side the Image tab takes, in pixels: 8192 × 8192 at most (a 12 MP
/// JPEG decodes to about 48 MB; the limit keeps the decode job bounded).
pub const MAX_IMAGE_SIDE: u32 = 8192;

/// The longest side a saved signature image keeps, in pixels.
pub const MAX_SAVED_SIDE: u32 = 4096;

/// The most pixels a saved signature image keeps.
pub const MAX_SAVED_PIXELS: u64 = 4 << 20;

/// The lightness (0 to 255) from which [`SignatureImage::remove_white_background`] treats a pixel as paper.
pub const PAPER_CUTOFF: u8 = 245;

/// The width of the feathered edge below [`PAPER_CUTOFF`].
pub const PAPER_FEATHER: u8 = 35;

/// The longest typed signature, in characters (PdfCraft `pdfcraft_fonts::MAX_SIGNATURE_CHARS`,
/// `crates/fonts/src/script.rs:17`).
pub const MAX_SIGNATURE_CHARS: usize = 256;

/// A signature image: straight RGBA, transparency kept. [`new`](Self::new) hashes the pixels once
/// into a key the painter caches the texture under; the key is not serialized, and deserializing
/// derives it again.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "RawSignatureImage", into = "RawSignatureImage")]
pub struct SignatureImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    key: u64,
}

/// The longest stored image text read: the base64 of a [`MAX_IMAGE_BYTES`] PNG. Longer text is refused before it is
/// decoded.
const MAX_STORED_CHARS: usize = MAX_IMAGE_BYTES.div_ceil(3) * 4;

/// [`SignatureImage`] as stored: a base64 PNG.
#[derive(serde::Serialize, serde::Deserialize)]
struct RawSignatureImage {
    png: String,
}

/// A stored image that was not read: text over [`MAX_STORED_CHARS`], not base64, or not a PNG within the image caps.
#[derive(Debug)]
struct RefusedImage;

impl std::fmt::Display for RefusedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a stored signature image that is too large or unreadable")
    }
}

impl TryFrom<RawSignatureImage> for SignatureImage {
    type Error = RefusedImage;

    fn try_from(raw: RawSignatureImage) -> Result<Self, RefusedImage> {
        if raw.png.len() > MAX_STORED_CHARS {
            return Err(RefusedImage);
        }
        let bytes = STANDARD.decode(raw.png.as_bytes()).map_err(|_| RefusedImage)?;
        decode_fitted(&bytes).map_err(|_| RefusedImage)
    }
}

/// Encodes the image as PNG. An image whose buffer doesn't fit its size can't be encoded (and paints nothing): it is
/// stored as empty text, which the reader refuses, so its slot reads back as `None` instead of failing the whole
/// settings section.
impl From<SignatureImage> for RawSignatureImage {
    fn from(image: SignatureImage) -> Self {
        let mut png = Vec::new();
        let encoded =
            image::codecs::png::PngEncoder::new(&mut png).write_image(&image.rgba, image.width, image.height, image::ExtendedColorType::Rgba8);
        Self { png: if encoded.is_ok() { STANDARD.encode(&png) } else { String::new() } }
    }
}

/// Why image bytes were not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickError {
    /// Unreadable, or not a PNG or JPEG.
    Unreadable,
    /// More than [`MAX_IMAGE_BYTES`] of file or [`MAX_IMAGE_SIDE`] pixels on a side.
    TooLarge,
}

/// Decodes a PNG or JPEG of at most [`MAX_IMAGE_BYTES`] and [`MAX_IMAGE_SIDE`] pixels a side into straight RGBA, turned
/// upright by its EXIF orientation and scaled down within [`MAX_SAVED_SIDE`] and [`MAX_SAVED_PIXELS`]. The size limits
/// apply while decoding, so an oversize image is refused before its pixels are allocated.
pub(crate) fn decode_fitted(bytes: &[u8]) -> Result<SignatureImage, PickError> {
    use image::ImageDecoder as _;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(PickError::TooLarge);
    }
    let refused = |error: image::ImageError| if matches!(error, image::ImageError::Limits(_)) { PickError::TooLarge } else { PickError::Unreadable };
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|_| PickError::Unreadable)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(refused)?;
    // An unreadable orientation tag leaves the image as stored, as a viewer would.
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = image::DynamicImage::from_decoder(decoder).map_err(refused)?;
    decoded.apply_orientation(orientation);
    let (width, height) = saved_size(decoded.width(), decoded.height());
    if (width, height) != (decoded.width(), decoded.height()) {
        decoded = decoded.thumbnail(width, height);
    }
    let rgba = decoded.into_rgba8();
    Ok(SignatureImage::new(rgba.width(), rgba.height(), rgba.into_raw()))
}

/// The size a `width` × `height` image is kept at: unchanged within [`MAX_SAVED_SIDE`] and [`MAX_SAVED_PIXELS`], else
/// scaled down keeping its aspect until both hold (8000 × 6000 becomes 2364 × 1773, 4.19 MP).
fn saved_size(width: u32, height: u32) -> (u32, u32) {
    let side = f64::from(MAX_SAVED_SIDE) / f64::from(width.max(height).max(1));
    let area = (MAX_SAVED_PIXELS as f64 / (f64::from(width) * f64::from(height)).max(1.0)).sqrt();
    let scale = side.min(area);
    if scale >= 1.0 {
        return (width, height);
    }
    let shrink = |v: u32| ((f64::from(v) * scale).floor() as u32).max(1);
    (shrink(width), shrink(height))
}

impl SignatureImage {
    /// An image of `width` × `height` pixels in straight RGBA (`rgba` holds 4 bytes a pixel; an
    /// image whose length doesn't fit paints nothing).
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        let mut hasher = std::hash::DefaultHasher::new();
        width.hash(&mut hasher);
        height.hash(&mut hasher);
        rgba.hash(&mut hasher);
        Self { width, height, rgba, key: hasher.finish() }
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Straight RGBA bytes, 4 per pixel, row by row.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The image with paper white made transparent, with a feathered edge of `feather` below `cutoff`: a pixel whose
    /// darkest channel is `cutoff` or lighter goes clear, one `feather` or more below it keeps its alpha, and those in
    /// between fade. Transparency already there stays.
    pub fn remove_white_background(&self, cutoff: u8, feather: u8) -> SignatureImage {
        let high = f32::from(cutoff);
        let span = f32::from(feather.max(1));
        let mut rgba = self.rgba.clone();
        for pixel in rgba.as_chunks_mut::<4>().0 {
            let whiteness = f32::from(pixel[0].min(pixel[1]).min(pixel[2]));
            let keep = ((high - whiteness) / span).clamp(0.0, 1.0);
            pixel[3] = (f32::from(pixel[3]) * keep).round() as u8;
        }
        SignatureImage::new(self.width, self.height, rgba)
    }

    /// The content key computed by [`new`](Self::new); equal images have equal keys.
    pub fn key(&self) -> u64 {
        self.key
    }
}

/// A signature or initials kept for reuse.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SavedSignature {
    /// Strokes in pad units (0…1 on both axes of the pad's box).
    Drawn(Vec<Vec<[f32; 2]>>),
    /// Text drawn in the signature face (craft-ui's `FontRef::signature()`).
    Typed(String),
    /// A signature image.
    Image(SignatureImage),
}

/// The pad's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SignatureTab {
    #[default]
    /// Type the signature as text.
    Type,
    /// Draw the signature with the pointer.
    Draw,
    /// Pick an image file.
    Image,
    /// A photo of a signature on paper, cleaned into an image by the app (PdfCraft's scan
    /// worker).
    Scan,
}

/// What the pad holds while open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SignatureDraft {
    /// The tab that is showing.
    pub tab: SignatureTab,
    /// Strokes drawn on the Draw tab, in pad units (0 to 1 on both axes).
    pub strokes: Vec<Vec<[f32; 2]>>,
    /// The text typed on the Type tab.
    pub text: String,
    /// The Image or Scan tab's result.
    pub image: Option<SignatureImage>,
    /// Creating initials (else the signature).
    pub initials: bool,
    /// Replacing a saved one (the title says Change).
    pub editing: bool,
}

impl SignatureDraft {
    /// A draft for `name`: the Type tab with the name, or for initials the first letter of each
    /// word of it.
    // pdfcraft: crates/ui-egui/src/tools/fill_sign.rs:97 `SigDraft::new`.
    pub fn new(initials: bool, name: &str) -> Self {
        let text = if initials { name.split_whitespace().filter_map(|word| word.chars().next()).collect() } else { name.trim().to_string() };
        Self { text, initials, ..Self::default() }
    }

    /// Whether Apply is enabled: the current tab has something (text that isn't blank and at most
    /// [`MAX_SIGNATURE_CHARS`] long, a stroke, an image).
    pub fn ready(&self) -> bool {
        match self.tab {
            SignatureTab::Type => !self.text.trim().is_empty() && self.text.chars().take(MAX_SIGNATURE_CHARS + 1).count() <= MAX_SIGNATURE_CHARS,
            SignatureTab::Draw => self.strokes.iter().any(|stroke| stroke.len() > 1),
            SignatureTab::Image | SignatureTab::Scan => self.image.is_some(),
        }
    }

    /// Undo on the Draw tab: drops the last stroke and any one-point strokes (bare clicks) behind
    /// it. Returns whether a visible stroke was removed.
    // pdfcraft: crates/ui-egui/src/tools/fill_sign.rs:114 `SigDraft::undo_stroke`.
    pub fn undo_stroke(&mut self) -> bool {
        while let Some(stroke) = self.strokes.pop() {
            if stroke.len() > 1 {
                return true;
            }
        }
        false
    }

    /// Clear: strokes, text and image (and the scan).
    pub fn clear(&mut self) {
        self.strokes.clear();
        self.text.clear();
        self.image = None;
    }

    /// What Apply saves: the current tab's content; `None` when not [`ready`](Self::ready).
    pub fn saved(&self) -> Option<SavedSignature> {
        if !self.ready() {
            return None;
        }
        Some(match self.tab {
            SignatureTab::Type => SavedSignature::Typed(self.text.trim().to_string()),
            SignatureTab::Draw => SavedSignature::Drawn(self.strokes.clone()),
            SignatureTab::Image | SignatureTab::Scan => SavedSignature::Image(self.image.clone()?),
        })
    }
}
