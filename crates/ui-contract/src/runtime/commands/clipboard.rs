//! The clipboard handle `Commands::clipboard` returns, over `runtime::egui::clipboard`.

use crate::runtime::egui::clipboard;
use crate::runtime::keys;
use crate::runtime::pending::Pending;
use craft_platform::clipboard::{ClipboardError, ClipboardImage, Flavor, Written};
use egui::{ColorImage, Context, Id};
use std::path::PathBuf;
use std::sync::Arc;

/// One clipboard item.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ClipItem {
    /// Plain text.
    Text(Arc<str>),
    /// HTML.
    Html(Arc<str>),
    /// An image.
    Image(Arc<ColorImage>),
    /// Rich text.
    Rtf(Arc<str>),
    /// MathML.
    MathMl(Arc<str>),
    /// An item of an app-defined type.
    #[non_exhaustive]
    Custom {
        /// The MIME type.
        mime: Arc<str>,
        /// The content.
        bytes: Arc<[u8]>,
    },
}

impl ClipItem {
    /// An item of an app-defined type: `bytes` under the MIME type `mime`.
    pub fn custom(mime: impl Into<Arc<str>>, bytes: impl Into<Arc<[u8]>>) -> ClipItem {
        let _ = (mime.into(), bytes.into());
        todo!("pf.C-R3")
    }
}

/// What a paste delivers.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ClipContent {
    /// The clipboard's text, when it holds any.
    pub text: Option<Arc<str>>,
}

/// Why a paste has no content.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ClipError(pub String);

/// The component's paste slot over the system clipboard. egui carries text and images, so `Html` and `Custom`
/// items are not written.
pub struct ClipboardHandle {
    ctx: Context,
    slot: Pending<ClipContent, ClipError>,
}

impl ClipboardHandle {
    pub(crate) fn new(ctx: &Context, owner: Option<Id>) -> Self {
        let slot = match owner.and_then(|id| keys::pasted_text(ctx, id)) {
            Some(text) => Pending::Ready(ClipContent { text: Some(text) }),
            None => Pending::Loading { progress: None },
        };
        ClipboardHandle { ctx: ctx.clone(), slot }
    }

    /// The clipboard handle of `ctx`, over the installed clipboard.
    pub fn of(ctx: &Context) -> ClipboardHandle {
        let _ = ctx;
        todo!("pf.C-R3")
    }

    /// Offers `items` in the caller's order, as one offer, and says which flavour the clipboard kept.
    pub fn offer(&self, items: &[ClipItem]) -> Result<Written, ClipboardError> {
        let _ = items;
        todo!("pf.C-R3")
    }

    /// The first of `mimes` the installed clipboard holds.
    pub fn read(&self, mimes: &[&str]) -> Result<Option<Flavor>, ClipboardError> {
        let _ = mimes;
        todo!("pf.C-R3")
    }

    /// The clipboard's text.
    pub fn read_text(&self) -> Result<Option<String>, ClipboardError> {
        todo!("pf.C-R3")
    }

    /// The clipboard's image as straight-alpha RGBA8 pixels.
    pub fn read_image(&self) -> Result<Option<ClipboardImage>, ClipboardError> {
        todo!("pf.C-R3")
    }

    /// The files the clipboard holds.
    pub fn read_files(&self) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        todo!("pf.C-R3")
    }

    /// Whether the clipboard holds one of `mimes`.
    pub fn has(&self, mimes: &[&str]) -> bool {
        let _ = mimes;
        todo!("pf.C-R3")
    }

    /// Whether the clipboard still holds what this app last offered.
    pub fn holds_ours(&self) -> bool {
        todo!("pf.C-R3")
    }

    /// The MIME names of the formats on the clipboard.
    pub fn formats(&self) -> Vec<String> {
        todo!("pf.C-R3")
    }

    /// Whether a late paste offer arrived this frame; true in every pass of that frame.
    pub fn arrived(&self) -> bool {
        todo!("pf.C-R3")
    }

    /// Writes every flavour of `items` that the clipboard carries.
    pub fn put(&self, items: &[ClipItem]) {
        if !crate::runtime::measure::effect(&self.ctx, "clipboard::put") {
            return;
        }
        for item in items {
            match item {
                ClipItem::Text(text) => clipboard::copy_text(&self.ctx, text.to_string()),
                ClipItem::Image(image) => clipboard::copy_image(&self.ctx, (**image).clone()),
                // P3(platform-w3): pf.C-R3: the new items are written through `offer`.
                ClipItem::Html(_) | ClipItem::Rtf(_) | ClipItem::MathMl(_) | ClipItem::Custom { .. } => {}
            }
        }
    }

    /// The pasted content: ready in the pass that delivers the paste to the focused component that lists Paste.
    pub fn paste(&self) -> &Pending<ClipContent, ClipError> {
        &self.slot
    }
}
