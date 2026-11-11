//! The panel menu's commands and the row context menus (Keyboard Shortcuts, Color; Save Set…).

use craft_keymap::Platform;
use craft_ui::contract::{CommandSet, ContextMenuRequest, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::Divider;
use craft_ui::list::label_color_label;
use craft_ui::menu::{Menu, MenuItem, PanelRow};
use craft_ui::runtime::commands::{CommandRef, CommandState};
use egui::{Response, Ui};

use super::body::{Body, delete_event};
use super::footer::{play_target, selected_action};
use super::panel::{ActionSelection, ActionsEvent, ActionsHost, ActionsPanelState, Session, ShortcutChoice};
use crate::actions::{ActionColor, ActionList};

/// The commands of the Actions panel: its panel-menu rows. They are the app's commands: the app
/// gives each its state with [`state`](ActionsPanelCommands::state) and runs the chosen one with
/// [`run`](ActionsPanelCommands::run).
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "actionsPanel")]
#[non_exhaustive]
pub enum ActionsPanelCommands {
    /// Ask for a name, then record a new action.
    #[command(label = "New Action…")]
    NewAction,
    /// Ask for a name, then create a set.
    #[command(label = "New Set…")]
    NewSet,
    /// The options of the selected action's set.
    #[command(label = "Set Options…")]
    SetOptions,
    /// Play the selection.
    #[command(label = "Play")]
    Play,
    /// Record into the selected action.
    #[command(label = "Start Recording")]
    StartRecording,
    /// Stop recording.
    #[command(label = "Stop Recording")]
    StopRecording,
    /// Delete the selection.
    #[command(label = "Delete")]
    Delete,
    /// Insert a stop after the selection.
    #[command(label = "Insert Stop…")]
    InsertStop,
    /// Run an action over many files.
    #[command(label = "Batch…")]
    Batch,
    /// Load actions from a file.
    #[command(label = "Load Actions…")]
    Load,
    /// Save every action to a file.
    #[command(label = "Save Actions…")]
    Save,
    /// Show each action as a button.
    #[command(label = "Button Mode")]
    ButtonMode,
}

/// What a panel-menu command asks of the app, from [`ActionsPanelCommands::run`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ActionsMenuAction {
    /// The command is one of the panel's events.
    Event(ActionsEvent),
    /// Button Mode toggled in the state: persist the [`ActionsSettings`](super::ActionsSettings).
    SettingsChanged,
}

/// The set the selection is in: the selected set, or the selected action's (or step's) set.
fn current_set(state: &ActionsPanelState, list: &ActionList) -> Option<String> {
    match state.selected.as_ref()? {
        ActionSelection::Set(name) => Some(name.clone()),
        ActionSelection::Action(index) | ActionSelection::Step(index, _) => list.set_of(*index).map(str::to_owned),
    }
}

impl ActionsPanelCommands {
    /// The state of the command for the app's command input: New Action… and New Set… not while
    /// recording, Set Options… needs a current set, Play and Batch… need an action, Delete follows
    /// the footer's Delete, Insert Stop… needs a selected action or step, Save needs an action,
    /// Start Recording is off while recording and Stop Recording only on while it runs, and Button
    /// Mode is checked with the state's setting and off while recording.
    pub fn state(self, panel: &ActionsPanelState, list: &ActionList, recording: bool) -> CommandState {
        let any = !list.actions().is_empty();
        let on_row = matches!(panel.selected, Some(ActionSelection::Action(_) | ActionSelection::Step(..)));
        let state = CommandState::new();
        match self {
            ActionsPanelCommands::NewAction | ActionsPanelCommands::NewSet | ActionsPanelCommands::StartRecording => state.enabled(!recording),
            ActionsPanelCommands::SetOptions => state.enabled(current_set(panel, list).is_some()),
            ActionsPanelCommands::Play => state.enabled(any && !recording),
            ActionsPanelCommands::StopRecording => state.enabled(recording),
            ActionsPanelCommands::Delete => {
                state.enabled(panel.selected.as_ref().is_some_and(|selected| delete_event(selected, recording).is_some()))
            }
            ActionsPanelCommands::InsertStop => state.enabled(on_row),
            ActionsPanelCommands::Batch | ActionsPanelCommands::Save => state.enabled(any),
            ActionsPanelCommands::Load => state,
            ActionsPanelCommands::ButtonMode => state.enabled(!recording).checked(panel.settings.button_mode),
        }
    }

