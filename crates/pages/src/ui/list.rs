//! `PageList`: Illustrator's Artboards panel and InDesign's page list as one list: a row per page or artboard with
//! its name, picked, reordered by dragging or with the stacking keys, renamed in place, with Rearrange, Move Up and
//! Down, New and Delete in the footer. Behaviour follows Photoshop's Artboards in the Layers panel.
//!
//! A pattern over craft-ui's [`Table`] with [`ListRow`]s (the options icon and the lock as trailing parts),
//! `reorder` for drags and stacking keys, `rename` for names, and a [`FooterBar`] whose New takes the rows dragged
//! onto it.
//!
//! ```ignore
//! let out = PageList::new("Artboards", &rows, &mut picked, active).show(ui);
//! for event in out.events {
//!     match event {
//!         PageListEvent::Go(row) => doc.set_active_artboard(row),
//!         PageListEvent::Rename { row, text } => doc.rename_artboard(row, &text),
//!         PageListEvent::Reorder(EditEvent::Commit { value, .. }) => doc.move_artboards(value),
//!         PageListEvent::Lock { row, locked } => doc.lock_artboard(row, locked),
//!         PageListEvent::New { duplicate, options } => doc.new_artboard(duplicate, options),
//!         _ => {}
//!     }
//! }
//! ```

#[cfg(feature = "suite")]
pub(crate) mod stories;

use craft_behavior::{ColumnKey, ColumnWidth, EditEvent, IconId, Mixed, Selection, SelectionChange, keyed};
use craft_ui::contract::{
    Bind, CollectionEvent, ColumnSpec, ContextMenuRequest, ListDrag, NoValue, Outcome, ReorderMove, ReorderRule, SelectMode, ShowCollection as _,
    ShowContainer as _, request_rename,
};
use craft_ui::list::{ListRow, RowEvent, TrailingFace, TrailingPart};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label, WordTemplate};
use craft_ui::sections::{FooterAction, FooterBar, Panel};
use craft_ui::table::Table;
use egui::{Id, Response, Ui};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::sync::Arc;

craft_ui::words! {
    /// The list's words (VectorCraft's strings).
    pub struct PageListWords {
        new_artboard: "pageList.new" = "New Artboard";
        delete: "pageList.delete" = "Delete Artboard";
        options: "pageList.options" = "Artboard Options";
        name: "pageList.name" = "Name";
        move_up: "pageList.moveUp" = "Move Up";
        move_down: "pageList.moveDown" = "Move Down";
        rearrange: "pageList.rearrange" = "Rearrange All Artboards";
        lock_row: "pageList.lockRow" = "Lock {row}";
        unlock_row: "pageList.unlockRow" = "Unlock {row}";
    }
}

const NAME: ColumnKey = ColumnKey::new("name");
const OPTIONS: TrailingPart = TrailingPart::new(0);
const LOCK: TrailingPart = TrailingPart::new(1);

// The footer's actions in the order given; the bar shows Rearrange at the start, then from the end edge Delete, New,
// Move Down, Move Up.
const REARRANGE: usize = 0;
const DELETE: usize = 1;
const NEW: usize = 2;
const MOVE_DOWN: usize = 3;
const MOVE_UP: usize = 4;

/// One row.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PageListRow {
    /// The page or artboard name.
    pub name: String,
    /// Whether the row is locked against edits.
    pub locked: bool,
}

impl PageListRow {
    /// A row named `name`, unlocked.
    pub fn new(name: impl Into<String>) -> Self {
        PageListRow { name: name.into(), locked: false }
    }

    /// Whether the row is locked against edits.
    #[must_use]
    pub const fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
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
    /// A double-click on a row's number: fit that page or artboard in the window.
    Fit(usize),
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
    /// A drag of rows, a stacking key (Cmd/Ctrl+] and [) or the footer's Move Up and Move Down, as the table reports
    /// reorders.
    Reorder(EditEvent<ReorderMove<usize>>),
    /// A click on a row's lock.
    #[non_exhaustive]
    Lock {
        /// The row whose lock was clicked.
        row: usize,
        /// Whether the row is locked after the click.
        locked: bool,
    },
    /// The Rearrange button.
    Rearrange,
    /// New, or a row dropped on New (`duplicate`, the dragged rows' grabbed row); `options` with Alt held
    /// (open the options for the new one first).
    #[non_exhaustive]
    New {
        /// The row to duplicate; `None` for a plain New.
        duplicate: Option<usize>,
        /// Whether Alt was held: open the options for the new row first.
        options: bool,
    },
    /// Delete (the button or the Delete key): the picked rows that aren't locked.
    Delete(Vec<usize>),
    /// The options icon of the active row, or a double-click on another row's (a single click on it makes the row
    /// active, as a click on the row does).
    Options(usize),
    /// The app's menu is asked for.
    ContextMenu(ContextMenuRequest<usize>),
}

