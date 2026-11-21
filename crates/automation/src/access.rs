//! Named controls over the window's access tree: the controls a client finds by name and role, describes, and places
//! a pointer on. Built with the in-process channel, under the `ui` feature.

use craft_ui::runtime::egui::access::{AccessTree, NodeKey};
use craft_ui::runtime::windows::WindowHost;
use egui::ViewportId;
use serde_json::Value;

/// The error of a request made before the first kept pass has an access tree.
pub const NO_TREE: &str = "the window has no access tree yet";

/// The error of a target no control matches.
pub const NO_CONTROL: &str = "no control matches the target";

/// The error of a target several controls match, with no `index`.
pub const AMBIGUOUS: &str = "more than one control matches the target; add `index`, `role` or `within`";

/// The error of a control with no bounds to place a point on.
pub const NO_PLACE: &str = "the control has no bounds to place a point on";

/// The largest `index` a target takes.
pub const MAX_INDEX: usize = 1024;

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

impl Target {
    /// The target `value` names: `{name, role?, within?, index?}` and no other field.
    pub fn from_params(value: &Value) -> Result<Self, String> {
        let _ = value;
        todo!("wg.W51")
    }

    /// The control of `tree` this target names: [`NO_CONTROL`] when none matches, [`AMBIGUOUS`] when several do and
    /// there is no `index`.
    pub fn find(&self, tree: &AccessTree) -> Result<NodeKey, String> {
        let _ = tree;
        todo!("wg.W51")
    }
}

/// What `ui.find` answers for the control `key`: `{name, role, rect or null, enabled, focused, value?, numeric?,
/// expanded?, selected?, toggled?}`, the rect in screen points.
pub fn describe(tree: &AccessTree, key: NodeKey) -> Value {
    let _ = (tree, key);
    todo!("wg.W52")
}

/// Runs the access method `method` with `params` in `viewport` from the tree the host kept ([`NO_TREE`] before the
/// first); `None` for a method this module does not have.
pub fn access_method(method: &str, params: &Value, host: &WindowHost, viewport: ViewportId) -> Option<Result<Value, String>> {
    let _ = (method, params, host, viewport);
    todo!("wg.W52")
}
