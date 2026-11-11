//! The body above the footer: the empty notice, Button Mode's buttons or the tree, and the mapping
//! of the list's events to the panel's.

use craft_behavior::{ColumnWidth, EditEvent, IconId, Mixed, Selection};
use craft_theme::variant::Tone;
use craft_ui::buttons::Button;
use craft_ui::contract::{
    Bind, CollectionEvent, ColumnKey, ColumnKind, ColumnSpec, ListDrag, OpenChange, Outcome as _, Props as _, ReorderMove, ReorderRule, Scroll,
    SelectMode, Show as _, ShowCollection as _, ShowContainer as _, ToggleColumn, ToggleEdit, request_rename,
};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListWords};
use craft_ui::notices::{Notice, NoticeIcon, NoticeLayout};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label};
use egui::{Id, Response, Ui};

use super::menu::OpenMenu;
use super::panel::{ActionSelection, ActionsEvent, ActionsHost, ActionsPanelState, Session};
use super::rows::{ActionRows, DIALOG_COLUMN, INCLUDE_COLUMN, NAME_COLUMN, build_row, cells};
use super::words::ActionsWords;
use crate::actions::{ActionList, moved_index};

/// What the pieces of the panel read: the app's side, the words and the list's id.
#[derive(Clone, Copy)]
pub(super) struct Body<'a> {
    pub(super) list: &'a ActionList,
    pub(super) host: Option<&'a dyn ActionsHost>,
    pub(super) words: &'a ActionsWords,
    pub(super) list_words: &'a ListWords,
    pub(super) env: &'a Env,
    /// The id the tree's list shows under, which its drags carry as `ListDrag::list`.
    pub(super) list_id: Id,
}

/// Where a moved row goes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActionPlace {
    /// An action dropped at list index `index` (after its removal), into `set` (`None`: loose).
    #[non_exhaustive]
    Action {
        /// The index in [`ActionList::actions`] after the removal.
        index: usize,
        /// The set it lands in.
        set: Option<String>,
    },
    /// A step dropped at position `index` of action `action`.
    #[non_exhaustive]
    Step {
        /// The action it lands in.
        action: usize,
        /// The position after the removal.
        index: usize,
    },
}

impl ActionPlace {
    /// An action dropped at list index `index` (after its removal), into `set` (`None`: loose).
    pub fn action(index: usize, set: Option<String>) -> Self {
        Self::Action { index, set }
    }

    /// A step dropped at position `index` of action `action`.
    pub fn step(action: usize, index: usize) -> Self {
        Self::Step { action, index }
    }
}

/// A row moved by a drag or by key.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ActionMove {
    /// The row that moved.
    pub row: ActionSelection,
    /// Where it lands.
    pub place: ActionPlace,
}

/// What the body returns.
pub(super) struct BodyOutput {
    pub(super) events: Vec<ActionsEvent>,
    pub(super) response: Response,
    /// The selection the list ended on, for the caller to store after the container; `None` when the body did not show the tree.
    pub(super) selected: Option<Option<ActionSelection>>,
}

/// The part of the panel above the footer: the empty notice, Button Mode's buttons or the tree.
pub(super) fn show_body(ui: &mut Ui, body: &Body<'_>, state: &mut ActionsPanelState, session: &mut Session, rename: Option<usize>) -> BodyOutput {
    let Body { list, host, words, .. } = *body;
    if list.actions().is_empty() && list.sets().is_empty() {
        let icon = NoticeIcon::Custom(IconSpec::svg(craft_behavior::IconId::new("circle")));
        let out = Notice::new(Tone::Neutral, words.empty.clone()).layout(NoticeLayout::Centred).icon(icon).show(ui);
        return BodyOutput { events: Vec::new(), response: out.response().clone(), selected: None };
    }
    if state.settings.button_mode && host.and_then(|host| host.recording()).is_none() {
        return show_buttons(ui, body);
    }
    show_tree(ui, body, state, session, rename)
}