impl PageListEvent {
    /// A name typed in place for `row`, which the list gives trimmed and never empty.
    pub fn rename(row: usize, text: impl Into<String>) -> Self {
        Self::Rename { row, text: text.into() }
    }

    /// A click on `row`'s lock, `locked` after the click.
    pub const fn lock(row: usize, locked: bool) -> Self {
        Self::Lock { row, locked }
    }

    /// New, duplicating `duplicate` when a row was dropped on it; `options` with Alt held.
    pub const fn new_row(duplicate: Option<usize>, options: bool) -> Self {
        Self::New { duplicate, options }
    }
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
    /// Key focus is in the list this pass (the table's response).
    pub focused: bool,
}

impl Outcome for PageListOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The list, built per pass.
///
/// - Rows are `ListRow`s under no header row: the number, the name, the lock (shown while locked, on hover
///   otherwise) and the options icon as trailing parts. The active row is the list's current item and its lead. A
///   double click on the number fits the row (`Fit`).
/// - Pointer, keys, rename and drag are the table's: click activates and picks; Shift-click extends,
///   Cmd/Ctrl-click toggles; one Tab stop, up and down move the active row, F2 renames, Enter activates, Delete and
///   Backspace delete, Cmd/Ctrl+] and [ move the picked rows, the Menu key and Shift+F10 open the menu.
/// - Footer: Rearrange, Delete, New, Move Down and Move Up as icon actions; New takes this list's dragged rows (a
///   row dropped on New duplicates the grabbed row).
#[derive(craft_ui::contract::Pattern)]
pub struct PageList<'a> {
    label: Label,
    rows: &'a [PageListRow],
    picked: &'a mut Selection<usize>,
    active: usize,
    rename: Option<usize>,
}

impl<'a> PageList<'a> {
    /// A list named `label` of `rows`, with `picked` as the selection and `active` as the page or artboard being
    /// edited.
    pub fn new(label: impl Into<Label>, rows: &'a [PageListRow], picked: &'a mut Selection<usize>, active: usize) -> Self {
        PageList { label: label.into(), rows, picked, active, rename: None }
    }

    /// Starts renaming `row` (the app's Rename command): its field opens at the list's next pass.
    #[must_use]
    pub fn start_rename(mut self, row: usize) -> Self {
        self.rename = Some(row);
        self
    }

