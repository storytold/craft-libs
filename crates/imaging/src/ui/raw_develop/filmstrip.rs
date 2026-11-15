//! The filmstrip of the raws being opened.

use std::borrow::Cow;
use std::sync::Arc;

use craft_behavior::{DisplayOrder, EditEvent, IconId, ItemSource, Mixed, Selection};
use craft_ui::contract::Pattern;
use craft_ui::contract::{ActivateOn, Bind, CollectionEvent, Props as _, SelectMode, ShowCollection as _, ShowContainer as _, Sizing};
use craft_ui::layout_components::Stack;
use craft_ui::list::{ThumbAspect, ThumbSize};
use craft_ui::runtime::env::Env;
use craft_ui::tile_grid::{Tile, TileGrid, TileMark};
use egui::{Rect, Response, Ui, load::SizedTexture, pos2};

use super::words::RawDevelopWords;

/// One filmstrip tile.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RawFilmItem {
    /// The raw's name.
    pub name: String,
    /// The raw has edits.
    pub edited: bool,
    /// The tile's picture.
    pub picture: Option<SizedTexture>,
}

impl RawFilmItem {
    /// A tile for the raw `name`, unedited and without a picture yet.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), edited: false, picture: None }
    }
}

/// The tiles in order, as the grid's item source: the key of a tile is its index.
struct Order<'a>(&'a [RawFilmItem]);

impl DisplayOrder<usize> for Order<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|item| Cow::Borrowed(item.name.as_str()))
    }
}

impl ItemSource<usize> for Order<'_> {}

/// A one-column grid of tiles at the start edge, shown for more than one raw: the edited dot and the current frame.
/// A click on a tile writes its index to `current`.
#[must_use = "show the filmstrip with `show`"]
#[derive(Pattern)]
pub struct RawFilmstrip<'a> {
    items: &'a [RawFilmItem],
    current: Bind<'a, usize>,
}

impl<'a> RawFilmstrip<'a> {
    /// The filmstrip over `items` with `current` picked.
    pub fn new(items: &'a [RawFilmItem], current: Bind<'a, usize>) -> Self {
        Self { items, current }
    }

    /// Draws the tiles.
    pub fn show(self, ui: &mut Ui) -> Response {
        let Self { items, mut current } = self;
        let words = Env::of(ui).words::<RawDevelopWords>();
        let shown = Stack::new().label(words.filmstrip.clone()).show(ui, |ui, cx| {
            let mut bind = current.reborrow();
            let at = bind.read(cx).and_then(|held| held.same().copied()).unwrap_or(0);
            let mut selection = Selection::single(at);
            let mut picked = None;
            let grid = TileGrid::<usize>::new(words.filmstrip.clone())
                .thumb_size(ThumbSize::Large)
                .card_columns(1)
                .sizing(Sizing::Content)
                .select(SelectMode::Single)
                .activate(ActivateOn::Click)
                .natural_height(true)
                .selection(Bind::bound(&mut selection))
                .show_items(ui, &Order(items), &mut |key, _| {
                    let item = &items[*key];
                    let mut tile = Tile::new(item.name.clone());
                    if let Some(picture) = item.picture {
                        let wide = picture.size.x.round().clamp(1.0, f32::from(u16::MAX)) as u16;
                        let high = picture.size.y.round().clamp(1.0, f32::from(u16::MAX)) as u16;
                        tile = tile.thumb(ThumbAspect::new(wide, high), move |cx, rect| {
                            cx.picture(picture.id, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), 0.0);
                        });
                    }
                    if item.edited {
                        tile = tile.mark(TileMark::new(IconId::new("circle_filled"), words.edited.clone()));
                    }
                    tile
                });
            for event in grid.response.events {
                if let CollectionEvent::Activate(key) = event {
                    picked = Some(key);
                }
            }
            if let Some(index) = picked.filter(|index| *index != at && *index < items.len()) {
                let name: Arc<str> = Arc::from(words.filmstrip.as_str());
                let _ = bind.write(cx, &EditEvent::set(Mixed::Same(at), index, name, None));
            }
        });
        shown.output
    }
}
