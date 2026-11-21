//! egui renderer for a [`WidgetTree`].
//!
//! [`show`] draws the tree and updates it in place with what the user did, so a text box keeps the
//! typed text while the plugin works on its reply. It also returns one [`UiEvent`] per change. The
//! host forwards each event to the plugin as a `panel` `action` invocation. When the plugin answers
//! with a new tree, the host replaces this one.

use std::collections::BTreeMap;

use craft_ui::ComboBox;
use egui::{DragValue, ScrollArea, Slider, Ui};
use serde_json::{Number, Value, json};

use crate::tree::{ParamDef, ParamType, Widget, WidgetTree, param_event_id};

/// Height of a `list` widget before it scrolls.
const LIST_MAX_HEIGHT: f32 = 200.0;

/// One user action. `widget_id` is the widget's `id`, or `"<form id>.<param name>"` for an edit
/// inside a `param_form`.
#[derive(Clone, Debug, PartialEq)]
pub struct UiEvent {
    pub widget_id: String,
    /// Button: null. Checkbox: bool. Slider, number: number. Text, dropdown: string. Color:
    /// `[r, g, b, a]`. List: selected index.
    pub value: Value,
}

/// Draw `tree` into `ui`. Returns the events caused this frame, in draw order.
pub fn show(ui: &mut Ui, tree: &mut WidgetTree) -> Vec<UiEvent> {
    let mut events = Vec::new();
    draw(ui, &mut tree.root, &mut events);
    events
}

fn emit(events: &mut Vec<UiEvent>, widget_id: &str, value: Value) {
    events.push(UiEvent { widget_id: widget_id.to_owned(), value });
}

fn num(v: f64) -> Value {
    // Validation and `edit_f64` keep every value finite, so this never yields null.
    Number::from_f64(v).map_or(Value::Null, Value::Number)
}

/// Keep a number the user typed only if it is finite. Returns true when the value changed.
fn edit_f64(value: &mut f64, edit: impl FnOnce(&mut f64) -> bool) -> bool {
    let before = *value;
    let changed = edit(value);
    if !value.is_finite() {
        *value = before;
        return false;
    }
    changed
}

fn draw(ui: &mut Ui, w: &mut Widget, events: &mut Vec<UiEvent>) {
    let id = w.id().to_owned();
    ui.push_id(&id, |ui| match w {
        Widget::Column { children, .. } => {
            ui.vertical(|ui| children.iter_mut().for_each(|c| draw(ui, c, events)));
        }
        Widget::Row { children, .. } => {
            ui.horizontal_wrapped(|ui| children.iter_mut().for_each(|c| draw(ui, c, events)));
        }
        Widget::Group { title, children, .. } => {
            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.strong(title.as_str());
                    children.iter_mut().for_each(|c| draw(ui, c, events));
                });
            });
        }
        Widget::Label { text, .. } => {
            ui.label(text.as_str());
        }
        Widget::Button { label, .. } => {
            if ui.button(label.as_str()).clicked() {
                emit(events, &id, Value::Null);
            }
        }
        Widget::Checkbox { label, value, .. } => {
            if ui.checkbox(value, label.as_str()).changed() {
                emit(events, &id, json!(*value));
            }
        }
        Widget::Slider { min, max, value, step, .. } => {
            if min < max {
                let mut s = Slider::new(value, *min..=*max);
                if let Some(step) = step {
                    s = s.step_by(*step);
                }
                // The slider's own drag keeps the value inside min..=max and finite.
                if ui.add(s).accessible_name(&id).changed() {
                    emit(events, &id, num(*value));
                }
            } else {
                ui.add_enabled(false, Slider::new(value, *min..=*max)).accessible_name(&id);
            }
        }
        Widget::Number { value, min, max, .. } => {
            let range = min.unwrap_or(f64::MIN)..=max.unwrap_or(f64::MAX);
            if edit_f64(value, |v| ui.add(DragValue::new(v).range(range)).accessible_name(&id).changed()) {
                emit(events, &id, num(*value));
            }
        }
        Widget::Text { value, multiline, .. } => {
            let r = if *multiline { ui.text_edit_multiline(value) } else { ui.text_edit_singleline(value) };
            let r = r.accessible_name(&id);
            if r.changed() {
                emit(events, &id, json!(value));
            }
        }
        Widget::Dropdown { options, value, .. } => {
            if let Some(pick) = combo(ui, &id, options, value, |r| r.accessible_name(&id)) {
                *value = pick.clone();
                emit(events, &id, json!(pick));
            }
        }
        Widget::Color { rgba, .. } => {
            if ui.color_edit_button_rgba_unmultiplied(rgba).changed() {
                emit(events, &id, json!(rgba));
            }
        }
        Widget::List { items, selected, .. } => {
            ScrollArea::vertical().max_height(LIST_MAX_HEIGHT).show(ui, |ui| {
                for (i, item) in items.iter().enumerate() {
                    if ui.selectable_label(*selected == Some(i), item.as_str()).clicked() && *selected != Some(i) {
                        *selected = Some(i);
                        emit(events, &id, json!(i));
                    }
                }
            });
        }
        Widget::Progress { value, .. } => {
            ui.add(egui::ProgressBar::new(*value as f32).show_percentage());
        }
        Widget::Separator { .. } => {
            ui.separator();
        }
        Widget::ParamForm { schema, values, .. } => param_form(ui, &id, schema, values, events),
    });
}

