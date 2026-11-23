//! `ui.inspect`: the reported widgets of a window's access tree, and the lookup of the one node a click or a focus
//! names.
//!
//! Both read the tree the window host keeps from the last presented frame ([`WindowHost::keep_access_tree`], which
//! the control server turns on) and walk only the nodes assistive technology reports ([`AccessTree::keys`]): a hidden node,
//! one below it and one behind an open modal are neither listed nor reachable.

use std::collections::HashSet;

use craft_behavior::Mixed;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::egui::access::{AccessTree, Declared, NodeKey};
use craft_ui::runtime::windows::WindowHost;
use egui::ViewportId;
use serde_json::{Map, Value, json};

use crate::protocol::Params;

/// The widgets `ui.inspect` lists when the request names no `limit`.
pub const DEFAULT_LIMIT: u64 = 500;

/// The error of a request that needs the access tree before the window presented a frame with one.
pub const NO_TREE: &str = "no accessibility tree yet; draw a frame first";

/// The node a method names: its id or its label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeTarget {
    /// The node's id.
    Id(u64),
    /// The node's label, matched case-insensitively.
    Label(String),
}

impl NodeTarget {
    /// The node `params` name (`id` or `label`), or none when they name none. `id` is the id `ui.inspect` lists, a
    /// decimal string or a number; `label` is a string; naming both is an error. Other fields are the caller's to
    /// check; a `null` field is absent.
    pub fn from_params(params: &Value) -> Result<Option<NodeTarget>, String> {
        let field = |key: &str| params.get(key).filter(|value| !value.is_null());
        match (field("id"), field("label")) {
            (Some(_), Some(_)) => Err("`id` and `label` exclude each other".into()),
            (Some(id), None) => {
                let raw = id.as_u64().or_else(|| id.as_str().and_then(|text| text.parse().ok()));
                raw.map(|raw| Some(NodeTarget::Id(raw))).ok_or_else(|| "id must be a widget id from ui.inspect".into())
            }
            (None, Some(Value::String(label))) => Ok(Some(NodeTarget::Label(label.clone()))),
            (None, Some(_)) => Err("label must be a string".into()),
            (None, None) => Ok(None),
        }
    }
}

/// What a node that takes `action` is, for the refusal of one that does not.
fn able(action: Declared) -> &'static str {
    match action {
        Declared::Click => "clickable",
        Declared::Focus => "focusable",
        Declared::ContextMenu => "a widget with a context menu",
        _ => "a widget that takes this action",
    }
}

/// The one reported, enabled node of `tree` that declares `action` and that `target` names; an error names none,
/// several, disabled or undeclared. An id must be a reported node; a label matches the reported, enabled nodes that
/// declare `action` and whose label equals it, ignoring case, and exactly one must match.
pub fn resolve(tree: &AccessTree, target: &NodeTarget, action: Declared) -> Result<NodeKey, String> {
    let reported = tree.keys();
    let label_of = |key: NodeKey| tree.name_of(key).unwrap_or("unlabelled").to_owned();
    let key = match target {
        NodeTarget::Id(raw) => {
            let key = NodeKey::from_raw(*raw);
            if !reported.contains(&key) {
                return Err(format!("no widget with id {raw} on screen (run ui.inspect again)"));
            }
            key
        }
        NodeTarget::Label(label) => {
            let wanted = label.to_lowercase();
            let named: Vec<NodeKey> =
                reported.iter().copied().filter(|key| tree.name_of(*key).is_some_and(|name| name.to_lowercase() == wanted)).collect();
            let usable: Vec<NodeKey> = named.iter().copied().filter(|key| !tree.is_disabled_key(*key) && tree.declares_key(*key, action)).collect();
            match usable.as_slice() {
                [one] => *one,
                [] if named.is_empty() => return Err(format!("no widget labelled {label:?} on screen (try ui.inspect with query)")),
                [] if named.iter().any(|key| tree.declares_key(*key, action)) => {
                    return Err(format!("the widget labelled {label:?} is disabled"));
                }
                [] => return Err(format!("the widget labelled {label:?} is not {}", able(action))),
                many => return Err(format!("{} widgets are labelled {label:?}; name one by id", many.len())),
            }
        }
    };
    if tree.is_disabled_key(key) {
        return Err(format!("widget {} ({}) is disabled", key.raw(), label_of(key)));
    }
    if !tree.declares_key(key, action) {
        return Err(format!("widget {} ({}) is not {}", key.raw(), label_of(key), able(action)));
    }
    Ok(key)
}

/// The accesskit role's name as agents' scripts read it (`Button`, `CheckBox`).
fn role_name(tree: &AccessTree, key: NodeKey) -> Option<String> {
    match tree.role_of(key)? {
        Role::Node(role) => Some(format!("{role:?}")),
        _ => None,
    }
}

