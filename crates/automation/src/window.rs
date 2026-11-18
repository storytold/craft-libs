//! `ui.window`: window commands through the framework's one window-command path.

use craft_ui::runtime::windows::WindowHost;
use egui::ViewportId;
use serde_json::Value;

/// `ui.window {maximized: bool}` sends `WindowAction::SetMaximized` through `WindowHost::act` and returns `Ok({})`, or
/// `Err` when `act` returns false (no window commands, as on wasm). `None` for any other method. It keeps no state.
pub fn window_method(host: &WindowHost, viewport: ViewportId, method: &str, params: &Value) -> Option<Result<Value, String>> {
    let _ = (host, viewport, method, params);
    todo!("CH-C23")
}
