//! What the panel's parts share in one pass: the tree, the settings, the thumbnails and the words.

use craft_ui::list::ListWords;
use craft_ui::primitives::textures::{ThumbSource, Thumbnails};
use egui::Id;

use super::LayersPanelWords;
use crate::ui::options::LayersOptions;
use crate::ui::thumbs::LayerThumb;
use crate::ui::tree::{LayerSelection, LayerTree};

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
}
