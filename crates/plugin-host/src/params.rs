//! Check the `params` of a data plane call against the contribution's `params` schema
//! (the host validates `params` against the schema before the guest sees them).
//!
//! The result is the object the guest receives: every declared parameter is present (a default
//! fills a missing one), and nothing undeclared passes. A value that breaks its type, its range, or
//! its option list is an error. Nothing is clamped, because a clamp hides a caller bug.

use std::fmt;

use craft_plugin_abi::manifest::{Contribution, ParamSpec, ParamType};
use serde_json::{Map, Value};

/// A parameter value that does not fit the schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamError(pub String);

impl fmt::Display for ParamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParamError {}

fn bad<T>(name: &str, why: impl fmt::Display) -> Result<T, ParamError> {
    Err(ParamError(format!("parameter `{name}` {why}")))
}

fn finite(name: &str, v: &Value) -> Result<f64, ParamError> {
    match v.as_f64().filter(|n| n.is_finite()) {
        Some(n) => Ok(n),
        None => bad(name, format_args!("must be a finite number, got {v}")),
    }
}

fn in_range(name: &str, n: f64, spec: &ParamSpec) -> Result<(), ParamError> {
    if spec.min.is_some_and(|min| n < min) || spec.max.is_some_and(|max| n > max) {
        let lo = spec.min.map_or("-inf".to_owned(), |v| v.to_string());
        let hi = spec.max.map_or("inf".to_owned(), |v| v.to_string());
        return bad(name, format_args!("is {n}, outside {lo}..{hi}"));
    }
    Ok(())
}

fn numbers(name: &str, v: &Value, n: usize) -> Result<(), ParamError> {
    match v.as_array().filter(|a| a.len() == n) {
        Some(items) => items.iter().try_for_each(|x| finite(name, x).map(drop)),
        None => bad(name, format_args!("must be a list of {n} numbers, got {v}")),
    }
}

fn check(name: &str, spec: &ParamSpec, v: &Value) -> Result<(), ParamError> {
    match spec.ty {
        ParamType::Number | ParamType::Angle => in_range(name, finite(name, v)?, spec),
        ParamType::Int => match v.as_i64() {
            Some(n) => in_range(name, n as f64, spec),
            None => bad(name, format_args!("must be a whole number, got {v}")),
        },
        ParamType::Bool => match v {
            Value::Bool(_) => Ok(()),
            _ => bad(name, format_args!("must be true or false, got {v}")),
        },
        ParamType::Choice => match (v, &spec.options) {
            (Value::String(s), Some(options)) if options.contains(s) => Ok(()),
            (Value::String(s), Some(options)) => bad(name, format_args!("is {s:?}, not one of {options:?}")),
            (Value::String(_), None) => bad(name, "is a choice with no options"),
            _ => bad(name, format_args!("must be one of its options, got {v}")),
        },
        ParamType::Text | ParamType::Layer | ParamType::File => match v {
            Value::String(_) => Ok(()),
            _ => bad(name, format_args!("must be text, got {v}")),
        },
        ParamType::Color => numbers(name, v, 4),
        ParamType::Point2 => numbers(name, v, 2),
        ParamType::Point3 => numbers(name, v, 3),
        ParamType::Json => Ok(()),
    }
}

/// Validate `given` (an object, or null for none) against `c.params` and return the full object.
pub fn resolve(c: &Contribution, given: &Value) -> Result<Map<String, Value>, ParamError> {
    let empty = Map::new();
    let given = match given {
        Value::Null => &empty,
        Value::Object(m) => m,
        other => return Err(ParamError(format!("parameters must be an object, got {other}"))),
    };
    let declared = c.params.as_ref();
    if let Some(unknown) = given.keys().find(|k| !declared.is_some_and(|d| d.contains_key(*k))) {
        return Err(ParamError(format!("unknown parameter `{unknown}`")));
    }
    let mut out = Map::new();
    for (name, spec) in declared.into_iter().flatten() {
        let value = match (given.get(name), &spec.default) {
            (Some(v), _) => v.clone(),
            (None, Some(default)) => default.clone(),
            (None, None) => return bad(name, "is required and has no default"),
        };
        check(name, spec, &value)?;
        out.insert(name.clone(), value);
    }
    Ok(out)
}
