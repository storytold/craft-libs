//! The rows of the tree: the display order over the list, and one row's `ListRow`.

use std::borrow::Cow;

use craft_behavior::{ColumnKey, DisplayOrder, IconId, ItemSource};
use craft_theme::variant::Tone;
use craft_ui::list::{ListRow, RowInk, ToggleCell, ToggleFace, ToggleState};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::Label;

use super::body::Body;
use super::panel::{ActionSelection, ActionsHost, ActionsPanelState};
use crate::actions::{ActionInclude, ActionList};

/// The include column: a toggle on action rows (every step) and step rows.
pub const INCLUDE_COLUMN: ColumnKey = ColumnKey::new("include");
/// The dialog column: a toggle on step rows.
pub const DIALOG_COLUMN: ColumnKey = ColumnKey::new("dialog");
/// The column that holds the disclosure, the icon, the name and the trailing items.
pub(super) const NAME_COLUMN: ColumnKey = ColumnKey::new("name");

/// One displayed row.
struct Shown {
    key: ActionSelection,
    depth: usize,
    group: bool,
    open: bool,
}

/// The rows the tree shows this pass, in order, with their depths: loose actions, then each set
/// header and (unless collapsed) its actions; an open action's steps and its pending steps after
/// it. Built once per pass from the list, the state and the host. Every action and every set is a
/// group, so it has a disclosure even with nothing inside, as PhotoCraft's does. The recording
/// action and its set are open whatever the state says.
pub(super) struct ActionRows<'a> {
    rows: Vec<Shown>,
    list: &'a ActionList,
    host: Option<&'a dyn ActionsHost>,
}

/// Whether the set `name` shows its actions: not collapsed, or holding the action being recorded.
fn set_open(list: &ActionList, state: &ActionsPanelState, host: Option<&dyn ActionsHost>, name: &str) -> bool {
    let recording = host.and_then(|host| host.recording());
    !state.settings.collapsed_sets.iter().any(|set| set == name) || recording.is_some_and(|action| list.set_of(action) == Some(name))
}

/// Whether the action `index` shows its steps: expanded, or being recorded.
fn action_open(list: &ActionList, state: &ActionsPanelState, host: Option<&dyn ActionsHost>, index: usize) -> bool {
    host.and_then(|host| host.recording()) == Some(index) || list.action(index).is_some_and(|action| state.expanded.contains(&action.name))
}

impl<'a> ActionRows<'a> {
    pub(super) fn build(list: &'a ActionList, state: &ActionsPanelState, host: Option<&'a dyn ActionsHost>) -> Self {
        let mut out = Self { rows: Vec::new(), list, host };
        let order = list.display_order();
        for &index in order.iter().filter(|&&index| list.set_of(index).is_none()) {
            out.push_action(state, index, 0);
        }
        for set in list.sets() {
            let open = set_open(list, state, host, set);
            out.rows.push(Shown { key: ActionSelection::Set(set.to_owned()), depth: 0, group: true, open });
            if open {
                for &index in order.iter().filter(|&&index| list.set_of(index) == Some(set)) {
                    out.push_action(state, index, 1);
                }
            }
        }
        out
    }

    fn push_action(&mut self, state: &ActionsPanelState, index: usize, depth: usize) {
        let open = action_open(self.list, state, self.host, index);
        self.rows.push(Shown { key: ActionSelection::Action(index), depth, group: true, open });
        if open {
            let shown = self.list.step_count(index) + self.host.map_or(0, |host| host.pending_steps(index).len());
            for step in 0..shown {
                self.rows.push(Shown { key: ActionSelection::Step(index, step), depth: depth + 1, group: false, open: false });
            }
        }
    }

    /// The keys in display order.
    pub(super) fn keys(&self) -> Vec<ActionSelection> {
        self.rows.iter().map(|row| row.key.clone()).collect()
    }

    /// Whether the row at `at` has a disclosure that is open.
    pub(super) fn is_open(&self, at: usize) -> bool {
        self.rows.get(at).is_some_and(|row| row.open)
    }

    pub(super) fn depth_at(&self, at: usize) -> usize {
        self.rows.get(at).map_or(0, |row| row.depth)
    }

    pub(super) fn is_group(&self, at: usize) -> bool {
        self.rows.get(at).is_some_and(|row| row.group)
    }

    /// Whether the row at `at` is a step the action holds (not a pending one).
    fn recorded_step(&self, at: usize) -> Option<(usize, usize)> {
        match self.rows.get(at)?.key {
            ActionSelection::Step(action, step) if step < self.list.action(action)?.steps.len() => Some((action, step)),
            _ => None,
        }
    }
}

impl DisplayOrder<ActionSelection> for ActionRows<'_> {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, at: usize) -> ActionSelection {
        self.rows[at].key.clone()
    }

    fn depth(&self, at: usize) -> usize {
        self.depth_at(at)
    }

    fn has_children(&self, at: usize) -> bool {
        self.is_group(at)
    }

    fn open(&self, at: usize) -> bool {
        self.is_open(at)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        match &self.rows.get(at)?.key {
            ActionSelection::Set(name) => Some(Cow::Borrowed(name.as_str())),
            ActionSelection::Action(index) => self.list.action(*index).map(|action| Cow::Borrowed(action.name.as_str())),
            ActionSelection::Step(..) => None,
        }
    }
}

