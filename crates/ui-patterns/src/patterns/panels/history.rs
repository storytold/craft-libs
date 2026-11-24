//! `HistoryPanel`: the undo history as a list of states and named snapshots, with a footer of snapshot buttons.

use super::words::PanelWords;
use crate::contract::{
    Bind, CollectionEvent, ColumnSpec, ListDrag, NoValue, Outcome, Pattern, Props as _, ScrollAnchor, SelectMode, Show as _, ShowCollection as _,
    ShowContainer as _, Sizing, ToggleColumn, request_rename,
};
use crate::layout_components::Stack;
use crate::list::{List, ListRow, RowInk, ThumbSize, ToggleCell, ToggleState};
use crate::menu::{Menu, MenuItem};
use crate::primitives::IconSpec;
use crate::primitives::textures::{Priority, ThumbPaint, ThumbRequest, ThumbSource, Thumbnails};
use crate::runtime::env::{Env, Label};
use crate::sections::{FooterAction, FooterBar};
use craft_behavior::{ColumnKey, ColumnWidth, GesturePhase, IconId, Selection, SelectionChange, keyed};
use craft_keymap::Mods;
use egui::{Response, Ui};
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The column of the source marks.
const SOURCE: ColumnKey = ColumnKey::new("historySource");
/// The column of the names.
const NAME: ColumnKey = ColumnKey::new("historyName");
/// The width in points a snapshot's picture is requested for.
const THUMB_POINTS: f32 = 32.0;

/// What a history row stands for: a named snapshot or a state of the undo stack, by number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HistoryKey {
    /// The snapshot with this number.
    Snapshot(u64),
    /// The state with this number.
    State(u64),
}

/// One row of the history list.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct HistoryRow {
    /// What the row stands for.
    pub key: HistoryKey,
    /// The row's name.
    pub label: Label,
    /// The icon before the name.
    pub icon: Option<IconId>,
    /// The state is one that was undone and can be redone.
    pub redo: bool,
}

impl HistoryRow {
    fn new(key: HistoryKey, label: Label) -> Self {
        HistoryRow { key, label, icon: None, redo: false }
    }

    /// A row for the snapshot `id` named `label`.
    pub fn snapshot(id: u64, label: impl Into<Label>) -> Self {
        Self::new(HistoryKey::Snapshot(id), label.into())
    }

    /// A row for the state `id` named `label`.
    pub fn state(id: u64, label: impl Into<Label>) -> Self {
        Self::new(HistoryKey::State(id), label.into())
    }

    /// Shows `icon` before the name.
    #[must_use]
    pub fn icon(mut self, icon: IconId) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Marks the state as one that can be redone.
    #[must_use]
    pub fn redo(mut self, redo: bool) -> Self {
        self.redo = redo;
        self
    }
}

/// Which buttons the history footer shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct HistoryFooter {
    /// The Delete button.
    pub delete: bool,
    /// The New Snapshot button.
    pub new_snapshot: bool,
    /// The New Document button.
    pub new_document: bool,
}

impl HistoryFooter {
    /// This footer with the Delete button shown when `shown`.
    pub const fn delete(mut self, shown: bool) -> Self {
        self.delete = shown;
        self
    }

    /// This footer with the New Snapshot button shown when `shown`.
    pub const fn new_snapshot(mut self, shown: bool) -> Self {
        self.new_snapshot = shown;
        self
    }

    /// This footer with the New Document button shown when `shown`.
    pub const fn new_document(mut self, shown: bool) -> Self {
        self.new_document = shown;
        self
    }
}

/// What the user did in the history panel.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum HistoryEvent {
    /// The user picked a row: the document goes to it.
    Select(HistoryKey),
    /// The user set the row as the source of the history brush.
    SetSource(HistoryKey),
    /// The user renamed a snapshot.
    #[non_exhaustive]
    Rename {
        /// The snapshot's number.
        snapshot: u64,
        /// The new name.
        text: String,
    },
    /// The user deleted the row, or the picked one when `None`.
    Delete(Option<HistoryKey>),
    /// The user asked for a new snapshot; `mods` were held.
    #[non_exhaustive]
    NewSnapshot {
        /// The modifiers held with the press.
        mods: Mods,
    },
    /// The user asked for a new document from the picked row.
    NewDocument,
}