    /// Runs the command: Button Mode toggles it in `panel` and returns
    /// [`ActionsMenuAction::SettingsChanged`]; every other command returns its event (New Action…
    /// is [`ActionsEvent::NewAction`]; Play plays the selection as the footer's Play does; Delete
    /// is refused during a recording as the footer's is). `None` when the command has nothing to
    /// do in this state. `recording` is what [`state`](Self::state) was given.
    pub fn run(self, panel: &mut ActionsPanelState, list: &ActionList, recording: bool) -> Option<ActionsMenuAction> {
        let selected = panel.selected.as_ref();
        let event = match self {
            ActionsPanelCommands::NewAction => ActionsEvent::NewAction,
            ActionsPanelCommands::NewSet => ActionsEvent::NewSet,
            ActionsPanelCommands::SetOptions => ActionsEvent::SetOptions(current_set(panel, list)?),
            ActionsPanelCommands::Play => {
                let (action, from) = play_target(selected, list)?;
                ActionsEvent::Play { action, from }
            }
            ActionsPanelCommands::StartRecording => ActionsEvent::BeginRecording { into: selected_action(selected, list) },
            ActionsPanelCommands::StopRecording => ActionsEvent::StopRecording,
            ActionsPanelCommands::Delete => delete_event(selected?, recording)?,
            ActionsPanelCommands::InsertStop => selected_action(selected, list).map(|_| ActionsEvent::InsertStop)?,
            ActionsPanelCommands::Batch => ActionsEvent::Batch(play_target(selected, list)?.0),
            ActionsPanelCommands::Load => ActionsEvent::Load,
            ActionsPanelCommands::Save => ActionsEvent::Save { set: None },
            ActionsPanelCommands::ButtonMode => {
                panel.settings.button_mode = !panel.settings.button_mode;
                return Some(ActionsMenuAction::SettingsChanged);
            }
        };
        Some(ActionsMenuAction::Event(event))
    }
}

/// The rows of the Actions panel's menu, for the dock's `PanelMenu`: New Action…, New Set…, Set
/// Options…, Play, Start and Stop Recording, Delete, Insert Stop…, Batch…, Load and Save Actions…
/// and Button Mode, in groups.
pub fn panel_rows() -> Vec<PanelRow> {
    let command = |command: ActionsPanelCommands| PanelRow::Command(CommandRef::from(command));
    vec![
        command(ActionsPanelCommands::NewAction),
        command(ActionsPanelCommands::NewSet),
        command(ActionsPanelCommands::SetOptions),
        PanelRow::Separator,
        command(ActionsPanelCommands::Play),
        command(ActionsPanelCommands::StartRecording),
        command(ActionsPanelCommands::StopRecording),
        command(ActionsPanelCommands::Delete),
        PanelRow::Separator,
        command(ActionsPanelCommands::InsertStop),
        command(ActionsPanelCommands::Batch),
        PanelRow::Separator,
        command(ActionsPanelCommands::Load),
        command(ActionsPanelCommands::Save),
        PanelRow::Separator,
        command(ActionsPanelCommands::ButtonMode),
    ]
}

/// The shortcut as the context menu has made it so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PendingShortcut {
    /// 0 for none, else 1..=12.
    key: u8,
    shift: bool,
    command: bool,
}

impl PendingShortcut {
    fn choice(self) -> Option<ShortcutChoice> {
        (self.key > 0).then_some(ShortcutChoice { key: self.key, shift: self.shift, command: self.command })
    }
}

/// The menu a context request opened, kept until the next request.
#[derive(Clone, Debug)]
pub(super) enum OpenMenu {
    /// A set's menu: Save Set….
    Set(String),
    /// An action's menu, with the shortcut as the menu has made it from the host's.
    Action { index: usize, shortcut: PendingShortcut },
}

