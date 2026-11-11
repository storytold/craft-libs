//! `PageList`: Illustrator's Artboards panel and InDesign's page list as one list: a row per page or artboard with
//! its name, a visibility and a lock column, picked, reordered by dragging or with the stacking keys, renamed in
//! place, with New, Duplicate and Delete in the footer. Behaviour follows Photoshop's Artboards in the Layers panel.
//!
//! A pattern over craft-ui's [`Table`] with [`ListRow`]s (an eye and a lock [`ToggleCell`] per row, the options
//! icon as a trailing part), `reorder` for drags and stacking keys, `rename` for names, and a [`FooterBar`] whose
//! New and Delete take the rows dragged onto them.
//!
//! ```ignore
//! let out = PageList::new("Artboards", &rows, &mut picked, active).show(ui);
//! for event in out.events {
//!     match event {
//!         PageListEvent::Go(row) => doc.set_active_artboard(row),
//!         PageListEvent::Rename { row, text } => doc.rename_artboard(row, &text),
//!         PageListEvent::Reorder(EditEvent::Commit { value, .. }) => doc.move_artboards(value),
//!         PageListEvent::Toggle(step) => doc.apply_toggle_step(step),
//!         PageListEvent::New { duplicate, options } => doc.new_artboard(duplicate, options),
//!         _ => {}
//!     }
//! }
//! ```

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::{ColumnKey, ColumnWidth, EditEvent, IconId, Mixed, Selection, SelectionChange, keyed};
use craft_ui::contract::{
    Bind, CollectionEvent, ColumnKind, ColumnSpec, ContextMenuRequest, ListDrag, NoValue, Outcome, ReorderMove, ReorderRule, SelectMode,
    ShowCollection as _, ShowContainer as _, SweepApply, SweepStep, ToggleColumn, ToggleEdit,
};
use craft_ui::list::{ListRow, RowEvent, ToggleCell, TrailingFace, TrailingPart};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::memory::Remember as _;
use craft_ui::sections::{FooterAction, FooterBar, Panel};
use craft_ui::table::Table;
use egui::{Id, Response, Ui};
use std::borrow::Cow;
use std::cell::RefCell;

craft_ui::words! {
    /// The list's words (VectorCraft's strings).
    pub struct PageListWords {
        new_artboard: "pageList.new" = "New Artboard";
        duplicate: "pageList.duplicate" = "Duplicate Artboard";
        delete: "pageList.delete" = "Delete Artboard";
        options: "pageList.options" = "Artboard Options";
        name: "pageList.name" = "Name";
        visible: "pageList.visible" = "Visible";
        visibility: "pageList.visibility" = "Visibility {row}";
        lock: "pageList.lock" = "Lock";
        locking: "pageList.locking" = "Lock {row}";
    }
}

const VISIBLE: ColumnKey = ColumnKey::new("visible");
const LOCKED: ColumnKey = ColumnKey::new("locked");
const NAME: ColumnKey = ColumnKey::new("name");
const OPTIONS: TrailingPart = TrailingPart::new(0);

/// One row.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PageListRow {
    /// The page or artboard name.
    pub name: String,
    /// Whether the row is locked against edits.
    pub locked: bool,
    /// Whether the row is hidden.
    pub hidden: bool,
}

impl PageListRow {
    /// A row named `name`, unlocked and shown.
    pub fn new(name: impl Into<String>) -> Self {
        PageListRow { name: name.into(), locked: false, hidden: false }
    }

    /// Whether the row is locked against edits.
    #[must_use]
    pub const fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    /// Whether the row is hidden.
    #[must_use]
    pub const fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }
}