/// Button Mode's body: each action of `list` in display order as a button in its colour, set names
/// between; a click, not a double click's second, reports `Play`.
fn show_buttons(ui: &mut Ui, body: &Body<'_>) -> BodyOutput {
    let list = body.list;
    let mut events = Vec::new();
    let order = list.display_order();
    let mut shown: Vec<(Option<&str>, Vec<usize>)> = vec![(None, order.iter().copied().filter(|&index| list.set_of(index).is_none()).collect())];
    for set in list.sets() {
        shown.push((Some(set), order.iter().copied().filter(|&index| list.set_of(index) == Some(set)).collect()));
    }
    let shown_buttons = Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| {
        for (set, actions) in &shown {
            if let Some(name) = set {
                Text::new((*name).to_owned()).show(ui);
            }
            Row::new().wrap(true).show(ui, |ui, _| {
                for &index in actions {
                    let Some(action) = list.action(index) else { continue };
                    let mut button = Button::new(action.name.clone());
                    if let Some(color) = action.color.label_color() {
                        button = button.tint(color.fill());
                    }
                    let response = button.show(ui);
                    if response.clicks == 1 {
                        events.push(ActionsEvent::Play { action: index, from: 0 });
                    }
                }
            });
        }
    });
    BodyOutput { events, response: shown_buttons.output, selected: None }
}

/// The column of a toggle edit, when it names one.
fn toggle_column(edit: &EditEvent<ToggleEdit<ActionSelection>>) -> Option<&ColumnKey> {
    match edit {
        EditEvent::Update { value: cell, .. } | EditEvent::Set { after: cell, .. } | EditEvent::Commit { after: cell, .. } => Some(&cell.column),
        EditEvent::Begin { before: Mixed::Same(cell), .. } | EditEvent::Cancel { before: Some(Mixed::Same(cell)), .. } => Some(&cell.column),
        _ => None,
    }
}

/// The tree on a [`List`] in tree mode, and the events of what happened on it.
fn show_tree(ui: &mut Ui, body: &Body<'_>, state: &mut ActionsPanelState, session: &mut Session, rename: Option<usize>) -> BodyOutput {
    let Body { list, host, words, list_id, .. } = *body;
    let editable = !host.is_some_and(|host| host.playing());
    let recording = host.and_then(|host| host.recording());
    let rows = ActionRows::build(list, state, host);
    let order = rows.keys();
    let mut selection = Selection::new(state.selected.iter().cloned().collect(), state.selected.clone(), state.selected.clone());
    if let Some(index) = rename.filter(|_| editable) {
        request_rename(ui.ctx(), list_id, &ActionSelection::Action(index));
    }
    let columns = vec![
        ColumnSpec::new(INCLUDE_COLUMN, Label::Empty, ColumnWidth::Controls(1.0)).kind(ColumnKind::Toggle(ToggleColumn::sweepable())),
        ColumnSpec::new(DIALOG_COLUMN, Label::Empty, ColumnWidth::Controls(1.0))
            .kind(ColumnKind::Toggle(ToggleColumn::sweepable()))
            .icon(IconId::new("note")),
        ColumnSpec::new(NAME_COLUMN, Label::Empty, ColumnWidth::fill(1.0)),
    ];
    let mut widget = List::new(words.name.clone())
        .id(list_id)
        .tree(true)
        .select(SelectMode::Single)
        .allow_empty(true)
        .deletable(true)
        .rename(true)
        .rename_column(NAME_COLUMN)
        .columns(columns)
        .selection(Bind::bound(&mut selection))
        .can_rename(|key| editable && matches!(key, ActionSelection::Action(_)))
        .drag_payload(|drag: &ListDrag<ActionSelection>| Some(drag.clone()));
    if editable {
        widget = widget.reorder(ReorderRule::new().into_groups()).accepts(|drag: &ListDrag<ActionSelection>, _over, insert| {
            let moved_before = order[..insert.min(order.len())].iter().filter(|key| drag.keys.contains(key)).count();
            let mv = ReorderMove::new(drag.grabbed.clone(), drag.keys.clone(), insert.saturating_sub(moved_before), false);
            place_of(list, &order, &mv, true).is_some()
        });
    }
    if let Some(key) = reveal_key(session, list, host, &order) {
        widget = widget.reveal(key);
    }
    let shown: &ActionsPanelState = state;
    let out = widget.show_items(ui, &rows, &mut |key, item| {
        let mut row = build_row(body, shown, key).cells(item.cells()).state(item.row).depth(rows.depth_at(item.at));
        for cell in cells(body, key) {
            row = row.toggle(cell);
        }
        if rows.is_group(item.at) { row.group(rows.is_open(item.at)) } else { row }
    });

    let mut events = Vec::new();
    let mut moves = Vec::new();
    for event in out.events {
        match event {
            CollectionEvent::Open(change) => apply_open(state, list, &change),
            CollectionEvent::Activate(key) => match key {
                ActionSelection::Action(action) if host.is_some() => events.push(ActionsEvent::Play { action, from: 0 }),
                ActionSelection::Set(ref name) => {
                    let open = state.settings.collapsed_sets.contains(name);
                    apply_open(state, list, &OpenChange::new(key, open));
                }
                _ => {}
            },
            CollectionEvent::Toggle(edit) if editable => {
                let sweep = if toggle_column(&edit) == Some(&INCLUDE_COLUMN) { &mut session.include } else { &mut session.dialog };
                events.extend(sweep.feed(&edit).map(ActionsEvent::Toggle));
            }
            CollectionEvent::Rename(edit) if editable => {
                if let EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } = edit
                    && let ActionSelection::Action(action) = after.key
                {
                    let text = after.name.trim();
                    if !text.is_empty() && list.action(action).is_some_and(|current| current.name != text) {
                        events.push(ActionsEvent::Rename { action, text: text.to_owned() });
                    }
                }
            }
            CollectionEvent::Reorder(EditEvent::Set { after, .. }) if editable => {
                if let Some(place) = place_of(list, &order, &after, false) {
                    moves.push((after.key.clone(), place.clone()));
                    events.push(ActionsEvent::Reorder(ActionMove { row: after.key, place }));
                }
            }
            CollectionEvent::Delete(keys) if editable => {
                events.extend(keys.first().and_then(|key| delete_event(key, recording.is_some())));
            }
            CollectionEvent::ContextMenu(request) => session.menu = OpenMenu::opened(request, list, host),
            _ => {}
        }
    }
    let mut selected = selection.items.first().cloned();
    for (source, to) in &moves {
        selected = selected.map(|selected| remapped(selected, source, to));
    }
    BodyOutput { events, response: out.response, selected: Some(selected) }
}