impl OpenMenu {
    /// The menu for `request`, starting from the host's shortcut; `None` for a step or an action
    /// the list no longer has.
    pub(super) fn opened(request: ContextMenuRequest<ActionSelection>, list: &ActionList, host: Option<&dyn ActionsHost>) -> Option<Self> {
        match request.key {
            ActionSelection::Set(name) => Some(OpenMenu::Set(name)),
            ActionSelection::Action(index) => {
                let current = host.and_then(|host| host.shortcut(list.action(index)?));
                let shortcut = PendingShortcut {
                    key: current.map_or(0, |choice| choice.key),
                    shift: current.is_some_and(|choice| choice.shift),
                    command: current.is_some_and(|choice| choice.command),
                };
                Some(OpenMenu::Action { index, shortcut })
            }
            ActionSelection::Step(..) => None,
        }
    }
}

/// Shows the menu the last request chose, hanging from `opener` (the list), and reports what its
/// rows ran:
/// - On an action: Keyboard Shortcuts (a caption), None and F1 to F12 as radio rows, then Shift and
///   Command (Ctrl off macOS) check rows, enabled only with a key; the check rows keep the menu
///   open so both can be ticked, and each tick is reported as it is made with the shortcut as it
///   stands. A separator, then Color: a submenu of None, Red, Orange, Yellow, Green, Blue, Violet
///   and Gray with the current one marked.
/// - On a set: Save Set….
pub(super) fn show_menu(ui: &mut Ui, body: &Body<'_>, opener: &Response, session: &mut Session) -> Vec<ActionsEvent> {
    let Body { list, words, list_words, .. } = *body;
    let mut events = Vec::new();
    match session.menu.as_mut() {
        None => {}
        Some(OpenMenu::Set(name)) => {
            Menu::context(words.row_menu.clone(), opener).show(ui, |ui, _| {
                if MenuItem::new(words.save_set.clone()).show(ui).ran {
                    events.push(ActionsEvent::Save { set: Some(name.clone()) });
                }
            });
        }
        Some(OpenMenu::Action { index, shortcut }) => {
            let (action, current) = (*index, list.action(*index).map(|held| held.color).unwrap_or_default());
            Menu::context(words.row_menu.clone(), opener).show(ui, |ui, _| {
                MenuItem::new(words.keyboard_shortcuts.clone()).enabled(false).show(ui);
                if MenuItem::new(words.none.clone()).radio_mark(shortcut.key == 0).show(ui).ran {
                    shortcut.key = 0;
                    events.push(ActionsEvent::AssignShortcut { action, choice: shortcut.choice() });
                }
                for key in 1..=12u8 {
                    if MenuItem::new(format!("F{key}")).radio_mark(shortcut.key == key).show(ui).ran {
                        shortcut.key = key;
                        events.push(ActionsEvent::AssignShortcut { action, choice: shortcut.choice() });
                    }
                }
                let has_key = shortcut.key > 0;
                if MenuItem::new(words.shift.clone()).check(shortcut.shift).keep_open(true).enabled(has_key).show(ui).ran {
                    shortcut.shift = !shortcut.shift;
                    events.push(ActionsEvent::AssignShortcut { action, choice: shortcut.choice() });
                }
                let modifier = if Platform::current().is_mac() { &words.command } else { &words.ctrl };
                if MenuItem::new(modifier.clone()).check(shortcut.command).keep_open(true).enabled(has_key).show(ui).ran {
                    shortcut.command = !shortcut.command;
                    events.push(ActionsEvent::AssignShortcut { action, choice: shortcut.choice() });
                }
                Divider::horizontal().show(ui);
                Menu::submenu_row(ui, words.color.clone(), |ui, _| {
                    for each in ActionColor::ALL {
                        let name = label_color_label(each.label_color(), list_words);
                        if MenuItem::new(name).radio_mark(each == current).show(ui).ran {
                            events.push(ActionsEvent::SetColor { action, color: each });
                        }
                    }
                });
            });
        }
    }
    events
}