/// What the list reports, in the order it happened this pass.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PageListEvent {
    /// The picked rows changed (applied to the selection given).
    Selection(SelectionChange<usize>),
    /// A click on a row, or the lead moved by a key: it becomes the active page or artboard.
    Go(usize),
    /// Enter on the lead row, or a double-click on a row.
    Activate(usize),
    /// A name typed in place (F2, a double-click on the name, the context menu's Rename): trimmed, never empty.
    /// With several rows picked the app gives the name to all of them numbered in list order as `{text}-1`,
    /// `{text}-2` (VectorCraft `artboards.rs:243`).
    #[non_exhaustive]
    Rename {
        /// The row that was renamed.
        row: usize,
        /// The new name.
        text: String,
    },
    /// A drag of rows or a stacking key (Cmd/Ctrl+] and [), as the table reports reorders.
    Reorder(EditEvent<ReorderMove<usize>>),
    /// A click or a sweep in the visibility or lock column, as the step to apply; the step's cell names the column.
    /// `Flip` and `Restore` set the cell to the step's value (Escape restores every flipped cell). `ShowOnly` (Alt on
    /// the first press of the eye column, VectorCraft's hide others) shows only that row, turning the other rows'
    /// eyes off, or on again when it is the only one on; `UndoShowOnly` puts the eyes back as they were. The lock
    /// column ignores Alt. The app groups one sweep as one undo step.
    Toggle(SweepStep<usize>),
    /// New, Duplicate, or a row dropped on New (`duplicate`, the dragged rows' grabbed row); `options` with Alt held
    /// (open the options for the new one first).
    #[non_exhaustive]
    New {
        /// The row to duplicate; `None` for a plain New.
        duplicate: Option<usize>,
        /// Whether Alt was held: open the options for the new row first.
        options: bool,
    },
    /// Delete (the button, the Delete key, or rows dropped on it): the picked rows that aren't locked, or the
    /// dropped rows.
    Delete(Vec<usize>),
    /// The options icon of a row.
    Options(usize),
    /// The app's menu is asked for.
    ContextMenu(ContextMenuRequest<usize>),
}

/// The list's result.
#[derive(Debug)]
#[must_use = "the app applies the list's events"]
#[non_exhaustive]
pub struct PageListOutput {
    /// The list's area.
    pub response: Response,
    /// What the user did this pass, in order.
    pub events: Vec<PageListEvent>,
}

impl Outcome for PageListOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The readers of the eye and lock sweeps, kept across passes while the pointer is held.
#[derive(Clone)]
struct Sweeps {
    eye: SweepApply,
    lock: SweepApply,
}

impl Default for Sweeps {
    /// Alt on the first press of an eye sweep shows only that row; the lock column ignores Alt.
    fn default() -> Self {
        Sweeps { eye: SweepApply::show_only(), lock: SweepApply::new() }
    }
}

/// The column of a toggle edit, when it names one.
fn toggle_column(edit: &EditEvent<ToggleEdit<usize>>) -> Option<&ColumnKey> {
    match edit {
        EditEvent::Update { value: cell, .. } | EditEvent::Set { after: cell, .. } | EditEvent::Commit { after: cell, .. } => Some(&cell.column),
        EditEvent::Begin { before: Mixed::Same(cell), .. } | EditEvent::Cancel { before: Some(Mixed::Same(cell)), .. } => Some(&cell.column),
        _ => None,
    }
}

/// The list, built per pass.
///
/// - Rows are `ListRow`s: the name, an eye and a lock toggle column (sweepable), the options icon as a trailing
///   part. The active row is the list's current item and its lead.
/// - Pointer, keys, rename and drag are the table's: click activates and picks; Shift-click extends,
///   Cmd/Ctrl-click toggles; one Tab stop, up and down move the active row, F2 renames, Enter activates, Delete and
///   Backspace delete, Cmd/Ctrl+] and [ move the picked rows, the Menu key and Shift+F10 open the menu.
/// - Footer: New, Duplicate and Delete as icon actions; New and Delete take this list's dragged rows (a row dropped
///   on New duplicates the grabbed row; on Delete deletes the dragged rows).
#[derive(craft_ui::contract::Pattern)]
pub struct PageList<'a> {
    label: Label,
    rows: &'a [PageListRow],
    picked: &'a mut Selection<usize>,
    active: usize,
}

impl<'a> PageList<'a> {
    /// A list named `label` of `rows`, with `picked` as the selection and `active` as the page or artboard being
    /// edited.
    pub fn new(label: impl Into<Label>, rows: &'a [PageListRow], picked: &'a mut Selection<usize>, active: usize) -> Self {
        PageList { label: label.into(), rows, picked, active }
    }