impl HistoryEvent {
    /// Renames snapshot `snapshot` to `text`.
    pub fn rename(snapshot: u64, text: impl Into<String>) -> Self {
        HistoryEvent::Rename { snapshot, text: text.into() }
    }

    /// Asks for a new snapshot with `mods` held.
    pub const fn new_snapshot(mods: Mods) -> Self {
        HistoryEvent::NewSnapshot { mods }
    }
}

/// What [`HistoryPanel`] reports.
#[non_exhaustive]
pub struct HistoryPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// What the user did this pass, in order.
    pub events: Vec<HistoryEvent>,
}

impl Outcome for HistoryPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Whether a row may be deleted.
type DeletePredicate<'a> = Box<dyn Fn(&HistoryKey) -> bool + 'a>;

/// The footer buttons, in the order they stand.
#[derive(Clone, Copy)]
enum Slot {
    Delete,
    NewSnapshot,
    NewDocument,
}

/// The history panel: `rows` with the `current` one marked, thumbnails of the snapshots and a footer of buttons.
#[must_use = "show the panel with `show`"]
#[derive(Pattern)]
pub struct HistoryPanel<'a> {
    rows: &'a [HistoryRow],
    current: Option<HistoryKey>,
    source: Option<HistoryKey>,
    has_source: bool,
    thumbs: Option<(&'a Thumbnails<u64>, &'a dyn ThumbSource<u64>)>,
    can_delete: Option<DeletePredicate<'a>>,
    footer: HistoryFooter,
    reveal: Option<HistoryKey>,
    rename: Option<u64>,
    enabled: bool,
}

impl<'a> HistoryPanel<'a> {
    /// A panel over `rows`, with `current` marked.
    pub fn new(rows: &'a [HistoryRow], current: Option<HistoryKey>) -> Self {
        HistoryPanel {
            rows,
            current,
            source: None,
            has_source: false,
            thumbs: None,
            can_delete: None,
            footer: HistoryFooter::default(),
            reveal: None,
            rename: None,
            enabled: true,
        }
    }

    /// Shows the source column and marks the row that is the source of the history brush.
    pub fn source(mut self, source: Option<HistoryKey>) -> Self {
        self.source = source;
        self.has_source = true;
        self
    }

    /// Shows the pictures of the snapshot rows from `source`.
    pub fn thumbnails(mut self, thumbnails: &'a Thumbnails<u64>, source: &'a dyn ThumbSource<u64>) -> Self {
        self.thumbs = Some((thumbnails, source));
        self
    }

    /// Which rows the Delete button and key take; the others refuse. Without it every row is taken.
    pub fn can_delete(mut self, can: impl Fn(&HistoryKey) -> bool + 'a) -> Self {
        self.can_delete = Some(Box::new(can));
        self
    }

    /// Which footer buttons show.
    pub fn footer(mut self, footer: HistoryFooter) -> Self {
        self.footer = footer;
        self
    }

    /// Scrolls to `key` and shows it.
    pub fn reveal(mut self, key: HistoryKey) -> Self {
        self.reveal = Some(key);
        self
    }

    /// Opens the rename field of the snapshot `snapshot`.
    pub fn start_rename(mut self, snapshot: u64) -> Self {
        self.rename = Some(snapshot);
        self
    }

