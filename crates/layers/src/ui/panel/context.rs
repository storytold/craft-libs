//! What the panel's parts share in one pass: the tree, the settings, the thumbnails and the words.

use craft_ui::contract::Show;
use craft_ui::list::{ListRow, ListWords};
use craft_ui::primitives::textures::{ThumbSource, Thumbnails};
use egui::Id;

use super::LayersPanelWords;
use crate::ui::options::LayersOptions;
use crate::ui::thumbs::LayerThumb;
use crate::ui::tree::{LayerInfo, LayerSelection, LayerTree};
use std::cell::RefCell;
use std::marker::PhantomData;

/// The thumbnail cache and the app's source of the rows' pictures.
pub(super) struct ThumbHost<'a, Id: Clone + Eq + std::hash::Hash + Send + Sync + 'static> {
    pub(super) thumbnails: &'a Thumbnails<LayerThumb<Id>>,
    pub(super) source: &'a dyn ThumbSource<LayerThumb<Id>>,
}

impl<Id: Clone + Eq + std::hash::Hash + Send + Sync + 'static> Clone for ThumbHost<'_, Id> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Id: Clone + Eq + std::hash::Hash + Send + Sync + 'static> Copy for ThumbHost<'_, Id> {}

/// The host's component at each layer row's trailing end (`LayersPanel::trailing`), with its output handed back.
pub(super) trait TrailingCell<Id> {
    /// Adds the cell of the layer `id` to its row.
    fn add<'r>(&'r self, row: ListRow<'r>, id: &Id, info: &LayerInfo) -> ListRow<'r>;
}

/// A [`TrailingCell`] of the host's `build` and `output`.
pub(super) struct HostCell<C, B, O> {
    pub(super) build: B,
    pub(super) output: RefCell<O>,
    pub(super) component: PhantomData<fn() -> C>,
}

impl<Id: Clone + 'static, C: Show, B: Fn(&Id, &LayerInfo) -> C, O: FnMut(&Id, C::Output)> TrailingCell<Id> for HostCell<C, B, O> {
    fn add<'r>(&'r self, row: ListRow<'r>, id: &Id, info: &LayerInfo) -> ListRow<'r> {
        let layer = id.clone();
        row.trailing_field((self.build)(id, info), move |shown| (self.output.borrow_mut())(&layer, shown))
    }
}

/// The pass's shared state, lent to the row mapping, the drag rules, the footer and the menus.
pub(super) struct PanelContext<'a, T: LayerTree> {
    /// The rows' list, which drags name.
    pub(super) list: Id,
    pub(super) tree: &'a T,
    pub(super) selection: LayerSelection<T::Id>,
    pub(super) options: &'a LayersOptions,
    pub(super) thumbs: Option<ThumbHost<'a, T::Id>>,
    pub(super) words: &'a LayersPanelWords,
    pub(super) list_words: &'a ListWords,
    pub(super) lock_column: bool,
    /// The app's key that flips the selected layers' eyes.
    pub(super) visibility_key: Option<craft_keymap::Chord>,
    /// The host's component at each layer row's trailing end.
    pub(super) trailing: Option<&'a (dyn TrailingCell<T::Id> + 'a)>,
    /// The modifiers held this pass: the thumbnails' load cursor follows them.
    pub(super) held: craft_keymap::Mods,
}