/// The `Delete` event for `selected`: a step always; a set or an action not while recording.
pub(super) fn delete_event(selected: &ActionSelection, recording: bool) -> Option<ActionsEvent> {
    (!recording || matches!(selected, ActionSelection::Step(..))).then(|| ActionsEvent::Delete(selected.clone()))
}

/// Opens or closes what `change` names in `state`: an action's steps, a set's actions, or with
/// `all` every one of them.
fn apply_open(state: &mut ActionsPanelState, list: &ActionList, change: &OpenChange<ActionSelection>) {
    if change.all {
        if change.open {
            state.expanded.extend(list.actions().iter().map(|action| action.name.clone()));
            state.settings.collapsed_sets.clear();
        } else {
            state.expanded.clear();
            state.settings.collapsed_sets = list.sets().into_iter().map(str::to_owned).collect();
        }
        return;
    }
    match &change.key {
        ActionSelection::Action(index) => {
            if let Some(action) = list.action(*index) {
                if change.open {
                    state.expanded.insert(action.name.clone());
                } else {
                    state.expanded.remove(&action.name);
                }
            }
        }
        ActionSelection::Set(name) => {
            state.settings.collapsed_sets.retain(|set| set != name);
            if !change.open {
                state.settings.collapsed_sets.push(name.clone());
            }
        }
        ActionSelection::Step(..) => {}
    }
}

/// The row to scroll into view while recording: the latest step, whenever the recording action or
/// its step count differs from the last pass; `None` otherwise.
fn reveal_key(session: &mut Session, list: &ActionList, host: Option<&dyn ActionsHost>, order: &[ActionSelection]) -> Option<ActionSelection> {
    let Some(recording) = host.and_then(|host| host.recording()) else {
        session.revealed = None;
        return None;
    };
    let total = list.step_count(recording) + host.map_or(0, |host| host.pending_steps(recording).len());
    if session.revealed == Some((recording, total)) {
        return None;
    }
    session.revealed = Some((recording, total));
    let key = if total > 0 { ActionSelection::Step(recording, total - 1) } else { ActionSelection::Action(recording) };
    order.contains(&key).then_some(key)
}

