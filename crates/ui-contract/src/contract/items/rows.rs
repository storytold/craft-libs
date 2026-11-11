//! The item step's row work beyond selection by row: rows with focusable children (17.5), cell selection with a
//! lead column (17.7) and auto-height rows measured into `LayoutMemory` (17.3, I1).

use super::{ItemsHooks, key_hash};
use crate::contract::{Collection, Cx, SelectMode};
use crate::runtime::egui::memory as store;
use crate::runtime::memory::sub;
use crate::runtime::{focus, lazy};
use crate::style::Flags;
use craft_behavior::{
    CellAt, CellDirection, CellSpace, Granularity, ItemLayout, MoveMods, NavMove, NavShape, PlacedSelection, RowAt, SelectionChange, SelectionModel,
};
use egui::{Context, Id, Vec2};
use std::sync::Arc;

/// The roving group of the children and parts of the item `item`'s row.
pub(super) fn row_group(item: Id) -> Id {
    sub(item, "row_children")
}

/// The stop focus is on when it is a child or part of the row group `group`.
pub(super) fn child_focus(ctx: &Context, group: Id) -> Option<focus::StopKey> {
    let key = focus::focused_stop(ctx)?;
    (focus::stop_rec(ctx, key)?.stop.group == Some(group)).then_some(key)
}

/// The `SELECTED` and `LEAD` flags of the visible cells of a row, by column position.
#[derive(Clone, Default)]
struct CellFlags(Arc<[(u64, Flags)]>);

pub(super) fn put_cell_flags(ctx: &Context, item: Id, flags: Vec<(u64, Flags)>) {
    store::put(ctx, sub(item, "cell_flags"), CellFlags(flags.into()));
}

/// The flags the item step wrote for column position `column` of the row `item`.
pub(in crate::contract) fn cell_flags(ctx: &Context, item: Id, column: u64) -> Flags {
    let Some(CellFlags(flags)) = store::peek_clone::<CellFlags>(ctx, sub(item, "cell_flags")) else { return Flags::empty() };
    flags.iter().find(|(at, _)| *at == column).map_or(Flags::empty(), |(_, flags)| *flags)
}

/// The id a row's measured height is kept under.
pub(super) fn measured_id<K: std::hash::Hash>(collection: Id, key: &K) -> Id {
    sub(collection, ("row_size", key_hash(key)))
}

/// Runs `f` with a reader of the heights auto rows had in the last pass that built them: the rows the pass does not
/// build, out of view, are placed by them (`runtime::lazy`).
pub(super) fn measured_heights<R>(ctx: &Context, f: impl FnOnce(&dyn Fn(Id) -> Option<f32>) -> R) -> R {
    lazy::reading(ctx, |read| f(&|id| read(id).map(|size| size.y)))
}

/// The id an auto-width item's measured widths (ideal, then minimum) are kept under.
pub(super) fn measured_width_id<K: std::hash::Hash>(collection: Id, key: &K) -> Id {
    sub(collection, ("item_width", key_hash(key)))
}

/// Runs `f` with a reader of the `(ideal, min)` widths auto-width items had in the last pass that built them.
pub(super) fn measured_widths<R>(ctx: &Context, f: impl FnOnce(&dyn Fn(Id) -> Option<(f32, f32)>) -> R) -> R {
    lazy::reading(ctx, |read| f(&|id| read(id).map(|size| (size.x, size.y))))
}

