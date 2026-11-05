//! Widget tree types and validation.
//!
//! A plugin returns JSON `{"type": "<widget>", "id": "<stable id>", ...props, "children": [...]}`.
//! [`WidgetTree::from_json`] and [`WidgetTree::new`] are the only ways to get a tree the renderer
//! accepts, so every limit is checked before any widget is drawn.

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// At most 2,000 nodes per tree. A `param_form` counts as one node plus one per
/// parameter, because each parameter draws a control.
pub const MAX_NODES: usize = 2_000;
/// Deepest allowed nesting. The root is depth 1.
pub const MAX_DEPTH: usize = 32;
/// Longest widget id in bytes.
pub const MAX_ID_LEN: usize = 128;
/// Longest text prop in bytes.
pub const MAX_TEXT_LEN: usize = 64 * 1024;
/// Most options in a dropdown or items in a list.
pub const MAX_ITEMS: usize = 10_000;
/// Most parameters in one `param_form` (matches `maxProperties` in the manifest schema).
pub const MAX_PARAMS: usize = 1_024;

/// One widget. `type` selects the variant; every widget has a stable `id`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Widget {
    Column {
        id: String,
        #[serde(default)]
        children: Vec<Widget>,
    },
    Row {
        id: String,
        #[serde(default)]
        children: Vec<Widget>,
    },
    Group {
        id: String,
        title: String,
        #[serde(default)]
        children: Vec<Widget>,
    },
    Label {
        id: String,
        text: String,
    },
    Button {
        id: String,
        label: String,
    },
    Checkbox {
        id: String,
        label: String,
        value: bool,
    },
    Slider {
        id: String,
        min: f64,
        max: f64,
        value: f64,
        #[serde(default)]
        step: Option<f64>,
    },
    Number {
        id: String,
        value: f64,
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
    },
    Text {
        id: String,
        value: String,
        #[serde(default)]
        multiline: bool,
    },
    Dropdown {
        id: String,
        options: Vec<String>,
        value: String,
    },
    /// `rgba` holds four straight-alpha components in 0.0 to 1.0.
    Color {
        id: String,
        rgba: [f32; 4],
    },
    /// `selected` is an index into `items`.
    List {
        id: String,
        items: Vec<String>,
        #[serde(default)]
        selected: Option<usize>,
    },
    /// `value` runs from 0.0 to 1.0.
    Progress {
        id: String,
        value: f64,
    },
    Separator {
        id: String,
    },
    /// Draws a params schema (manifest `$defs/params`). `values` holds the current value per name.
    ParamForm {
        id: String,
        schema: BTreeMap<String, ParamDef>,
        #[serde(default)]
        values: BTreeMap<String, Value>,
    },
}

impl Widget {
    /// The widget's stable id.
    pub fn id(&self) -> &str {
        match self {
            Widget::Column { id, .. }
            | Widget::Row { id, .. }
            | Widget::Group { id, .. }
            | Widget::Label { id, .. }
            | Widget::Button { id, .. }
            | Widget::Checkbox { id, .. }
            | Widget::Slider { id, .. }
            | Widget::Number { id, .. }
            | Widget::Text { id, .. }
            | Widget::Dropdown { id, .. }
            | Widget::Color { id, .. }
            | Widget::List { id, .. }
            | Widget::Progress { id, .. }
            | Widget::Separator { id }
            | Widget::ParamForm { id, .. } => id,
        }
    }

    /// Child widgets of a container, empty for every other widget.
    pub fn children(&self) -> &[Widget] {
        match self {
            Widget::Column { children, .. } | Widget::Row { children, .. } | Widget::Group { children, .. } => children,
            _ => &[],
        }
    }

    /// Wire name of the widget kind, as in the `type` field.
    pub fn kind(&self) -> &'static str {
        match self {
            Widget::Column { .. } => "column",
            Widget::Row { .. } => "row",
            Widget::Group { .. } => "group",
            Widget::Label { .. } => "label",
            Widget::Button { .. } => "button",
            Widget::Checkbox { .. } => "checkbox",
            Widget::Slider { .. } => "slider",
            Widget::Number { .. } => "number",
            Widget::Text { .. } => "text",
            Widget::Dropdown { .. } => "dropdown",
            Widget::Color { .. } => "color",
            Widget::List { .. } => "list",
            Widget::Progress { .. } => "progress",
            Widget::Separator { .. } => "separator",
            Widget::ParamForm { .. } => "param_form",
        }
    }
}

