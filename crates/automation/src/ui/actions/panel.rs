//! `ActionsPanel`: the whole panel, its state, output and events.

use std::cell::RefCell;
use std::collections::BTreeSet;

use craft_keymap::{Chord, Key, Mods, Platform};
use craft_ui::contract::{NoValue, Outcome, ShowContainer as _, SweepApply};
use craft_ui::layout_components::Stack;
use craft_ui::list::ListWords;
use craft_ui::runtime::env::Env;
use craft_ui::runtime::memory::Remember as _;
use egui::{Response, Ui};

use super::body::{Body, show_body};
use super::footer::show_footer;
use super::menu::{OpenMenu, show_menu};
use super::words::ActionsWords;
use crate::actions::{Action, ActionColor, ActionList, ActionStep};

use super::body::ActionMove;

/// What the panel asks of the app while it draws: words and state the list does not hold.
pub trait ActionsHost {
    /// A step's label in the tree ("Levels", "Play Action “Sharpen”"), from the app's command
    /// catalogue.
    fn step_label(&self, step: &ActionStep) -> String;

    /// The action's keyboard shortcut, if it has one (the app's `actions.play:<name>`
    /// override). None by default.
    fn shortcut(&self, action: &Action) -> Option<ShortcutChoice> {
        let _ = action;
        None
    }

    /// The action being recorded into, if a recording runs. None by default.
    fn recording(&self) -> Option<usize> {
        None
    }

    /// The steps recorded into `action` but not yet appended to it (the journal since the
    /// recording started), shown after its steps. None by default.
    fn pending_steps(&self, action: usize) -> &[ActionStep] {
        let _ = action;
        &[]
    }

    /// An action is playing: the panel takes no edits (renames, moves, toggles) while it plays.
    /// False by default.
    fn playing(&self) -> bool {
        false
    }
}

impl ActionsSettings {
    /// Settings with Button Mode `button_mode` and `collapsed_sets` shown collapsed.
    pub fn new(button_mode: bool, collapsed_sets: Vec<String>) -> Self {
        Self { button_mode, collapsed_sets }
    }
}

/// The Actions panel's persisted section: Button Mode and the collapsed sets.
/// Selection and expansion are not persisted.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct ActionsSettings {
    /// Button Mode: each action is a coloured button that plays it.
    pub button_mode: bool,
    /// The sets shown collapsed.
    pub collapsed_sets: Vec<String>,
}

/// A row of the tree: a set header, an action or one of its steps. Also the selection (one row at
/// a time) and the list's key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ActionSelection {
    /// The set header of this name.
    Set(String),
    /// The action at this index.
    Action(usize),
    /// Step `.1` of action `.0` (pending steps of a recording follow the action's own).
    Step(usize, usize),
}

/// What the panel keeps between passes. The app holds it beside the [`ActionList`]; only
/// `settings` is persisted.
///
/// The panel remaps `selected` itself when it reports a [`Reorder`](ActionsEvent::Reorder)
/// that moves a row, since it owns the selection. After a [`Delete`](ActionsEvent::Delete) the
/// app clears `selected` (the panel cannot tell which rows the delete removed).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ActionsPanelState {
    /// The persisted section.
    pub settings: ActionsSettings,
    /// The selected row (one at most).
    pub selected: Option<ActionSelection>,
    /// The expanded actions, by name (names are unique, so expansion survives moves).
    pub expanded: BTreeSet<String>,
}

/// A function-key shortcut chosen in the context menu (Photoshop's Action Options).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ShortcutChoice {
    /// F1 to F12: 1..=12.
    pub key: u8,
    /// Shift is part of the shortcut.
    pub shift: bool,
    /// Command (macOS) or Ctrl (elsewhere) is part of it.
    pub command: bool,
}

impl ShortcutChoice {
    /// Function key `key` (1 to 12) with the given modifiers.
    pub const fn new(key: u8, shift: bool, command: bool) -> Self {
        Self { key, shift, command }
    }

    /// The shortcut as a row shows it, in the platform's chord text: "F2" and "⇧⌘F3" on macOS,
    /// "Ctrl+Shift+F3" elsewhere.
    pub fn text(&self) -> String {
        self.chord().display(Platform::current())
    }

    /// The shortcut as a keymap chord; a key above 12 is taken as F12.
    fn chord(&self) -> Chord {
        let key = match self.key {
            1 => Key::F1,
            2 => Key::F2,
            3 => Key::F3,
            4 => Key::F4,
            5 => Key::F5,
            6 => Key::F6,
            7 => Key::F7,
            8 => Key::F8,
            9 => Key::F9,
            10 => Key::F10,
            11 => Key::F11,
            _ => Key::F12,
        };
        Chord::new(Mods { cmd: self.command, shift: self.shift, ..Mods::NONE }, key)
    }
}

