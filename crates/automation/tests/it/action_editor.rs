//! The shared editor changes only an unpublished value; host effects remain requests.

use craft_automation::actions::{Action, ActionColor, ActionStep};
use craft_automation::ui::actions::{
    ActionEditor, ActionEditorActivity, ActionEditorEvent as Event, ActionEditorHost, ActionEditorState, ActionEditorValue, ActionStepMove,
};
use craft_ui::testing::{Fixture, Target, pointer, trial};
use egui::{Key, Modifiers, Pos2, Vec2};
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

struct Host;
impl ActionEditorHost for Host {
    fn step_label(&self, step: &ActionStep) -> String {
        format!("Run {}", step.command)
    }
    fn step_summary(&self, step: &ActionStep) -> Option<String> {
        Some(format!("Settings: {}", step.command))
    }
    fn can_edit_step(&self, _: &ActionStep) -> bool {
        true
    }
}

struct App {
    published: Action,
    value: ActionEditorValue,
    state: ActionEditorState,
    activity: ActionEditorActivity,
    events: Vec<Event>,
    changed: bool,
    refused: bool,
    step_moves: Vec<ActionStepMove>,
    measure: bool,
    sizing_passes: usize,
    sizing_moves: Vec<ActionStepMove>,
    sizing_changed: bool,
    sizing_events: Vec<Event>,
}

fn rig() -> (Fixture, Rc<RefCell<App>>) {
    let mut action =
        Action::new("Imported", ["first", "second", "third"].into_iter().map(|command| ActionStep::new(command, json!({"amount": 2}))).collect());
    action.description = "Keep this description\nand its line break".into();
    action.icon = "future-icon".into();
    action.color = ActionColor::Unknown(u16::MAX);
    action.options.insert("future".into(), json!({"opaque": [7, 9]}));
    action.disabled.insert(0);
    action.dialogs.insert(2);
    let shared = Rc::new(RefCell::new(App {
        published: action.clone(),
        value: ActionEditorValue::new(action, "legacy-unsupported-shortcut"),
        state: ActionEditorState::default(),
        activity: ActionEditorActivity::default(),
        events: Vec::new(),
        changed: false,
        refused: false,
        step_moves: Vec::new(),
        measure: false,
        sizing_passes: 0,
        sizing_moves: Vec::new(),
        sizing_changed: false,
        sizing_events: Vec::new(),
    }));
    let shown = Rc::clone(&shared);
    let mut fixture = Fixture::new(move |ui| {
        let App {
            value,
            state,
            activity,
            events,
            changed,
            refused,
            step_moves,
            measure,
            sizing_passes,
            sizing_moves,
            sizing_changed,
            sizing_events,
            ..
        } = &mut *shown.borrow_mut();
        if *measure {
            let builder = egui::UiBuilder::new().max_rect(ui.available_rect_before_wrap()).sizing_pass().invisible();
            let mut measured = ui.new_child(builder);
            let out = ActionEditor::new(state, value).host(&Host).activity(*activity).show(&mut measured);
            *sizing_passes += 1;
            *sizing_changed |= out.changed;
            sizing_moves.extend(out.step_moves);
            sizing_events.extend(out.events);
        }
        let mut editor = ActionEditor::new(state, value).host(&Host).activity(*activity);
        if *refused {
            editor = editor.refusal("The draft changed; reopen it before saving.");
        }
        let out = editor.show(ui);
        *changed |= out.changed;
        step_moves.extend(out.step_moves);
        events.extend(out.events);
    });
    fixture.room(Vec2::new(900.0, 1200.0)).settle();
    (fixture, shared)
}

fn node(fixture: &Fixture, label: &str) -> Target {
    fixture.by_label(label).unwrap_or_else(|| panic!("missing editor control {label}"))
}
fn press(fixture: &mut Fixture, label: &str) {
    let target = node(fixture, label);
    fixture.click(&target).idle(2);
}
fn take(shared: &Rc<RefCell<App>>) -> Vec<Event> {
    std::mem::take(&mut shared.borrow_mut().events)
}

#[trial]
fn action_editor_keeps_unknown_metadata_until_an_explicit_choice() {
    let (mut fixture, shared) = rig();
    press(&mut fixture, "Icon");
    press(&mut fixture, "Icon");
    assert_eq!(shared.borrow().value.action, shared.borrow().published, "opening the picker does not replace an unavailable icon or colour");
    assert_eq!(shared.borrow().value.shortcut, "legacy-unsupported-shortcut");
    assert!(!shared.borrow().changed);
    let name = node(&fixture, "Name");
    fixture.click(&name).type_text(" edited").key(Key::Tab, Modifiers::NONE).idle(2);
    let app = shared.borrow();
    assert!(app.changed && app.value.action.name != app.published.name, "an actual name edit is staged");
    assert_eq!(app.value.action.icon, "future-icon");
    assert_eq!(app.value.action.color.wire_name(), "@atn-color:65535");
    assert_eq!(app.value.action.description, app.published.description);
    assert_eq!(app.value.action.options, app.published.options);
    assert_eq!(app.value.shortcut, "legacy-unsupported-shortcut");
    drop(app);
    press(&mut fixture, "Blue");
    assert_eq!(shared.borrow().value.action.color, ActionColor::Blue2, "modern Blue is distinct from classic Light blue");
    press(&mut fixture, "Icon");
    press(&mut fixture, "Crop");
    assert_eq!(shared.borrow().value.action.icon, "ActionsCrop");
    assert_eq!(fixture.tree().focused_key(), node(&fixture, "Icon").node, "choosing a glyph returns real focus to its trigger");
    assert_eq!(shared.borrow().published.icon, "future-icon", "no published action was bound to the editor");
    press(&mut fixture, "Icon");
    press(&mut fixture, "Default icon");
    assert!(shared.borrow().value.action.icon.is_empty());
    press(&mut fixture, "Icon");
    press(&mut fixture, "Click");
    assert_eq!(shared.borrow().value.action.icon, "ClickAction", "default is distinct from the named click glyph");
}

