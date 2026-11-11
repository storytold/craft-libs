//! Saved signatures and initials, and the signature pad's draft (N5.04).

use std::hash::{Hash as _, Hasher as _};

/// The largest image file the Image tab takes: 4 MiB (PdfCraft's "Choose a PNG or JPEG image
/// (up to 4 MiB)").
pub const MAX_IMAGE_BYTES: usize = 4 << 20;

/// The largest decoded image side the Image tab takes, in pixels: 8192 × 8192 at most (a 12 MP
/// JPEG decodes to about 48 MB; the limit keeps the decode job bounded).
pub const MAX_IMAGE_SIDE: u32 = 8192;

/// The longest typed signature, in characters (PdfCraft `pdfcraft_fonts::MAX_SIGNATURE_CHARS`,
/// `crates/fonts/src/script.rs:17`).
pub const MAX_SIGNATURE_CHARS: usize = 256;

/// A signature image: straight RGBA, transparency kept. [`new`](Self::new) hashes the pixels once
/// into a key the painter caches the texture under; the key is not serialized, and deserializing
/// derives it again.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(from = "RawSignatureImage", into = "RawSignatureImage")]
pub struct SignatureImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    key: u64,
}

/// [`SignatureImage`] as stored.
#[derive(serde::Serialize, serde::Deserialize)]
struct RawSignatureImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl From<RawSignatureImage> for SignatureImage {
    fn from(raw: RawSignatureImage) -> Self {
        Self::new(raw.width, raw.height, raw.rgba)
    }
}

impl From<SignatureImage> for RawSignatureImage {
    fn from(image: SignatureImage) -> Self {
        Self { width: image.width, height: image.height, rgba: image.rgba }
    }
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
    /// Text drawn in the proportional face (the library has no script face).
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
