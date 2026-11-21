//! `craft bindgen`: turn an app's command list into a typed Rust client for plugin code.
//!
//! Input is the command list an app serves: `engine.commands`, which `<app>-cli commands --json`
//! prints too. Each row has an `id`, an optional `label`, and either a `schema` (JSON Schema) or the
//! `params` doc string. A doc string is read with `craft_command::schema::from_doc`, the code the
//! apps use, so the client and the app agree on what a doc means.
//!
//! Output is one Rust module. Each command becomes a params struct and a function that calls
//! `craft_plugin_sdk::cmd::execute`:
//!
//! ```text
//! let reply = commands::file_new_project(&commands::FileNewProjectParams { name: "Demo".into() })?;
//! ```

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::process::Command;

use craft_command::schema::from_doc;
use serde_json::Value;

use crate::error::{CliError, Result};

/// One command from the list.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandRow {
    pub id: String,
    pub label: Option<String>,
    /// JSON Schema object for the params.
    pub schema: Value,
    /// `derived`, `doc` or `opaque`: how much the schema can be trusted.
    pub source: String,
}

/// Read the rows from the JSON an app printed: an array, or `{"commands": [...]}`.
pub fn parse_command_list(list: &Value) -> Result<Vec<CommandRow>> {
    let rows = match list {
        Value::Array(rows) => rows,
        Value::Object(map) => {
            map.get("commands").and_then(Value::as_array).ok_or_else(|| CliError::new("the command list is an object with no \"commands\" array"))?
        }
        _ => return Err(CliError::new("the command list must be a JSON array or {\"commands\": [...]}")),
    };
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let id = row.get("id").and_then(Value::as_str).ok_or_else(|| CliError::new(format!("command row {i} has no string id")))?.to_owned();
        let schema = match row.get("schema") {
            Some(s) if s.is_object() => s.clone(),
            _ => from_doc(row.get("params").and_then(Value::as_str).unwrap_or("{}")).value,
        };
        let source = schema.get("x-craft-source").and_then(Value::as_str).unwrap_or("doc").to_owned();
        out.push(CommandRow { id, label: row.get("label").and_then(Value::as_str).map(str::to_owned), schema, source });
    }
    Ok(out)
}

/// Run `<app>-cli commands --json`.
pub fn fetch_headless(app: &str) -> Result<Value> {
    let valid = !app.is_empty() && app.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        return Err(CliError::new(format!("app name {app:?} must be lowercase letters, digits and '-'")));
    }
    let program = format!("{app}-cli");
    let out = Command::new(&program)
        .args(["commands", "--json"])
        .output()
        .map_err(|e| CliError::new(format!("cannot run {program}: {e}. Put it on PATH, or pass --from <commands.json>")))?;
    if !out.status.success() {
        return Err(CliError::new(format!("{program} commands --json failed ({}): {}", out.status, String::from_utf8_lossy(&out.stderr).trim())));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| CliError::new(format!("{program} printed JSON that does not parse: {e}")))
}

const KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
    "move", "mut", "pub", "ref", "return", "self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "async",
    "await", "dyn", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
];

/// `file.newProject` gives `file_new_project`. Anything that is not a letter or digit is a break.
pub fn snake_case(text: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && prev_lower {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
            prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        } else {
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
            prev_lower = false;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn ident(name: &str) -> String {
    let snake = snake_case(name);
    if KEYWORDS.contains(&snake.as_str()) { format!("{snake}_") } else { snake }
}

fn pascal_case(snake: &str) -> String {
    snake
        .split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut chars = p.chars();
            chars.next().map(|f| f.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
        })
        .collect()
}

/// One line of doc text.
fn doc_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn rust_type(prop: &Value) -> &'static str {
    match prop.get("type").and_then(Value::as_str) {
        Some("string") => "String",
        Some("integer") => "i64",
        Some("number") => "f64",
        Some("boolean") => "bool",
        Some("array") => "Vec<Value>",
        // An object, or a property the schema does not type (every key-list doc), stays JSON.
        _ => "Value",
    }
}