    /// Shows the list and returns its response and the events for the app to apply.
    pub fn show(self, ui: &mut Ui) -> PageListOutput {
        let PageList { label, rows, picked, active, rename } = self;
        let env = Env::of(ui);
        let words = env.words::<PageListWords>();
        let unlocked = |from: &[usize]| from.iter().copied().filter(|&row| rows.get(row).is_some_and(|r| !r.locked)).collect::<Vec<_>>();
        let deletable = unlocked(&picked.items);
        // With several rows picked the field opens empty, so the typed name goes to all of them.
        let several = (picked.items.len() > 1).then(|| picked.items.clone());
        let columns = vec![ColumnSpec::new(NAME, words.name.clone(), ColumnWidth::fill(1.0))];
        let events = RefCell::new(Vec::new());
        let list_id: RefCell<Option<Id>> = RefCell::new(None);
        let focused = Cell::new(false);
        let order: Vec<usize> = (0..rows.len()).collect();
        let number = |row: usize| env.format().integer(row as u64 + 1);
        let widest = number(rows.len().saturating_sub(1));
        let source = keyed(&order, |&row| row).with_text(|&row| Cow::Owned(rows[row].name.clone()));
        let step = |to: usize, label: &Label| {
            let before = ReorderMove::new(active, vec![active], active, false);
            let after = ReorderMove::new(active, vec![active], to, false);
            PageListEvent::Reorder(EditEvent::set(Mixed::Same(before), after, Arc::from(label.as_str()), None))
        };
        let response = Panel::new(label.clone())
            .footer(|ui, _| {
                let list = *list_id.borrow();
                let ours = move |drag: &ListDrag<usize>| Some(drag.list) == list;
                let actions = [
                    FooterAction::new(IconId::new("layout_grid"), words.rearrange.clone()).enabled(rows.len() > 1),
                    FooterAction::new(IconId::new("trash"), words.delete.clone()).enabled(rows.len() > 1 && !deletable.is_empty()),
                    FooterAction::new(IconId::new("plus"), words.new_artboard.clone()).accepts_drops(ours),
                    FooterAction::new(IconId::new("arrow_down"), words.move_down.clone()).enabled(active + 1 < rows.len()),
                    FooterAction::new(IconId::new("arrow_up"), words.move_up.clone()).enabled(active > 0),
                ];
                let footer = FooterBar::actions(actions).at_start(1).show_footer(ui);
                let mut events = events.borrow_mut();
                if let Some(drop) = &footer.dropped {
                    let Some(drag) = drop.payload::<ListDrag<usize>>() else { return };
                    events.push(PageListEvent::New { duplicate: Some(drag.grabbed), options: false });
                } else if let Some(picked_action) = footer.picked {
                    events.push(match picked_action {
                        REARRANGE => PageListEvent::Rearrange,
                        DELETE => PageListEvent::Delete(deletable.clone()),
                        NEW => PageListEvent::New { duplicate: None, options: footer.mods.alt },
                        MOVE_DOWN => step(active + 1, &words.move_down),
                        MOVE_UP => step(active.saturating_sub(1), &words.move_up),
                        _ => return,
                    });
                }
            })
            .show(ui, |ui, _| {
                let shown = Table::new(label.clone(), columns.clone())
                    .header(false)
                    .select(SelectMode::Multi)
                    .selection(Bind::bound(&mut *picked))
                    .reorder(ReorderRule::new())
                    .rename(true)
                    .rename_draft(|row| several.as_ref().filter(|items| items.contains(row)).map(|_| String::new()))
                    .deletable(true)
                    .current(Some(active))
                    .natural_height(true)
                    .show_items(ui, &source, &mut |&row, icx| {
                        let options = IconSpec::svg(IconId::new("settings"));
                        let item = ListRow::new(rows[row].name.clone()).cells(icx.cells()).lead_text(number(row), widest.clone()).trailing_part(
                            OPTIONS,
                            TrailingFace::Icon(options),
                            words.options.clone(),
                        );
                        // A trailing part's name is used as given, so the row's name is filled in here.
                        let fill = |template: &Label| Label::from(WordTemplate::new(template.as_str()).fill(&[("row", &rows[row].name)]));
                        if rows[row].locked {
                            item.trailing_part(LOCK, TrailingFace::Icon(IconSpec::svg(IconId::new("lock"))), fill(&words.unlock_row))
                        } else {
                            item.trailing_part_revealed(LOCK, TrailingFace::Icon(IconSpec::svg(IconId::new("lock_open"))), fill(&words.lock_row))
                        }
                    });
                *list_id.borrow_mut() = Some(shown.response.id);
                focused.set(shown.response.has_focus());
                if let Some(row) = rename {
                    request_rename(ui.ctx(), shown.response.id, &row);
                }
                let mut events = events.borrow_mut();
                for event in shown.events {
                    match event {
                        CollectionEvent::Selection { change, .. } => events.push(PageListEvent::Selection(change)),
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
                        CollectionEvent::Reorder(edit) => events.push(PageListEvent::Reorder(edit)),
                        CollectionEvent::ContextMenu(request) => events.push(PageListEvent::ContextMenu(request)),
                        CollectionEvent::Delete(rows) => {
                            let rows = unlocked(&rows);
                            if !rows.is_empty() {
                                events.push(PageListEvent::Delete(rows));
                            }
                        }
                        // The press moved the lead, which reported Go: a click opens the options when the row was
                        // already active, or as the second click of a double click.
                        CollectionEvent::Item(row, RowEvent::Trailing { part, clicks, was_lead, .. })
                            if part == OPTIONS && (was_lead || clicks >= 2) =>
                        {
                            events.push(PageListEvent::Options(row));
                        }
                        CollectionEvent::Item(row, RowEvent::Trailing { part, .. }) if part == LOCK => {
                            events.push(PageListEvent::Lock { row, locked: !rows[row].locked });
                        }
                        CollectionEvent::Item(row, RowEvent::LeadClicked { clicks: 2.., .. }) => events.push(PageListEvent::Fit(row)),
                        _ => {}
                    }
                }
                shown.response
            });
        PageListOutput { response: response.output.response, events: events.into_inner(), focused: focused.get() }
    }
}
