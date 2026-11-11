//! Size set files: schema `craft-design-sizes`, version 1. Read, checked and written with the
//! conventions of theme files: unknown top-level keys are kept, every error is reported with
//! its path. Unlike a theme, `base` takes only the paths in the table in
//! [`sizes`](crate::sizes).

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use super::write::{emit, number, set_path};
use super::{ErrorKind, FormatError, FormatErrors, LENGTH, MAX_THEME_BYTES};
use crate::ThemeId;
use crate::sizes::{FULL, Leaf, Preset, SizeSet, Slot, leaves};
use crate::tokens::{Edge, FontSlot, RowStyle};

/// The `schema` value of a size set file.
pub const SIZES_SCHEMA: &str = "craft-design-sizes";
/// The size set file format this crate reads and writes.
pub const SIZES_SCHEMA_VERSION: u64 = 1;

const FONT: &str = "\"ui\", \"ui_semibold\" or \"mono\"";
const FONTS: [(&str, FontSlot); 3] = [("ui", FontSlot::Ui), ("ui_semibold", FontSlot::UiSemibold), ("mono", FontSlot::Mono)];
const TOP_ORDER: [&str; 5] = ["schema", "schema_version", "id", "name", "base"];

/// A parsed size set file: the set and the JSON it came from, so that [`write_sizes`] keeps
/// unknown keys.
#[derive(Clone, Debug, PartialEq)]
pub struct SizeSetFile {
    /// The set. [`write_sizes`] refuses one that [`SizeSet::validate`] rejects.
    pub set: SizeSet,
    raw: Map<String, Value>,
}

/// A size set file for `set`, as if parsed from the file [`write_sizes`] produces.
pub fn sizes_to_file(set: &SizeSet) -> Result<SizeSetFile, FormatErrors> {
    set.validate()?;
    Ok(SizeSetFile { set: set.clone(), raw: Map::new() })
}

fn err(path: &[String], kind: ErrorKind) -> FormatError {
    FormatError { path: path.to_vec(), kind }
}

fn path_of(path: &str) -> Vec<String> {
    path.split('.').map(str::to_owned).collect()
}

/// Reads and validates a size set file. Every field error is reported, not only the first.
pub fn parse_sizes(bytes: &[u8]) -> Result<SizeSetFile, FormatErrors> {
    let stop = |path: &[&str], kind| Err(FormatErrors(vec![FormatError { path: path.iter().map(|s| (*s).to_owned()).collect(), kind }]));
    if bytes.len() > MAX_THEME_BYTES {
        return stop(&[], ErrorKind::TooLarge { len: bytes.len() });
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => return stop(&[], ErrorKind::Json(e.to_string())),
    };
    let Value::Object(raw) = value else { return stop(&[], ErrorKind::NotObject) };
    match raw.get("schema") {
        Some(Value::String(s)) if s == SIZES_SCHEMA => {}
        Some(Value::String(s)) => return stop(&["schema"], ErrorKind::OtherSchema { schema: s.clone() }),
        Some(_) => return stop(&["schema"], ErrorKind::WrongType { expected: "a string" }),
        None => return stop(&["schema"], ErrorKind::Missing),
    }
    match raw.get("schema_version").and_then(Value::as_u64) {
        Some(SIZES_SCHEMA_VERSION) => {}
        Some(v) if v > SIZES_SCHEMA_VERSION => return stop(&["schema_version"], ErrorKind::TooNew { schema_version: v }),
        _ => return stop(&["schema_version"], ErrorKind::BadSchemaVersion),
    }

    let mut errors = Vec::new();
    let id = match raw.get("id") {
        None => {
            errors.push(err(&path_of("id"), ErrorKind::Missing));
            None
        }
        Some(Value::String(s)) => ThemeId::parse(s).map(|_| s.clone()).map_err(|e| errors.push(err(&path_of("id"), ErrorKind::BadId(e)))).ok(),
        Some(_) => {
            errors.push(err(&path_of("id"), ErrorKind::WrongType { expected: "a string" }));
            None
        }
    };
    let name = match raw.get("name") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => {
            errors.push(err(&path_of("name"), ErrorKind::WrongType { expected: "a string" }));
            None
        }
    };

    let mut base = Preset::default();
    {
        let mut table = leaves(&mut base);
        let index: HashMap<&'static str, usize> = table.iter().enumerate().map(|(i, l)| (l.path, i)).collect();
        let mut seen = HashSet::new();
        let section = "base";
        match raw.get(section) {
            Some(Value::Object(o)) => walk(o, &mut vec![section.to_owned()], &mut table, &index, &mut seen, &mut errors),
            None | Some(Value::Null) => errors.push(err(&path_of(section), ErrorKind::Missing)),
            Some(_) => errors.push(err(&path_of(section), ErrorKind::WrongType { expected: "an object" })),
        }
        for leaf in &table {
            let section_ok = matches!(raw.get(leaf.path.split('.').next().unwrap_or_default()), Some(Value::Object(_)));
            if section_ok && !seen.contains(leaf.path) {
                errors.push(err(&path_of(leaf.path), ErrorKind::Missing));
            }
        }
    }
    let Some(id) = id else { return Err(FormatErrors(errors)) };
    if !errors.is_empty() {
        return Err(FormatErrors(errors));
    }
    let set = SizeSet { name: name.unwrap_or_else(|| id.clone()), id, base };
    set.validate()?;
    Ok(SizeSetFile { set, raw })
}