struct Field {
    json_name: String,
    rust_name: String,
    ty: &'static str,
    required: bool,
    doc: String,
}

fn fields_of(row: &CommandRow) -> Result<Option<(Vec<Field>, bool)>> {
    let Some(props) = row.schema.get("properties").and_then(Value::as_object) else {
        return Ok(None);
    };
    let required: Vec<&str> = row.schema.get("required").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    let mut fields = Vec::new();
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for (name, prop) in props {
        let rust_name = ident(name);
        if let Some(other) = seen.insert(rust_name.clone(), name.clone()) {
            return Err(CliError::new(format!("{}: params {other:?} and {name:?} both become the field `{rust_name}`", row.id)));
        }
        let doc = prop.get("description").and_then(Value::as_str).map(doc_line).unwrap_or_default();
        fields.push(Field { json_name: name.clone(), rust_name, ty: rust_type(prop), required: required.contains(&name.as_str()), doc });
    }
    let open = row.schema.get("additionalProperties") == Some(&Value::Bool(true));
    Ok(Some((fields, open)))
}

/// Generate the module text for `rows`. Fails if two commands become the same function name.
pub fn generate(app: &str, rows: &[CommandRow]) -> Result<String> {
    let mut out = String::new();
    let _ = writeln!(out, "// Typed client for the {} commands of {app}. Generated by `craft bindgen`. Do not edit.", rows.len());
    let _ = writeln!(out, "// Include it from a plugin with `mod commands;` and call `commands::<name>(&params)` from a handler.");
    let _ = writeln!(out, "#![allow(dead_code, clippy::all)]\n");
    let _ = writeln!(out, "use craft_plugin_sdk::{{OpError, Value, cmd, serde}};\n");
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for row in rows {
        let fn_name = ident(&row.id);
        if let Some(other) = seen.insert(fn_name.clone(), row.id.clone()) {
            return Err(CliError::new(format!("commands {other:?} and {:?} both become the function `{fn_name}`", row.id)));
        }
        let label = row.label.as_deref().map(doc_line).filter(|l| !l.is_empty());
        let _ = writeln!(out, "/// {} (`{}`, schema source: {}).", label.unwrap_or_else(|| row.id.clone()), row.id, row.source);
        match fields_of(row)? {
            Some((fields, open)) => {
                let ty = format!("{}Params", pascal_case(&fn_name));
                let _ = writeln!(
                    out,
                    "#[derive(Clone, Debug, PartialEq, serde::Serialize)]\n#[serde(crate = \"craft_plugin_sdk::serde\")]\npub struct {ty} {{"
                );
                for f in &fields {
                    if !f.doc.is_empty() {
                        let _ = writeln!(out, "    /// {}", f.doc);
                    }
                    if f.rust_name != f.json_name {
                        let _ = writeln!(out, "    #[serde(rename = {:?})]", f.json_name);
                    }
                    if f.required {
                        let _ = writeln!(out, "    pub {}: {},", f.rust_name, f.ty);
                    } else {
                        let _ = writeln!(out, "    #[serde(skip_serializing_if = \"Option::is_none\")]\n    pub {}: Option<{}>,", f.rust_name, f.ty);
                    }
                }
                if open {
                    let _ = writeln!(
                        out,
                        "    /// Further params the schema allows.\n    #[serde(flatten)]\n    pub extra: craft_plugin_sdk::serde_json::Map<String, Value>,"
                    );
                }
                let _ = writeln!(out, "}}\n");
                let _ = writeln!(
                    out,
                    "/// Run `{}`. Returns its result.\npub fn {fn_name}(params: &{ty}) -> Result<Value, OpError> {{\n    cmd::execute({:?}, params)\n}}\n",
                    row.id, row.id
                );
            }
            None => {
                let _ = writeln!(out, "/// The params of this command are not described by a schema, so they are plain JSON.");
                let _ = writeln!(out, "pub fn {fn_name}(params: &Value) -> Result<Value, OpError> {{\n    cmd::execute({:?}, params)\n}}\n", row.id);
            }
        }
    }
    Ok(out)
}
