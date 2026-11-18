//! `ui.window`: window commands through the framework's one window-command path.

use craft_ui::runtime::windows::{WindowAction, WindowHost};
use egui::ViewportId;
use serde_json::{Value, json};

use crate::protocol::Params;

/// `ui.window {maximized: bool}` sends `WindowAction::SetMaximized` through `WindowHost::act` and returns `Ok({})`, or
/// `Err` when `act` returns false (no window commands, as on wasm). `None` for any other method. It keeps no state.
pub fn window_method(host: &WindowHost, viewport: ViewportId, method: &str, params: &Value) -> Option<Result<Value, String>> {
    if method != "ui.window" {
        return None;
    }
    Some(run(host, viewport, params))
}

fn run(host: &WindowHost, viewport: ViewportId, params: &Value) -> Result<Value, String> {
    let params = Params(params).only(None, &["maximized"])?;
    let maximized = Params(&params).bool_field("maximized")?.ok_or("missing `maximized`")?;
    if !host.act(viewport, WindowAction::SetMaximized(maximized)) {
        return Err("this platform has no window commands".to_owned());
    }
    Ok(json!({}))
}