#[trial]
fn action_editor_save_discard_and_leave_wait_for_host_acknowledgement() {
    let (mut fixture, shared) = rig();
    for label in ["Save", "Discard", "Back to Actions"] {
        press(&mut fixture, label);
    }
    assert_eq!(take(&shared), [Event::Save, Event::Discard, Event::Leave]);
    assert!(fixture.by_label("Name").is_some(), "requesting an effect never closes the page");
    assert_eq!(shared.borrow().value.action, shared.borrow().published);
    shared.borrow_mut().activity.busy = true;
    shared.borrow_mut().refused = true;
    fixture.settle();
    for label in ["Save", "Play action", "Record", "Discard", "Back to Actions"] {
        press(&mut fixture, label);
    }
    assert!(take(&shared).is_empty(), "busy host transitions refuse new effects");
    assert!(fixture.by_label("The draft changed; reopen it before saving.").is_some());
    assert_eq!(shared.borrow().value.action, shared.borrow().published, "a refusal preserves the complete local value");
}

#[trial]
fn action_editor_preview_and_capture_requests_use_valid_selected_boundaries() {
    let (mut fixture, shared) = rig();
    shared.borrow_mut().state.selected = Some(usize::MAX);
    fixture.settle();
    press(&mut fixture, "Play from selected step");
    assert!(take(&shared).is_empty(), "a stale selection cannot address a step");
    press(&mut fixture, "Record");
    assert_eq!(take(&shared), [Event::BeginCapture { at: 3 }], "no selection appends at the valid end");
    assert!(!shared.borrow().activity.recording, "a request cannot grant its own capture lease");
    press(&mut fixture, "Run first");
    press(&mut fixture, "Play from selected step");
    press(&mut fixture, "Record");
    press(&mut fixture, "Edit step settings");
    assert_eq!(take(&shared), [Event::Preview { from: 0 }, Event::BeginCapture { at: 3 }, Event::EditStep { step: 0 }]);
    shared.borrow_mut().activity.recording = true;
    fixture.settle();
    press(&mut fixture, "Play action");
    assert_eq!(take(&shared), [Event::Preview { from: 0 }], "preview requests the owned capture snapshot without ending capture");
    assert!(shared.borrow().activity.recording);
    press(&mut fixture, "Stop recording");
    assert_eq!(take(&shared), [Event::EndCapture]);
    assert!(shared.borrow().activity.recording, "the host acknowledges the end of capture");
}

#[trial]
fn action_editor_include_and_expanded_step_reorder_keep_flags_and_metadata() {
    let (mut fixture, shared) = rig();
    press(&mut fixture, "Run first");
    fixture.key(Key::ArrowRight, Modifiers::NONE).idle(2);
    assert!(shared.borrow().state.expanded.contains(&0), "Right opens the focused step's settings");
    assert!(fixture.texts().iter().any(|run| run.text == "Settings: first"), "the expanded summary is painted inside its owning row");
    fixture.key(Key::Space, Modifiers::NONE).idle(2);
    assert!(shared.borrow().value.action.step_enabled(0), "Space uses the shared include column");
    let from = node(&fixture, "Run first").center();
    let last = node(&fixture, "Run third");
    let to = Pos2::new(last.rect.center().x, last.rect.bottom() - 1.0);
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![pointer::press(from)]);
    fixture.frame(vec![pointer::moved(from + Vec2::new(0.0, 8.0))]);
    fixture.frame(vec![pointer::moved(to)]);
    fixture.frame(vec![pointer::moved(to)]);
    fixture.frame(vec![pointer::release(to)]);
    fixture.idle(2);
    let app = shared.borrow();
    assert_eq!(app.value.action.steps.iter().map(|step| step.command.as_str()).collect::<Vec<_>>(), ["second", "third", "first"]);
    assert_eq!(app.value.action.dialogs.iter().copied().collect::<Vec<_>>(), [1], "dialog flags follow their original step");
    assert!(app.value.action.disabled.is_empty());
    assert!(app.state.expanded.contains(&2), "the expanded summary follows its moved step");
    assert_eq!(app.value.action.description, app.published.description);
    assert_eq!(app.value.action.icon, app.published.icon);
    assert_eq!(app.value.action.color, app.published.color);
    assert_eq!(app.value.action.options, app.published.options);
    assert_eq!(app.published.steps[0].command, "first");
    drop(app);
    fixture.key(Key::ArrowUp, Modifiers::ALT).idle(2);
    let app = shared.borrow();
    assert_eq!(
        app.value.action.steps.iter().map(|step| step.command.as_str()).collect::<Vec<_>>(),
        ["second", "first", "third"],
        "keyboard reorder also moves an expanded step as one row"
    );
    assert_eq!(app.state.selected, Some(1));
    assert!(app.state.expanded.contains(&1));
    assert_eq!(app.value.action.dialogs.iter().copied().collect::<Vec<_>>(), [2]);
    let before = app.value.clone();
    drop(app);
    take(&shared);
    press(&mut fixture, "Duplicate step");
    press(&mut fixture, "Delete step");
    assert_eq!(take(&shared), [Event::DuplicateStep { step: 1 }, Event::DeleteStep { step: 1 }]);
    assert_eq!(shared.borrow().value, before, "structural requests wait for guarded host acknowledgement");
    press(&mut fixture, "Show dialog during playback: Run first");
    let mut expected = before.clone();
    expected.action.dialogs.insert(1);
    assert_eq!(shared.borrow().value, expected, "the dialog cell changes only its flag, retaining opaque metadata and steps");
    shared.borrow_mut().activity.busy = true;
    fixture.settle();
    for label in ["Duplicate step", "Delete step", "Show dialog during playback: Run first"] {
        press(&mut fixture, label);
    }
    assert!(take(&shared).is_empty());
    assert_eq!(shared.borrow().value, expected, "busy controls cannot mutate the flag or request structural edits");
}