/// A combo box over `options`. `name` gives the box its accessible name. Returns the newly
/// picked option.
fn combo(ui: &mut Ui, salt: &str, options: &[String], current: &str, name: impl FnOnce(egui::Response) -> egui::Response) -> Option<String> {
    let mut pick = None;
    let boxed = ComboBox::from_id_salt(salt).selected_text(current).show_ui(ui, |ui| {
        for opt in options {
            if ui.selectable_label(opt == current, opt.as_str()).clicked() && opt != current {
                pick = Some(opt.clone());
            }
        }
    });
    name(boxed.response);
    pick
}

/// The value shown for a parameter: the current value, else the schema default, else a neutral one.
fn current_value(def: &ParamDef, values: &BTreeMap<String, Value>, name: &str) -> Value {
    if let Some(v) = values.get(name).or(def.default.as_ref()) {
        return v.clone();
    }
    match def.kind {
        ParamType::Number | ParamType::Angle => num(def.min.unwrap_or(0.0).max(0.0).min(def.max.unwrap_or(f64::MAX))),
        ParamType::Int => num(def.min.unwrap_or(0.0).max(0.0).min(def.max.unwrap_or(f64::MAX)).ceil()),
        ParamType::Bool => json!(false),
        ParamType::Choice => json!(def.options.first().cloned().unwrap_or_default()),
        ParamType::Text => json!(""),
        ParamType::Color => json!([0.0, 0.0, 0.0, 1.0]),
        ParamType::Point2 | ParamType::Point3 | ParamType::Layer | ParamType::File | ParamType::Json => Value::Null,
    }
}

fn param_form(ui: &mut Ui, form_id: &str, schema: &BTreeMap<String, ParamDef>, values: &mut BTreeMap<String, Value>, events: &mut Vec<UiEvent>) {
    // Group order: ungrouped params first, then groups by name; names sort inside each.
    let mut order: Vec<(&String, &ParamDef)> = schema.iter().collect();
    order.sort_by(|a, b| (a.1.group.as_deref(), a.0).cmp(&(b.1.group.as_deref(), b.0)));
    let mut last_group: Option<Option<&str>> = None;
    egui::Grid::new(form_id).num_columns(2).show(ui, |ui| {
        for (name, def) in order {
            if last_group != Some(def.group.as_deref()) {
                last_group = Some(def.group.as_deref());
                if let Some(g) = &def.group {
                    ui.strong(g.as_str());
                    ui.end_row();
                }
            }
            let title = def.label.as_deref().unwrap_or(name);
            // A bool carries its title on the checkbox so assistive tools read it. Other editors
            // point at the title label.
            let title_id = if def.kind == ParamType::Bool {
                ui.label("");
                None
            } else {
                Some(ui.label(title).id)
            };
            let cur = current_value(def, values, name);
            if let Some(new) = edit_param(ui, name, title, title_id, def, &cur) {
                emit(events, &param_event_id(form_id, name), new.clone());
                values.insert(name.clone(), new);
            }
            ui.end_row();
        }
    });
}

/// Draw the editor for one parameter. Returns the new value when the user changed it.
fn edit_param(ui: &mut Ui, name: &str, title: &str, title_id: Option<egui::Id>, def: &ParamDef, cur: &Value) -> Option<Value> {
    let tag = |r: egui::Response| match title_id {
        Some(id) => r.labelled_by(id),
        None => r,
    };
    match def.kind {
        ParamType::Number | ParamType::Angle | ParamType::Int => {
            let mut v = cur.as_f64().unwrap_or(0.0);
            let is_int = def.kind == ParamType::Int;
            let range = def.min.unwrap_or(f64::MIN)..=def.max.unwrap_or(f64::MAX);
            let changed = edit_f64(&mut v, |v| {
                let mut dv = DragValue::new(v).range(range);
                if is_int {
                    dv = dv.speed(1.0).fixed_decimals(0);
                } else if let Some(d) = def.decimals {
                    dv = dv.fixed_decimals(d as usize);
                }
                if def.kind == ParamType::Angle {
                    dv = dv.suffix(" deg");
                }
                tag(ui.add(dv)).changed()
            });
            changed.then(|| num(if is_int { v.round() } else { v }))
        }
        ParamType::Bool => {
            let mut b = cur.as_bool().unwrap_or(false);
            ui.checkbox(&mut b, title).changed().then(|| json!(b))
        }
        ParamType::Choice => {
            let current = cur.as_str().unwrap_or("");
            combo(ui, name, &def.options, current, tag).map(|p| json!(p))
        }
        ParamType::Text => {
            let mut s = cur.as_str().unwrap_or("").to_owned();
            tag(ui.text_edit_singleline(&mut s)).changed().then(|| json!(s))
        }
        ParamType::Color => {
            let mut c = color_of(cur);
            tag(ui.color_edit_button_rgba_unmultiplied(&mut c)).changed().then(|| json!(c))
        }
        ParamType::Point2 | ParamType::Point3 | ParamType::Layer | ParamType::File | ParamType::Json => {
            ui.weak(format!("{} (not editable here)", if cur.is_null() { "-".to_owned() } else { cur.to_string() }));
            None
        }
    }
}

fn color_of(v: &Value) -> [f32; 4] {
    let mut out = [0.0, 0.0, 0.0, 1.0];
    if let Some(a) = v.as_array() {
        for (slot, c) in out.iter_mut().zip(a) {
            *slot = c.as_f64().map_or(*slot, |c| c as f32);
        }
    }
    out
}
