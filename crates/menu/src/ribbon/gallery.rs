//! `Gallery`: an in-ribbon gallery with stepping and a More button.

use std::marker::PhantomData;

use craft_keymap::CommandRef;
use craft_ui::contract::ContextMenuRequest;
use craft_ui::primitives::textures::Revision;
use craft_ui::primitives::{ContentPainterCx, ThumbSource, Thumbnails};
use craft_ui::runtime::env::Label;
use egui::{ColorImage, Rect, Ui};

/// An in-ribbon gallery of tiles with stepping and More. The tile count follows the group's
/// `GroupLevel`. `Image` faces draw from `Picture`'s store, which packs, so packed previews need
/// no other face.
#[must_use = "show the gallery with `show`"]
pub struct Gallery<'a, K> {
    stub: PhantomData<(&'a (), K)>,
}

impl<'a, K> Gallery<'a, K> {
    /// A gallery named `label` over `items`, with `current` marked.
    pub fn new(label: impl Into<Label>, items: &'a [GalleryItem<'a, K>], current: Option<&'a K>) -> Self {
        let _ = (label.into(), items, current);
        todo!("rb.R13")
    }

    /// How many tiles show at the full and the compact level, each at least 1.
    pub fn tiles(self, full: u8, compact: u8) -> Self {
        let _ = (self, full, compact);
        todo!("rb.R13")
    }

    /// What the More button opens.
    pub fn more(self, more: GalleryMore<'a>) -> Self {
        let _ = (self, more);
        todo!("rb.R13")
    }

    /// Shows the gallery.
    pub fn show(self, ui: &mut Ui) -> GalleryOutput<K> {
        let _ = (self, ui);
        todo!("rb.R13")
    }
}

/// One tile of a gallery.
pub struct GalleryItem<'a, K> {
    /// What the tile picks.
    pub key: K,
    /// The tile's name.
    pub label: Label,
    /// How the tile is drawn.
    pub face: GalleryFace<'a>,
}

/// How a gallery tile is drawn. Each tile builds its `Picture` from the face when it shows.
pub enum GalleryFace<'a> {
    /// An image from `Picture`'s store, made again when `revision` changes.
    Image {
        /// The store key.
        key: u64,
        /// The image's revision.
        revision: Revision,
        /// Makes the image.
        image: &'a dyn Fn() -> ColorImage,
    },
    /// A rendered thumbnail.
    Rendered {
        /// The thumbnail key.
        key: u64,
        /// The thumbnail store.
        thumbnails: &'a Thumbnails<u64>,
        /// What renders the thumbnail.
        source: &'a dyn ThumbSource<u64>,
    },
    /// A tile painted directly.
    Paint(&'a dyn Fn(&ContentPainterCx, Rect)),
}

/// What the More button of a gallery opens.
pub enum GalleryMore<'a> {
    /// A grid of every tile.
    Grid,
    /// The command that opens the app's own surface.
    Command(CommandRef),
    /// The app's menu body.
    Body(&'a mut dyn FnMut(&mut Ui)),
}

/// What a gallery reports for a pass.
pub struct GalleryOutput<K> {
    /// The tile picked this pass.
    pub picked: Option<K>,
    /// A context menu was asked for on a tile.
    pub menu: Option<ContextMenuRequest<K>>,
}
