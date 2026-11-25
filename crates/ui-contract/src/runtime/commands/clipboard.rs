//! The clipboard handle `Commands::clipboard` returns, over the clipboard `runtime::egui::clipboard` keeps installed.

use crate::runtime::egui::clipboard;
use crate::runtime::keys;
use crate::runtime::pending::Pending;
use craft_platform::clipboard::{ClipboardError, ClipboardImage, Flavor, HTML, MATHML, RTF, Written};
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
        ClipItem::Custom { mime: mime.into(), bytes: bytes.into() }
    }

    /// The flavour the clipboard carries this item as; an image goes as straight-alpha `RGBA8`.
    fn flavor(&self) -> Result<Flavor, ClipboardError> {
        Ok(match self {
            ClipItem::Text(text) => Flavor::text(text),
            ClipItem::Html(html) => Flavor::new(HTML, html.as_bytes().to_vec()),
            ClipItem::Rtf(rtf) => Flavor::new(RTF, rtf.as_bytes().to_vec()),
            ClipItem::MathMl(mathml) => Flavor::new(MATHML, mathml.as_bytes().to_vec()),
            ClipItem::Custom { mime, bytes } => Flavor::new(mime.to_string(), bytes.to_vec()),
            ClipItem::Image(image) => {
                let image = clipboard::clipboard_image(image);
                if !image.is_whole() {
                    return Err(ClipboardError::BadImage);
                }
                Flavor::rgba8(&image)
            }
        })
    }
}

/// What a paste delivers.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ClipContent {
    /// The clipboard's text, when it holds any.
    pub text: Option<Arc<str>>,
    /// The clipboard's image, when it holds one.
    pub image: Option<Arc<ColorImage>>,
}

/// Why a paste has no content.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ClipError(pub String);

/// The component's paste slot over the installed clipboard (the host's, else egui's). Every call goes to the
/// clipboard installed in the context; the handle holds nothing else.
pub struct ClipboardHandle {
    ctx: Context,
    slot: Pending<ClipContent, ClipError>,
}

impl ClipboardHandle {
    pub(crate) fn new(ctx: &Context, owner: Option<Id>) -> Self {
        let slot = match owner.and_then(|id| keys::pasted(ctx, id)) {
            Some(content) => Pending::Ready(content),
            None => Pending::Loading { progress: None },
        };
        ClipboardHandle { ctx: ctx.clone(), slot }
    }

    /// The clipboard handle of `ctx`, over the installed clipboard, for code outside a component; its paste slot
    /// stays loading, as no component takes the paste.
    pub fn of(ctx: &Context) -> ClipboardHandle {
        ClipboardHandle::new(ctx, None)
    }

    /// Offers `items` in the caller's order, as one offer, and says which flavour the clipboard kept. A measuring run
    /// writes nothing and answers `Written::Fenced`.
    pub fn offer(&self, items: &[ClipItem]) -> Result<Written, ClipboardError> {
        if !crate::runtime::measure::effect(&self.ctx, "clipboard::offer") {
            return Ok(Written::Fenced);
        }
        let flavors = items.iter().map(ClipItem::flavor).collect::<Result<Vec<_>, _>>()?;
        clipboard::with(&self.ctx, |clipboard| clipboard.write(&flavors))
    }

    /// The first of `mimes` the installed clipboard holds.
    pub fn read(&self, mimes: &[&str]) -> Result<Option<Flavor>, ClipboardError> {
        clipboard::with(&self.ctx, |clipboard| clipboard.read(mimes))
    }

    /// The clipboard's text.
    pub fn read_text(&self) -> Result<Option<String>, ClipboardError> {
        clipboard::with(&self.ctx, |clipboard| clipboard.read_text())
    }

    /// The clipboard's image as straight-alpha RGBA8 pixels; `ColorImage::from_rgba_unmultiplied` turns them back.
    pub fn read_image(&self) -> Result<Option<ClipboardImage>, ClipboardError> {
        clipboard::with(&self.ctx, |clipboard| clipboard.read_image())
    }

    /// The files the clipboard holds.
    pub fn read_files(&self) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        clipboard::with(&self.ctx, |clipboard| clipboard.read_files())
    }

    /// Whether the clipboard holds one of `mimes`.
    pub fn has(&self, mimes: &[&str]) -> bool {
        clipboard::with(&self.ctx, |clipboard| clipboard.has(mimes))
    }

    /// Whether the clipboard still holds what this app last offered.
    pub fn holds_ours(&self) -> bool {
        clipboard::with(&self.ctx, |clipboard| clipboard.holds_ours())
    }

    /// The MIME names of the formats on the clipboard.
    pub fn formats(&self) -> Vec<String> {
        clipboard::with(&self.ctx, |clipboard| clipboard.formats())
    }

    /// Whether a late paste offer arrived this frame; true in every pass of that frame.
    pub fn arrived(&self) -> bool {
        clipboard::arrived(&self.ctx)
    }

    /// Offers `items` as one offer ([`offer`](Self::offer)), for a caller that does not ask what was kept.
    pub fn put(&self, items: &[ClipItem]) {
        let _ = self.offer(items);
    }

    /// The pasted content: ready in the pass that delivers the paste to the focused component that lists Paste.
    pub fn paste(&self) -> &Pending<ClipContent, ClipError> {
        &self.slot
    }
}
