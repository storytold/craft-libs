//! Renderer tests. egui_kittest 0.37 drives a headless egui context with scripted clicks and key
//! input, finds widgets through the accessibility tree, and the tests assert the returned events.

use craft_plugin_ui::{UiEvent, WidgetTree, show};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

struct App {
    tree: WidgetTree,
    events: Vec<UiEvent>,
}

fn harness(tree: Value) -> Harness<'static, App> {
    let tree = WidgetTree::from_json(&tree.to_string()).expect("test tree validates");
    let mut h = Harness::new_ui_state(
        |ui, app: &mut App| {
            let ev = show(ui, &mut app.tree);
            app.events.extend(ev);
        },
        App { tree, events: Vec::new() },
    );
    h.run();
    h
}

fn take(h: &mut Harness<'static, App>) -> Vec<UiEvent> {
    std::mem::take(&mut h.state_mut().events)
}

fn ev(id: &str, value: Value) -> UiEvent {
    UiEvent { widget_id: id.to_owned(), value }
}

#[test]
fn idle_frames_emit_no_events() {
    let mut h = harness(json!({"type": "column", "id": "root", "children": [
        {"type": "button", "id": "go", "label": "Go"},
        {"type": "checkbox", "id": "cb", "label": "On", "value": false}
    ]}));
    h.run();
    h.run();
    assert_eq!(take(&mut h), vec![]);
}

#[test]
fn button_click_emits_null_value() {
    let mut h = harness(json!({"type": "column", "id": "root", "children": [
        {"type": "button", "id": "apply", "label": "Apply"},
        {"type": "button", "id": "reset", "label": "Reset"}
    ]}));
    h.get_by_label("Reset").click();
    h.run();
    assert_eq!(take(&mut h), vec![ev("reset", Value::Null)]);
}

#[test]
fn checkbox_click_toggles_value_and_tree() {
    let mut h = harness(json!({"type": "checkbox", "id": "cb", "label": "Enabled", "value": false}));
    h.get_by_label("Enabled").click();
    h.run();
    assert_eq!(take(&mut h), vec![ev("cb", json!(true))]);
    // The tree holds the new value, so the next click flips it back.
    h.get_by_label("Enabled").click();
    h.run();
    assert_eq!(take(&mut h), vec![ev("cb", json!(false))]);
}

#[test]
fn list_click_emits_index_and_repeat_click_is_silent() {
    let mut h = harness(json!({"type": "list", "id": "li", "items": ["alpha", "beta", "gamma"], "selected": 0}));
    h.get_by_label("gamma").click();
    h.run();
    assert_eq!(take(&mut h), vec![ev("li", json!(2))]);
    h.get_by_label("gamma").click();
    h.run();
    assert_eq!(take(&mut h), vec![]);
}

#[test]
fn text_typing_emits_full_text_and_keeps_it_in_the_tree() {
    let mut h = harness(json!({"type": "text", "id": "t", "value": "ab", "multiline": false}));
    h.get_by_role(egui::accesskit::Role::TextInput).focus();
    h.run();
    h.get_by_role(egui::accesskit::Role::TextInput).type_text("cd");
    h.run();
    let events = take(&mut h);
    assert!(!events.is_empty(), "typing must emit events");
    assert!(events.iter().all(|e| e.widget_id == "t"));
    assert_eq!(events.last().map(|e| e.value.clone()), Some(json!("abcd")));
}

#[test]
fn dropdown_pick_emits_option_string() {
    let mut h = harness(json!({"type": "dropdown", "id": "dd", "options": ["red", "blue"], "value": "red"}));
    h.get_by_role(egui::accesskit::Role::ComboBox).click();
    h.run();
    h.get_by_label("blue").click();
    h.run();
    assert_eq!(take(&mut h), vec![ev("dd", json!("blue"))]);
}

