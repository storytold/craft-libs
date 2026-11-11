//! The Clipboard: size limits, the synthetic-input fence and which copy is ours.

use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use super::*;

/// The app's clipboard: one per window, on the UI thread.
///
/// ```ignore
/// let mut clipboard = Clipboard::new(SystemClipboard::new().decoder(decode_png));
/// // Each frame, before running commands: input injected by automation must not copy.
/// clipboard.set_synthetic(app.automation_input);
/// // Copy
/// let _ = clipboard.write_image(&ClipboardImage { width, height, rgba })?;
/// // Paste: a new image from another app replaces the in-app copy.
/// if let Some(image) = clipboard.read_image()? && !clipboard.is_ours(&image) { session.clipboard = image.into(); }
/// ```
pub struct Clipboard {
    backend: Box<dyn ClipboardBackend>,
    limits: ClipboardLimits,
    synthetic: bool,
    ours: Option<Ours>,
}

/// What the last write put on the clipboard, as a fingerprint.
pub(super) enum Ours {
    /// [`image_signature`] of a bitmap.
    Image(u64),
    /// The flavor the backend wrote: its type and a hash of its bytes.
    Flavor { mime: String, hash: u64 },
}

impl Clipboard {
    /// A clipboard over `backend` with [`ClipboardLimits::DEFAULT`] and the synthetic fence off.
    pub fn new(backend: impl ClipboardBackend + 'static) -> Self {
        Self { backend: Box::new(backend), limits: ClipboardLimits::DEFAULT, synthetic: false, ours: None }
    }

    /// Sets the size limits that writes are checked against.
    pub fn limits(mut self, limits: ClipboardLimits) -> Self {
        self.limits = limits;
        self
    }

    /// While `true` (a frame that processes input injected by automation or a test's control
    /// channel), writes are dropped with [`Written::Fenced`] and reads still work. FigCraft's
    /// rule: "Input injected through the control channel never replaces what the user copied."
    /// The app sets it at the start of each frame.
    // figcraft: crates/ui-egui/src/app/clipboard.rs:6 clipboard_set_text
    pub fn set_synthetic(&mut self, synthetic: bool) {
        self.synthetic = synthetic;
    }

    /// Puts `text` on the clipboard as a plain-text flavor; see [`write`](Self::write).
    pub fn write_text(&mut self, text: &str) -> Result<Written, ClipboardError> {
        self.write(&[Flavor::text(text)])
    }

    /// Puts a bitmap on the clipboard and remembers its [`image_signature`].
    // pc: crates/ui-egui/src/app/clipboard.rs:13 export_os_clipboard
    pub fn write_image(&mut self, image: &ClipboardImage) -> Result<Written, ClipboardError> {
        if !image.is_whole() {
            return Err(ClipboardError::BadImage);
        }
        let written = self.write(&[rgba8_flavor(image)])?;
        if written == Written::Written {
            self.ours = Some(Ours::Image(image_signature(image.width, image.height, &image.rgba)));
        }
        Ok(written)
    }

    /// Replaces the clipboard with `flavors`, best first, unless fenced. What
    /// [`holds_ours`](Self::holds_ours) compares is the flavour the backend reports it wrote.
    pub fn write(&mut self, flavors: &[Flavor]) -> Result<Written, ClipboardError> {
        if self.synthetic {
            return Ok(Written::Fenced);
        }
        let wrote = self.backend.write(flavors)?;
        self.ours = flavors.get(wrote).map(|f| Ours::Flavor { mime: f.mime.to_string(), hash: hash_bytes(&f.data) });
        Ok(Written::Written)
    }

    /// Clipboard text; text that isn't UTF-8 is `None` (PhotoCraft `clipboard_text`).
    // pc: apps/photocraft/src/services.rs:611 clipboard_text
    pub fn read_text(&mut self) -> Result<Option<String>, ClipboardError> {
        Ok(self.read(&[TEXT])?.and_then(|flavor| String::from_utf8(flavor.data).ok()))
    }

    /// The first of `mimes` the clipboard holds, within [`ClipboardLimits::text_bytes`] for text
    /// types and [`ClipboardLimits::image_bytes`] for the rest.
    pub fn read(&mut self, mimes: &[&str]) -> Result<Option<Flavor>, ClipboardError> {
        let limit = if mimes.iter().all(|mime| is_text(mime)) { self.limits.text_bytes } else { self.limits.image_bytes };
        let Some(flavor) = self.backend.read(mimes, limit)? else { return Ok(None) };
        let cap = if is_text(&flavor.mime) { self.limits.text_bytes } else { self.limits.image_bytes };
        if flavor.data.len() as u64 > cap {
            return Err(ClipboardError::TooLarge { limit: cap });
        }
        Ok(Some(flavor))
    }

