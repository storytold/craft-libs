//! `ui.dialog.set`, `ui.dialog.confirm` and `ui.dialog.cancel` on an app's [`DialogHost`], for every dialog type.
//!
//! The app's method table hands these methods to [`dialog_method`] and lists [`inspect`] under `ui.inspect`'s
//! `dialogs`. A dialog is named by the `id` `inspect` lists, or `"top"` for the top one; its fields are its
//! [`StackDialog::json_fields`](craft_ui::dialog::StackDialog::json_fields). Opening a dialog (`ui.dialog.open`) and
//! running the command its answer is for stay the app's: an answer waits for the app's `DialogHost::take` as a
//! click's would.

use craft_ui::dialog::{DialogHost, DialogId, HeldOk, StandardPrompt};
use egui::Context;
use serde_json::{Value, json};

use crate::protocol::Params;

/// The most `fields` one `ui.dialog.set` call takes.
pub const MAX_DIALOG_FIELDS: usize = 256;

/// The open dialogs, the top one last: `{id, parent, key, fields}`, `null` for a missing parent, key or fields. Ids
/// are 64-bit hashes, so they are listed as decimal strings, which every JSON reader keeps exact.
pub fn inspect(host: &DialogHost) -> Value {
    host.open_dialogs()
        .into_iter()
        .map(|dialog| {
            json!({
                "id": dialog.id.value().to_string(),
                "parent": dialog.parent.map(|id| id.value().to_string()),
                "key": dialog.key,
                "fields": dialog.fields,
            })
        })
        .collect()
}

/// Runs a `ui.dialog.*` method on `host`; `None` when `method` is not one of them.
///
/// - `ui.dialog.set {dialog, field, value}` sets one field ([`DialogHost::set_json_field`]); `{dialog, fields: [{field,
///   value?}]}` sets a list of at most [`MAX_DIALOG_FIELDS`], all or none ([`DialogHost::set_json_fields`]).
/// - `ui.dialog.confirm {dialog, choice?}` answers OK, as the default button would, after setting the `choice` field
///   when one is given (which button of a prompt). A dialog that holds its OK files it for the app, which answers it.
/// - `ui.dialog.cancel {dialog}` closes the dialog as Escape would: a standard prompt with a button for its Escape
///   answer gives that answer, with its tag; any other dialog closes with its children with Cancel.
pub fn dialog_method(host: &mut DialogHost, ctx: &Context, method: &str, params: &Value) -> Option<Result<Value, String>> {
    let fields: &[&str] = match method {
        "ui.dialog.set" => &["dialog", "field", "value", "fields"],
        "ui.dialog.confirm" => &["dialog", "choice"],
        "ui.dialog.cancel" => &["dialog"],
        _ => return None,
    };
    Some(run(host, ctx, method, params, fields))
}

fn run(host: &mut DialogHost, ctx: &Context, method: &str, params: &Value, fields: &[&str]) -> Result<Value, String> {
    let params = Params(params).only(None, fields)?;
    let read = Params(&params);
    let id = dialog_id(host, &params)?;
    match method {
        "ui.dialog.set" => match params.get("fields") {
            Some(list) => {
                if params.get("field").is_some() || params.get("value").is_some() {
                    return Err("`fields` takes the place of `field` and `value`".to_owned());
                }
                host.set_json_fields(ctx, id, &field_list(list)?)?;
            }
            None => {
                let field = read.str_field("field")?.ok_or("missing `field`")?;
                host.set_json_field(ctx, id, field, params.get("value").cloned().unwrap_or(Value::Null))?;
            }
        },
        "ui.dialog.confirm" => {
            if let Some(choice) = params.get("choice") {
                host.set_json_field(ctx, id, "choice", choice.clone())?;
            }
            // A held OK is filed for the app, which answers it: OK was taken, not refused.
            if !host.confirm_by_id(ctx, id) && host.held_state(id) != Some(HeldOk::Waiting) {
                return Err(
                    "the dialog refuses OK: it is disabled, its answer has no enabled button, or the dialog has no programmatic OK".to_owned()
                );
            }
        }
        _ => {
            // A prompt's Escape is an answer the app reads with the prompt's tag, so it goes through the prompt's
            // own OK path rather than the host's outside close.
            let escape = host.get_dyn_mut(id).and_then(|dialog| dialog.downcast_mut::<StandardPrompt>()).and_then(|prompt| {
                let choice = prompt.escape_choice()?;
                prompt.set_choice(choice);
                Some(choice)
            });
            if escape.is_none() || !host.confirm_by_id(ctx, id) {
                host.cancel_by_id(ctx, id);
            }
        }
    }
    Ok(Value::Null)
}

/// The `(field, value)` pairs of a `fields` list, checked whole before anything is set; a missing `value` is `null`.
fn field_list(list: &Value) -> Result<Vec<(String, Value)>, String> {
    let items = list.as_array().ok_or("`fields` is not a list")?;
    if items.len() > MAX_DIALOG_FIELDS {
        return Err(format!("`fields` holds {} items; at most {MAX_DIALOG_FIELDS}", items.len()));
    }
    items
        .iter()
        .map(|item| {
            let field = item.get("field").and_then(Value::as_str).ok_or("an item of `fields` has no string `field`")?;
            Ok((field.to_owned(), item.get("value").cloned().unwrap_or(Value::Null)))
        })
        .collect()
}

/// The open dialog `params.dialog` names: the id `inspect` lists, as a decimal string or a number, or `"top"` for the
/// top dialog of the stack.
pub fn dialog_id(host: &DialogHost, params: &Value) -> Result<DialogId, String> {
    let params = Params(params);
    let value = match params.0.get("dialog") {
        Some(Value::String(id)) if id == "top" => return host.stack().top().ok_or_else(|| "no hosted dialog is open".to_owned()),
        Some(Value::String(id)) => id.parse().map_err(|_| format!("`dialog` {id:?} is not an id `ui.inspect` lists"))?,
        _ => params.uint_field("dialog")?.ok_or("missing `dialog`")?,
    };
    host.stack().ids().find(|id| id.value() == value).ok_or_else(|| format!("no dialog {value}"))
}
