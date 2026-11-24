//! Named controls over the window's access tree (`ui.find`, `control` targets) and `ui.inspect` (the reported
//! widgets of a window's access tree, and the lookup of the one node a click or a focus names).
//!
//! Both read the tree the window host keeps from the last presented frame ([`WindowHost::keep_access_tree`], which
//! the control server turns on) and walk only the nodes assistive technology reports ([`AccessTree::keys`]): a hidden node,
//! one below it and one behind an open modal are neither listed nor reachable.

use std::collections::HashSet;

use craft_behavior::Mixed;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::egui::access::{AccessTree, Declared, NodeKey, name};
use craft_ui::runtime::windows::WindowHost;
use egui::{Rect, ViewportId};
use serde_json::{Map, Value, json};

use crate::protocol::Params;

/// The error of a request made before the first kept pass has an access tree.
pub const NO_TREE: &str = "no accessibility tree yet: send the request after a frame was drawn";

/// The error of a target no control matches.
pub const NO_CONTROL: &str = "no control matches";

/// The error of a target several controls match, with no `index`.
pub const AMBIGUOUS: &str = "more than one control matches";

/// The error of a control with no bounds to place a point on.
pub const NO_PLACE: &str = "the control has no place on the screen";

/// The largest `index` a target takes.
pub const MAX_INDEX: usize = 1024;

/// The widgets `ui.inspect` lists when the request names no `limit`.
pub const DEFAULT_LIMIT: u64 = 500;

/// The place of a control a target names: its rect in screen points, or the error why it has none.
pub type ControlMap<'a> = &'a dyn Fn(&Target) -> Result<Rect, String>;

/// A control named by `{name, role?, within?, index?}`.
///
/// A name match ignores surrounding white space and one trailing `:`; `role` is an AccessKit role name, compared
/// ignoring case; `within` is the name of any matching ancestor. Without `index` exactly one control matches; the
/// reading order is depth first.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Target {
    /// The control's accessible name.
    pub name: String,
    /// The control's role.
    pub role: Option<String>,
    /// The name of an ancestor the control is inside.
    pub within: Option<String>,
    /// Which match to take, in reading order.
    pub index: Option<usize>,
}

/// `text` as a name is compared: without surrounding white space and one trailing `:`.
fn plain(text: &str) -> &str {
    let text = text.trim();
    text.strip_suffix(':').unwrap_or(text).trim()
}

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

/// Queues `action` on the node `target` names in `viewport`'s kept access tree.
fn inject_node(host: &WindowHost, viewport: ViewportId, target: &NodeTarget, action: Declared) -> Result<(), String> {
    let tree = host.access_tree_view(viewport).ok_or_else(|| NO_TREE.to_owned())?;
    let node = resolve(&tree, target, action)?;
    host.inject_access(viewport, node, action);
    Ok(())
}

/// Queues a click of the node `target` names in `viewport`, once the kept tree has it as one reported, enabled,
/// clickable node ([`resolve`]).
pub fn inject_click(host: &WindowHost, viewport: ViewportId, target: &NodeTarget) -> Result<(), String> {
    inject_node(host, viewport, target, Declared::Click)
}

/// Queues the context menu of the node `target` names in `viewport`, once the kept tree has it as one reported,
/// enabled node with a context menu ([`resolve`]).
pub fn inject_context_menu(host: &WindowHost, viewport: ViewportId, target: &NodeTarget) -> Result<(), String> {
    inject_node(host, viewport, target, Declared::ContextMenu)
}

