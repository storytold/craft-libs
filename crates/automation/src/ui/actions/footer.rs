//! The Actions footer: Stop, Record, Play, New Set, New Action, Delete and the Button Mode toggle.

use craft_behavior::IconId;
use craft_theme::variant::Tone;
use craft_ui::contract::ListDrag;
use craft_ui::primitives::IconSpec;
use craft_ui::sections::{FooterAction, FooterBar};
use egui::Ui;

use super::body::{Body, delete_event};
use super::panel::{ActionSelection, ActionsEvent, ActionsPanelState};
use crate::actions::ActionList;

// Reading order of the footer's buttons.
const STOP: usize = 0;
const RECORD: usize = 1;
const PLAY: usize = 2;
const NEW_SET: usize = 3;
const NEW_ACTION: usize = 4;
const DELETE: usize = 5;
const BUTTON_MODE: usize = 6;

/// The footer under the list, in this order, each a Tab stop with its name and tooltip:
///
/// | Button | Icon | Reports | Enabled |
/// |---|---|---|---|
/// | Stop playing/recording | `square` | `StopRecording` | while recording or playing |
/// | Begin recording | `circle`, pressed in the alert tone while recording | `BeginRecording { into: selected action }` | not playing; while recording it stays live and ignores the click |
/// | Play selection | `play` | `Play { action, from }`: the selected action from 0, a selected step's action from that step, else the first action | some action, not recording |
/// | Create new set | `new_folder` | `NewSet` | not recording |
/// | Create new action | `plus` | `BeginRecording { into: None }` | not recording |
/// | Delete | `trash` | `Delete(selection)`; a row of the list dropped on it reports `Delete(row)` | a selection, and while recording only a step; a dropped row is accepted on the same rule |
/// | Button Mode | `toggle_right`, pressed | toggles `ActionsSettings::button_mode` in `state` | not recording |
pub(super) fn show_footer(ui: &mut Ui, body: &Body<'_>, state: &mut ActionsPanelState) -> Vec<ActionsEvent> {
    let Body { list, host, words, list_id, .. } = *body;
    let recording = host.and_then(|host| host.recording());
    let playing = host.is_some_and(|host| host.playing());
    let hosted = host.is_some();
    let idle = recording.is_none() && !playing;
    let target = play_target(state.selected.as_ref(), list);
    let delete = state.selected.as_ref().filter(|_| !playing).and_then(|selected| delete_event(selected, recording.is_some()));
    let record_into = selected_action(state.selected.as_ref(), list);
    let icon = |name: &'static str| IconSpec::svg(IconId::new(name));
    let mut record = FooterAction::new(icon("circle"), words.record.clone()).enabled(hosted && !playing).pressed(recording.is_some());
    if recording.is_some() {
        record = record.tone(Tone::Bad);
    }
    let actions = [
        FooterAction::new(icon("square"), words.stop.clone()).enabled(hosted && !idle),
        record,
        FooterAction::new(icon("play"), words.play.clone()).enabled(hosted && idle && target.is_some()),
        FooterAction::new(icon("new_folder"), words.new_set.clone()).enabled(idle),
        FooterAction::new(icon("plus"), words.new_action.clone()).enabled(hosted && idle),
        FooterAction::new(icon("trash"), words.delete.clone()).enabled(delete.is_some()).takes_drops_disabled(true).accepts_drops(
            move |drag: &ListDrag<ActionSelection>| !playing && drag.list == list_id && delete_event(&drag.grabbed, recording.is_some()).is_some(),
        ),
        FooterAction::new(icon("toggle_right"), words.button_mode.clone()).enabled(idle).pressed(state.settings.button_mode),
    ];
    let out = FooterBar::actions(actions).show_footer(ui);
    // The footer runs twice per pass: the sizing pass measures it and must report nothing.
    if ui.is_sizing_pass() {
        return Vec::new();
    }

    let mut events = Vec::new();
    match out.picked {
        Some(STOP) => events.push(ActionsEvent::StopRecording),
        // Record stays live while recording: it shows the alert state and ignores the click.
        Some(RECORD) if idle => events.push(ActionsEvent::BeginRecording { into: record_into }),
        Some(PLAY) => {
            if let Some((action, from)) = target {
                events.push(ActionsEvent::Play { action, from });
            }
        }
        Some(NEW_SET) => events.push(ActionsEvent::NewSet),
        Some(NEW_ACTION) => events.push(ActionsEvent::BeginRecording { into: None }),
        Some(DELETE) => events.extend(delete),
        Some(BUTTON_MODE) => state.settings.button_mode = !state.settings.button_mode,
        _ => {}
    }
    if let Some(dropped) = &out.dropped
        && dropped.index == DELETE
        && let Some(drag) = dropped.payload::<ListDrag<ActionSelection>>()
    {
        events.extend(delete_event(&drag.grabbed, recording.is_some()));
    }
    events
}

/// What Play plays: the selected action from its first step, a selected step's action from that
/// step, else the first action. `None` without actions.
pub(super) fn play_target(selected: Option<&ActionSelection>, list: &ActionList) -> Option<(usize, usize)> {
    let len = list.actions().len();
    match selected {
        Some(&ActionSelection::Action(action)) if action < len => Some((action, 0)),
        Some(&ActionSelection::Step(action, step)) if action < len => Some((action, step)),
        _ => (len > 0).then_some((0, 0)),
    }
}

/// The action the selection is, or a selected step belongs to.
pub(super) fn selected_action(selected: Option<&ActionSelection>, list: &ActionList) -> Option<usize> {
    match selected {
        Some(&ActionSelection::Action(action) | &ActionSelection::Step(action, _)) if action < list.actions().len() => Some(action),
        _ => None,
    }
}
