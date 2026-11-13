//! `ui.dialog.set`, `ui.dialog.confirm` and `ui.dialog.cancel` on an app's [`DialogHost`], for every dialog type.
//!
//! The app's method table hands these methods to [`dialog_method`] and lists [`inspect`] under `ui.inspect`'s
//! `dialogs`. A dialog is named by the `id` `inspect` lists; its fields are its
//! [`StackDialog::json_fields`](craft_ui::dialog::StackDialog::json_fields). Opening a dialog (`ui.dialog.open`) and
//! running the command its answer is for stay the app's: an answer waits for the app's `DialogHost::take` as a
//! click's would.

use craft_ui::dialog::{DialogHost, DialogId};
use egui::Context;
use serde_json::{Value, json};

use crate::protocol::Params;

/// The open dialogs, the top one last: `{id, parent, key, fields}`, `null` for a missing parent, key or fields. Ids
/// are 64-bit hashes, so they are listed as decimal strings, which every JSON reader keeps exact.
pub fn inspect(host: &DialogHost) -> Value {
    host.open_dialogs()
        .into_iter()
        .map(|dialog| {
            json!({"id": dialog.id.value().to_string(), "parent": dialog.parent.map(|id| id.value().to_string()), "key": dialog.key, "fields": dialog.fields})
        })
        .collect()
}

/// Whether `params.dialog` names an open dialog of `host`, so an app that lists dialogs of its own beside the host's
/// knows which of them a `ui.dialog.*` request is for.
pub fn names_open_dialog(host: &DialogHost, params: &Value) -> bool {
    dialog(host, Params(params)).is_ok()
}

/// Runs a `ui.dialog.*` method on `host`; `None` when `method` is not one of them.
///
/// - `ui.dialog.set {dialog, field, value}` sets one field ([`DialogHost::set_json_field`]).
/// - `ui.dialog.confirm {dialog, choice?}` answers OK, as the default button would, after setting the `choice` field
///   when one is given (which button of a prompt).
/// - `ui.dialog.cancel {dialog}` closes the dialog and its children with Cancel.
pub fn dialog_method(host: &mut DialogHost, ctx: &Context, method: &str, params: &Value) -> Option<Result<Value, String>> {
    let fields: &[&str] = match method {
        "ui.dialog.set" => &["dialog", "field", "value"],
        "ui.dialog.confirm" => &["dialog", "choice"],
        "ui.dialog.cancel" => &["dialog"],
        _ => return None,
    };
    Some(run(host, ctx, method, params, fields))
}

fn run(host: &mut DialogHost, ctx: &Context, method: &str, params: &Value, fields: &[&str]) -> Result<Value, String> {
    let params = Params(params).only(None, fields)?;
    let read = Params(&params);
    let id = dialog(host, read)?;
    match method {
        "ui.dialog.set" => {
            let field = read.str_field("field")?.ok_or("missing `field`")?;
            host.set_json_field(ctx, id, field, params.get("value").cloned().unwrap_or(Value::Null))?;
        }
        "ui.dialog.confirm" => {
            if let Some(choice) = params.get("choice") {
                host.set_json_field(ctx, id, "choice", choice.clone())?;
            }
            if !host.confirm_by_id(ctx, id) {
                return Err(
                    "the dialog refuses OK: it is disabled, its answer has no enabled button, or the dialog has no programmatic OK".to_owned()
                );
            }
        }
        _ => {
            host.cancel_by_id(ctx, id);
        }
    }
    Ok(Value::Null)
}

/// The open dialog `params.dialog` names: the id `inspect` lists, as a string or a number. A dialog the app shows
/// beside the host is on the stack but not the host's, so it is no dialog here.
fn dialog(host: &DialogHost, params: Params<'_>) -> Result<DialogId, String> {
    let value = match params.0.get("dialog") {
        Some(Value::String(id)) => id.parse().map_err(|_| format!("`dialog` {id:?} is not an id `ui.inspect` lists"))?,
        _ => params.uint_field("dialog")?.ok_or("missing `dialog`")?,
    };
    host.stack().ids().find(|id| id.value() == value && host.get_dyn(*id).is_some()).ok_or_else(|| format!("no dialog {value}"))
}
