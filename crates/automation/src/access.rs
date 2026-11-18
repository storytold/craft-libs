//! `ui.inspect`: the reported widgets of a window's access tree, and the lookup of the one node a click or a focus
//! names.

use craft_ui::runtime::egui::access::{AccessTree, Declared, NodeKey};
use craft_ui::runtime::windows::WindowHost;
use egui::ViewportId;
use serde_json::Value;

/// The node a method names: its id or its label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeTarget {
    /// The node's id.
    Id(u64),
    /// The node's label, matched case-insensitively.
    Label(String),
}

impl NodeTarget {
    /// The node `params` name (`id` or `label`), or none when they name none.
    pub fn from_params(params: &Value) -> Result<Option<NodeTarget>, String> {
        let _ = params;
        todo!("SH-C15")
    }
}

/// The one reported, enabled node of `tree` that declares `action` and that `target` names; an error names none,
/// several, disabled or undeclared.
pub fn resolve(tree: &AccessTree, target: &NodeTarget, action: Declared) -> Result<NodeKey, String> {
    let _ = (tree, target, action);
    todo!("SH-C15")
}

/// `inspect {query?, role?, focused?, limit?=500}`: the reply `{widgets, count, truncated}` over the reported nodes
/// of `viewport`'s access tree, in tree order.
pub fn inspect(host: &WindowHost, viewport: ViewportId, params: &Value) -> Result<Value, String> {
    let _ = (host, viewport, params);
    todo!("SH-C15")
}