#[test]
fn param_form_edits_report_form_dot_param_ids_and_update_values() {
    let mut h = harness(json!({"type": "param_form", "id": "pf",
        "schema": {
            "invert": {"type": "bool", "label": "Invert"},
            "mode": {"type": "choice", "label": "Mode", "options": ["soft", "hard"], "default": "soft"}
        },
        "values": {"invert": false}}));
    h.get_by_label("Invert").click();
    h.run();
    assert_eq!(take(&mut h), vec![ev("pf.invert", json!(true))]);
    h.get_by_role(egui::accesskit::Role::ComboBox).click();
    h.run();
    h.get_by_label("hard").click();
    h.run();
    assert_eq!(take(&mut h), vec![ev("pf.mode", json!("hard"))]);
    let back = serde_json::to_value(&h.state().tree).expect("serializes");
    assert_eq!(back["values"], json!({"invert": true, "mode": "hard"}));
}

#[test]
fn number_edit_through_keyboard_emits_number() {
    let mut h = harness(json!({"type": "number", "id": "n", "value": 1.0, "min": 0.0, "max": 50.0}));
    // A click without drag puts DragValue into text edit mode. Entering it selects the old text.
    h.get_by_role(egui::accesskit::Role::SpinButton).click();
    h.run();
    h.get_by_role(egui::accesskit::Role::SpinButton).type_text("4");
    h.key_press(egui::Key::Enter);
    h.run();
    let events = take(&mut h);
    assert_eq!(events.last().map(|e| e.value.clone()), Some(json!(4.0)), "{events:?}");
}

#[test]
fn number_edit_above_max_is_clamped_to_max() {
    let mut h = harness(json!({"type": "number", "id": "n", "value": 1.0, "min": 0.0, "max": 5.0}));
    h.get_by_role(egui::accesskit::Role::SpinButton).click();
    h.run();
    h.get_by_role(egui::accesskit::Role::SpinButton).type_text("9");
    h.key_press(egui::Key::Enter);
    h.run();
    let events = take(&mut h);
    assert_eq!(events.last().map(|e| e.value.clone()), Some(json!(5.0)), "{events:?}");
}

#[test]
fn layout_draws_every_widget_in_a_tree() {
    let mut h = harness(json!({"type": "column", "id": "root", "children": [
        {"type": "group", "id": "g", "title": "Tools", "children": [
            {"type": "row", "id": "r", "children": [
                {"type": "label", "id": "l", "text": "Hello panel"},
                {"type": "button", "id": "b", "label": "Press"}
            ]}
        ]},
        {"type": "progress", "id": "p", "value": 0.5},
        {"type": "separator", "id": "s"},
        {"type": "color", "id": "c", "rgba": [1.0, 0.5, 0.0, 1.0]},
        {"type": "slider", "id": "sl", "min": 0.0, "max": 1.0, "value": 0.25}
    ]}));
    h.run();
    for label in ["Tools", "Hello panel", "Press"] {
        assert!(h.query_by_label(label).is_some(), "missing {label}");
    }
    let button = h.get_by_label("Press").rect();
    let label = h.get_by_label("Hello panel").rect();
    assert!(label.right() <= button.left() + 1.0, "row lays children left to right: {label:?} {button:?}");
    assert!(button.height() > 0.0 && button.width() > 0.0);
}

#[test]
fn param_form_number_and_text_edits_emit_typed_values() {
    let mut h = harness(json!({"type": "param_form", "id": "pf",
    "schema": {
        "gain": {"type": "number", "label": "Gain", "min": 0.0, "max": 10.0, "default": 1.0},
        "name": {"type": "text", "label": "Name", "default": "a"},
        "steps": {"type": "int", "label": "Steps", "min": 1, "max": 9, "default": 2},
        "tint": {"type": "color", "label": "Tint", "default": [0, 0, 0, 1]},
        "curve": {"type": "json", "label": "Curve"}
    }}));
    for (label, typed) in [("Gain", "7"), ("Steps", "4")] {
        h.get_by_label(label).click();
        h.run();
        h.get_by_label(label).type_text(typed);
        h.key_press(egui::Key::Enter);
        h.run();
    }
    assert_eq!(take(&mut h), vec![ev("pf.gain", json!(7.0)), ev("pf.steps", json!(4.0))]);
    h.get_by_label("Name").focus();
    h.run();
    h.get_by_label("Name").type_text("z");
    h.run();
    assert_eq!(take(&mut h).last().map(|e| e.widget_id.clone()), Some("pf.name".to_owned()));
    assert!(h.query_by_label("Curve").is_some());
    assert!(h.query_by_label_contains("not editable here").is_some());
}
