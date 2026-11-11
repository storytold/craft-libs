//! Panel widget trees (JSON; the host renders it).
//!
//! `{"type": "<widget>", "id": "<stable id>", ...props, "children": [...]}`. Build a tree with the
//! constructors below, then return it from a `panel` handler. [`Widget::build`] checks the two
//! rules the host enforces: ids are unique, and a tree has at most 2,000 nodes.

use std::collections::BTreeSet;
use std::fmt;

use serde_json::{Map, Value, json};

/// The host rejects a larger tree.
pub const MAX_NODES: usize = 2000;

/// One node of a panel tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    kind: &'static str,
    id: String,
    props: Map<String, Value>,
    children: Vec<Widget>,
}

/// Why a tree cannot be sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeError {
    TooManyNodes { count: usize },
    DuplicateId(String),
    EmptyId,
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TreeError::TooManyNodes { count } => write!(f, "panel tree has {count} nodes, the host takes at most {MAX_NODES}"),
            TreeError::DuplicateId(id) => write!(f, "widget id {id:?} is used twice"),
            TreeError::EmptyId => f.write_str("a widget has an empty id"),
        }
    }
}

impl std::error::Error for TreeError {}

impl Widget {
    fn new(kind: &'static str, id: &str) -> Widget {
        Widget { kind, id: id.to_owned(), props: Map::new(), children: Vec::new() }
    }

    fn with(mut self, key: &str, value: impl Into<Value>) -> Widget {
        self.props.insert(key.to_owned(), value.into());
        self
    }

    /// Set a property the constructors do not cover, such as `multiline` on `text`.
    pub fn prop(self, key: &str, value: impl Into<Value>) -> Widget {
        self.with(key, value)
    }

    pub fn child(mut self, child: Widget) -> Widget {
        self.children.push(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = Widget>) -> Widget {
        self.children.extend(children);
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Nodes in this subtree, this node included.
    pub fn node_count(&self) -> usize {
        1 + self.children.iter().map(Widget::node_count).sum::<usize>()
    }

    fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("type".to_owned(), json!(self.kind));
        map.insert("id".to_owned(), json!(self.id));
        for (k, v) in &self.props {
            map.insert(k.clone(), v.clone());
        }
        if !self.children.is_empty() {
            map.insert("children".to_owned(), Value::Array(self.children.iter().map(Widget::to_value).collect()));
        }
        Value::Object(map)
    }

    fn check(&self, seen: &mut BTreeSet<String>) -> Result<(), TreeError> {
        if self.id.is_empty() {
            return Err(TreeError::EmptyId);
        }
        if !seen.insert(self.id.clone()) {
            return Err(TreeError::DuplicateId(self.id.clone()));
        }
        self.children.iter().try_for_each(|c| c.check(seen))
    }

    /// Check the tree and turn it into the JSON the host reads.
    pub fn build(&self) -> Result<Value, TreeError> {
        let count = self.node_count();
        if count > MAX_NODES {
            return Err(TreeError::TooManyNodes { count });
        }
        self.check(&mut BTreeSet::new())?;
        Ok(self.to_value())
    }
}

pub fn column(id: &str) -> Widget {
    Widget::new("column", id)
}

pub fn row(id: &str) -> Widget {
    Widget::new("row", id)
}

pub fn group(id: &str, title: &str) -> Widget {
    Widget::new("group", id).with("title", title)
}

pub fn label(id: &str, text: &str) -> Widget {
    Widget::new("label", id).with("text", text)
}

pub fn button(id: &str, label: &str) -> Widget {
    Widget::new("button", id).with("label", label)
}

pub fn checkbox(id: &str, label: &str, value: bool) -> Widget {
    Widget::new("checkbox", id).with("label", label).with("value", value)
}

pub fn slider(id: &str, min: f64, max: f64, value: f64, step: f64) -> Widget {
    Widget::new("slider", id).with("min", min).with("max", max).with("value", value).with("step", step)
}

pub fn number(id: &str, value: f64) -> Widget {
    Widget::new("number", id).with("value", value)
}

pub fn text(id: &str, value: &str) -> Widget {
    Widget::new("text", id).with("value", value)
}

pub fn dropdown(id: &str, options: &[&str], value: &str) -> Widget {
    Widget::new("dropdown", id).with("options", json!(options)).with("value", value)
}

pub fn color(id: &str, rgba: [f64; 4]) -> Widget {
    Widget::new("color", id).with("rgba", json!(rgba))
}

pub fn list(id: &str, items: &[&str], selected: Option<usize>) -> Widget {
    Widget::new("list", id).with("items", json!(items)).with("selected", json!(selected))
}

/// `value` is 0.0 to 1.0.
pub fn progress(id: &str, value: f64) -> Widget {
    Widget::new("progress", id).with("value", value)
}

pub fn separator(id: &str) -> Widget {
    Widget::new("separator", id)
}

/// A form the host builds from a JSON Schema and fills with `values`.
pub fn param_form(id: &str, schema: Value, values: Value) -> Widget {
    Widget::new("param_form", id).with("schema", schema).with("values", values)
}
