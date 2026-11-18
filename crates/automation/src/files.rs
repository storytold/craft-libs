//! `ui.pick`: answers the next file picker of a running app.

use serde_json::Value;

/// `None` for any method other than `ui.pick`. `ui.pick` takes exactly one of `{files: [path, ..]}`, `{folder: path}`,
/// `{save: path}` or `{cancel: true}` and answers through `Files::answer_next` with `Picked::Paths` or `Cancelled`.
pub fn pick_method(files: &dyn craft_host::Files, method: &str, params: &Value) -> Option<Result<Value, String>> {
    let _ = (files, method, params);
    todo!("WF-W17")
}
