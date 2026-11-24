//! [`FilterTree`]: the folders column's category folders with thumbnails.

use std::borrow::Cow;

use craft_behavior::{DisplayOrder, ItemSource, Selection};
use craft_ui::contract::{
    ActivateOn, Bind, CollectionEvent, NoValue, Outcome, Props as _, SaveKey, SelectMode, ShowCollection as _, ShowContainer as _, Sizing,
};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow, ThumbAspect, ThumbSize};
use craft_ui::primitives::ContentPainterCx;
use craft_ui::primitives::textures::{Priority, ThumbFit, ThumbPaint, ThumbRequest, ThumbSource, Thumbnails};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::sections::Section;
use craft_ui::tile_grid::{Tile, TileGrid};
use egui::{Rect, Response, Ui};

use super::catalog::{FilterCategory, FilterIndex, GalleryThumb};
use super::words::FilterGalleryWords;

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

/// The categories that hold filters, in menu order; a key is the category's.
struct Categories<'a> {
    listed: Vec<&'a FilterCategory>,
}

impl Categories<'_> {
    fn get(&self, key: &str) -> Option<&FilterCategory> {
        self.listed.iter().copied().find(|category| category.key == key)
    }
}

impl DisplayOrder<String> for Categories<'_> {
    fn len(&self) -> usize {
        self.listed.len()
    }

    fn key(&self, at: usize) -> String {
        self.listed[at].key.clone()
    }

    fn index_of(&self, key: &String) -> Option<usize> {
        self.listed.iter().position(|category| category.key == *key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.listed[at].label.as_str()))
    }
}

impl ItemSource<String> for Categories<'_> {}

/// Paints the thumbnail of `filter` from the cache, covering the cell: the one painter of the tile and of the list row.
fn thumb_painter<'a>(
    thumbs: &'a Thumbnails<GalleryThumb>,
    source: &'a dyn ThumbSource<GalleryThumb>,
    filter: GalleryThumb,
) -> impl FnOnce(&ContentPainterCx, Rect) + 'a {
    move |cx, rect| {
        let ctx = cx.painter.ctx();
        let request = ThumbRequest::for_cell(rect.width(), ctx.pixels_per_point());
        let state = thumbs.state(ctx, source, &filter, &request, Priority::VISIBLE);
        thumbs.paint(cx, rect, &state, &ThumbPaint::default().fit(ThumbFit::Cover).checker(false));
    }
}

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

/// How a [`FilterTree`] lays out the categories and filters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FilterTreeLayout {
    /// The category folders of tile grids.
    #[default]
    Folders,
    /// A categories list beside a filters list, each filter row with its thumbnail; the current filter's category is
    /// selected, a category press picks its first filter, a filter row press, Enter or Space picks it, and empty
    /// categories are not listed.
    Lists,
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
    layout: FilterTreeLayout,
}

impl<'a> FilterTree<'a> {
    /// The folders of `index`, with the filter `selected` current; `source` renders the pictures `thumbs`
    /// asks for.
    pub fn new(index: &'a FilterIndex, selected: &'a str, thumbs: &'a Thumbnails<GalleryThumb>, source: &'a dyn ThumbSource<GalleryThumb>) -> Self {
        Self { index, selected, thumbs, source, layout: FilterTreeLayout::default() }
    }

    /// How the categories and filters are laid out; the folders by default.
    pub fn layout(mut self, layout: FilterTreeLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Draws the categories and filters in the chosen layout and reports the filter picked.
    pub fn show(self, ui: &mut Ui) -> FilterTreeOutput {
        match self.layout {
            FilterTreeLayout::Folders => self.show_folders(ui),
            FilterTreeLayout::Lists => self.show_lists(ui),
        }
    }

    /// A categories list beside a filters list of the current filter's category. A category press picks its first
    /// filter, except for the current category, whose press changes nothing.
    fn show_lists(self, ui: &mut Ui) -> FilterTreeOutput {
        let Self { index, selected, thumbs, source, .. } = self;
        let words = Env::of(ui).words::<FilterGalleryWords>();
        let categories = Categories { listed: index.categories().iter().filter(|category| !category.filters.is_empty()).collect() };
        let current = categories
            .listed
            .iter()
            .copied()
            .find(|category| category.filters.iter().any(|key| key == selected))
            .or_else(|| categories.listed.first().copied());
        let mut picked = None;
        let row = Row::new().show(ui, |ui, _| {
            let Some(current) = current else { return };
            let mut chosen = Selection::single(current.key.clone());
            let out = List::<String>::new(words.categories.clone())
                .select(SelectMode::Single)
                .activate(ActivateOn::Click)
                .natural_height(true)
                .selection(Bind::bound(&mut chosen))
                .show_items(ui, &categories, &mut |key, _| {
                    ListRow::new(categories.get(key).map_or_else(|| Label::from(key.clone()), |category| category.label.clone()))
                });
            for event in out.events {
                if let CollectionEvent::Activate(key) = event
                    && key != current.key
                {
                    picked = categories.get(&key).and_then(|category| category.filters.first().cloned());
                }
            }

            let folder = Folder { keys: &current.filters, index };
            let mut chosen =
                if current.filters.iter().any(|key| key == selected) { Selection::single(selected.to_owned()) } else { Selection::default() };
            let (wide, high) = THUMB_ASPECT;
            let out = List::<String>::new(words.filters.clone())
                .select(SelectMode::Single)
                .activate(ActivateOn::Click)
                .natural_height(true)
                .selection(Bind::bound(&mut chosen))
                .show_items(ui, &folder, &mut |key, _| {
                    ListRow::new(index.label(key)).thumb_aspect(
                        ThumbSize::Large,
                        ThumbAspect::new(wide, high),
                        thumb_painter(thumbs, source, GalleryThumb::new(key.clone())),
                    )
                });
            for event in out.events {
                if let CollectionEvent::Activate(key) = event {
                    picked = Some(key);
                }
            }
        });
        FilterTreeOutput { response: row.output, picked }
    }

    /// The category folders of tile grids.
    fn show_folders(self, ui: &mut Ui) -> FilterTreeOutput {
        let Self { index, selected, thumbs, source, .. } = self;
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
                            Tile::new(index.label(key)).thumb(ThumbAspect::new(wide, high), thumb_painter(thumbs, source, filter))
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
