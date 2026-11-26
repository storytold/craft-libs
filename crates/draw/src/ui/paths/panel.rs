//! [`PathsPanel`]: the Paths panel on a `List` with a footer.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt::Debug;
use std::hash::Hash;

use craft_behavior::geometry::selection_op::SelectionOperation;
use craft_behavior::{ColumnWidth, DisplayOrder, EditEvent, ItemSource, Selection, SelectionChange};
use craft_ui::contract::{
    Bind, CollectionEvent, ColumnSpec, ListDrag, Props as _, SelectMode, ShowCollection as _, ShowContainer as _, request_rename,
};
use craft_ui::layout_components::Stack;
use craft_ui::list::{List, RowEvent};
use craft_ui::runtime::commands::{CommandState, Commands as _};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::memory::Remember as _;
use egui::Ui;

use super::footer::{PathDrag, show_footer};
use super::menu::show_row_menu;
use super::rows::{NAME, path_row};
use super::{PathKind, PathRow, PathsCommands, PathsEvent, PathsOutput, PathsWords};

/// What the panel keeps between passes, in its root slot: the row the last context request opened the menu on.
#[derive(Clone)]
struct Session<K> {
    menu: Option<K>,
}

impl<K> Default for Session<K> {
    fn default() -> Self {
        Session { menu: None }
    }
}

/// The rows as the list reads them: flat, in the app's order.
struct Order<'a, K> {
    rows: &'a [PathRow<K>],
}

impl<K: Clone> DisplayOrder<K> for Order<'_, K> {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, at: usize) -> K {
        self.rows[at].key.clone()
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.rows[at].name.as_str()))
    }
}

impl<K: Clone> ItemSource<K> for Order<'_, K> {}

/// The Paths panel: one row per path with a thumbnail of its outline, rename in place, and the footer's buttons.
///
/// A Cmd/Ctrl-click on the thumbnail loads the path as a selection. Fill Path and Stroke Path are greyed while
/// [`can_fill`](Self::can_fill) is false.
///
/// - **Selection**: rebuilt each pass from [`PathRow::selected`]; a click reports [`PathsEvent::Select`], `None` on
///   the empty area. A Cmd/Ctrl-click on the name is a plain select.
/// - **Load**: a Cmd/Ctrl-click on the thumbnail reports [`PathsEvent::LoadSelection`] with the operation of Shift and
///   Alt, and no `Select` for that click.
/// - **Rename**: a double click on a saved or work row, F2 or [`start_rename`](Self::start_rename). A work row's
///   field starts with [`work_name`](Self::work_name) and a commit reports [`PathsEvent::Rename`] even when the text
///   is unchanged; a saved row's empty or unchanged text reports nothing.
/// - **Drag**: saved, work and vector-mask rows drop on the footer's Delete button.
/// - **Menus**: a right-click opens the row's menu; commands of the panel menu act on the selected row.
/// - **Passes**: a pass reports only its own input and takes the commands queued for it, so a second pass in one
///   frame repeats nothing.
#[must_use = "show the panel with `show`"]
#[derive(craft_ui::contract::Pattern)]
#[pattern(probe(PathsPanel<'static, u32>))]
pub struct PathsPanel<'a, K: Clone + Eq + Hash + Debug + Send + Sync + 'static> {
    rows: &'a [PathRow<K>],
    document: kurbo::Size,
    can_fill: bool,
    work_name: Option<Label>,
    rename: Option<K>,
}