/// Reads the values under `obj` into `table`. A key that leads to no table path is unknown.
fn walk(
    obj: &Map<String, Value>,
    path: &mut Vec<String>,
    table: &mut [Leaf<'_>],
    index: &HashMap<&'static str, usize>,
    seen: &mut HashSet<&'static str>,
    errors: &mut Vec<FormatError>,
) {
    for (key, value) in obj {
        path.push(key.clone());
        let dotted = path.join(".");
        if let Some(&i) = index.get(dotted.as_str()) {
            let leaf = &mut table[i];
            if assign(&mut leaf.slot, value, path, errors) {
                seen.insert(leaf.path);
            }
        } else if index.keys().any(|p| p.strip_prefix(dotted.as_str()).is_some_and(|rest| rest.starts_with('.'))) {
            match value {
                Value::Object(o) => walk(o, path, table, index, seen, errors),
                _ => errors.push(err(path, ErrorKind::WrongType { expected: "an object" })),
            }
        } else {
            errors.push(err(path, ErrorKind::UnknownToken));
        }
        path.pop();
    }
}

fn number_of(v: &Value) -> Option<f32> {
    v.as_f64().map(|n| n as f32)
}

/// Stores `value` in `slot`; false (with an error) when it has the wrong kind.
fn assign(slot: &mut Slot<'_>, value: &Value, path: &[String], errors: &mut Vec<FormatError>) -> bool {
    let mut fail = |kind| {
        errors.push(err(path, kind));
        false
    };
    match slot {
        Slot::Num(x) => match number_of(value) {
            Some(n) => {
                **x = n;
                true
            }
            None => fail(ErrorKind::WrongType { expected: "a number" }),
        },
        Slot::Length(x) => match (value, number_of(value)) {
            (Value::String(s), _) if s == "full" => {
                **x = FULL;
                true
            }
            (_, Some(n)) => {
                **x = n;
                true
            }
            _ => fail(ErrorKind::BadValue { expected: LENGTH }),
        },
        Slot::Font(f) => match value {
            Value::String(s) => match FONTS.iter().find(|(name, _)| name == s) {
                Some(&(_, font)) => {
                    **f = font;
                    true
                }
                None => fail(ErrorKind::BadValue { expected: FONT }),
            },
            _ => fail(ErrorKind::WrongType { expected: "a string" }),
        },
        Slot::Edge(e) => match value {
            Value::String(s) if s == "flat" => {
                **e = Edge::Flat;
                true
            }
            _ => fail(ErrorKind::BadValue { expected: EDGE_FLAT }),
        },
        Slot::Rows(r) => match value {
            Value::String(s) if s == "divided" => {
                **r = RowStyle::Divided;
                true
            }
            Value::String(s) if s == "rounded" => {
                **r = RowStyle::Rounded;
                true
            }
            _ => fail(ErrorKind::BadValue { expected: ROWS }),
        },
    }
}

const ROWS: &str = "\"divided\" or \"rounded\"";

const EDGE_FLAT: &str = "\"flat\"";

impl SizeSet {
    /// Every value that [`parse_sizes`] would reject in the file [`write_sizes`] makes of this
    /// set, with its path there: a value outside its range and a broken cross rule.
    pub fn validate(&self) -> Result<(), FormatErrors> {
        let mut errors = Vec::new();
        if let Err(e) = ThemeId::parse(&self.id) {
            errors.push(err(&path_of("id"), ErrorKind::BadId(e)));
        }
        let mut base = self.base;
        for leaf in leaves(&mut base) {
            let in_range = |v: f32| (leaf.lo..=leaf.hi).contains(&v);
            let ok = match &leaf.slot {
                Slot::Num(v) => in_range(**v) && v.is_finite(),
                Slot::Length(v) => **v == FULL || in_range(**v),
                Slot::Font(_) | Slot::Edge(_) | Slot::Rows(_) => true,
            };
            if !ok {
                errors.push(err(&path_of(leaf.path), ErrorKind::OutOfRange { min: leaf.lo, max: leaf.hi }));
            }
        }
        if errors.is_empty() {
            self.cross_rules(&mut errors);
        }
        if errors.is_empty() { Ok(()) } else { Err(FormatErrors(errors)) }
    }

    /// Relations between values that each lie in range. The error is at the value that is too
    /// large (or too small, for a line height).
    pub(super) fn cross_rules(&self, errors: &mut Vec<FormatError>) {
        let s = &self.base.size;
        let mut le = |path: &str, v: f32, other: &'static str, limit: f32| {
            if v > limit {
                errors.push(err(&path_of(path), ErrorKind::CrossRule { rule: other }));
            }
        };
        le("base.size.control.sm", s.control.sm, "at most base.size.control.md", s.control.md);
        le("base.size.icon.sm", s.icon.sm, "at most base.size.icon.md", s.icon.md);
        le("base.size.icon.md", s.icon.md, "at most base.size.icon.lg", s.icon.lg);
        le("base.size.icon.glyph_sm", s.icon.glyph_sm, "at most base.size.icon.sm", s.icon.sm);
        le("base.size.icon.glyph_md", s.icon.glyph_md, "at most base.size.icon.md", s.icon.md);
        le("base.stroke.thin", self.base.stroke.thin, "at most base.stroke.medium", self.base.stroke.medium);
        le("base.stroke.medium", self.base.stroke.medium, "at most base.stroke.thick", self.base.stroke.thick);
        le("base.stroke.thick", self.base.stroke.thick, "at most base.stroke.heavy", self.base.stroke.heavy);
        le("base.size.thumb.sm", s.thumb.sm, "at most base.size.thumb.md", s.thumb.md);
        le("base.size.thumb.md", s.thumb.md, "at most base.size.thumb.lg", s.thumb.lg);
        let t = &self.base.typography;
        for (name, style) in [
            ("caption", t.caption),
            ("label", t.label),
            ("body", t.body),
            ("body_strong", t.body_strong),
            ("title", t.title),
            ("heading", t.heading),
            ("body_large", t.body_large),
            ("mono", t.mono),
        ] {
            if style.line_height < style.size {
                errors.push(err(&path_of(&format!("base.typography.{name}.line_height")), ErrorKind::CrossRule { rule: "at least the role's size" }));
            }
        }
    }
}

