//! `LayerThumb`, `LayerThumbKind` and `ThumbContents`: the thumbnail key of a row (F11.02).
//!
//! The panel asks [`Thumbnails`](craft_ui::primitives::textures::Thumbnails) for one picture per
//! thumbnail of a row, keyed by a [`LayerThumb`]; the app implements
//! [`ThumbSource<LayerThumb<Id>>`](craft_ui::primitives::textures::ThumbSource) and renders what the
//! key names:
//!
//! | Kind | Picture | Frame |
//! |---|---|---|
//! | `Pixels` | the layer's pixels over transparency (the checkerboard shows through) | `contents`: the canvas, or the layer's bounds clipped to the canvas (the canvas when empty there) |
//! | `Mask` | the mask as opaque grey | the canvas |
//! | `VectorMask` | white where it reveals, grey 128 where it hides (128 + 127 v); disabled masks still show their shape | the canvas |
//! | `SmartFilterMask` | as `Mask` | the canvas |
//!
//! Groups, adjustments and type layers answer [`ThumbRender::Blank`](craft_ui::primitives::textures::ThumbRender::Blank)
//! and the row paints their kind tile; solid and gradient fills answer a small `Ready` picture.
//! The revision covers pixels only, so a selection-only step keeps the thumbnails.

use super::tree::LayerPart;

/// What a layer's thumbnail frames (Panel Options › Thumbnail Contents; the panel menu's Clip
/// Thumbnails to Layer Bounds is the same setting).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum ThumbContents {
    /// The layer's content bounds, clipped to the canvas.
    LayerBounds,
    /// The whole canvas (PhotoCraft's and Photoshop's default).
    #[default]
    EntireDocument,
}

/// Which picture of a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LayerThumbKind {
    /// The layer's own pixels.
    Pixels,
    /// The pixel mask.
    Mask,
    /// The vector mask.
    VectorMask,
    /// A smart object's shared Smart Filter mask.
    SmartFilterMask,
}

/// The key of one thumbnail: the layer, which picture, and the frame. Changing Thumbnail Contents
/// keeps both pictures cached, since the setting is part of the key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct LayerThumb<L> {
    /// The layer.
    pub layer: L,
    /// Which picture.
    pub kind: LayerThumbKind,
    /// The frame; always [`ThumbContents::EntireDocument`] for the mask kinds.
    pub contents: ThumbContents,
}

impl<L> LayerThumb<L> {
    /// The layer's pixels, framed by `contents`.
    pub fn pixels(layer: L, contents: ThumbContents) -> Self {
        Self { layer, kind: LayerThumbKind::Pixels, contents }
    }

    /// The pixel mask, framed by the canvas.
    pub fn mask(layer: L) -> Self {
        Self { layer, kind: LayerThumbKind::Mask, contents: ThumbContents::EntireDocument }
    }

    /// The vector mask, framed by the canvas.
    pub fn vector_mask(layer: L) -> Self {
        Self { layer, kind: LayerThumbKind::VectorMask, contents: ThumbContents::EntireDocument }
    }

    /// The shared Smart Filter mask, framed by the canvas.
    pub fn filter_mask(layer: L) -> Self {
        Self { layer, kind: LayerThumbKind::SmartFilterMask, contents: ThumbContents::EntireDocument }
    }

    /// The thumbnail of `part`: [`pixels`](Self::pixels) for [`LayerPart::Pixels`], else the
    /// matching mask.
    pub fn of_part(layer: L, part: LayerPart, contents: ThumbContents) -> Self {
        match part {
            LayerPart::Pixels => Self::pixels(layer, contents),
            LayerPart::LayerMask => Self::mask(layer),
            LayerPart::VectorMask => Self::vector_mask(layer),
            LayerPart::FilterMask => Self::filter_mask(layer),
        }
    }
}
