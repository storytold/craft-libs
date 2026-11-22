//! What the panel's parts share in one pass: the tree, the settings, the thumbnails and the words.

use craft_ui::contract::{DropAt, DropKind, DropPlace, Show};
use craft_ui::list::{List, ListRow, ListWords};
use craft_ui::primitives::textures::{ThumbSource, Thumbnails};
use egui::Id;

use super::events::LayerPosition;
use super::{LayerRowLook, LayersPanelWords};
use crate::ui::options::LayersOptions;
use crate::ui::thumbs::LayerThumb;
use crate::ui::tree::{LayerInfo, LayerRowKey, LayerSelection, LayerTree};
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

/// Where a drop at a row or gap lands among the layers, when it lands on a place the host may take.
pub(super) type HostPlace<'l, Id> = dyn Fn(&DropAt<LayerRowKey<Id>>) -> Option<(Id, LayerPosition)> + 'l;

/// The host's payload type dragged from outside the rows (`LayersPanel::takes_drop`), with its type erased so the panel
/// holds one.
pub(super) trait HostDrops<Id> {
    /// `list` taking the payload between rows and onto them where `place` names a layer place the host accepts.
    fn kinds<'l>(&'l self, list: List<'l, LayerRowKey<Id>>, place: &'l HostPlace<'l, Id>) -> List<'l, LayerRowKey<Id>>
    where
        Id: Clone + Eq + std::hash::Hash + Send + Sync + 'static;
}

/// A [`HostDrops`] of the payload type `P` and the host's `accepts`.
pub(super) struct HostDrop<P, A> {
    pub(super) accepts: A,
    pub(super) payload: PhantomData<fn() -> P>,
}

impl<Id, P: Send + Sync + 'static, A: Fn(&P, &Id, LayerPosition) -> bool> HostDrops<Id> for HostDrop<P, A> {
    fn kinds<'l>(&'l self, list: List<'l, LayerRowKey<Id>>, place: &'l HostPlace<'l, Id>) -> List<'l, LayerRowKey<Id>>
    where
        Id: Clone + Eq + std::hash::Hash + Send + Sync + 'static,
    {
        let accepting =
            move |payload: &P, at: &DropAt<LayerRowKey<Id>>| place(at).is_some_and(|(target, position)| (self.accepts)(payload, &target, position));
        list.takes_drop(DropKind::new(DropPlace::Between, accepting)).takes_drop(DropKind::new(DropPlace::Onto, accepting))
    }
}

/// The pass's shared state, lent to the row mapping, the drag rules, the footer and the menus.
pub(super) struct PanelContext<'a, T: LayerTree> {
    /// The rows' list, which drags name.
    pub(super) list: Id,
    /// The hash of the document the panel shows, which the drags it starts carry.
    pub(super) document: Option<u64>,
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
    /// How the rows are drawn.
    pub(super) look: LayerRowLook,
    /// Delete and Backspace in the rows are reported.
    pub(super) deletable: bool,
    /// The host's payload type the rows take from outside.
    pub(super) drops: Option<&'a (dyn HostDrops<T::Id> + 'a)>,
}
