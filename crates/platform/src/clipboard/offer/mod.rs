//! What the system clipboard offers and how it is written, per platform.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(all(unix, not(target_os = "macos")))]
mod unix;
#[cfg(windows)]
mod windows;

use super::backend::Decode;
use super::types::{ClipboardError, Flavor};

/// The platform's offer state: the cached X11 `TARGETS` connection and the macOS change count recorded at a write.
pub(super) struct Offer {
    _state: (),
}

impl Offer {
    /// An offer with nothing cached.
    pub(super) fn new() -> Self {
        Self { _state: () }
    }
}

/// Every flavour the platform can hold in one offer, tagged with `owner`; the index of the first one written.
#[allow(dead_code)] // P3(platform-w2): pf.C-P7
pub(super) fn write(
    handle: &mut arboard::Clipboard,
    offer: &mut Offer,
    flavors: &[Flavor],
    decode: Option<&Decode>,
    owner: &[u8],
) -> Result<usize, ClipboardError> {
    let _ = (handle, offer, flavors, decode, owner);
    todo!("pf.C-P7")
}

/// The clipboard's RTF text, at most `limit` bytes of it.
#[allow(dead_code)] // P3(platform-w2): pf.C-P7
pub(super) fn read_rtf(offer: &mut Offer, limit: u64) -> Result<Option<String>, ClipboardError> {
    let _ = (offer, limit);
    todo!("pf.C-P7")
}

/// The MIME names of the formats on offer.
#[allow(dead_code)] // P3(platform-w2): pf.C-P7
pub(super) fn formats(offer: &mut Offer) -> Vec<String> {
    let _ = offer;
    todo!("pf.C-P7")
}

/// Whether the offer holds one of `mimes`.
#[allow(dead_code)] // P3(platform-w2): pf.C-P7
pub(super) fn has(offer: &mut Offer, mimes: &[&str]) -> bool {
    let _ = (offer, mimes);
    todo!("pf.C-P7")
}

/// Whether this process owns the clipboard, by the `owner` marker; `None` when the platform cannot tell.
#[allow(dead_code)] // P3(platform-w2): pf.C-P7
pub(super) fn owns(offer: &mut Offer, owner: &[u8]) -> Option<bool> {
    let _ = (offer, owner);
    todo!("pf.C-P7")
}