/// Queues the focus request for the node `target` names in `viewport`, once the kept tree has it as one reported,
/// enabled, focusable node ([`resolve`]).
pub fn inject_focus(host: &WindowHost, viewport: ViewportId, target: &NodeTarget) -> Result<(), String> {
    inject_node(host, viewport, target, Declared::Focus)
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

impl Target {
    /// The target `value` names: `{name, role?, within?, index?}` and no other field.
    pub fn from_params(value: &Value) -> Result<Self, String> {
        let p = Params(value).only(None, &["name", "role", "within", "index"])?;
        let p = Params(&p);
        let target = p.str_field("name")?.ok_or_else(|| "missing `name`".to_owned())?;
        if target.trim().is_empty() {
            return Err("`name` must not be empty".into());
        }
        let index = match p.uint_field("index")? {
            Some(index) if index > MAX_INDEX as u64 => return Err(format!("`index` must be at most {MAX_INDEX} (got {index})")),
            index => index.map(|index| index as usize),
        };
        Ok(Self { name: target.to_owned(), role: p.str_field("role")?.map(str::to_owned), within: p.str_field("within")?.map(str::to_owned), index })
    }

    /// Whether the control `key` has this target's name, role and ancestor.
    fn accepts(&self, tree: &AccessTree, key: NodeKey) -> bool {
        let named = |key: NodeKey, wanted: &str| name(tree, key).is_some_and(|found| plain(found) == plain(wanted));
        if !named(key, &self.name) {
            return false;
        }
        if self.role.as_deref().is_some_and(|wanted| !role_name(tree, key).is_some_and(|role| role.eq_ignore_ascii_case(wanted))) {
            return false;
        }
        let Some(within) = self.within.as_deref() else { return true };
        let mut above = tree.parent_of_key(key);
        while let Some(ancestor) = above {
            if named(ancestor, within) {
                return true;
            }
            above = tree.parent_of_key(ancestor);
        }
        false
    }

    /// The error of no control matching, with the fields the target gave.
    fn missing(&self) -> String {
        let mut error = format!("{NO_CONTROL}: name `{}`", self.name);
        if let Some(role) = &self.role {
            error.push_str(&format!(" role `{role}`"));
        }
        if let Some(within) = &self.within {
            error.push_str(&format!(" within `{within}`"));
        }
        if let Some(index) = self.index {
            error.push_str(&format!(" index {index}"));
        }
        error
    }

    /// The control of `tree` this target names: [`NO_CONTROL`] when none matches, [`AMBIGUOUS`] when several do and
    /// there is no `index`.
    pub fn find(&self, tree: &AccessTree) -> Result<NodeKey, String> {
        let reported: HashSet<NodeKey> = tree.keys().into_iter().collect();
        let mut matches = Vec::new();
        let mut pending: Vec<NodeKey> = tree.root_key().into_iter().collect();
        while let Some(key) = pending.pop() {
            if reported.contains(&key) && self.accepts(tree, key) {
                matches.push(key);
            }
            pending.extend(tree.children_of(key).into_iter().rev());
        }
        match (self.index, matches.as_slice()) {
            (_, []) => Err(self.missing()),
            (Some(index), found) => found.get(index).copied().ok_or_else(|| format!("index {index} is past the {} matching controls", found.len())),
            (None, [only]) => Ok(*only),
            (None, found) => Err(format!("{AMBIGUOUS} ({}): add `role`, `within` or `index`", found.len())),
        }
    }

    /// The rect of this target's control in the tree the host kept for `viewport`: [`NO_TREE`] before the first,
    /// [`NO_PLACE`] for a control with no area.
    pub fn rect(&self, host: &WindowHost, viewport: ViewportId) -> Result<Rect, String> {
        let tree = host.access_tree(viewport).ok_or_else(|| NO_TREE.to_owned())?;
        tree.bounds_of(self.find(&tree)?)
            .filter(|rect| rect.is_finite() && rect.width() > 0.0 && rect.height() > 0.0)
            .ok_or_else(|| NO_PLACE.to_owned())
    }
}

/// What `ui.find` answers for the control `key`: `{name, role, rect or null, enabled, focused, value?, numeric?,
/// expanded?, selected?, toggled?}`, the rect in screen points.
pub fn describe(tree: &AccessTree, key: NodeKey) -> Value {
    let mut out = Map::new();
    out.insert("name".into(), json!(name(tree, key)));
    out.insert("role".into(), json!(role_name(tree, key)));
    out.insert("rect".into(), json!(tree.bounds_of(key).map(|rect| [rect.left(), rect.top(), rect.width(), rect.height()])));
    out.insert("enabled".into(), json!(!tree.is_disabled_key(key)));
    out.insert("focused".into(), json!(tree.focused_key() == key));
    if let Some(value) = tree.value_of(key) {
        out.insert("value".into(), json!(value));
    }
    if let Some(numeric) = tree.numeric_of(key) {
        out.insert("numeric".into(), json!({"value": numeric.value, "range": [numeric.range.start(), numeric.range.end()], "step": numeric.step}));
    }
    if let Some(expanded) = tree.expanded_of(key) {
        out.insert("expanded".into(), json!(expanded));
    }
    if let Some(selected) = tree.selected_of(key) {
        out.insert("selected".into(), json!(selected));
    }
    if let Some(toggled) = tree.toggled_of(key) {
        out.insert(
            "toggled".into(),
            match toggled {
                Mixed::Same(on) => json!(on),
                Mixed::Mixed => json!("mixed"),
            },
        );
    }
    Value::Object(out)
}

/// Runs the access method `method` with `params` in `viewport` from the tree the host kept ([`NO_TREE`] before the
/// first); `None` for a method this module does not have.
pub fn access_method(method: &str, params: &Value, host: &WindowHost, viewport: ViewportId) -> Option<Result<Value, String>> {
    match method {
        "ui.find" => Some(find(params, host, viewport)),
        _ => None,
    }
}

/// `ui.find`: the description of the control `params` names.
fn find(params: &Value, host: &WindowHost, viewport: ViewportId) -> Result<Value, String> {
    let target = Target::from_params(params)?;
    let tree = host.access_tree(viewport).ok_or_else(|| NO_TREE.to_owned())?;
    Ok(describe(&tree, target.find(&tree)?))
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
