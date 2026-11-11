//! Validation and serde tests for the widget tree.

use craft_plugin_ui::tree::{MAX_DEPTH, MAX_ID_LEN, MAX_ITEMS, MAX_NODES, MAX_TEXT_LEN};
use craft_plugin_ui::{TreeError, Widget, WidgetTree};
use serde_json::json;

fn label(id: &str) -> Widget {
    Widget::Label { id: id.to_owned(), text: "x".to_owned() }
}

fn column(id: &str, children: Vec<Widget>) -> Widget {
    Widget::Column { id: id.to_owned(), children }
}

/// Validate `root` and return the error. Fails the test when the tree is accepted.
fn expect_err(root: Widget) -> TreeError {
    match WidgetTree::new(root) {
        Err(e) => e,
        Ok(t) => panic!("tree should be rejected but passed with {} nodes", t.node_count()),
    }
}

fn from_value(v: serde_json::Value) -> Result<WidgetTree, TreeError> {
    WidgetTree::from_json(&v.to_string())
}

#[test]
fn accepts_exactly_max_nodes_and_rejects_one_more() {
    let leaves = |n: usize| (0..n).map(|i| label(&format!("l{i}"))).collect::<Vec<_>>();
    let ok = WidgetTree::new(column("root", leaves(MAX_NODES - 1)));
    assert_eq!(ok.map(|t| t.node_count()), Ok(MAX_NODES));
    assert_eq!(expect_err(column("root", leaves(MAX_NODES))), TreeError::TooManyNodes { max: MAX_NODES });
}

#[test]
fn param_form_params_count_as_nodes() {
    let mut schema = serde_json::Map::new();
    for i in 0..MAX_NODES {
        schema.insert(format!("p{i}"), json!({"type": "bool"}));
    }
    let v = json!({"type": "param_form", "id": "f", "schema": schema});
    assert_eq!(from_value(v), Err(TreeError::TooManyNodes { max: MAX_NODES }));
}

#[test]
fn depth_limit_is_inclusive_and_does_not_overflow_the_stack() {
    let nest = |depth: usize| {
        let mut w = label("leaf");
        for i in (1..depth).rev() {
            w = column(&format!("c{i}"), vec![w]);
        }
        w
    };
    assert!(WidgetTree::new(nest(MAX_DEPTH)).is_ok());
    assert_eq!(expect_err(nest(MAX_DEPTH + 1)), TreeError::TooDeep { max: MAX_DEPTH });
}

