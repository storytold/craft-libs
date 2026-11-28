//! Clipboard backends: the trait every clipboard implements and the OS clipboard.

use std::path::PathBuf;

#[cfg(all(feature = "clipboard-system", not(target_arch = "wasm32")))]
use super::convert::{trim_cr, uri_list};
#[cfg(all(feature = "clipboard-system", not(target_arch = "wasm32")))]
use super::offer;
#[cfg(all(feature = "clipboard-system", not(target_arch = "wasm32")))]
use super::types::{BITMAP, HTML, RGBA8, RTF, TEXT, URI_LIST};
use super::types::{ClipboardError, ClipboardImage, Flavor};

/// The slow part of a read, run off the UI thread ([`ClipboardBackend::read_later`]); `None` when it reads nothing.
pub type LaterRead = Box<dyn FnOnce() -> Option<Flavor> + Send>;

/// What a write did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum Written {
    /// The clipboard holds the new contents.
    Written,
    /// Dropped: the write came from synthetic input ([`Clipboard::set_synthetic`](crate::clipboard::Clipboard::set_synthetic)), and the
    /// clipboard keeps what the user copied.
    Fenced,
}

/// Where clipboard bytes go: the OS clipboard, the browser's, or a test's.
///
/// `Send` so a [`ClipboardProbe`](crate::clipboard::ClipboardProbe) can own one on its worker thread. Every method may block on a
/// slow clipboard owner (an X11 client that never answers); the UI thread calls only
/// [`Clipboard`](crate::clipboard::Clipboard)'s reads and writes, on a user's copy or paste.
pub trait ClipboardBackend: Send {
    /// Replaces the clipboard's contents with `flavors`, best first, as one offer; none empties
    /// it. A backend may hold fewer flavours than an app offers, so it answers the index in
    /// `flavors` of the one it wrote (`0` when it emptied the clipboard);
    /// [`Clipboard::holds_ours`](crate::clipboard::Clipboard::holds_ours) compares that one.
    fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError>;
    /// The first of `mimes` the clipboard holds, at most `limit` bytes of it (more is
    /// [`ClipboardError::TooLarge`]). `BITMAP` matches any bitmap, returned as an `RGBA8` flavor.
    fn read(&mut self, mimes: &[&str], limit: u64) -> Result<Option<Flavor>, ClipboardError>;
    /// The clipboard's bitmap as RGBA8, at most `limit` bytes of pixels.
    fn read_image(&mut self, limit: u64) -> Result<Option<ClipboardImage>, ClipboardError>;
    /// Copied files (Finder, Explorer, GNOME Files), at most `limit` of them.
    fn read_files(&mut self, limit: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError>;
    /// Does the clipboard hold one of `mimes`? Cheap enough to ask a few times a second.
    fn has(&mut self, mimes: &[&str]) -> bool;
    /// As [`read`](Self::read), but the slow part (the Wayland pipe, a DIB conversion) is left in the returned
    /// closure, which runs off the UI thread. Reads now and wraps the result by default.
    fn read_later(&mut self, mimes: &[&str], limit: u64) -> Result<Option<LaterRead>, ClipboardError> {
        Ok(self.read(mimes, limit)?.map(|flavor| Box::new(move || Some(flavor)) as LaterRead))
    }
    /// Whether the clipboard can be read at any time; `false` where it can be read only inside a paste event (the
    /// web).
    fn readable_any_time(&self) -> bool {
        true
    }
    /// The flavours of a paste the host delivered as an event (the browser's Paste). Only
    /// [`EventClipboard`](crate::clipboard::EventClipboard) keeps them; the system clipboard ignores them.
    fn offer(&mut self, _flavors: Vec<Flavor>) {}
    /// The MIME names of the formats on offer.
    fn formats(&mut self) -> Vec<String> {
        Vec::new()
    }
    /// Whether this process owns the clipboard; `None` when the backend cannot tell.
    fn owns(&mut self) -> Option<bool> {
        None
    }
    /// Whether a late paste arrived since the last call.
    fn take_arrived(&mut self) -> bool {
        false
    }
}

/// A boxed backend is the backend it holds, so a host can choose one at run time
/// (`Clipboard::new(services.clipboard)`).
impl<B: ClipboardBackend + ?Sized> ClipboardBackend for Box<B> {
    fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError> {
        (**self).write(flavors)
    }
    fn read(&mut self, mimes: &[&str], limit: u64) -> Result<Option<Flavor>, ClipboardError> {
        (**self).read(mimes, limit)
    }
    fn read_image(&mut self, limit: u64) -> Result<Option<ClipboardImage>, ClipboardError> {
        (**self).read_image(limit)
    }
    fn read_files(&mut self, limit: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        (**self).read_files(limit)
    }
    fn has(&mut self, mimes: &[&str]) -> bool {
        (**self).has(mimes)
    }
    fn read_later(&mut self, mimes: &[&str], limit: u64) -> Result<Option<LaterRead>, ClipboardError> {
        (**self).read_later(mimes, limit)
    }
    fn readable_any_time(&self) -> bool {
        (**self).readable_any_time()
    }
    fn offer(&mut self, flavors: Vec<Flavor>) {
        (**self).offer(flavors);
    }
    fn formats(&mut self) -> Vec<String> {
        (**self).formats()
    }
    fn owns(&mut self) -> Option<bool> {
        (**self).owns()
    }
    fn take_arrived(&mut self) -> bool {
        (**self).take_arrived()
    }
}

/// The desktop's clipboard through arboard and the platform's own offer code: X11 and Wayland,
/// macOS, Windows. Not on wasm.
///
/// The handle opens on first use and stays open (on X11 the last handle alive hands what the app
/// copied to the clipboard manager at exit). When no backend opens, every call answers
/// [`ClipboardError::Unavailable`] and [`unreadable`](Self::unreadable) turns true. On Wayland a
/// bitmap arboard can't read (it asks for `image/png` only; a screenshot tool may offer only
/// `image/jpeg` or `image/bmp`, storytold/photocraft#2546) reads as `None`.
///
/// A write puts every flavour it can hold on the clipboard as one offer, tagged with a token that
/// is unique to this `SystemClipboard`, so [`owns`](ClipboardBackend::owns) can tell its copy from
/// another app's.
#[cfg(all(feature = "clipboard-system", not(target_arch = "wasm32")))]
pub struct SystemClipboard {
    handle: Option<arboard::Clipboard>,
    unreadable: bool,
    decode: Option<Decode>,
    offer: offer::Offer,
    owner: [u8; 16],
}

/// The app's decoder for encoded bitmaps ([`SystemClipboard::decoder`]).
#[cfg(all(feature = "clipboard-system", not(target_arch = "wasm32")))]
pub(crate) type Decode = Box<dyn Fn(&[u8]) -> Option<ClipboardImage> + Send>;

/// A token no other process or `SystemClipboard` shares: random hasher keys, the process id and
/// the time.
#[cfg(all(feature = "clipboard-system", not(target_arch = "wasm32")))]
fn owner_token() -> [u8; 16] {
    use std::hash::{BuildHasher, Hash, Hasher};
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or_default();
    let mut token = [0u8; 16];
    for (half, chunk) in token.chunks_mut(8).enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        half.hash(&mut hasher);
        std::process::id().hash(&mut hasher);
        nanos.hash(&mut hasher);
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    token
}

#[cfg(all(feature = "clipboard-system", not(target_arch = "wasm32")))]
impl SystemClipboard {
    /// A clipboard that connects to the system clipboard on first use.
    pub fn new() -> Self {
        Self { handle: None, unreadable: false, decode: None, offer: offer::Offer::new(), owner: owner_token() }
    }

