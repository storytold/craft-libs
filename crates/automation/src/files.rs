//! `ui.pick`: answers the next file picker of a running app.

use std::fmt::Display;
use std::path::PathBuf;

use craft_host::{FileError, Files, Picked};
use serde_json::{Value, json};

use crate::protocol::Params;

/// The fields of `ui.pick`, of which a request names exactly one.
const FIELDS: [&str; 4] = ["files", "folder", "save", "cancel"];

/// `None` for any method other than `ui.pick`. `ui.pick` takes exactly one of `{files: [path, ..]}`, `{folder: path}`,
/// `{save: path}` or `{cancel: true}` and answers through `Files::answer_next` with `Picked::Paths` or `Cancelled`.
/// Paths are taken as given; the service settles them as it would a panel's answer.
pub fn pick_method(files: &dyn Files, method: &str, params: &Value) -> Option<Result<Value, String>> {
    (method == "ui.pick").then(|| pick(files, params))
}

fn pick(files: &dyn Files, params: &Value) -> Result<Value, String> {
    let params = Params(params).only(None, &FIELDS)?;
    let read = Params(&params);
    let given: Vec<&str> = FIELDS.iter().copied().filter(|field| params.get(*field).is_some()).collect();
    let [field] = given.as_slice() else { return Err(refused("give exactly one field")) };
    let picked = match *field {
        "files" => Picked::Paths(paths(params.get("files"))?),
        "cancel" => match read.bool_field("cancel").map_err(refused)? {
            Some(true) => Picked::Cancelled,
            _ => return Err(refused("cancel must be true")),
        },
        one => Picked::Paths(vec![PathBuf::from(read.str_field(one).map_err(refused)?.ok_or_else(|| refused(format!("missing `{one}`")))?)]),
    };
    files.answer_next(picked).map_err(|e| match e {
        FileError::Unsupported => "this window's file service takes no automated answers".to_owned(),
        other => other.to_string(),
    })?;
    Ok(json!({"answered": "next"}))
}

/// `files` as a non-empty list of paths.
fn paths(files: Option<&Value>) -> Result<Vec<PathBuf>, String> {
    let list = files.and_then(Value::as_array).filter(|list| !list.is_empty());
    let paths = list.and_then(|list| list.iter().map(|path| path.as_str().map(PathBuf::from)).collect::<Option<Vec<_>>>());
    paths.ok_or_else(|| refused("files must be a non-empty array of path strings"))
}

fn refused(why: impl Display) -> String {
    format!("{why}; ui.pick takes exactly one of: {}", FIELDS.join(", "))
}
