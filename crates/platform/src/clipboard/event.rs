//! A clipboard fed by browser clipboard events.

use std::path::PathBuf;

use super::*;

/// A clipboard fed by the app from paste events: the browser's (its Paste event carries text,
/// pictures and files; a page can't read the clipboard on its own) and any host that delivers
/// pastes as events. Writes go out through `out`.
///
/// [`offer`](Self::offer) holds one paste's flavours for the pass it was offered in. The
/// clipboard counts passes itself (`pass`), so a menu Paste in a later pass finds nothing and
/// [`Clipboard::has`] is false: the browser can't be read outside a paste event. The app does
/// nothing at the end of a pass.
pub struct EventClipboard {
    pasted: Vec<Flavor>,
    offered_in: u64,
    pass: PassClock,
    out: CopyOut,
}

/// The number of the host's current pass; it changes when a pass ends.
pub(super) type PassClock = Box<dyn Fn() -> u64 + Send>;

/// Where an [`EventClipboard`]'s writes go.
pub(super) type CopyOut = Box<dyn FnMut(&[Flavor]) -> Result<usize, ClipboardError> + Send>;

impl EventClipboard {
    /// `pass` answers the host's pass number (any counter that changes when a pass ends). `out`
    /// puts the flavours where the host's copy goes and answers the index of the one it wrote
    /// ([`ClipboardBackend::write`]).
    pub fn new(pass: impl Fn() -> u64 + Send + 'static, out: impl FnMut(&[Flavor]) -> Result<usize, ClipboardError> + Send + 'static) -> Self {
        Self { pasted: Vec::new(), offered_in: 0, pass: Box::new(pass), out: Box::new(out) }
    }

    /// The first of `mimes` this pass's paste offered; [`BITMAP`] is any offered `image/*`.
    fn offered(&self, mimes: &[&str]) -> Option<&Flavor> {
        if (self.pass)() != self.offered_in {
            return None;
        }
        mimes.iter().find_map(|&mime| self.pasted.iter().find(|f| if mime == BITMAP { f.mime.starts_with("image/") } else { f.mime == mime }))
    }
}

impl ClipboardBackend for EventClipboard {
    // new: the browser has no clipboard read; PhotoCraft's web shell maps Event::PasteImage (apps/photocraft-web/src/web.rs:344)
    fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError> {
        (self.out)(flavors)
    }

    // new: first offered flavour among `mimes`, limit checked
    fn read(&mut self, mimes: &[&str], limit: u64) -> Result<Option<Flavor>, ClipboardError> {
        let Some(flavor) = self.offered(mimes) else { return Ok(None) };
        if flavor.data.len() as u64 > limit {
            return Err(ClipboardError::TooLarge { limit });
        }
        Ok(Some(flavor.clone()))
    }

    // new: only an offered RGBA8 flavour reads (no codec here); an offered PNG is the app's to decode through `read`
    fn read_image(&mut self, limit: u64) -> Result<Option<ClipboardImage>, ClipboardError> {
        let Some(image) = self.offered(&[RGBA8]).and_then(|flavor| rgba8_image(&flavor.data)) else { return Ok(None) };
        if image.rgba.len() as u64 > limit {
            return Err(ClipboardError::TooLarge { limit });
        }
        Ok(Some(image))
    }

    // new: an offered text/uri-list, one file:// URI per line
    fn read_files(&mut self, limit: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        let Some(flavor) = self.offered(&[URI_LIST]) else { return Ok(None) };
        let files = parse_uri_list(&String::from_utf8_lossy(&flavor.data));
        if files.len() > limit {
            return Err(ClipboardError::TooLarge { limit: limit as u64 });
        }
        Ok((!files.is_empty()).then_some(files))
    }

    fn has(&mut self, mimes: &[&str]) -> bool {
        self.offered(mimes).is_some()
    }

    /// Holds one paste's flavours, best first, until the pass it was offered in ends.
    fn offer(&mut self, flavors: Vec<Flavor>) {
        self.pasted = flavors;
        self.offered_in = (self.pass)();
    }
}