    /// The clipboard's bitmap. An image that isn't [`ClipboardImage::is_whole`] reads as `None`.
    pub fn read_image(&mut self) -> Result<Option<ClipboardImage>, ClipboardError> {
        let limit = self.limits.image_bytes;
        let Some(image) = self.backend.read_image(limit)? else { return Ok(None) };
        if image.rgba.len() as u64 > limit {
            return Err(ClipboardError::TooLarge { limit });
        }
        Ok(Some(image).filter(ClipboardImage::is_whole))
    }

    /// Copied files, in the order the file manager listed them.
    pub fn read_files(&mut self) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        let limit = self.limits.files;
        let Some(files) = self.backend.read_files(limit)? else { return Ok(None) };
        if files.len() > limit {
            return Err(ClipboardError::TooLarge { limit: limit as u64 });
        }
        Ok(Some(files).filter(|files| !files.is_empty()))
    }

    /// Does the clipboard hold one of `mimes`? For Paste's enabled state on the web and in
    /// tests; on the desktop a [`ClipboardProbe`] asks off the UI thread.
    pub fn has(&mut self, mimes: &[&str]) -> bool {
        self.backend.has(mimes)
    }

    /// Does the clipboard still hold what this app wrote last (nothing copied since, here or in
    /// another app)? Reads the clipboard's flavour of the type the backend wrote last and compares
    /// it with what was written (an image by [`image_signature`]); for a pasted image the app
    /// already read, [`is_ours`](Self::is_ours) answers without another read.
    // vectorcraft: crates/ui-egui/src/sysclip.rs:25 SystemClipboard::holds_ours
    pub fn holds_ours(&mut self) -> bool {
        match &self.ours {
            None => false,
            Some(Ours::Image(signature)) => {
                let signature = *signature;
                let limit = self.limits.image_bytes;
                matches!(self.backend.read_image(limit), Ok(Some(image)) if image_signature(image.width, image.height, &image.rgba) == signature)
            }
            Some(Ours::Flavor { mime, hash }) => {
                let (mime, hash) = (mime.clone(), *hash);
                let limit = if is_text(&mime) { self.limits.text_bytes } else { self.limits.image_bytes };
                matches!(self.backend.read(&[mime.as_str()], limit), Ok(Some(flavor)) if hash_bytes(&flavor.data) == hash)
            }
        }
    }

    /// Whether `image`, already read, is the one this app wrote last: PhotoCraft's check that a
    /// pasted bitmap is its own copy (paste the in-app clipboard with its layers) or another
    /// app's, without a second read.
    // pc: crates/ui-egui/src/app/clipboard.rs:69 import_os_clipboard (os_clip_sig == sig)
    pub fn is_ours(&self, image: &ClipboardImage) -> bool {
        matches!(self.ours, Some(Ours::Image(signature)) if signature == image_signature(image.width, image.height, &image.rgba))
    }

    /// A paste event's flavours for the backend ([`ClipboardBackend::offer`]); the web shell
    /// calls it with [`flavors_from_event`] before the paste command runs.
    pub fn offer_paste(&mut self, flavors: Vec<Flavor>) {
        self.backend.offer(flavors);
    }

    /// Forgets the last write's signature (the in-app copy was cleared).
    pub fn forget_ours(&mut self) {
        self.ours = None;
    }

    /// The backend, for calls the wrapper does not forward.
    pub fn backend_mut(&mut self) -> &mut dyn ClipboardBackend {
        &mut *self.backend
    }
}

/// A cheap fingerprint of an image: its size and every 997th byte, rotated in (PhotoCraft
/// `clip_signature`). Two images with the same signature are taken to be the same copy.
// pc: crates/ui-egui/src/app/clipboard.rs:3 clip_signature
pub fn image_signature(width: u32, height: u32, rgba: &[u8]) -> u64 {
    let mut signature = u64::from(width) << 32 | u64::from(height);
    for byte in rgba.iter().step_by(997) {
        signature = signature.rotate_left(5) ^ u64::from(*byte);
    }
    signature
}

/// Text types read under [`ClipboardLimits::text_bytes`]: plain, HTML and RTF text and SVG markup.
pub(super) fn is_text(mime: &str) -> bool {
    mime.starts_with("text/") || mime == SVG
}

pub(super) fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}
