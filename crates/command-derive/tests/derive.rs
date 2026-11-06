//! `#[command]` on handlers with typed params: the generated row, the run wrapper, the schema.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_command::{CommandEntry, CommandSpec, Registry};
use craft_command_derive::command;
use serde::Deserialize;
use serde_json::{Value, json};

pub struct Session {
    pub names: Vec<String>,
    pub locked: bool,
}

#[derive(Debug, PartialEq)]
pub enum AppError {
    BadParams(String),
    Refused(&'static str),
}

impl From<String> for AppError {
    fn from(message: String) -> Self {
        AppError::BadParams(message)
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NestParams {
    name: String,
    #[serde(default)]
    parent: Option<u32>,
}

/// Rename-free handler with a doc string, a menu path, a shortcut and the default enablement.
#[command(id = "clip.nest", label = "Nest", doc = r#"{"name":str,"parent":binId?}"#, menu = ["Clip", "Nest"], shortcut = "Ctrl+N", alt_shortcuts = ["Cmd+N", "Ctrl+Alt+N"])]
pub fn nest(session: &mut Session, params: NestParams) -> Result<Value, AppError> {
    session.names.push(params.name.clone());
    Ok(json!({ "nested": params.name, "parent": params.parent }))
}

fn unlocked(session: &Session) -> Result<(), String> {
    if session.locked { Err("the project is locked".to_owned()) } else { Ok(()) }
}

fn bad_params(message: String) -> AppError {
    AppError::BadParams(format!("custom: {message}"))
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Empty {}

#[command(id = "clip.lock", label = "Lock", doc = "{}", enabled = unlocked, bad_params = bad_params, journal = false)]
pub fn lock(session: &mut Session, _params: Empty) -> Result<Value, AppError> {
    if session.locked {
        return Err(AppError::Refused("already locked"));
    }
    session.locked = true;
    Ok(Value::Null)
}

fn session() -> Session {
    Session { names: Vec::new(), locked: false }
}

#[test]
fn the_row_carries_the_attribute_fields() {
    let row: &CommandSpec<Session, AppError> = &NEST_COMMAND;
    assert_eq!(row.id, "clip.nest");
    assert_eq!(row.label, "Nest");
    assert_eq!(row.menu, &["Clip", "Nest"]);
    assert_eq!(row.shortcut, Some("Ctrl+N"));
    assert_eq!(row.alt_shortcuts, &["Cmd+N", "Ctrl+Alt+N"]);
    assert_eq!(row.params, r#"{"name":str,"parent":binId?}"#);
    assert!(row.journal, "journal defaults to true");
    let lock: &CommandSpec<Session, AppError> = &LOCK_COMMAND;
    assert!(!lock.journal);
    assert_eq!(lock.shortcut, None);
    assert!(lock.alt_shortcuts.is_empty(), "alt_shortcuts defaults to empty");
    assert!(lock.menu.is_empty());
}

#[test]
fn run_parses_params_into_the_struct_and_calls_the_handler() {
    let mut s = session();
    let out = (NEST_COMMAND.run)(&mut s, &json!({ "name": "A", "parent": 7 })).unwrap();
    assert_eq!(out, json!({ "nested": "A", "parent": 7 }));
    assert_eq!(s.names, ["A"], "the handler ran on the caller's session");
    let out = (NEST_COMMAND.run)(&mut s, &json!({ "name": "B" })).unwrap();
    assert_eq!(out["parent"], Value::Null, "serde defaults apply");
}

#[test]
fn bad_params_do_not_reach_the_handler_and_use_the_error_conversion() {
    let mut s = session();
    for bad in [json!({}), json!({ "name": 3 }), json!({ "name": "A", "extra": 1 }), json!("text")] {
        let err = (NEST_COMMAND.run)(&mut s, &bad).unwrap_err();
        let AppError::BadParams(message) = err else { panic!("{bad}: wrong error") };
        assert!(message.starts_with("clip.nest: bad params"), "{message}");
    }
    assert!(s.names.is_empty(), "no handler call happened");
    let err = (LOCK_COMMAND.run)(&mut s, &json!(5)).unwrap_err();
    assert_eq!(err, AppError::BadParams("custom: clip.lock: bad params: invalid type: integer `5`, expected struct Empty".to_owned()));
}

#[test]
fn enablement_defaults_to_always_and_takes_a_custom_function() {
    let mut s = session();
    assert_eq!((NEST_COMMAND.enabled)(&s), Ok(()));
    assert_eq!((LOCK_COMMAND.enabled)(&s), Ok(()));
    s.locked = true;
    assert_eq!((NEST_COMMAND.enabled)(&s), Ok(()));
    assert_eq!((LOCK_COMMAND.enabled)(&s), Err("the project is locked".to_owned()));
}

#[test]
fn the_derived_schema_comes_from_the_params_struct_and_is_tagged() {
    let schema = nest_schema();
    assert_eq!(schema["x-craft-source"], "derived");
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"]["name"]["type"], "string");
    assert_eq!(schema["required"], json!(["name"]), "the Option field is not required");
    assert_eq!(schema["additionalProperties"], false, "deny_unknown_fields shows in the schema");
    let (id, f) = NEST_SCHEMA;
    assert_eq!(id, "clip.nest");
    assert_eq!(f(), schema);
}

#[test]
fn the_row_works_in_a_registry_and_the_registry_still_serves_the_doc_schema() {
    static ROWS: [CommandSpec<Session, AppError>; 2] = [NEST_COMMAND, LOCK_COMMAND];
    let registry: Registry<Session, AppError> = Registry::new(&ROWS);
    let Some(CommandEntry::Static(spec)) = registry.resolve("clip.nest") else { panic!("clip.nest did not resolve") };
    let mut s = session();
    assert_eq!((spec.run)(&mut s, &json!({ "name": "Z" })).unwrap()["nested"], "Z");
    // The registry builds its schema from the `doc` string, so it names the same keys. The typed
    // schema from `nest_schema()` is not served by the registry yet; that needs a side table in craft-command.
    let served = registry.schema("clip.nest").unwrap();
    assert_eq!(served["properties"].as_object().unwrap().keys().collect::<Vec<_>>(), ["name", "parent"]);
}