impl<'a, K: Clone + Eq + Hash + Debug + Send + Sync + 'static> PathsPanel<'a, K> {
    /// The panel showing `rows` in their order, thumbnails scaled to the `document`'s size.
    pub fn new(rows: &'a [PathRow<K>], document: kurbo::Size) -> Self {
        PathsPanel { rows, document, can_fill: true, work_name: None, rename: None }
    }

    /// Whether the document can be filled and stroked; true by default.
    pub fn can_fill(mut self, can_fill: bool) -> Self {
        self.can_fill = can_fill;
        self
    }

    /// The name the work path's rename field starts with.
    pub fn work_name(mut self, name: impl Into<Label>) -> Self {
        self.work_name = Some(name.into());
        self
    }

    /// Opens the rename field on the row `key`.
    pub fn start_rename(mut self, key: K) -> Self {
        self.rename = Some(key);
        self
    }

    /// Shows the panel.
    pub fn show(self, ui: &mut Ui) -> PathsOutput<K> {
        let PathsPanel { rows, document, can_fill, work_name, rename } = self;
        let env = Env::of(ui);
        let words = env.words::<PathsWords>();
        let list = ui.scope_id().with("paths-rows");
        let events: RefCell<Vec<PathsEvent<K>>> = RefCell::new(Vec::new());
        let ctx = ui.ctx().clone();
        let lead = rows.iter().find(|row| row.selected);

        let shown = Stack::new()
            .fill()
            .label(words.panel.clone())
            .command_scope(<PathsCommands as craft_ui::contract::CommandSet>::SCOPE)
            .footer(|ui, _| {
                let out = show_footer(ui, rows, &words, list);
                events.borrow_mut().extend(out);
            })
            .show(ui, |ui, stack| {
                while let Some((command, _)) = stack.take_command::<PathsCommands>() {
                    match lead {
                        Some(row) if command.applies_to(Some(row.kind), can_fill) => match command {
                            PathsCommands::RenamePath => request_rename(&ctx, list, &row.key),
                            command => events.borrow_mut().push(PathsEvent::command(command, Some(row.key.clone()))),
                        },
                        _ => {}
                    }
                }
                for command in PathsCommands::ALL {
                    stack.publish(command, CommandState::new().enabled(command.applies_to(lead.map(|row| row.kind), can_fill)));
                }
                if let Some(key) = &rename
                    && rows.iter().any(|row| row.key == *key && row.kind.renames())
                {
                    request_rename(&ctx, list, key);
                }

                let order = Order { rows };
                let selected: Vec<K> = rows.iter().filter(|row| row.selected).map(|row| row.key.clone()).collect();
                let lead_key = selected.last().cloned();
                let mut selection = Selection::new(selected, lead_key.clone(), lead_key);
                let kinds: Vec<(K, PathKind)> = rows.iter().map(|row| (row.key.clone(), row.kind)).collect();
                let payload_kinds = kinds.clone();
                let work_name = work_name.as_ref().map(|name| name.as_str().to_owned());
                let kind_of = |key: &K| kinds.iter().find(|(k, _)| k == key).map(|(_, kind)| *kind);
                let columns = vec![ColumnSpec::new(NAME, Label::Empty, ColumnWidth::fill(1.0))];
                let out = List::new(words.list.clone())
                    .id(list)
                    .allow_empty(true)
                    .select(SelectMode::Single)
                    .rename(true)
                    .rename_column(NAME)
                    .columns(columns)
                    .empty(words.empty.clone())
                    .selection(Bind::bound(&mut selection))
                    .can_rename(|key| kind_of(key).is_some_and(PathKind::renames))
                    .rename_draft(|key| if kind_of(key) == Some(PathKind::Work) { work_name.clone() } else { None })
                    .drag_payload(move |drag: &ListDrag<K>| {
                        let kind = payload_kinds.iter().find(|(key, _)| *key == drag.grabbed).map(|(_, kind)| *kind)?;
                        kind.drags().then(|| PathDrag { list: drag.list, grabbed: drag.grabbed.clone(), kind })
                    })
                    .show_items(ui, &order, &mut |_key, item| path_row(&rows[item.at], document).cells(item.cells()).state(item.row.clone()));

                let mut found = Vec::new();
                map_events(rows, out.events, &mut found, stack.memory::<Session<K>>());
                events.borrow_mut().extend(found);

                let open = stack.memory::<Session<K>>().menu.clone();
                if let Some(row) = open.and_then(|key| rows.iter().find(|row| row.key == key))
                    && let Some(command) = show_row_menu(ui, &out.response, row, can_fill, &words)
                {
                    match command {
                        PathsCommands::RenamePath => request_rename(&ctx, list, &row.key),
                        command => events.borrow_mut().push(PathsEvent::command(command, Some(row.key.clone()))),
                    }
                }
                out.response
            });
        PathsOutput { events: events.into_inner(), response: shown.output }
    }
}

/// What the list's events mean for the app: selects, loads and renames.
///
/// A Cmd/Ctrl-click on a thumbnail arrives as a thumbnail click and, in either order, as a selection of that row; the
/// batch is scanned first so the selection that only picks a loaded row is dropped.
fn map_events<K: Clone + PartialEq>(
    rows: &[PathRow<K>],
    list: Vec<CollectionEvent<K, RowEvent>>,
    events: &mut Vec<PathsEvent<K>>,
    session: &mut Session<K>,
) {
    let loaded: Vec<K> = list
        .iter()
        .filter_map(|event| match event {
            CollectionEvent::Item(key, RowEvent::ThumbClicked { mods, clicks: 1, .. }) if mods.cmd => Some(key.clone()),
            _ => None,
        })
        .collect();
    for event in list {
        match event {
            CollectionEvent::Selection { change: SelectionChange::Clear, .. } => events.push(PathsEvent::Select(None)),
            CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => {
                if matches!(keys.as_slice(), [only] if loaded.contains(only)) {
                    continue;
                }
                events.push(PathsEvent::Select(keys.last().cloned()));
            }
            CollectionEvent::Item(key, RowEvent::ThumbClicked { mods, clicks: 1, .. }) if mods.cmd => {
                events.push(PathsEvent::load_selection(key, SelectionOperation::from_mods(mods)));
            }
            CollectionEvent::Rename(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) => {
                let name = after.name.trim();
                let Some(row) = rows.iter().find(|row| row.key == after.key) else { continue };
                // The work path is saved even under its proposed name; an unchanged saved name is left alone.
                if !name.is_empty() && (row.kind == PathKind::Work || row.name != name) {
                    events.push(PathsEvent::rename(after.key.clone(), name));
                }
            }
            CollectionEvent::ContextMenu(request) => session.menu = Some(request.key),
            _ => {}
        }
    }
}