/// Records the size an auto row measured this pass; the row is placed by it in the same pass.
pub(super) fn record_measure(ctx: &Context, id: Id, size: Vec2) {
    lazy::remember(ctx, id, size);
}

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// The row-child move of 17.5, tried before the navigation core: Forward on the lead row enters its first
    /// focusable child or part. A closed parent expands instead, a row with no children leaves the move to the
    /// core (an open parent then moves to its first child row), and so does every other move. `true` when focus
    /// moved in and the move is taken.
    pub(super) fn row_child_move(&self, cx: &Cx<'_, C>, layout: &C::ItemLayout, mv: NavMove, mods: MoveMods) -> bool {
        if !is_forward(mv, mods) || matches!(layout.nav(), NavShape::Cell) {
            return false;
        }
        let Some(at) = self.lead.key.as_ref().and_then(|key| self.index_of(key)) else { return false };
        if self.source.has_children(at) && !self.source.open(at) {
            return false;
        }
        let item = sub(cx.core.id, ("key", key_hash(&self.source.key(at))));
        let group = row_group(item);
        focus::row_has_stops(&cx.core.ctx, group) && focus::enter_row(&cx.core.ctx, group)
    }

    /// Whether the row at position `at` is selected.
    pub(super) fn holds(&self, at: usize) -> bool {
        let key = self.source.key(at);
        self.selection.contains(RowAt::new(&key, at, self.source))
    }

    /// Whether `key`, standing at position `at`, is selected. `key` need not be in the source: a selection of ranges
    /// answers for the position.
    pub(super) fn holds_key(&self, key: &C::Key, at: usize) -> bool {
        self.selection.contains(RowAt::new(key, at, self.source))
    }

    /// The selection placed once in the source's order; the reads of a pass go by position.
    pub(super) fn placed(&self) -> PlacedSelection {
        self.selection.placed(self.source)
    }

    /// The keys of the selected rows of `selection`, in the display order.
    pub(super) fn selected_keys(&self, selection: &C::Selection) -> Vec<C::Key> {
        selection.placed(self.source).rows().into_iter().map(|at| self.source.key(at)).collect()
    }

    /// A pick of the rows `from` to `to` in a model of cells: one range of whole rows (the rows' cells in every column
    /// of the layout), active and anchored at its first cell.
    pub(super) fn select_rows(&mut self, comp: &C, from: usize, to: usize) {
        let Some(mut cells) = self.selection.cells().cloned() else { return };
        let source = self.source;
        let space = comp.cell_space(CellSpace::new(source, self.columns));
        let range = space.keyed_range(space.whole_rows(from as u32, to as u32));
        if cells.select_range(range, &space) {
            self.change(cells.change());
        }
    }

    /// Runs `f` with the grid of the collection's cells (17.7): the source's rows, the layout's columns with the rows
    /// and columns it hides left out, and what the collection adds (`Collection::cell_space`).
    fn with_cell_space<R>(&self, comp: &C, layout: &C::ItemLayout, f: impl FnOnce(&CellSpace<'_, C::Key>) -> R) -> R {
        let source = self.source;
        let row_hidden = |row: u32| !layout.shown(row as usize);
        let column_hidden = |column: u32| !layout.column_shown(column as usize);
        let space = comp.cell_space(CellSpace::new(source, layout.column_count() as u32).hidden_rows(&row_hidden).hidden_columns(&column_hidden));
        f(&space)
    }

    /// The head of the selection's current range as a row position and a column: the end Shift+arrows move.
    pub(super) fn cell_head(&self, comp: &C, layout: &C::ItemLayout) -> Option<(usize, usize)> {
        let cells = self.selection.cells()?;
        let head = self.with_cell_space(comp, layout, |space| cells.head(space))?;
        Some((head.row as usize, head.column as usize))
    }

    /// A move of the lead in a sheet (17.7): `mv` toward the cell at `row` and `column`, which the navigation
    /// computed from the lead (from the head while the range extends). The selection follows unless the lead moves
    /// alone (Ctrl): a one-cell arrow runs the collection's cell rules as an arrow (`step`, or `extend` with Shift),
    /// any other move as a click on the target (`select`, or `extend_to` with Shift). The lead goes to where the
    /// rules put the active cell. Returns the row to bring into view: the head while the range extends, else the
    /// lead.
    pub(super) fn cell_move(&mut self, comp: &C, layout: &C::ItemLayout, mv: NavMove, row: usize, column: usize, mods: MoveMods) -> usize {
        let Some(mut cells) = self.selection.cells().cloned() else {
            if self.lead.key.as_ref() != Some(&self.source.key(row)) {
                self.move_lead(row, super::LeadSource::Keys);
            }
            self.lead.column = column;
            if !mods.lead {
                let anchor = if mods.extend { self.anchor_at(row) } else { row };
                self.change(SelectionChange::Set(self.range(anchor, row)));
            }
            return row;
        };
        let (mut at, mut across, mut reveal) = (row, column, row);
        let mut change = None;
        if !mods.lead {
            let target = CellAt::new(self.source.key(row), column as u32);
            let from = self.lead.key.clone().map(|key| CellAt::new(key, self.lead.column as u32));
            let direction = cell_direction(mv, mods);
            let (moved, active, head) = self.with_cell_space(comp, layout, |space| {
                if mods.extend
                    && cells.anchor_cell().is_none()
                    && let Some(from) = from
                {
                    cells.select(from, space);
                }
                let moved = match (direction, mods.extend) {
                    (Some(direction), true) => cells.extend(direction, space),
                    (Some(direction), false) => cells.step(direction, space),
                    (None, true) => cells.extend_to(target, space),
                    (None, false) => cells.select(target, space),
                };
                (moved, cells.active().and_then(|active| space.place(active)), cells.head(space))
            });
            if let Some(active) = active {
                (at, across) = (active.row as usize, active.column as usize);
            }
            reveal = head.filter(|_| mods.extend).map_or(at, |head| head.row as usize);
            change = moved.then(|| cells.change());
        }
        if self.lead.key.as_ref() != Some(&self.source.key(at)) {
            self.move_lead(at, super::LeadSource::Keys);
        }
        self.lead.column = across;
        if let Some(change) = change {
            self.change(change);
        }
        reveal
    }

    /// A press on row `at` at `column`: the lead cell goes there and the selection follows the modifiers. A press
    /// selects the cell, Shift extends from the anchor cell, and Cmd/Ctrl adds a range at an unselected cell or
    /// takes a selected one out.
    pub(super) fn cell_press(&mut self, comp: &C, layout: &C::ItemLayout, at: usize, column: usize, extend: bool, toggle: bool) {
        self.lead.column = column;
        let key = self.source.key(at);
        let Some(mut cells) = self.selection.cells().cloned() else {
            if toggle {
                self.change(SelectionChange::Toggle(key));
            } else {
                let anchor = if extend { self.anchor_at(self.lead.at) } else { at };
                self.change(SelectionChange::Set(self.range(anchor, at)));
            }
            return;
        };
        let target = CellAt::new(key, column as u32);
        let changed = self.with_cell_space(comp, layout, |space| {
            if toggle {
                cells.toggle(target, space)
            } else if extend {
                cells.extend_to(target, space)
            } else {
                cells.select(target, space)
            }
        });
        if changed {
            self.change(cells.change());
        }
    }

    /// The flags of the visible cells of the row `at`: `SELECTED` for a selected cell and `LEAD` for the lead
    /// cell while the collection has focus. `columns` are the column positions of the row's cells in order, `placed`
    /// the selection of the pass.
    pub(super) fn cell_flag_list(&self, placed: &PlacedSelection, at: usize, columns: &[u64], focused: bool) -> Vec<(u64, Flags)> {
        let lead = self.lead.key.as_ref() == Some(&self.source.key(at));
        columns
            .iter()
            .filter_map(|&column| {
                let mut flags = Flags::empty();
                flags.set(Flags::SELECTED, placed.holds_cell(at, column as u32));
                flags.set(Flags::LEAD, lead && focused && column as usize == self.lead.column);
                (!flags.is_empty()).then_some((column, flags))
            })
            .collect()
    }

    /// Whether the collection selects cells.
    pub(super) fn selects_cells(comp: &C) -> bool {
        Self::select_mode(comp) == SelectMode::Cells
    }
}

/// Whether the move is the one that enters a row's children.
fn is_forward(mv: NavMove, mods: MoveMods) -> bool {
    mv == NavMove::Forward && !mods.extend && !mods.lead
}

/// The direction of an arrow: a move of one unit toward a side. Columns are logical, so `Back` is toward the first
/// column whichever way the sheet reads; any other move is a jump.
pub(super) fn cell_direction(mv: NavMove, mods: MoveMods) -> Option<CellDirection> {
    if mods.by != Granularity::Unit {
        return None;
    }
    match mv {
        NavMove::Up => Some(CellDirection::Up),
        NavMove::Down => Some(CellDirection::Down),
        NavMove::Back => Some(CellDirection::Left),
        NavMove::Forward => Some(CellDirection::Right),
        _ => None,
    }
}