/// What the user did, in order. Each is one app command (PhotoCraft's `actions.*`); edits that
/// need no app logic are applied by the app with the matching [`ActionList`] call.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ActionsEvent {
    /// Play `action` from step `from` (0 for the whole action).
    #[non_exhaustive]
    Play {
        /// The action.
        action: usize,
        /// The first step.
        from: usize,
    },
    /// Start recording into `into`, or into a new action when `None` (New Action…, Create new
    /// action).
    #[non_exhaustive]
    BeginRecording {
        /// The action to record into.
        into: Option<usize>,
    },
    /// Stop recording or playing.
    StopRecording,
    /// New Action… from the panel menu: the app asks for a name, then records.
    NewAction,
    /// New Set… or Create new set: the app asks for a name ([`ActionList::new_set`]).
    NewSet,
    /// Set Options… for this set.
    SetOptions(String),
    /// Delete this row ([`ActionList::delete_action`], [`ActionList::delete_step`]; a set deletes
    /// its actions, the app confirms).
    Delete(ActionSelection),
    /// An in-place rename ended with `text` ([`ActionList::rename`]).
    #[non_exhaustive]
    Rename {
        /// The action.
        action: usize,
        /// The new name, as typed.
        text: String,
    },
    /// A row was dragged or moved by key to a place: apply it with [`ActionList::move_action`] and
    /// [`ActionList::assign_set`] for an action, [`ActionList::move_step`] for a step.
    Reorder(ActionMove),
    /// A press or sweep on the include or dialog column, as the step to apply to a cell
    /// ([`SweepStep`](craft_ui::contract::SweepStep)). `Flip` and `Restore` set the cell to the step's value as the
    /// pointer crosses it or Escape cancels the sweep; `ShowOnly` (Alt on the first press of the include column) turns
    /// every other include cell off, or on again when it is the only one on; `UndoShowOnly` puts the include cells back
    /// as they were before it. The host groups one sweep as one undo step. The cell's column
    /// [`INCLUDE_COLUMN`](super::INCLUDE_COLUMN) of a step row is
    /// [`ActionList::set_step`], of an action row [`ActionList::set_all`];
    /// [`DIALOG_COLUMN`](super::DIALOG_COLUMN) of a step row is [`ActionList::set_dialog`].
    Toggle(craft_ui::contract::SweepStep<ActionSelection>),
    /// The context menu's Color ([`ActionList::set_color`]).
    #[non_exhaustive]
    SetColor {
        /// The action.
        action: usize,
        /// The colour.
        color: ActionColor,
    },
    /// The context menu's Keyboard Shortcuts: a key with its modifiers, or `None` to remove it.
    #[non_exhaustive]
    AssignShortcut {
        /// The action.
        action: usize,
        /// The shortcut.
        choice: Option<ShortcutChoice>,
    },
    /// Insert Stop… after the selection (the app's dialog).
    InsertStop,
    /// Batch… with this action (the app's dialog).
    Batch(usize),
    /// Load Actions… (the app's file dialog).
    Load,
    /// Save Actions…, or Save Set… for one set.
    #[non_exhaustive]
    Save {
        /// The set; `None` for every action.
        set: Option<String>,
    },
}

impl ActionsEvent {
    /// Play `action` from step `from`.
    pub fn play(action: usize, from: usize) -> Self {
        Self::Play { action, from }
    }

    /// Start recording into `into`, or into a new action when `None`.
    pub fn begin_recording(into: Option<usize>) -> Self {
        Self::BeginRecording { into }
    }

    /// An in-place rename of `action` ended with `text`.
    pub fn rename(action: usize, text: impl Into<String>) -> Self {
        Self::Rename { action, text: text.into() }
    }

    /// The context menu's Color for `action`.
    pub fn set_color(action: usize, color: ActionColor) -> Self {
        Self::SetColor { action, color }
    }

    /// The context menu's Keyboard Shortcuts for `action`: `choice`, or `None` to remove it.
    pub fn assign_shortcut(action: usize, choice: Option<ShortcutChoice>) -> Self {
        Self::AssignShortcut { action, choice }
    }

    /// Save Actions… (`None`) or Save Set… for `set`.
    pub fn save(set: Option<String>) -> Self {
        Self::Save { set }
    }
}

/// What [`ActionsPanel::show`] returns.
#[derive(Debug)]
#[must_use = "handle the events and save the settings when `settings_changed`"]
#[non_exhaustive]
pub struct ActionsPanelOutput {
    /// The panel's area.
    pub response: Response,
    /// What the user did this pass, in order.
    pub events: Vec<ActionsEvent>,
    /// The persisted [`ActionsSettings`] changed this pass (Button Mode, a set opened or
    /// closed): the app saves them.
    pub settings_changed: bool,
}

impl Outcome for ActionsPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What the panel keeps between passes, in its root slot.
#[derive(Clone)]
pub(super) struct Session {
    /// The menu the last context request chose.
    pub(super) menu: Option<OpenMenu>,
    /// The recording action and its row count when the list was last scrolled to the latest step.
    pub(super) revealed: Option<(usize, usize)>,
    /// The readers of the include and dialog sweeps: Alt on the first press of an include sweep shows only that
    /// cell (Photoshop); the dialog column ignores Alt.
    pub(super) include: SweepApply,
    pub(super) dialog: SweepApply,
}