/// Where the move `mv` of the displayed rows `order` lands in the list, or `None` when the tree
/// does not take it (more than one row, a copy, or a place that is no action's or step's):
/// - an action lands before an action that is next, after the action that is before, as the first
///   of the set it is dropped into, or at the end of the set before a later set (where the drop
///   line is drawn); before the first set it is loose;
/// - a step lands in the action it is dropped into or beside, at the position among that action's
///   steps.
///
/// `relaxed` ignores the group the list reports for the place, for the check while the pointer is
/// still moving, when the list has no group to report yet.
pub(super) fn place_of(list: &ActionList, order: &[ActionSelection], mv: &ReorderMove<ActionSelection>, relaxed: bool) -> Option<ActionPlace> {
    if mv.copy || mv.keys.len() != 1 {
        return None;
    }
    let moved = &mv.keys[0];
    let remaining: Vec<&ActionSelection> = order.iter().filter(|key| *key != moved).collect();
    let to = mv.to.min(remaining.len());
    let next = remaining.get(to).copied();
    let prev = to.checked_sub(1).map(|at| remaining[at]);
    match moved {
        ActionSelection::Action(from) => {
            let len = list.actions().len();
            let parent_set = match &mv.parent {
                Some(ActionSelection::Set(name)) => Some(name.as_str()),
                Some(_) => return None,
                None => None,
            };
            let end_of = |set: &str| list.actions().iter().rposition(|action| action.set.as_deref() == Some(set)).map_or(len, |last| last + 1);
            let (at, set) = match (prev, next) {
                (Some(ActionSelection::Set(header)), _) if parent_set == Some(header.as_str()) => {
                    let first = list.actions().iter().position(|action| action.set.as_deref() == Some(header.as_str()));
                    (first.unwrap_or(len), Some(header.clone()))
                }
                (_, Some(ActionSelection::Action(over))) if relaxed || list.set_of(*over) == parent_set => {
                    (*over, list.set_of(*over).map(str::to_owned))
                }
                (Some(ActionSelection::Action(over) | ActionSelection::Step(over, _)), _) => (over + 1, list.set_of(*over).map(str::to_owned)),
                (Some(ActionSelection::Set(header)), _) => (end_of(header), Some(header.clone())),
                (None, _) => (0, None),
            };
            Some(ActionPlace::Action { index: at - usize::from(*from < at), set })
        }
        ActionSelection::Step(from, step) => {
            if *step >= list.step_count(*from) {
                return None;
            }
            let action = match (&mv.parent, prev) {
                (Some(ActionSelection::Action(action)), _) => *action,
                (Some(_), _) => return None,
                (None, Some(ActionSelection::Action(action) | ActionSelection::Step(action, _))) => *action,
                (None, _) => return None,
            };
            let held = list.step_count(action) - usize::from(action == *from);
            let slot = remaining[..to].iter().filter(|key| matches!(key, ActionSelection::Step(owner, _) if *owner == action)).count();
            Some(ActionPlace::Step { action, index: slot.min(held) })
        }
        ActionSelection::Set(_) => None,
    }
}

/// Where `selected` is after `source` moved to `to`: the moved row and the rows it passed follow
/// the list's own shifts.
fn remapped(selected: ActionSelection, source: &ActionSelection, to: &ActionPlace) -> ActionSelection {
    match (source, to) {
        (ActionSelection::Action(from), ActionPlace::Action { index, .. }) => match selected {
            ActionSelection::Action(at) => ActionSelection::Action(moved_index(at, *from, *index)),
            ActionSelection::Step(at, step) => ActionSelection::Step(moved_index(at, *from, *index), step),
            set @ ActionSelection::Set(_) => set,
        },
        (ActionSelection::Step(from, moved), ActionPlace::Step { action: into, index }) => match selected {
            ActionSelection::Step(at, step) if at == *from && step == *moved => ActionSelection::Step(*into, *index),
            ActionSelection::Step(at, step) if at == *from && from == into => ActionSelection::Step(at, moved_index(step, *moved, *index)),
            ActionSelection::Step(at, step) if at == *from && step > *moved => ActionSelection::Step(at, step - 1),
            ActionSelection::Step(at, step) if at == *into && step >= *index => ActionSelection::Step(at, step + 1),
            other => other,
        },
        _ => selected,
    }
}