/// Parameter types of the manifest `params` schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    Number,
    Int,
    Bool,
    Choice,
    Text,
    Color,
    Angle,
    Point2,
    Point3,
    Layer,
    File,
    Json,
}

impl ParamType {
    /// True when `param_form` can edit this type. The others show their value read-only.
    pub fn is_editable(self) -> bool {
        matches!(
            self,
            ParamType::Number | ParamType::Int | ParamType::Bool | ParamType::Choice | ParamType::Text | ParamType::Color | ParamType::Angle
        )
    }
}

/// One parameter definition (manifest `$defs/params/additionalProperties`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamDef {
    #[serde(rename = "type")]
    pub kind: ParamType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

/// Why a tree failed validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeError {
    /// The JSON did not parse into widgets.
    Json(String),
    TooManyNodes {
        max: usize,
    },
    TooDeep {
        max: usize,
    },
    EmptyId,
    IdTooLong {
        max: usize,
    },
    DuplicateId(String),
    /// A number prop was NaN or infinite.
    NonFinite {
        id: String,
        field: &'static str,
    },
    /// A prop was finite but outside what the widget allows (min above max, index past the end, ...).
    OutOfRange {
        id: String,
        field: &'static str,
    },
    TooLong {
        id: String,
        field: &'static str,
        max: usize,
    },
    /// A text prop held a control, bidi or zero-width character.
    BadText {
        id: String,
        field: &'static str,
    },
    /// A `param_form` schema or value broke a rule.
    BadParam {
        id: String,
        param: String,
        reason: &'static str,
    },
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TreeError::Json(e) => write!(f, "widget tree JSON is invalid: {e}"),
            TreeError::TooManyNodes { max } => write!(f, "widget tree has more than {max} nodes"),
            TreeError::TooDeep { max } => write!(f, "widget tree is nested deeper than {max} levels"),
            TreeError::EmptyId => write!(f, "a widget has an empty id"),
            TreeError::IdTooLong { max } => write!(f, "a widget id is longer than {max} bytes"),
            TreeError::DuplicateId(id) => write!(f, "widget id {id:?} is used twice"),
            TreeError::NonFinite { id, field } => write!(f, "widget {id:?}: {field} is not a finite number"),
            TreeError::OutOfRange { id, field } => write!(f, "widget {id:?}: {field} is out of range"),
            TreeError::TooLong { id, field, max } => write!(f, "widget {id:?}: {field} is longer than {max}"),
            TreeError::BadText { id, field } => write!(f, "widget {id:?}: {field} holds a control, bidi or zero-width character"),
            TreeError::BadParam { id, param, reason } => write!(f, "param_form {id:?}, param {param:?}: {reason}"),
        }
    }
}

impl std::error::Error for TreeError {}

/// A widget tree that passed validation. The renderer takes only this type.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct WidgetTree {
    pub(crate) root: Widget,
}

impl<'de> Deserialize<'de> for WidgetTree {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let root = Widget::deserialize(d)?;
        WidgetTree::new(root).map_err(serde::de::Error::custom)
    }
}

impl WidgetTree {
    /// Validate `root` and wrap it.
    pub fn new(root: Widget) -> Result<Self, TreeError> {
        validate(&root)?;
        Ok(WidgetTree { root })
    }

    /// Parse and validate the JSON a plugin returned.
    pub fn from_json(json: &str) -> Result<Self, TreeError> {
        let root: Widget = serde_json::from_str(json).map_err(|e| TreeError::Json(e.to_string()))?;
        WidgetTree::new(root)
    }

    pub fn root(&self) -> &Widget {
        &self.root
    }

    /// Total nodes, counting each `param_form` parameter.
    pub fn node_count(&self) -> usize {
        let mut count = 0;
        let mut stack = vec![&self.root];
        while let Some(w) = stack.pop() {
            count += node_weight(w);
            stack.extend(w.children());
        }
        count
    }
}

fn node_weight(w: &Widget) -> usize {
    match w {
        Widget::ParamForm { schema, .. } => 1 + schema.len(),
        _ => 1,
    }
}

/// Id the renderer reports for parameter `param` of the form `form_id`.
pub fn param_event_id(form_id: &str, param: &str) -> String {
    format!("{form_id}.{param}")
}

