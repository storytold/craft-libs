//! `ui.dock.apply`: one dock action applied to the app's workspace, and the workspace's `ui.inspect` listing.

use std::hash::Hash;

use craft_behavior::dock::{Action, Workspace};
use craft_ui::docking::PanelSpecs;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::protocol::Params;

/// `ui.dock.apply {action}`: deserializes one dock action and applies it to `workspace` with the permissions `specs`
/// gives, as the dock area does. It returns `Ok({applied: [...]})`, the panels the action closed, in group order; an
/// error is the dock error's text. `None` for any other method.
pub fn dock_method<P: Clone + Eq + Hash + Serialize + DeserializeOwned>(
    workspace: &mut Workspace<P>,
    specs: &impl PanelSpecs<P>,
    locked: bool,
    method: &str,
    params: &Value,
) -> Option<Result<Value, String>> {
    if method != "ui.dock.apply" {
        return None;
    }
    Some(apply(workspace, specs, locked, params))
}

fn apply<P: Clone + Eq + Hash + Serialize + DeserializeOwned>(
    workspace: &mut Workspace<P>,
    specs: &impl PanelSpecs<P>,
    locked: bool,
    params: &Value,
) -> Result<Value, String> {
    let params = Params(params).only(None, &["action"])?;
    let action = params.get("action").cloned().ok_or("missing `action`")?;
    let action: Action<P> = serde_json::from_value(action).map_err(|e| format!("`action` is not a dock action: {e}"))?;
    let applied = workspace.apply(action, locked, |panel| specs.permissions(panel)).map_err(|e| e.to_string())?;
    let applied = serde_json::to_value(applied).map_err(|e| e.to_string())?;
    Ok(json!({ "applied": applied }))
}

/// The `dock` field of the app's one `ui.inspect`: the workspace as JSON.
pub fn inspect<P: Serialize>(workspace: &Workspace<P>) -> Value {
    // A panel id that cannot be written lists the dock as null rather than failing the whole `ui.inspect`.
    serde_json::to_value(workspace).unwrap_or(Value::Null)
}
