//! A clipboard fed by browser clipboard events.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use super::backend::ClipboardBackend;
use super::convert::{parse_uri_list, rgba8_image};
use super::types::{BITMAP, ClipboardError, ClipboardImage, Flavor, RGBA8, URI_LIST};

/// A clipboard fed by the app from paste events: the browser's (its Paste event carries text,
/// pictures and files; a page can't read the clipboard on its own) and any host that delivers
/// pastes as events. Writes go out through `out`.
///
/// [`offer`](Self::offer) holds one paste's flavours for the pass it was offered in. The
/// clipboard counts passes itself (`pass`), so a menu Paste in a later pass finds nothing and
/// [`Clipboard::has`](crate::clipboard::Clipboard::has) is false: the browser can't be read outside a paste event. The app does
/// nothing at the end of a pass.
pub struct EventClipboard {
    pasted: Vec<Flavor>,
    offered_in: u64,
    pass: PassClock,
    out: CopyOut,
    late: LateOffers,
}

/// What a late paste needs: the queue and the wake that tells the app to look at it.
struct LateInner {
    queue: Mutex<VecDeque<Vec<Flavor>>>,
    wake: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

/// The queue of an [`EventClipboard`](crate::clipboard::EventClipboard) for a paste the browser delivers outside a pass; `Send + Sync`.
#[derive(Clone)]
pub struct LateOffers {
    inner: Arc<LateInner>,
}

impl LateOffers {
    /// Queues a paste's flavours and wakes the app.
    pub fn offer(&self, flavors: Vec<Flavor>) {
        self.inner.queue.lock().unwrap_or_else(PoisonError::into_inner).push_back(flavors);
        if let Some(wake) = self.inner.wake.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            wake();
        }
    }
}

/// The number of the host's current pass; it changes when a pass ends.
pub(super) type PassClock = Box<dyn Fn() -> u64 + Send>;

/// Where an [`EventClipboard`](crate::clipboard::EventClipboard)'s writes go.
pub(super) type CopyOut = Box<dyn FnMut(&[Flavor]) -> Result<usize, ClipboardError> + Send>;

impl EventClipboard {
    /// `pass` answers the host's pass number (any counter that changes when a pass ends). `out`
    /// puts the flavours where the host's copy goes and answers the index of the one it wrote
    /// ([`ClipboardBackend::write`](crate::clipboard::ClipboardBackend::write)).
    pub fn new(pass: impl Fn() -> u64 + Send + 'static, out: impl FnMut(&[Flavor]) -> Result<usize, ClipboardError> + Send + 'static) -> Self {
        let late = LateOffers { inner: Arc::new(LateInner { queue: Mutex::new(VecDeque::new()), wake: Mutex::new(None) }) };
        Self { pasted: Vec::new(), offered_in: 0, pass: Box::new(pass), out: Box::new(out), late }
    }

    /// The handle a thread outside the pass queues a late paste through.
    pub fn late_offers(&self) -> LateOffers {
        self.late.clone()
    }

    /// Runs `wake` when a late paste is queued, so the app takes it on its next pass.
    pub fn with_wake(self, wake: impl Fn() + Send + Sync + 'static) -> Self {
        *self.late.inner.wake.lock().unwrap_or_else(PoisonError::into_inner) = Some(Box::new(wake));
        self
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

    fn readable_any_time(&self) -> bool {
        false
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

    /// Takes the oldest queued late paste as this pass's offer, one per call.
    fn take_arrived(&mut self) -> bool {
        let next = self.late.inner.queue.lock().unwrap_or_else(PoisonError::into_inner).pop_front();
        match next {
            Some(flavors) => {
                self.offer(flavors);
                true
            }
            None => false,
        }
    }

    fn formats(&mut self) -> Vec<String> {
        if (self.pass)() != self.offered_in {
            return Vec::new();
        }
        self.pasted.iter().map(|flavor| flavor.mime.to_string()).collect()
    }

    /// Holds one paste's flavours, best first, until the pass it was offered in ends.
    fn offer(&mut self, flavors: Vec<Flavor>) {
        self.pasted = flavors;
        self.offered_in = (self.pass)();
    }
}