fn validate(root: &Widget) -> Result<(), TreeError> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut nodes = 0usize;
    // Explicit stack: depth is checked before children are pushed, so hostile nesting cannot
    // overflow the call stack.
    let mut stack: Vec<(&Widget, usize)> = vec![(root, 1)];
    while let Some((w, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            return Err(TreeError::TooDeep { max: MAX_DEPTH });
        }
        nodes += node_weight(w);
        if nodes > MAX_NODES {
            return Err(TreeError::TooManyNodes { max: MAX_NODES });
        }
        claim_id(&mut seen, w.id())?;
        check_widget(w, &mut seen)?;
        for c in w.children() {
            stack.push((c, depth + 1));
        }
    }
    Ok(())
}

fn claim_id(seen: &mut HashSet<String>, id: &str) -> Result<(), TreeError> {
    if id.is_empty() {
        return Err(TreeError::EmptyId);
    }
    if id.len() > MAX_ID_LEN {
        return Err(TreeError::IdTooLong { max: MAX_ID_LEN });
    }
    if !seen.insert(id.to_owned()) {
        return Err(TreeError::DuplicateId(id.to_owned()));
    }
    Ok(())
}

fn finite(id: &str, field: &'static str, v: f64) -> Result<(), TreeError> {
    if v.is_finite() { Ok(()) } else { Err(TreeError::NonFinite { id: id.to_owned(), field }) }
}

fn out_of_range(id: &str, field: &'static str) -> TreeError {
    TreeError::OutOfRange { id: id.to_owned(), field }
}

fn text_len(id: &str, field: &'static str, s: &str) -> Result<(), TreeError> {
    if s.len() > MAX_TEXT_LEN {
        return Err(TreeError::TooLong { id: id.to_owned(), field, max: MAX_TEXT_LEN });
    }
    // Panel text may hold newline and tab. No other control, bidi or zero-width character.
    if craft_plugin_abi::text::has_unsafe_multiline(s) {
        return Err(TreeError::BadText { id: id.to_owned(), field });
    }
    Ok(())
}

fn items_len(id: &str, field: &'static str, items: &[String]) -> Result<(), TreeError> {
    if items.len() > MAX_ITEMS {
        return Err(TreeError::TooLong { id: id.to_owned(), field, max: MAX_ITEMS });
    }
    items.iter().try_for_each(|s| text_len(id, field, s))
}

fn check_widget(w: &Widget, seen: &mut HashSet<String>) -> Result<(), TreeError> {
    match w {
        Widget::Column { .. } | Widget::Row { .. } | Widget::Separator { .. } => Ok(()),
        Widget::Group { id, title, .. } => text_len(id, "title", title),
        Widget::Label { id, text } => text_len(id, "text", text),
        Widget::Button { id, label } | Widget::Checkbox { id, label, .. } => text_len(id, "label", label),
        Widget::Slider { id, min, max, value, step } => {
            finite(id, "min", *min)?;
            finite(id, "max", *max)?;
            finite(id, "value", *value)?;
            if min > max {
                return Err(out_of_range(id, "min"));
            }
            if value < min || value > max {
                return Err(out_of_range(id, "value"));
            }
            if let Some(s) = step {
                finite(id, "step", *s)?;
                if *s <= 0.0 {
                    return Err(out_of_range(id, "step"));
                }
            }
            Ok(())
        }
        Widget::Number { id, value, min, max } => {
            finite(id, "value", *value)?;
            if let Some(m) = min {
                finite(id, "min", *m)?;
            }
            if let Some(m) = max {
                finite(id, "max", *m)?;
            }
            if let (Some(lo), Some(hi)) = (min, max)
                && lo > hi
            {
                return Err(out_of_range(id, "min"));
            }
            if min.is_some_and(|lo| *value < lo) || max.is_some_and(|hi| *value > hi) {
                return Err(out_of_range(id, "value"));
            }
            Ok(())
        }
        Widget::Text { id, value, .. } => text_len(id, "value", value),
        Widget::Dropdown { id, options, value } => {
            items_len(id, "options", options)?;
            if !value.is_empty() && !options.contains(value) {
                return Err(out_of_range(id, "value"));
            }
            Ok(())
        }
        Widget::Color { id, rgba } => check_color(id, rgba),
        Widget::List { id, items, selected } => {
            items_len(id, "items", items)?;
            if selected.is_some_and(|i| i >= items.len()) {
                return Err(out_of_range(id, "selected"));
            }
            Ok(())
        }
        Widget::Progress { id, value } => {
            finite(id, "value", *value)?;
            if !(0.0..=1.0).contains(value) {
                return Err(out_of_range(id, "value"));
            }
            Ok(())
        }
        Widget::ParamForm { id, schema, values } => check_param_form(id, schema, values, seen),
    }
}