/// The file as pretty-printed JSON ending in a newline. Known values come from `file.set`;
/// unknown keys come from the file that was parsed. Fails with [`SizeSet::validate`]'s errors,
/// so the text always parses.
pub fn write_sizes(file: &SizeSetFile) -> Result<String, FormatErrors> {
    file.set.validate()?;
    let mut raw = file.raw.clone();
    raw.insert("schema".to_owned(), SIZES_SCHEMA.into());
    raw.insert("schema_version".to_owned(), SIZES_SCHEMA_VERSION.into());
    raw.insert("id".to_owned(), file.set.id.as_str().into());
    raw.insert("name".to_owned(), file.set.name.as_str().into());
    let mut base = file.set.base;
    for leaf in leaves(&mut base) {
        let value = match &leaf.slot {
            Slot::Num(v) => number(**v),
            Slot::Length(v) if **v == FULL => "full".into(),
            Slot::Length(v) => number(**v),
            Slot::Font(f) => FONTS.iter().find(|(_, s)| s == *f).map_or(Value::Null, |(name, _)| (*name).into()),
            Slot::Edge(_) => "flat".into(),
            Slot::Rows(r) => match **r {
                RowStyle::Divided => "divided".into(),
                RowStyle::Rounded => "rounded".into(),
            },
        };
        let path: Vec<&str> = leaf.path.split('.').collect();
        set_path(&mut raw, &path, value);
    }
    let mut out = String::new();
    emit(&Value::Object(raw), 0, Some(&TOP_ORDER), &mut out);
    out.push('\n');
    Ok(out)
}
