//! [`FilterTree`]: the folders column's category folders with thumbnails.

use std::borrow::Cow;

use craft_behavior::{DisplayOrder, ItemSource, Selection};
use craft_ui::contract::{
    ActivateOn, Bind, CollectionEvent, NoValue, Outcome, Props as _, SaveKey, SelectMode, ShowCollection as _, ShowContainer as _, Sizing,
};
use craft_ui::layout_components::Stack;
use craft_ui::list::{ThumbAspect, ThumbSize};
use craft_ui::primitives::textures::{Priority, ThumbFit, ThumbPaint, ThumbRequest, ThumbSource, Thumbnails};
use craft_ui::sections::Section;
use craft_ui::tile_grid::{Tile, TileGrid};
use egui::{Response, Ui};

use super::catalog::{FilterIndex, GalleryThumb};

/// A filter thumbnail's shape: PhotoCraft's 80 by 56.
const THUMB_ASPECT: (u16, u16) = (10, 7);

/// A thumbnail's height over its width.
const THUMB_HEIGHT_SHARE: f32 = 0.7;

/// Thumbnails in a row of a folder.
const COLUMNS: usize = 3;

/// The keys of one folder, in menu order.
struct Folder<'a> {
    keys: &'a [String],
    index: &'a FilterIndex,
}

impl DisplayOrder<String> for Folder<'_> {
    fn len(&self) -> usize {
        self.keys.len()
    }

    fn key(&self, at: usize) -> String {
        self.keys[at].clone()
    }

    fn index_of(&self, key: &String) -> Option<usize> {
        self.keys.iter().position(|held| held == key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(self.index.label(&self.keys[at]).as_str().to_owned()))
    }
}

impl ItemSource<String> for Folder<'_> {}

/// What [`FilterTree::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct FilterTreeOutput {
    /// The tree's response.
    pub response: Response,
    /// The filter whose thumbnail was clicked (or picked with Enter or Space).
    pub picked: Option<String>,
}

impl Outcome for FilterTreeOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The folders column: one collapsible folder per category (its open state remembered; the folder of the
/// filter the tree opens on starts open), each a `TileGrid` of the folder's filters, with the filter's name
/// under its thumbnail. The current filter's tile is the grid's selection. Thumbnails come from a
/// `Thumbnails<GalleryThumb>` cache over the app's `ThumbSource` (the placeholder while loading).
///
/// Keyboard: the grid's keys; arrows move, Enter or Space picks. A click picks.
#[derive(craft_ui::contract::Pattern)]
#[must_use = "show the tree with `show`"]
pub struct FilterTree<'a> {
    index: &'a FilterIndex,
    selected: &'a str,
    thumbs: &'a Thumbnails<GalleryThumb>,
    source: &'a dyn ThumbSource<GalleryThumb>,
}

impl<'a> FilterTree<'a> {
    /// The folders of `index`, with the filter `selected` current; `source` renders the pictures `thumbs`
    /// asks for.
    pub fn new(index: &'a FilterIndex, selected: &'a str, thumbs: &'a Thumbnails<GalleryThumb>, source: &'a dyn ThumbSource<GalleryThumb>) -> Self {
        Self { index, selected, thumbs, source }
    }

    /// Draws the category folders and filter thumbnails and reports the filter picked.
    pub fn show(self, ui: &mut Ui) -> FilterTreeOutput {
        let Self { index, selected, thumbs, source } = self;
        let mut picked = None;
        let stack = Stack::new().show(ui, |ui, _| {
            for category in index.categories() {
                let current = category.filters.iter().any(|key| key == selected);
                let key = SaveKey::named(format!("craft_imaging.filter_gallery.{}", category.key));
                Section::new(category.label.clone()).id_salt(category.key.as_str()).initially_open(current).remember(key).show(ui, |ui, _| {
                    let folder = Folder { keys: &category.filters, index };
                    let mut selection = if current { Selection::single(selected.to_owned()) } else { Selection::default() };
                    let out = TileGrid::<String>::new(category.label.clone())
                        .thumb_size(ThumbSize::Large)
                        .aspect(1.0 / THUMB_HEIGHT_SHARE)
                        .card_columns(COLUMNS)
                        .sizing(Sizing::Content)
                        .select(SelectMode::Single)
                        .activate(ActivateOn::Click)
                        .natural_height(true)
                        .selection(Bind::bound(&mut selection))
                        .show_items(ui, &folder, &mut |key, _| {
                            let filter = GalleryThumb::new(key.clone());
                            let (wide, high) = THUMB_ASPECT;
                            Tile::new(index.label(key)).thumb(ThumbAspect::new(wide, high), move |cx, rect| {
                                let ctx = cx.painter.ctx();
                                let request = ThumbRequest::for_cell(rect.width(), ctx.pixels_per_point());
                                let state = thumbs.state(ctx, source, &filter, &request, Priority::VISIBLE);
                                thumbs.paint(cx, rect, &state, &ThumbPaint::default().fit(ThumbFit::Cover).checker(false));
                            })
                        });
                    for event in out.response.events {
                        if let CollectionEvent::Activate(key) = event {
                            picked = Some(key);
                        }
                    }
                });
            }
        });
        FilterTreeOutput { response: stack.output, picked }
    }
}