fn check_color(id: &str, rgba: &[f32; 4]) -> Result<(), TreeError> {
    for c in rgba {
        finite(id, "rgba", f64::from(*c))?;
        if !(0.0..=1.0).contains(c) {
            return Err(out_of_range(id, "rgba"));
        }
    }
    Ok(())
}

fn valid_param_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric()) && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn bad(id: &str, param: &str, reason: &'static str) -> TreeError {
    TreeError::BadParam { id: id.to_owned(), param: param.to_owned(), reason }
}

fn check_param_form(
    id: &str,
    schema: &BTreeMap<String, ParamDef>,
    values: &BTreeMap<String, Value>,
    seen: &mut HashSet<String>,
) -> Result<(), TreeError> {
    if schema.len() > MAX_PARAMS {
        return Err(TreeError::TooManyNodes { max: MAX_NODES });
    }
    for (name, def) in schema {
        if !valid_param_name(name) {
            return Err(bad(id, name, "name must match [A-Za-z0-9][A-Za-z0-9_]*"));
        }
        // The renderer reports edits under "<form>.<param>", so those ids must stay unique too.
        claim_id(seen, &param_event_id(id, name))?;
        check_def(id, name, def)?;
        if let Some(v) = &def.default {
            check_value(id, name, def, v)?;
        }
    }
    for (name, v) in values {
        let Some(def) = schema.get(name) else {
            return Err(bad(id, name, "value has no schema entry"));
        };
        check_value(id, name, def, v)?;
    }
    Ok(())
}

fn check_def(id: &str, name: &str, def: &ParamDef) -> Result<(), TreeError> {
    for v in [def.min, def.max].into_iter().flatten() {
        if !v.is_finite() {
            return Err(bad(id, name, "min or max is not finite"));
        }
    }
    if let (Some(lo), Some(hi)) = (def.min, def.max)
        && lo > hi
    {
        return Err(bad(id, name, "min is above max"));
    }
    if def.options.len() > MAX_ITEMS || def.options.iter().any(|o| o.len() > MAX_TEXT_LEN) {
        return Err(bad(id, name, "options are too long"));
    }
    if def.kind == ParamType::Choice && def.options.is_empty() {
        return Err(bad(id, name, "choice needs at least one option"));
    }
    Ok(())
}

/// Check a default or current value against its definition.
fn check_value(id: &str, name: &str, def: &ParamDef, v: &Value) -> Result<(), TreeError> {
    match def.kind {
        ParamType::Number | ParamType::Angle | ParamType::Int => {
            let Some(n) = v.as_f64().filter(|n| n.is_finite()) else {
                return Err(bad(id, name, "value must be a finite number"));
            };
            if def.kind == ParamType::Int && n.fract() != 0.0 {
                return Err(bad(id, name, "int value must be whole"));
            }
            if def.min.is_some_and(|lo| n < lo) || def.max.is_some_and(|hi| n > hi) {
                return Err(bad(id, name, "value is outside min and max"));
            }
            Ok(())
        }
        ParamType::Bool => v.is_boolean().then_some(()).ok_or_else(|| bad(id, name, "value must be a boolean")),
        ParamType::Text => match v.as_str() {
            Some(s) if s.len() <= MAX_TEXT_LEN => Ok(()),
            Some(_) => Err(bad(id, name, "text is too long")),
            None => Err(bad(id, name, "value must be a string")),
        },
        ParamType::Choice => match v.as_str() {
            Some(s) if def.options.iter().any(|o| o == s) => Ok(()),
            _ => Err(bad(id, name, "value must be one of the options")),
        },
        ParamType::Color => {
            let ok = v.as_array().is_some_and(|a| a.len() == 4 && a.iter().all(|c| c.as_f64().is_some_and(|c| (0.0..=1.0).contains(&c))));
            ok.then_some(()).ok_or_else(|| bad(id, name, "color must be [r, g, b, a] in 0 to 1"))
        }
        ParamType::Point2 | ParamType::Point3 | ParamType::Layer | ParamType::File | ParamType::Json => Ok(()),
    }
}