/// One listed widget: `{id, role, depth, enabled, clickable, focusable}` and, where the node says, `label`, `value`,
/// `description`, `rect` (`[x0, y0, x1, y1]` in points), `toggled` (`true`, `false` or `mixed`), `selected`, and
/// `focused: true` for the focused node. The id is a decimal string: 64-bit ids do not survive JSON readers that use
/// doubles.
fn widget(tree: &AccessTree, key: NodeKey, depth: usize, focused: bool) -> Value {
    let mut w = Map::new();
    w.insert("id".into(), json!(key.raw().to_string()));
    w.insert("role".into(), json!(role_name(tree, key)));
    w.insert("depth".into(), json!(depth));
    w.insert("enabled".into(), json!(!tree.is_disabled_key(key)));
    w.insert("clickable".into(), json!(tree.declares_key(key, Declared::Click)));
    w.insert("focusable".into(), json!(tree.declares_key(key, Declared::Focus)));
    if let Some(label) = tree.name_of(key) {
        w.insert("label".into(), json!(label));
    }
    if let Some(value) = tree.value_of(key) {
        w.insert("value".into(), json!(value));
    }
    if let Some(description) = tree.description_of(key) {
        w.insert("description".into(), json!(description));
    }
    if let Some(rect) = tree.bounds_of(key) {
        w.insert("rect".into(), json!([rect.min.x, rect.min.y, rect.max.x, rect.max.y]));
    }
    if let Some(toggled) = tree.toggled_of(key) {
        let toggled = match toggled {
            Mixed::Same(true) => "true",
            Mixed::Same(false) => "false",
            Mixed::Mixed => "mixed",
        };
        w.insert("toggled".into(), json!(toggled));
    }
    if let Some(selected) = tree.selected_of(key) {
        w.insert("selected".into(), json!(selected));
    }
    if focused {
        w.insert("focused".into(), json!(true));
    }
    Value::Object(w)
}

/// `inspect {query?, role?, focused?, limit?=500}`: the reply `{widgets, count, truncated}` over the reported nodes
/// of `viewport`'s access tree, in tree order (depth first from the root, which is not listed, children in their
/// order). `query` matches a widget whose label, value or description contains it, ignoring case; `role` its role's
/// name, ignoring case; `focused` whether it is the focused node. `count` is every widget the filters keep, `widgets`
/// the first `limit` of them. No other fields; [`NO_TREE`] before the window presented a frame.
pub fn inspect(host: &WindowHost, viewport: ViewportId, params: &Value) -> Result<Value, String> {
    let p = Params(params).only(None, &["query", "role", "focused", "limit"])?;
    let p = Params(&p);
    let query = p.str_field("query")?.map(str::to_lowercase);
    let role = p.str_field("role")?.map(str::to_lowercase);
    let focused = p.bool_field("focused")?;
    let limit = usize::try_from(p.uint_field("limit")?.unwrap_or(DEFAULT_LIMIT)).unwrap_or(usize::MAX);
    let tree = host.access_tree_view(viewport).ok_or_else(|| NO_TREE.to_owned())?;

    let reported: HashSet<NodeKey> = tree.keys().into_iter().collect();
    let root = tree.root_key();
    let focus = tree.focused_key();
    let (mut widgets, mut count) = (Vec::new(), 0usize);
    // A stack, not recursion: the tree comes from the app and may be deep. `walked` keeps a malformed tree that lists
    // a node twice from listing it twice or looping.
    let mut walked = HashSet::new();
    let mut pending: Vec<(NodeKey, usize)> = root.into_iter().map(|key| (key, 0)).collect();
    while let Some((key, depth)) = pending.pop() {
        if !reported.contains(&key) || !walked.insert(key) {
            continue;
        }
        pending.extend(tree.children_of(key).into_iter().rev().map(|child| (child, depth + 1)));
        if Some(key) == root {
            continue;
        }
        let text =
            [tree.name_of(key), tree.value_of(key), tree.description_of(key)].into_iter().flatten().collect::<Vec<_>>().join(" ").to_lowercase();
        let keep = query.as_ref().is_none_or(|query| text.contains(query.as_str()))
            && role.as_ref().is_none_or(|role| role_name(&tree, key).is_some_and(|name| name.to_lowercase() == *role))
            && focused.is_none_or(|wanted| (key == focus) == wanted);
        if !keep {
            continue;
        }
        count += 1;
        if widgets.len() < limit {
            widgets.push(widget(&tree, key, depth, key == focus));
        }
    }
    Ok(json!({"widgets": widgets, "count": count, "truncated": count > widgets.len()}))
}
