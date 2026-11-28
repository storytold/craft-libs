//! An app's `sizes.json`: the block [`AppSizes`] reads. Errors carry the section and the path.

use serde_json::{Map, Value};

use super::{ErrorKind, FormatError, FormatErrors, LENGTH, MAX_THEME_BYTES};
use crate::app_sizes::{AppSizes, SizeMap, SizeValue, target};
use crate::sizes::{FULL, Leaf, Preset, SizeSet, Slot, leaves};
use crate::theme::{Size, TextSizeSteps};

const SECTIONS: [&str; 3] = ["all", "compact", "comfortable"];

fn err(section: &str, path: &str, kind: ErrorKind) -> FormatError {
    let mut full = vec![section.to_owned()];
    full.extend(path.split('.').filter(|s| !s.is_empty()).map(str::to_owned));
    FormatError { path: full, kind }
}

/// Reads and checks an app's size overrides. Every error is reported, not only the first.
pub(super) fn parse(bytes: &[u8]) -> Result<AppSizes, FormatErrors> {
    let stop = |kind| Err(FormatErrors(vec![FormatError { path: Vec::new(), kind }]));
    if bytes.len() > MAX_THEME_BYTES {
        return stop(ErrorKind::TooLarge { len: bytes.len() });
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => return stop(ErrorKind::Json(e.to_string())),
    };
    let Value::Object(top) = value else { return stop(ErrorKind::NotObject) };

    let mut errors = Vec::new();
    for key in top.keys().filter(|k| !SECTIONS.contains(&k.as_str())) {
        errors.push(err(key, "", ErrorKind::UnknownToken));
    }
    let mut section = |name: &str| match top.get(name) {
        None | Some(Value::Null) => SizeMap::new(),
        Some(Value::Object(o)) => map(name, o, &mut errors),
        Some(_) => {
            errors.push(err(name, "", ErrorKind::WrongType { expected: "an object" }));
            SizeMap::new()
        }
    };
    let sizes =
        AppSizes { all: section("all"), compact: section("compact"), comfortable: section("comfortable"), text_steps: TextSizeSteps::default() };
    if errors.is_empty() {
        cross_rules(&sizes, &mut errors);
    }
    if errors.is_empty() { Ok(sizes) } else { Err(FormatErrors(errors)) }
}

/// One section's values, each checked against its path's kind and range.
fn map(section: &str, obj: &Map<String, Value>, errors: &mut Vec<FormatError>) -> SizeMap {
    let mut out = SizeMap::new();
    for (path, value) in obj {
        let mut fail = |kind| errors.push(err(section, path, kind));
        let Some(target) = target(path) else {
            fail(ErrorKind::UnknownToken);
            continue;
        };
        let mut base = Preset::default();
        let found = leaves(&mut base).into_iter().find(|l| l.path == target);
        let Some(Leaf { lo, hi, slot, .. }) = found else { continue };
        let (range, length) = ((lo, hi), matches!(slot, Slot::Length(_)));
        let parsed = match value {
            Value::String(s) if length && s == "full" => Some(SizeValue::Number(FULL)),
            Value::Number(n) => {
                let n = n.as_f64().unwrap_or(f64::NAN) as f32;
                if (length && n == FULL) || (range.0..=range.1).contains(&n) {
                    Some(SizeValue::Number(n))
                } else {
                    fail(ErrorKind::OutOfRange { min: range.0, max: range.1 });
                    None
                }
            }
            _ if length => {
                fail(ErrorKind::BadValue { expected: LENGTH });
                None
            }
            _ => {
                fail(ErrorKind::WrongType { expected: "a number" });
                None
            }
        };
        out.extend(parsed.map(|v| (path.clone(), v)));
    }
    out
}

/// The size set's cross rules on each set's base values with the overrides applied. The error
/// is reported in the section of the set, at the value that breaks the rule.
fn cross_rules(sizes: &AppSizes, errors: &mut Vec<FormatError>) {
    for (name, size) in [("compact", Size::Compact), ("comfortable", Size::Comfortable)] {
        let mut set: SizeSet = crate::sizes::builtin(size).clone();
        sizes.apply_base(size, &mut set.base);
        let mut found = Vec::new();
        set.cross_rules(&mut found);
        for e in found {
            if let Some(path) = e.path.first().filter(|_| e.path.len() > 1 && e.path[0] == "base").map(|_| e.path[1..].join(".")) {
                errors.push(err(name, &path, e.kind));
            }
        }
    }
}

/// The block as pretty-printed JSON ending in a newline: the sections that have values, each
/// with its paths in order. [`AppSizes::parse`] reads it back to the same value when the values
/// are valid.
pub fn write_app_sizes(sizes: &AppSizes) -> String {
    let mut top = Map::new();
    for (name, map) in [("all", &sizes.all), ("compact", &sizes.compact), ("comfortable", &sizes.comfortable)] {
        if map.is_empty() {
            continue;
        }
        let section = map
            .iter()
            .map(|(path, value)| {
                let json = match value {
                    SizeValue::Number(n) if *n == FULL && path.starts_with("radius.") => "full".into(),
                    SizeValue::Number(n) => super::write::number(*n),
                };
                (path.clone(), json)
            })
            .collect();
        top.insert(name.to_owned(), Value::Object(section));
    }
    let mut out = String::new();
    super::write::emit(&Value::Object(top), 0, Some(&SECTIONS), &mut out);
    out.push('\n');
    out
}