// Identical shared steps may still be different physical records in an app's native codec.
// Receipts, rather than value matching, are the source-occurrence permutation.
#[trial]
fn action_editor_moves_identical_occurrences_once_and_never_during_sizing() {
    let (mut fixture, shared) = rig();
    {
        let mut app = shared.borrow_mut();
        let equal = ActionStep::new("same", json!({"amount": 2}));
        app.value.action.steps = vec![equal.clone(), equal];
        app.value.action.disabled = [0].into_iter().collect();
        app.value.action.dialogs = [1].into_iter().collect();
        app.published = app.value.action.clone();
        app.measure = true;
        app.changed = false;
    }
    fixture.settle();
    press(&mut fixture, "Run same");
    fixture.key(Key::ArrowDown, Modifiers::ALT).idle(2);
    let moves = std::mem::take(&mut shared.borrow_mut().step_moves);
    assert_eq!(moves, [ActionStepMove { from: 0, to: 1 }], "one receipt for the applied occurrence move, not another for layout/idle passes");
    let mut native_origins: Vec<(&str, Vec<u8>)> = vec![("record-A", vec![0x01, 0x80]), ("record-B", vec![0x02, 0xff])];
    for receipt in moves {
        let origin = native_origins.remove(receipt.from);
        native_origins.insert(receipt.to, origin);
    }
    assert_eq!(native_origins[0].0, "record-B");
    assert_eq!(native_origins[1], ("record-A", vec![0x01, 0x80]));
    {
        let app = shared.borrow();
        assert_eq!(app.value.action.steps[0], app.value.action.steps[1], "command and params cannot recover the occurrence order");
        assert_eq!(app.value.action.disabled.iter().copied().collect::<Vec<_>>(), [1]);
        assert_eq!(app.value.action.dialogs.iter().copied().collect::<Vec<_>>(), [0]);
        assert_eq!(app.value.action.options, app.published.options);
        assert_eq!(app.value.action.icon, app.published.icon);
        assert_eq!(app.value.action.color, app.published.color);
        assert!(app.changed);
    }
    fixture.key(Key::ArrowUp, Modifiers::ALT).idle(2);
    let moves = std::mem::take(&mut shared.borrow_mut().step_moves);
    assert_eq!(moves, [ActionStepMove { from: 1, to: 0 }], "the next receipt indexes the evolving, already-moved sequence");
    for receipt in moves {
        let origin = native_origins.remove(receipt.from);
        native_origins.insert(receipt.to, origin);
    }
    assert_eq!(native_origins, [("record-A", vec![0x01, 0x80]), ("record-B", vec![0x02, 0xff])]);
    {
        let mut app = shared.borrow_mut();
        assert_eq!(app.value.action, app.published, "the reverse move restores all flags and metadata");
        app.value.action.disabled.clear();
        app.value.action.dialogs.clear();
        app.changed = false;
    }
    fixture.key(Key::ArrowDown, Modifiers::ALT).idle(2);
    let app = shared.borrow();
    assert_eq!(app.step_moves, [ActionStepMove { from: 0, to: 1 }]);
    assert!(!app.changed, "value equality does not suppress an occurrence receipt");
    assert!(app.sizing_passes > 0, "the real editor also ran on explicit sizing UIs");
    assert!(app.sizing_moves.is_empty() && app.sizing_events.is_empty() && !app.sizing_changed, "sizing emits no receipts, effects or changed claim");
}