#[test]
fn hostile_json_nesting_is_refused_before_validation() {
    // serde_json stops at 128 levels, so a 5,000 level payload is an error, not a stack overflow.
    let mut text = String::new();
    for i in 0..5_000 {
        text.push_str(&format!(r#"{{"type":"column","id":"c{i}","children":["#));
    }
    assert!(matches!(WidgetTree::from_json(&text), Err(TreeError::Json(_))));
}

#[test]
fn rejects_duplicate_and_bad_ids() {
    let dup = column("root", vec![label("a"), column("c", vec![label("a")])]);
    assert_eq!(expect_err(dup), TreeError::DuplicateId("a".to_owned()));
    assert_eq!(expect_err(label("")), TreeError::EmptyId);
    assert_eq!(expect_err(label(&"i".repeat(MAX_ID_LEN + 1))), TreeError::IdTooLong { max: MAX_ID_LEN });
    assert!(WidgetTree::new(label(&"i".repeat(MAX_ID_LEN))).is_ok());
}

#[test]
fn param_form_event_ids_cannot_collide_with_widgets() {
    let v = json!({"type": "column", "id": "root", "children": [
        {"type": "label", "id": "f.gain", "text": "hi"},
        {"type": "param_form", "id": "f", "schema": {"gain": {"type": "number"}}}
    ]});
    assert_eq!(from_value(v), Err(TreeError::DuplicateId("f.gain".to_owned())));
}

#[test]
fn rejects_non_finite_numbers() {
    let nan = f64::NAN;
    let inf = f64::INFINITY;
    let slider = |min, max, value| Widget::Slider { id: "s".into(), min, max, value, step: None };
    assert!(matches!(expect_err(slider(0.0, 1.0, nan)), TreeError::NonFinite { field: "value", .. }));
    assert!(matches!(expect_err(slider(0.0, inf, 0.5)), TreeError::NonFinite { field: "max", .. }));
    assert!(matches!(expect_err(slider(-inf, 1.0, 0.5)), TreeError::NonFinite { field: "min", .. }));
    let step = Widget::Slider { id: "s".into(), min: 0.0, max: 1.0, value: 0.5, step: Some(nan) };
    assert!(matches!(expect_err(step), TreeError::NonFinite { field: "step", .. }));
    let num = Widget::Number { id: "n".into(), value: inf, min: None, max: None };
    assert!(matches!(expect_err(num), TreeError::NonFinite { field: "value", .. }));
    let prog = Widget::Progress { id: "p".into(), value: nan };
    assert!(matches!(expect_err(prog), TreeError::NonFinite { field: "value", .. }));
    let color = Widget::Color { id: "c".into(), rgba: [0.0, f32::NAN, 0.0, 1.0] };
    assert!(matches!(expect_err(color), TreeError::NonFinite { field: "rgba", .. }));
}

#[test]
fn rejects_out_of_range_props() {
    let slider = |min, max, value, step| Widget::Slider { id: "s".into(), min, max, value, step };
    assert!(matches!(expect_err(slider(2.0, 1.0, 1.5, None)), TreeError::OutOfRange { field: "min", .. }));
    assert!(matches!(expect_err(slider(0.0, 1.0, 1.5, None)), TreeError::OutOfRange { field: "value", .. }));
    assert!(matches!(expect_err(slider(0.0, 1.0, 0.5, Some(0.0))), TreeError::OutOfRange { field: "step", .. }));
    let num = |value, min, max| Widget::Number { id: "n".into(), value, min, max };
    assert!(matches!(expect_err(num(5.0, Some(6.0), None)), TreeError::OutOfRange { field: "value", .. }));
    assert!(matches!(expect_err(num(5.0, None, Some(4.0))), TreeError::OutOfRange { field: "value", .. }));
    assert!(matches!(expect_err(num(5.0, Some(9.0), Some(1.0))), TreeError::OutOfRange { field: "min", .. }));
    let prog = |value| Widget::Progress { id: "p".into(), value };
    assert!(matches!(expect_err(prog(1.01)), TreeError::OutOfRange { field: "value", .. }));
    assert!(matches!(expect_err(prog(-0.01)), TreeError::OutOfRange { field: "value", .. }));
    let color = Widget::Color { id: "c".into(), rgba: [0.0, 0.0, 0.0, 1.5] };
    assert!(matches!(expect_err(color), TreeError::OutOfRange { field: "rgba", .. }));
    let list = Widget::List { id: "l".into(), items: vec!["a".into()], selected: Some(1) };
    assert!(matches!(expect_err(list), TreeError::OutOfRange { field: "selected", .. }));
    let dd = Widget::Dropdown { id: "d".into(), options: vec!["a".into()], value: "zzz".into() };
    assert!(matches!(expect_err(dd), TreeError::OutOfRange { field: "value", .. }));
}

#[test]
fn rejects_oversize_text_and_item_lists() {
    let big = "x".repeat(MAX_TEXT_LEN + 1);
    assert!(matches!(expect_err(Widget::Label { id: "a".into(), text: big.clone() }), TreeError::TooLong { field: "text", .. }));
    assert!(matches!(expect_err(Widget::Text { id: "a".into(), value: big, multiline: true }), TreeError::TooLong { field: "value", .. }));
    let items: Vec<String> = (0..=MAX_ITEMS).map(|i| i.to_string()).collect();
    assert!(matches!(expect_err(Widget::List { id: "a".into(), items, selected: None }), TreeError::TooLong { field: "items", .. }));
}

/// Hostile text in a panel is refused, multi-line text stays.
#[test]
fn rejects_control_bidi_and_zero_width_text() {
    for bad in ["x\u{1b}[31m", "\u{202E}fdp.exe", "a\u{200B}b", "a\r\nb", "\u{2066}x"] {
        assert!(matches!(expect_err(Widget::Label { id: "a".into(), text: bad.into() }), TreeError::BadText { field: "text", .. }), "{bad:?}");
        assert!(matches!(expect_err(Widget::Button { id: "a".into(), label: bad.into() }), TreeError::BadText { field: "label", .. }), "{bad:?}");
    }
    let items = vec!["ok".to_owned(), "\u{202E}evil".to_owned()];
    assert!(matches!(expect_err(Widget::List { id: "a".into(), items, selected: None }), TreeError::BadText { field: "items", .. }));
    assert!(WidgetTree::new(Widget::Label { id: "a".into(), text: "two\nlines\tand tab, \u{e9}\u{4e2d}".into() }).is_ok());
}

#[test]
fn param_form_schema_and_values_are_checked() {
    let form = |schema: serde_json::Value, values: serde_json::Value| {
        from_value(json!({"type": "param_form", "id": "f", "schema": schema, "values": values}))
    };
    let reason = |r: Result<WidgetTree, TreeError>| match r {
        Err(TreeError::BadParam { reason, .. }) => reason,
        other => panic!("expected BadParam, got {other:?}"),
    };
    assert!(form(json!({"g": {"type": "number", "min": 0, "max": 2}}), json!({"g": 1.5})).is_ok());
    assert_eq!(reason(form(json!({"g": {"type": "number", "min": 0, "max": 2}}), json!({"g": 3}))), "value is outside min and max");
    assert_eq!(reason(form(json!({"g": {"type": "int"}}), json!({"g": 1.5}))), "int value must be whole");
    assert_eq!(reason(form(json!({"g": {"type": "bool"}}), json!({"g": "yes"}))), "value must be a boolean");
    assert_eq!(reason(form(json!({"g": {"type": "choice", "options": ["a"]}}), json!({"g": "b"}))), "value must be one of the options");
    assert_eq!(reason(form(json!({"g": {"type": "choice"}}), json!({}))), "choice needs at least one option");
    assert_eq!(reason(form(json!({"g": {"type": "number", "min": 3, "max": 1}}), json!({}))), "min is above max");
    assert_eq!(reason(form(json!({"g": {"type": "color"}}), json!({"g": [1, 2]}))), "color must be [r, g, b, a] in 0 to 1");
    assert_eq!(reason(form(json!({"g": {"type": "number", "default": "x"}}), json!({}))), "value must be a finite number");
    assert_eq!(reason(form(json!({"bad name": {"type": "bool"}}), json!({}))), "name must match [A-Za-z0-9][A-Za-z0-9_]*");
    assert_eq!(reason(form(json!({"g": {"type": "bool"}}), json!({"zz": true}))), "value has no schema entry");
}

#[test]
fn from_json_reports_unknown_widgets_and_fields() {
    assert!(matches!(from_value(json!({"type": "marquee", "id": "a"})), Err(TreeError::Json(_))));
    assert!(matches!(from_value(json!({"type": "label", "id": "a", "text": "t", "color": "red"})), Err(TreeError::Json(_))));
    assert!(matches!(from_value(json!({"type": "label", "id": "a"})), Err(TreeError::Json(_))));
}

#[test]
fn every_widget_kind_round_trips_through_json() {
    let tree = json!({"type": "column", "id": "root", "children": [
        {"type": "row", "id": "r", "children": [{"type": "label", "id": "l", "text": "hi"}]},
        {"type": "group", "id": "g", "title": "T", "children": [{"type": "button", "id": "b", "label": "Go"}]},
        {"type": "checkbox", "id": "cb", "label": "On", "value": true},
        {"type": "slider", "id": "sl", "min": 0.0, "max": 10.0, "value": 2.5, "step": 0.5},
        {"type": "number", "id": "n", "value": 3.0, "min": 0.0, "max": 9.0},
        {"type": "text", "id": "t", "value": "abc", "multiline": true},
        {"type": "dropdown", "id": "dd", "options": ["a", "b"], "value": "b"},
        {"type": "color", "id": "co", "rgba": [0.25, 0.5, 0.75, 1.0]},
        {"type": "list", "id": "li", "items": ["x", "y"], "selected": 1},
        {"type": "progress", "id": "pr", "value": 0.5},
        {"type": "separator", "id": "se"},
        {"type": "param_form", "id": "pf",
         "schema": {"gain": {"type": "number", "label": "Gain", "min": 0.0, "max": 2.0, "decimals": 2, "default": 1.0},
                    "mode": {"type": "choice", "options": ["a", "b"], "group": "Advanced"}},
         "values": {"gain": 1.5, "mode": "b"}}
    ]});
    let parsed = from_value(tree.clone()).expect("tree validates");
    let kinds = parsed.root().children().iter().map(Widget::kind).collect::<Vec<_>>();
    assert_eq!(kinds, ["row", "group", "checkbox", "slider", "number", "text", "dropdown", "color", "list", "progress", "separator", "param_form"]);
    let back = serde_json::to_value(&parsed).expect("serializes");
    assert_eq!(back, tree);
    let again: WidgetTree = serde_json::from_value(back).expect("deserializes");
    assert_eq!(again, parsed);
}

#[test]
fn deserialize_enforces_validation() {
    let bad = json!({"type": "column", "id": "a", "children": [{"type": "label", "id": "a", "text": "x"}]});
    assert!(serde_json::from_value::<WidgetTree>(bad).is_err());
}