impl ItemSource<ActionSelection> for ActionRows<'_> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        let Some(row) = self.rows.get(at) else { return false };
        match (&row.key, column) {
            (ActionSelection::Action(index), column) if *column == INCLUDE_COLUMN => {
                self.list.action(*index).is_some_and(|action| action.include() == ActionInclude::All)
            }
            _ => match self.recorded_step(at) {
                Some((action, step)) if *column == INCLUDE_COLUMN => self.list.action(action).is_some_and(|held| held.step_enabled(step)),
                Some((action, step)) if *column == DIALOG_COLUMN => self.list.action(action).is_some_and(|held| held.dialogs.contains(&step)),
                _ => false,
            },
        }
    }

    fn toggle_enabled(&self, at: usize, column: &ColumnKey) -> bool {
        let Some(row) = self.rows.get(at) else { return false };
        match &row.key {
            ActionSelection::Action(index) => *column == INCLUDE_COLUMN && self.list.action(*index).is_some_and(|action| !action.steps.is_empty()),
            ActionSelection::Step(..) => self.recorded_step(at).is_some(),
            ActionSelection::Set(_) => false,
        }
    }
}

fn include_state(include: ActionInclude) -> ToggleState {
    match include {
        ActionInclude::All => ToggleState::On,
        ActionInclude::Partial => ToggleState::Mixed,
        ActionInclude::None => ToggleState::Off,
    }
}

/// The `ListRow` of `key`:
/// - a set: its folder icon, open or closed, its name and "{n} actions" at the end;
/// - an action: the play glyph, its name, the include cell unless it has no steps, the trailing
///   text (the shortcut and the step count, or "2/3 steps" when some are off), and while it is
///   recorded into a dot that names the recording;
/// - a step: the host's label, dimmed when switched off, the include and dialog cells.
pub(super) fn build_row(body: &Body<'_>, state: &ActionsPanelState, key: &ActionSelection) -> ListRow<'static> {
    let Body { list, host, words, env, .. } = *body;
    let digits = |n: usize| env.format().integer(n as u64);
    match key {
        ActionSelection::Set(name) => {
            let count = list.actions().iter().filter(|action| action.set.as_deref() == Some(name.as_str())).count();
            let open = set_open(list, state, host, name);
            let icon = IconId::new(if open { "folder_open" } else { "folder" });
            ListRow::new(name.clone()).heading().icon(IconSpec::svg(icon)).trailing(words.action_count.text(count as u64, &digits(count)))
        }
        ActionSelection::Action(index) => {
            let Some(action) = list.action(*index) else { return ListRow::new(Label::Empty) };
            let mut row = ListRow::new(action.name.clone()).icon(IconSpec::svg(IconId::new("play")));
            let steps = list.step_count(*index);
            let mut text = words.steps.text(steps as u64, &digits(steps));
            if action.include() == ActionInclude::Partial {
                let enabled = action.enabled_steps().count();
                text = words.partial_steps.fill(&[("enabled", &digits(enabled)), ("steps", &text)]);
            }
            if let Some(shortcut) = host.and_then(|host| host.shortcut(action)).map(|choice| choice.text()) {
                text = words.shortcut_steps.fill(&[("shortcut", &shortcut), ("steps", &text)]);
            }
            if host.and_then(|host| host.recording()) == Some(*index) {
                let name = words.recording_suffix.fill(&[("name", &action.name)]);
                row = row.accessible_name(name).trailing_icon(IconSpec::svg(IconId::new("circle")), words.recording.clone()).trailing_tone(Tone::Bad);
            }
            row.optional(text)
        }
        ActionSelection::Step(index, step) => {
            let Some(action) = list.action(*index) else { return ListRow::new(Label::Empty) };
            match action.steps.get(*step) {
                Some(recorded) => {
                    let label = host.map_or_else(|| recorded.command.clone(), |host| host.step_label(recorded));
                    ListRow::new(label).ink(if action.step_enabled(*step) { RowInk::Muted } else { RowInk::Dimmed })
                }
                None => {
                    let pending = host.and_then(|host| host.pending_steps(*index).get(*step - action.steps.len()));
                    let label =
                        pending.map_or_else(String::new, |pending| host.map_or_else(|| pending.command.clone(), |host| host.step_label(pending)));
                    ListRow::new(label).ink(RowInk::Muted)
                }
            }
        }
    }
}

/// The toggle cells of a row: the include cell of an action with steps and of a recorded step, the
/// dialog cell of a recorded step.
pub(super) fn cells(body: &Body<'_>, key: &ActionSelection) -> Vec<ToggleCell> {
    let Body { list, words, .. } = *body;
    let include = |state: ToggleState| ToggleCell::check(INCLUDE_COLUMN, state, words.include.clone()).tooltip(words.include_tooltip.clone());
    match key {
        ActionSelection::Action(index) => match list.action(*index) {
            Some(action) if !action.steps.is_empty() => vec![include(include_state(action.include()))],
            _ => Vec::new(),
        },
        ActionSelection::Step(index, step) => match list.action(*index) {
            Some(action) if *step < action.steps.len() => {
                let note = IconId::new("note");
                let mut dialog = ToggleCell::new(
                    DIALOG_COLUMN,
                    ToggleFace::icons(note.clone(), note),
                    ToggleState::of(action.dialogs.contains(step)),
                    words.dialog.clone(),
                )
                .tooltip(words.dialog_tooltip.clone());
                dialog.check_box = true;
                vec![include(ToggleState::of(action.step_enabled(*step))), dialog]
            }
            _ => Vec::new(),
        },
        ActionSelection::Set(_) => Vec::new(),
    }
}