    /// Whether the panel takes input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Shows the panel.
    pub fn show(self, ui: &mut Ui) -> HistoryPanelOutput {
        let HistoryPanel { rows, current, source, has_source, thumbs, can_delete, footer, reveal, rename, enabled } = self;
        let ctx = ui.ctx().clone();
        let env = Env::of(ui);
        let words = env.words::<PanelWords>();
        let list_id = ui.scope_id().with("history-rows");
        let pixels_per_point = ui.ctx().pixels_per_point();
        // The drag path needs a `'static` rule, so the host's borrowed `can_delete` is evaluated once per pass
        // into this set; the drag payload, the drop acceptance, the Delete key and the menu all read the set.
        let deletable: Arc<HashSet<HistoryKey>> =
            Arc::new(rows.iter().map(|row| row.key).filter(|key| can_delete.as_ref().is_none_or(|can| can(key))).collect());
        let events: RefCell<Vec<HistoryEvent>> = RefCell::new(Vec::new());

        if let Some(snapshot) = rename {
            request_rename(&ctx, list_id, &HistoryKey::Snapshot(snapshot));
        }

        let slots: Vec<(Slot, FooterAction)> = {
            let icon = |name: &'static str| IconSpec::svg(IconId::new(name));
            let mut slots = Vec::new();
            if footer.delete {
                let takes = current.is_some_and(|key| deletable.contains(&key));
                let set = Arc::clone(&deletable);
                let action = FooterAction::new(icon("trash"), words.history_delete_tip.clone())
                    .enabled(enabled && takes)
                    .takes_drops_disabled(enabled)
                    .accepts_drops(move |drag: &ListDrag<HistoryKey>| drag.list == list_id && set.contains(&drag.grabbed));
                slots.push((Slot::Delete, action));
            }
            if footer.new_snapshot {
                slots.push((Slot::NewSnapshot, FooterAction::new(icon("scan"), words.history_new_snapshot.clone()).enabled(enabled)));
            }
            if footer.new_document {
                slots.push((
                    Slot::NewDocument,
                    FooterAction::new(icon("new_file"), words.history_new_document.clone()).enabled(enabled && current.is_some()),
                ));
            }
            slots
        };

        let mut stack = Stack::new().fill();
        if !slots.is_empty() {
            stack = stack.footer(|ui, _| {
                let out = FooterBar::actions(slots.iter().map(|(_, action)| action.clone())).show_footer(ui);
                // The footer runs twice per pass: the sizing pass measures it and must report nothing.
                if ui.is_sizing_pass() {
                    return;
                }
                let mut events = events.borrow_mut();
                if let Some(picked) = out.picked.and_then(|at| slots.get(at)) {
                    match picked.0 {
                        Slot::Delete => events.push(HistoryEvent::Delete(None)),
                        Slot::NewSnapshot => events.push(HistoryEvent::NewSnapshot { mods: out.mods }),
                        Slot::NewDocument => events.push(HistoryEvent::NewDocument),
                    }
                }
                if let Some(dropped) = &out.dropped
                    && matches!(slots.get(dropped.index), Some((Slot::Delete, _)))
                    && let Some(drag) = dropped.payload::<ListDrag<HistoryKey>>()
                    && deletable.contains(&drag.grabbed)
                {
                    events.push(HistoryEvent::Delete(Some(drag.grabbed)));
                }
            });
        }

        let shown = stack.show(ui, |ui, _| {
            // The row menu holds Rename and Delete only, so it opens only where one of them applies.
            let renamable = |key: &HistoryKey| enabled && matches!(key, HistoryKey::Snapshot(_));
            let removable = |key: &HistoryKey| enabled && deletable.contains(key);
            let mut held = current.map_or_else(Selection::default, Selection::single);
            let set = Arc::clone(&deletable);
            let mut list = List::<HistoryKey>::new(words.history_list.clone())
                .id(list_id)
                .select(SelectMode::Single)
                .rename(true)
                .can_rename(|key: &HistoryKey| matches!(key, HistoryKey::Snapshot(_)))
                .empty(words.history_empty.clone())
                .selection(Bind::bound(&mut held))
                .sizing(Sizing::Fill)
                .scroll_anchor(ScrollAnchor::End)
                .enabled(enabled)
                .drag_payload(move |drag: &ListDrag<HistoryKey>| set.contains(&drag.grabbed).then(|| drag.clone()));
            if has_source {
                list = list.header(false).columns(vec![
                    ColumnSpec::toggle(SOURCE, words.history_source.clone(), ToggleColumn::exclusive()),
                    ColumnSpec::new(NAME, Label::Empty, ColumnWidth::fill(1.0)),
                ]);
            }
            if let Some(key) = reveal {
                list = list.reveal(key);
            }
            let items = keyed(rows, |row: &HistoryRow| row.key).with_toggled(move |row, _| source == Some(row.key));
            let out = list.show_items(ui, &items, &mut |_, icx| {
                let row = &rows[icx.at];
                let mut item = ListRow::new(row.label.clone()).cells(icx.cells()).state(icx.row.clone());
                if let Some(icon) = row.icon.clone() {
                    item = item.icon(IconSpec::svg(icon));
                }
                if row.redo {
                    item = item.ink(RowInk::Dimmed);
                }
                if has_source {
                    let on = ToggleState::of(source == Some(row.key));
                    item = item.toggle(ToggleCell::check(SOURCE, on, words.history_source_cell.clone()));
                }
                if let (HistoryKey::Snapshot(id), Some((thumbnails, picture))) = (row.key, thumbs) {
                    let request = ThumbRequest::for_cell(THUMB_POINTS, pixels_per_point);
                    let state = thumbnails.state(&ctx, picture, &id, &request, Priority::VISIBLE);
                    item = item.thumb(ThumbSize::Small, move |cx, rect| {
                        thumbnails.paint(cx, rect, &state, &ThumbPaint::default());
                    });
                }
                item
            });
            let response = out.response.clone();
            // The row whose menu is open, as the list keeps it while the menu stays open.
            let mut target = out.menu.as_ref().map(|request| request.key);
            let mut picked = events.borrow_mut();
            for event in out.events {
                match event {
                    CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => {
                        if let Some(key) = keys.last() {
                            picked.push(HistoryEvent::Select(*key));
                        }
                    }
                    CollectionEvent::Toggle(edit) => {
                        let ended = matches!(edit.phase(), Some(GesturePhase::Commit) | None);
                        if let (true, Some(cell)) = (ended, edit.value())
                            && cell.on
                            && cell.column == SOURCE
                        {
                            picked.push(HistoryEvent::SetSource(cell.key));
                        }
                    }
                    CollectionEvent::Rename(edit) => {
                        let ended = matches!(edit.phase(), Some(GesturePhase::Commit) | None);
                        if let (true, Some(rename)) = (ended, edit.value())
                            && let HistoryKey::Snapshot(snapshot) = rename.key
                        {
                            let text = rename.name.trim();
                            let changed = rows.iter().find(|row| row.key == rename.key).is_some_and(|row| row.label.as_str() != text);
                            if changed && !text.is_empty() {
                                picked.push(HistoryEvent::Rename { snapshot, text: text.to_owned() });
                            }
                        }
                    }
                    CollectionEvent::Delete(keys) => {
                        picked.extend(keys.into_iter().filter(|key| deletable.contains(key)).map(|key| HistoryEvent::Delete(Some(key))));
                    }
                    CollectionEvent::ContextMenu(request) if renamable(&request.key) || removable(&request.key) => {
                        target = Some(request.key);
                    }
                    _ => {}
                }
            }
            drop(picked);

            if let Some(key) = target {
                let renamable = renamable(&key);
                let removable = removable(&key);
                let mut deleted = false;
                Menu::context(words.history_list.clone(), &response).show(ui, |ui, _| {
                    if MenuItem::new(words.history_rename.clone()).enabled(renamable).show(ui).ran {
                        request_rename(&ctx, list_id, &key);
                    }
                    if MenuItem::new(words.history_delete.clone()).enabled(removable).show(ui).ran {
                        deleted = true;
                    }
                });
                if deleted {
                    events.borrow_mut().push(HistoryEvent::Delete(Some(key)));
                }
            }
        });
        HistoryPanelOutput { response: shown.output, events: events.into_inner() }
    }
}