    /// Decodes a `image/png` flavor to pixels for [`ClipboardBackend::write`](crate::clipboard::ClipboardBackend::write), which hands the
    /// platform pixels only. Without one, a write offers no PNG.
    pub fn decoder(mut self, decode: impl Fn(&[u8]) -> Option<ClipboardImage> + Send + 'static) -> Self {
        self.decode = Some(Box::new(decode));
        self
    }

    /// True when the last attempt to open the clipboard found no backend at all.
    pub fn unreadable(&self) -> bool {
        self.unreadable
    }

    fn open(&mut self) -> Result<&mut arboard::Clipboard, ClipboardError> {
        if self.handle.is_none() {
            match arboard::Clipboard::new() {
                Ok(handle) => {
                    self.unreadable = false;
                    self.handle = Some(handle);
                }
                Err(error) => {
                    log::debug!("no system clipboard: {error}");
                    self.unreadable = true;
                }
            }
        }
        self.handle.as_mut().ok_or(ClipboardError::Unavailable)
    }
}

#[cfg(all(feature = "clipboard-system", not(target_arch = "wasm32")))]
pub(super) fn system_error(error: arboard::Error) -> ClipboardError {
    match error {
        arboard::Error::ClipboardOccupied => ClipboardError::Busy,
        arboard::Error::ClipboardNotSupported => ClipboardError::Unavailable,
        error => ClipboardError::Other(error.to_string()),
    }
}

/// A read's value, or `None` when the clipboard holds nothing in that format. `unconvertible`
/// also counts content the format can't be converted from (text that isn't UTF-8) as nothing.
#[cfg(all(feature = "clipboard-system", not(target_arch = "wasm32")))]
pub(super) fn present<T>(read: Result<T, arboard::Error>, unconvertible: bool) -> Result<Option<T>, ClipboardError> {
    match read {
        Ok(value) => Ok(Some(value)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(arboard::Error::ConversionFailure) if unconvertible => Ok(None),
        Err(error) => Err(system_error(error)),
    }
}

#[cfg(all(feature = "clipboard-system", not(target_arch = "wasm32")))]
impl Default for SystemClipboard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(feature = "clipboard-system", not(target_arch = "wasm32")))]
impl ClipboardBackend for SystemClipboard {
    // pc: apps/photocraft/src/services.rs:702 clipboard_set_image (open on first use, "clipboard is busy")
    fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError> {
        self.open()?;
        let Self { handle, offer, decode, owner, .. } = self;
        let handle = handle.as_mut().ok_or(ClipboardError::Unavailable)?;
        offer::write(handle, offer, flavors, decode.as_ref(), owner)
    }

