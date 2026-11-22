//! Named controls over the window's access tree: the controls a client finds by name and role, describes, and places
//! a pointer on. Built with the in-process channel, under the `ui` feature.

use std::collections::HashSet;

use craft_behavior::Mixed;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::egui::access::{AccessTree, NodeKey, name};
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

/// The AccessKit role name of a node, as `role` fields spell it.
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
