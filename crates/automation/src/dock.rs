//! `ui.dock.apply`: one dock action applied to the app's workspace, and the workspace's `ui.inspect` listing.

use std::hash::Hash;

use craft_behavior::dock::Workspace;
use craft_ui::docking::DockPanels;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// `ui.dock.apply {action}`: deserializes one dock action and applies it to `workspace` with the permissions `panels`
/// gives, as the dock area does. It returns `Ok({applied: [...]})`, the panels the action closed, in group order; an
/// error is the dock error's text. `None` for any other method.
pub fn dock_method<P: Clone + Eq + Hash + Serialize + DeserializeOwned>(
    workspace: &mut Workspace<P>,
    panels: &dyn DockPanels<P>,
    locked: bool,
    method: &str,
    params: &Value,
) -> Option<Result<Value, String>> {
    let _ = (workspace, panels, locked, method, params);
    todo!("CH-C22")
}

/// The `dock` field of the app's one `ui.inspect`: the workspace as JSON.
pub fn inspect<P: Serialize>(workspace: &Workspace<P>) -> Value {
    let _ = workspace;
    todo!("CH-C22")
}