    // vectorcraft: crates/ui-egui/src/sysclip.rs:27 SystemClipboard::read
    fn read(&mut self, mimes: &[&str], limit: u64) -> Result<Option<Flavor>, ClipboardError> {
        for &mime in mimes {
            let found = match mime {
                TEXT => present(self.open()?.get().text(), true)?.map(|text| Flavor::text(&text)),
                HTML => present(self.open()?.get().html(), true)?.map(|html| Flavor::new(HTML, html.into_bytes())),
                RTF => offer::read_rtf(&mut self.offer, limit)?.map(|text| Flavor::new(RTF, text.into_bytes())),
                BITMAP | RGBA8 => self.read_image(limit)?.map(|image| Flavor::rgba8(&image)),
                URI_LIST => {
                    let files = present(self.open()?.get().file_list(), false)?.filter(|files| !files.is_empty());
                    files.map(|files| Flavor::new(URI_LIST, uri_list(&files, cfg!(windows)).into_bytes()))
                }
                _ => None,
            };
            if let Some(flavor) = found {
                if flavor.data.len() as u64 > limit {
                    return Err(ClipboardError::TooLarge { limit });
                }
                return Ok(Some(flavor));
            }
        }
        Ok(None)
    }

    // pc: apps/photocraft/src/services.rs:714 clipboard_get_image, :536 wayland_clipboard_image, :601 clipboard_read
    fn read_image(&mut self, limit: u64) -> Result<Option<ClipboardImage>, ClipboardError> {
        let Some(image) = present(self.open()?.get_image(), false)? else { return Ok(None) };
        if image.bytes.len() as u64 > limit {
            return Err(ClipboardError::TooLarge { limit });
        }
        Ok(Some(ClipboardImage { width: image.width as u32, height: image.height as u32, rgba: image.bytes.into_owned() }))
    }

    // pc: apps/photocraft/src/services.rs:506 image_from_files (trailing '\r' of text/uri-list lines trimmed)
    fn read_files(&mut self, limit: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        let Some(files) = present(self.open()?.get().file_list(), false)? else { return Ok(None) };
        // arboard splits a text/uri-list on LF only, so a path copied in GNOME Files keeps its CR.
        let files: Vec<PathBuf> = files.into_iter().map(trim_cr).collect();
        if files.len() > limit {
            return Err(ClipboardError::TooLarge { limit: limit as u64 });
        }
        Ok((!files.is_empty()).then_some(files))
    }

    // vectorcraft: crates/ui-egui/src/sysclip.rs:29 SystemClipboard::has
    fn has(&mut self, mimes: &[&str]) -> bool {
        offer::has(&mut self.offer, mimes)
    }

    fn formats(&mut self) -> Vec<String> {
        offer::formats(&mut self.offer)
    }

    fn owns(&mut self) -> Option<bool> {
        offer::owns(&mut self.offer, &self.owner)
    }
}
