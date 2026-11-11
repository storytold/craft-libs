//! The clipboard handle `Commands::clipboard` returns, over `runtime::egui::clipboard`.

use crate::runtime::egui::clipboard;
use crate::runtime::keys;
use crate::runtime::pending::Pending;
use egui::{ColorImage, Context, Id};
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
    /// An item of an app-defined type.
    #[non_exhaustive]
    Custom {
        /// The MIME type.
        mime: Arc<str>,
        /// The content.
        bytes: Arc<[u8]>,
    },
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

    /// Writes every flavour of `items` that the clipboard carries.
    pub fn put(&self, items: &[ClipItem]) {
        if !crate::runtime::measure::effect(&self.ctx, "clipboard::put") {
            return;
        }
        for item in items {
            match item {
                ClipItem::Text(text) => clipboard::copy_text(&self.ctx, text.to_string()),
                ClipItem::Image(image) => clipboard::copy_image(&self.ctx, (**image).clone()),
                ClipItem::Html(_) | ClipItem::Custom { .. } => {}
            }
        }
    }

    /// The pasted content: ready in the pass that delivers the paste to the focused component that lists Paste.
    pub fn paste(&self) -> &Pending<ClipContent, ClipError> {
        &self.slot
    }
}