impl Default for Session {
    fn default() -> Self {
        Session { menu: None, revealed: None, include: SweepApply::show_only(), dialog: SweepApply::new() }
    }
}

/// The Actions panel (Photoshop's Window › Actions): sets, actions and steps as a tree, the
/// include and dialog cells, in-place rename, drag to move, Button Mode, the footer and the
/// context menus. It runs nothing and changes nothing: it reads the [`ActionList`] and reports
/// [`ActionsEvent`]s.
///
/// ```ignore
/// let out = ActionsPanel::new(&mut ui_state.actions, &app.actions).host(&app_host).show(ui);
/// for event in out.events {
///     match event {
///         ActionsEvent::Play { action, from, .. } => app.play(action, from),
///         ActionsEvent::BeginRecording { into, .. } => app.record(into),
///         ActionsEvent::Rename { action, text, .. } => { app.actions.rename(action, &text, |_, _, _| {})?; }
///         _ => app.run_action_event(event),
///     }
/// }
/// ```
///
/// - **Rows**: a [`List`](craft_ui::list::List) in tree mode over the room above the footer:
///   loose actions first, then each set (a header with a folder icon and its action count; empty
///   sets show their header), each action (a play glyph, its name, at the end its shortcut and
///   step count, "2/3 steps" when some are off; the recording action a dot, open, its latest step
///   scrolled into view) and an open action's steps (the host's labels, dimmed when off), the
///   pending steps of a recording after them. The include and dialog cells are the list's toggle
///   columns before the label; an action with no steps has no include cell.
/// - **Pointer**: a click selects a row; a click on the disclosure toggles it (Alt opens or closes
///   all); a double click on an action's name renames it, elsewhere on an action it plays it. The
///   cells sweep down their column as one gesture. A drag moves an action (into or out of a set)
///   or a step (within or between actions), or drops it on the footer's Delete. Right-click on an
///   action opens Keyboard Shortcuts and Color; on a set, Save Set….
/// - **Keys**: the rows are one Tab stop: arrows move, Left and Right close and open, Enter plays
///   an action or toggles a set, Space toggles the include cell, F2 renames, Delete deletes the
///   selection (not an action while recording), Alt+Up and Alt+Down move the row. The footer
///   buttons are Tab stops after the list. Edits are refused while [`ActionsHost::playing`].
/// - **Button Mode**: each action is a button in its colour, set names between; a click plays it
///   once. Button Mode is off while recording.
/// - **Empty**: a centred notice with the Record glyph.
#[must_use = "show the panel with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct ActionsPanel<'a> {
    state: &'a mut ActionsPanelState,
    list: &'a ActionList,
    host: Option<&'a dyn ActionsHost>,
    rename: Option<usize>,
}

impl<'a> ActionsPanel<'a> {
    /// A panel over `list` with `state`.
    pub fn new(state: &'a mut ActionsPanelState, list: &'a ActionList) -> Self {
        Self { state, list, host: None, rename: None }
    }

    /// The app's side; without one, steps show their command ids and nothing records or plays.
    pub fn host(mut self, host: &'a dyn ActionsHost) -> Self {
        self.host = Some(host);
        self
    }

    /// Opens the in-place rename of `action` this pass (the app's Rename command). Call it in the
    /// pass that handles the command.
    pub fn start_rename(mut self, action: usize) -> Self {
        self.rename = Some(action);
        self
    }

    /// Draws the tree (or the buttons) and the footer, and reports this pass's events in order:
    /// the list's, the cells', the context menu's, the footer's.
    pub fn show(self, ui: &mut Ui) -> ActionsPanelOutput {
        let ActionsPanel { state, list, host, rename } = self;
        let env = Env::of(ui);
        let words = env.words::<ActionsWords>();
        let list_words = env.words::<ListWords>();
        let list_id = ui.scope_id().with("actions-rows");
        let settings_before = state.settings.clone();
        let events = RefCell::new(Vec::new());
        let state = RefCell::new(state);
        let body = Body { list, host, words: &words, list_words: &list_words, env: &env, list_id };

        let shown = Stack::new()
            .fill()
            .footer(|ui, _| {
                let mut state = state.borrow_mut();
                events.borrow_mut().extend(show_footer(ui, &body, &mut state));
            })
            .show(ui, |ui, stack| {
                let mut state = state.borrow_mut();
                let session = stack.memory::<Session>();
                let out = show_body(ui, &body, &mut state, session, rename);
                let menu = show_menu(ui, &body, &out.response, session);
                let mut reported = out.events;
                reported.extend(menu);
                (reported, out.selected)
            });
        // Only the real run's result comes back, so the body's events are reported once; the footer's follow them.
        let mut reported = Vec::new();
        if let Some((body_events, selected)) = shown.inner {
            reported = body_events;
            if let Some(selected) = selected {
                state.borrow_mut().selected = selected;
            }
        }
        reported.extend(events.into_inner());
        let settings_changed = state.borrow().settings != settings_before;
        ActionsPanelOutput { response: shown.output, events: reported, settings_changed }
    }
}
