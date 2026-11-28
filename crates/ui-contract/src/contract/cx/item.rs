//! `ItemCx`: what the item closure reads for one item, and the parts it opens menus from.

use crate::runtime::env::EnvView;
use crate::runtime::memory::{MemoryStage, StageAccess, sealed};
use egui::{Id, Rect, Ui};
use std::fmt::Debug;

/// Rich tooltip content an item stored.
pub(crate) type ItemTip<'r> = Box<dyn FnOnce(&mut Ui) + 'r>;

/// The part of one item that opens a menu or popover: `ItemCx::part_opener` names it and
/// `CollectionResponse::opener` gives its response after the collection has been shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemOpener {
    pub(crate) item: Id,
    pub(crate) part: craft_behavior::PartRef,
}

/// What the item closure reads for one item: a view over the pipeline's memory stage keyed by the item.
///
/// The row-hook contract. The collection measures an auto row (auto height or auto widths) at step 5, before it
/// places its rows, from the item this closure and `Collection::dress` make with the row state known before the pass
/// senses input: the `RowState` fields marked "measured" as step 4 read them, the depth and the set position, under
/// the collection's env. The size of an auto row depends only on those. It must not depend on app values the body
/// adds at step 13, the surface, focus, or the pointer and gesture state (`hovered`, `pointer_over`, `pressed`,
/// `dragging`, `drop_target`, `menu_open`, `activated_by_key`): only the build that shows the row has them. A row
/// whose measured state this pass's input changes (a click that selects it, a press that starts its rename) is
/// measured again when it is built; a size other than the one the pass placed it by makes the pass run again, so
/// the frame shows the row at its new size.
#[non_exhaustive]
pub struct ItemCx<'a, 'r, K> {
    /// The item's key.
    pub key: &'a K,
    /// The item's position in the source.
    pub at: usize,
    /// The item's rect.
    pub rect: Rect,
    /// Selection, lead, open and renaming state of the row.
    pub row: crate::contract::RowState,
    /// The nesting depth, 0 for a root.
    pub depth: usize,
    /// The item's place in the set.
    pub set: crate::runtime::access::SetPosition,
    /// The row's zero-based index in the collection.
    pub row_index: usize,
    /// The number of rows in the collection.
    pub row_count: usize,
    /// The number of columns the rows show, 0 without columns.
    pub column_count: usize,
    /// What the inherited context holds that component code may read.
    pub env: EnvView<'a>,
    /// Base id of the item's stage: `sub(item_id, "item")`.
    pub(crate) id: Id,
    pub(crate) memory: &'a mut MemoryStage,
    pub(crate) tip: Option<ItemTip<'r>>,
    pub(crate) detail: Option<ItemTip<'r>>,
    pub(crate) openers: Vec<ItemOpener>,
    pub(crate) item: Id,
    pub(crate) cells: Vec<(craft_behavior::ColumnKey, Rect)>,
    pub(crate) grid: Option<&'a craft_behavior::GridView>,
}

impl<'r, K> ItemCx<'_, 'r, K> {
    /// The visible columns of the row with their rects.
    pub fn cells(&self) -> &[(craft_behavior::ColumnKey, Rect)] {
        &self.cells
    }

    /// The zero-based position of column `key` among the columns the row shows.
    pub fn column_index(&self, key: &craft_behavior::ColumnKey) -> Option<usize> {
        self.cells.iter().position(|(column, _)| column == key)
    }

    /// The rect of column `key` in this row.
    pub fn cell(&self, key: &craft_behavior::ColumnKey) -> Option<Rect> {
        self.cells.iter().find(|(column, _)| column == key).map(|(_, rect)| *rect)
    }

    /// The sections of a sheet layout.
    pub fn grid(&self) -> Option<&craft_behavior::GridView> {
        self.grid
    }

    /// Rich tooltip content for the item; it may borrow app data.
    pub fn tip(&mut self, tip: impl FnOnce(&mut Ui) + 'r) {
        self.tip = Some(Box::new(tip));
    }

    /// Shows `body` under the item, across the content width, in a child `Ui` with id `sub(item_id, "detail")`. It adds
    /// its height to the item's slot, which keeps its last shown height out of view.
    pub fn detail(&mut self, body: impl FnOnce(&mut Ui) + 'r) {
        self.detail = Some(Box::new(body));
    }

    /// The detail body the item gave, for a row that takes it: one of auto height (`auto`).
    pub(crate) fn take_detail(&mut self, auto: bool) -> Option<ItemTip<'r>> {
        let detail = self.detail.take();
        debug_assert!(detail.is_none() || auto, "an item's detail needs an auto-height row; a grid and a fixed-height row ignore it");
        detail.filter(|_| auto)
    }

    /// The opener for the item's part `part`: after the collection is shown, `CollectionResponse::opener` gives the
    /// part's response (its click this pass and its rect), and a layer-5 menu opens from it with `opened_by`.
    pub fn part_opener(&mut self, part: impl Into<craft_behavior::PartRef>) -> ItemOpener {
        let opener = ItemOpener { item: self.item, part: part.into() };
        if !self.openers.contains(&opener) {
            self.openers.push(opener);
        }
        opener
    }
}

impl<K> sealed::Sealed for ItemCx<'_, '_, K> {}

impl<K> StageAccess for ItemCx<'_, '_, K> {
    fn stage(&self) -> &MemoryStage {
        self.memory
    }

    fn stage_mut(&mut self) -> &mut MemoryStage {
        self.memory
    }

    fn base(&self) -> Id {
        self.id
    }
}