    /// Shows the list and returns its response and the events for the app to apply.
    pub fn show(self, ui: &mut Ui) -> PageListOutput {
        let PageList { label, rows, picked, active } = self;
        let env = Env::of(ui);
        let words = env.words::<PageListWords>();
        let unlocked = |from: &[usize]| from.iter().copied().filter(|&row| rows.get(row).is_some_and(|r| !r.locked)).collect::<Vec<_>>();
        let deletable = unlocked(&picked.items);
        let any_picked = !picked.items.is_empty();
        let columns = vec![
            ColumnSpec::new(VISIBLE, words.visible.clone(), ColumnWidth::Controls(1.0)).kind(ColumnKind::Toggle(ToggleColumn::sweepable())),
            ColumnSpec::new(LOCKED, words.lock.clone(), ColumnWidth::Controls(1.0)).kind(ColumnKind::Toggle(ToggleColumn::sweepable())),
            ColumnSpec::new(NAME, words.name.clone(), ColumnWidth::fill(1.0)),
        ];
        let events = RefCell::new(Vec::new());
        let list_id: RefCell<Option<Id>> = RefCell::new(None);
        let order: Vec<usize> = (0..rows.len()).collect();
        let source = keyed(&order, |&row| row)
            .with_text(|&row| Cow::Owned(rows[row].name.clone()))
            .with_toggled(|&row, column| (*column == VISIBLE && !rows[row].hidden) || (*column == LOCKED && rows[row].locked));
        let response = Panel::new(label.clone())
            .headerless()
            .footer(|ui, _| {
                let list = *list_id.borrow();
                let ours = move |drag: &ListDrag<usize>| Some(drag.list) == list;
                let actions = [
                    FooterAction::new(IconId::new("plus"), words.new_artboard.clone()).accepts_drops(ours),
                    FooterAction::new(IconId::new("copy"), words.duplicate.clone()).enabled(any_picked),
                    FooterAction::new(IconId::new("trash"), words.delete.clone()).enabled(!deletable.is_empty()).accepts_drops(ours),
                ];
                let footer = FooterBar::actions(actions).show_footer(ui);
                let mut events = events.borrow_mut();
                if let Some(drop) = &footer.dropped {
                    let Some(drag) = drop.payload::<ListDrag<usize>>() else { return };
                    if drop.index == 0 {
                        events.push(PageListEvent::New { duplicate: Some(drag.grabbed), options: false });
                    } else {
                        let dropped = unlocked(&drag.keys);
                        if !dropped.is_empty() {
                            events.push(PageListEvent::Delete(dropped));
                        }
                    }
                } else if let Some(picked_action) = footer.picked {
                    events.push(match picked_action {
                        0 => PageListEvent::New { duplicate: None, options: footer.mods.alt },
                        1 => PageListEvent::New { duplicate: Some(active), options: false },
                        _ => PageListEvent::Delete(deletable.clone()),
                    });
                }
            })
            .show(ui, |ui, cx| {
                let shown = Table::new(label.clone(), columns.clone())
                    .select(SelectMode::Multi)
                    .selection(Bind::bound(&mut *picked))
                    .reorder(ReorderRule::new())
                    .rename(true)
                    .deletable(true)
                    .current(Some(active))
                    .natural_height(true)
                    .show_items(ui, &source, &mut |&row, icx| {
                        let options = IconSpec::svg(IconId::new("settings"));
                        ListRow::new(rows[row].name.clone())
                            .cells(icx.cells())
                            .toggle(ToggleCell::eye(VISIBLE, !rows[row].hidden).name(words.visibility.clone()))
                            .toggle(ToggleCell::lock(LOCKED, rows[row].locked).name(words.locking.clone()))
                            .trailing_part(OPTIONS, TrailingFace::Icon(options), words.options.clone())
                    });
                *list_id.borrow_mut() = Some(shown.response.id);
                let mut events = events.borrow_mut();
                for event in shown.events {
                    match event {
                        CollectionEvent::Selection(change) => events.push(PageListEvent::Selection(change)),
                        CollectionEvent::LeadMoved { key, .. } => events.push(PageListEvent::Go(key)),
                        CollectionEvent::Activate(row) => events.push(PageListEvent::Activate(row)),
                        CollectionEvent::Rename(edit) => {
                            // VectorCraft renames only to a name with something in it.
                            if let Some(rename) = edit.value() {
                                let text = rename.name.trim();
                                if !text.is_empty() {
                                    events.push(PageListEvent::Rename { row: rename.key, text: text.to_owned() });
                                }
                            }
                        }
                        CollectionEvent::Toggle(edit) => {
                            let sweeps = cx.keep::<Sweeps>();
                            let sweep = if toggle_column(&edit) == Some(&VISIBLE) { &mut sweeps.eye } else { &mut sweeps.lock };
                            events.extend(sweep.feed(&edit).map(PageListEvent::Toggle));
                        }
                        CollectionEvent::Reorder(edit) => events.push(PageListEvent::Reorder(edit)),
                        CollectionEvent::ContextMenu(request) => events.push(PageListEvent::ContextMenu(request)),
                        CollectionEvent::Delete(rows) => {
                            let rows = unlocked(&rows);
                            if !rows.is_empty() {
                                events.push(PageListEvent::Delete(rows));
                            }
                        }
                        CollectionEvent::Item(row, RowEvent::Trailing { part, .. }) if part == OPTIONS => events.push(PageListEvent::Options(row)),
                        _ => {}
                    }
                }
                shown.response
            });
        PageListOutput { response: response.output.response, events: events.into_inner() }
    }
}
