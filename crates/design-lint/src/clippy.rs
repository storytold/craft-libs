//! The clippy side: check that an app's `clippy.toml` carries the template
//! entries and merge them in.
//!
//! Entries are matched by `path`. The template owns the entries it lists; any
//! other entry in the app's arrays is kept after them, comments included.

use std::fmt;
use std::path::{Path, PathBuf};

use toml_edit::{Array, DocumentMut, Item, Value};

use crate::{CLIPPY_TEMPLATE, Error};

const KEYS: [&str; 2] = ["disallowed-methods", "disallowed-types"];
const BEGIN: &str = "BEGIN craft-design-system";
const END: &str = "END craft-design-system";
const TEMPLATE_ORIGIN: &str = "clippy.template.toml";

/// How an app's entry differs from the template's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriftKind {
    /// The app has no entry with the template's path.
    Missing,
    /// The app's entry has a different `reason` or `allow-invalid`.
    Differs,
}

/// One template entry the app's `clippy.toml` lacks or has changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClippyDrift {
    /// `disallowed-methods` or `disallowed-types`.
    pub key: &'static str,
    /// The entry's `path`.
    pub path: String,
    /// Whether the entry is missing or differs.
    pub kind: DriftKind,
}

impl fmt::Display for ClippyDrift {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (id, what) = match self.kind {
            DriftKind::Missing => ("clippy/missing", "is missing"),
            DriftKind::Differs => ("clippy/differs", "differs from the template"),
        };
        write!(f, "clippy.toml: error[{id}]: {} entry `{}` {what}", self.key, self.path)
    }
}

/// A configuration that makes clippy read some other `clippy.toml` (or none) instead of the app's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClippyShadow {
    /// The `clippy.toml` that is read instead, or the `CLIPPY_CONF_DIR` directory.
    pub path: PathBuf,
    /// Why that file or directory wins over the app's `clippy.toml`.
    pub reason: String,
}

impl fmt::Display for ClippyShadow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: error[clippy/shadowed]: {}", self.path.display(), self.reason)
    }
}

/// Fails with [`Error::ClippyDrift`] unless the `clippy.toml` at `clippy_toml`
/// (or in that directory) carries every template entry, and with
/// [`Error::ClippyShadowed`] when another `clippy.toml` in the workspace or
/// `CLIPPY_CONF_DIR` would make clippy skip it. A missing file counts as empty.
pub fn check_clippy(clippy_toml: impl AsRef<Path>) -> Result<(), Error> {
    let path = resolve(clippy_toml.as_ref());
    let drift = clippy_drift(&path, &read_or_empty(&path)?)?;
    if !drift.is_empty() {
        return Err(Error::ClippyDrift(drift));
    }
    let shadows = clippy_shadows(&path, std::env::var_os("CLIPPY_CONF_DIR").as_deref())?;
    if shadows.is_empty() { Ok(()) } else { Err(Error::ClippyShadowed(shadows)) }
}

const CONFIG_NAMES: [&str; 2] = ["clippy.toml", ".clippy.toml"];

fn has_config(dir: &Path) -> bool {
    CONFIG_NAMES.iter().any(|n| dir.join(n).is_file())
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

/// What `Cargo.toml` in `dir` declares: a package, a workspace, or neither.
fn manifest_kind(dir: &Path) -> (bool, bool) {
    let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) else { return (false, false) };
    let Ok(table) = text.parse::<toml::Table>() else { return (false, false) };
    (table.contains_key("package"), table.contains_key("workspace"))
}

/// Clippy reads the first `clippy.toml` or `.clippy.toml` it finds walking up from `CLIPPY_CONF_DIR`,
/// or from each package's directory when that is unset. The configuration at `clippy_toml` is shadowed by
/// a config file in a package directory or one of its parents below it, by a `CLIPPY_CONF_DIR` whose
/// walk finds another file (or none), and nothing else. Nested workspaces and hidden or `target`
/// directories are not searched.
pub fn clippy_shadows(clippy_toml: &Path, conf_dir: Option<&std::ffi::OsStr>) -> Result<Vec<ClippyShadow>, Error> {
    let root = clippy_toml.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut shadows = Vec::new();
    if let Some(conf_dir) = conf_dir.filter(|d| !d.is_empty()) {
        let conf_dir = Path::new(conf_dir);
        let found = canonical(conf_dir).ancestors().find(|d| has_config(d)).map(Path::to_owned);
        if found.as_deref() != Some(canonical(root).as_path()) {
            let reason = match found {
                Some(found) => format!("CLIPPY_CONF_DIR makes clippy read the config in `{}` instead of the root clippy.toml", found.display()),
                None => "CLIPPY_CONF_DIR points where clippy finds no config, so the root clippy.toml is not read".to_owned(),
            };
            shadows.push(ClippyShadow { path: conf_dir.to_owned(), reason });
        }
    }
    let (mut config_dirs, mut package_dirs) = (Vec::new(), Vec::new());
    collect_dirs(root, root, &mut config_dirs, &mut package_dirs).map_err(|source| Error::Io { path: root.to_owned(), source })?;
    for dir in config_dirs {
        if package_dirs.iter().any(|p| p.starts_with(&dir)) {
            shadows.push(ClippyShadow {
                path: dir.clone(),
                reason: "a clippy.toml here is read instead of the root one by the packages at or below this directory".to_owned(),
            });
        }
    }
    Ok(shadows)
}

fn collect_dirs(root: &Path, dir: &Path, configs: &mut Vec<PathBuf>, packages: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if dir != root {
        let (package, workspace) = manifest_kind(dir);
        if workspace {
            return Ok(());
        }
        if has_config(dir) {
            configs.push(dir.to_owned());
        }
        if package {
            packages.push(dir.to_owned());
        }
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?.map(|e| e.map(|e| e.path())).collect::<Result<_, _>>()?;
    entries.sort();
    for entry in entries {
        let name = entry.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if entry.is_dir() && !name.starts_with('.') && name != "target" {
            collect_dirs(root, &entry, configs, packages)?;
        }
    }
    Ok(())
}

/// Merges the template into the `clippy.toml` at `clippy_toml` (or in that
/// directory), creating it when absent. Returns whether the file changed.
pub fn write_clippy(clippy_toml: impl AsRef<Path>) -> Result<bool, Error> {
    let path = resolve(clippy_toml.as_ref());
    let old = read_or_empty(&path)?;
    let new = merge_clippy(&path, &old)?;
    if new == old {
        return Ok(false);
    }
    std::fs::write(&path, new).map_err(|source| Error::Io { path, source })?;
    Ok(true)
}

/// The template entries that `text` (the content of the `clippy.toml` at `origin`) lacks or has changed.
pub fn clippy_drift(origin: &Path, text: &str) -> Result<Vec<ClippyDrift>, Error> {
    let doc = parse(origin, text)?;
    let template = template_doc()?;
    let mut drift = Vec::new();
    for key in KEYS {
        let have = doc.get(key).and_then(Item::as_array);
        for entry in template_entries(&template, key)? {
            let path = entry_path(entry).unwrap_or_default();
            let found = have.and_then(|a| a.iter().find(|v| entry_path(v) == Some(path)));
            let kind = match found {
                None => DriftKind::Missing,
                Some(v) if same_entry(v, entry) => continue,
                Some(_) => DriftKind::Differs,
            };
            drift.push(ClippyDrift { key, path: path.to_owned(), kind });
        }
    }
    Ok(drift)
}

/// `text` (the content of the `clippy.toml` at `origin`) with the template
/// merged in: per array, the template entries first, then the app's other
/// entries. Everything else in the file is untouched. Merging twice gives the
/// same text.
pub fn merge_clippy(origin: &Path, text: &str) -> Result<String, Error> {
    let mut doc = parse(origin, text)?;
    let template = template_doc()?;
    for key in KEYS {
        let wanted = template_entries(&template, key)?;
        let mut merged = Array::new();
        for entry in &wanted {
            merged.push_formatted((*entry).clone());
        }
        if let Some(item) = doc.get(key) {
            let have = item.as_array().ok_or_else(|| toml_error(origin, format!("`{key}` is not an array")))?;
            for value in have {
                if !wanted.iter().any(|w| entry_path(w) == entry_path(value) && entry_path(value).is_some()) {
                    let mut extra = value.clone();
                    if !extra.decor().prefix().and_then(|p| p.as_str()).is_some_and(|p| p.contains('\n')) {
                        extra.decor_mut().set_prefix("\n    ");
                    }
                    merged.push_formatted(extra);
                }
            }
        }
        merged.set_trailing("\n");
        merged.set_trailing_comma(true);
        doc[key] = Item::Value(Value::Array(merged));
    }
    add_markers(&mut doc, &template);
    Ok(doc.to_string())
}

/// Puts the template's header comment before `disallowed-methods` and its
/// closing comment at the end, unless the file has them.
fn add_markers(doc: &mut DocumentMut, template: &DocumentMut) {
    let text = doc.to_string();
    if !text.contains(BEGIN) {
        let begin = template.key("disallowed-methods").and_then(|k| k.leaf_decor().prefix()).and_then(|p| p.as_str()).unwrap_or_default().to_owned();
        if let Some(mut key) = doc.key_mut("disallowed-methods") {
            let old = key.leaf_decor().prefix().and_then(|p| p.as_str()).unwrap_or_default().to_owned();
            key.leaf_decor_mut().set_prefix(format!("{old}{begin}"));
        }
    }
    if !text.contains(END) {
        let end = template.trailing().as_str().unwrap_or_default();
        let old = doc.trailing().as_str().unwrap_or_default().to_owned();
        doc.set_trailing(format!("{old}{end}"));
    }
}

fn resolve(path: &Path) -> PathBuf {
    if path.is_dir() { path.join("clippy.toml") } else { path.to_owned() }
}

fn read_or_empty(path: &Path) -> Result<String, Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(source) => Err(Error::Io { path: path.to_owned(), source }),
    }
}

fn parse(origin: &Path, text: &str) -> Result<DocumentMut, Error> {
    text.parse().map_err(|e: toml_edit::TomlError| toml_error(origin, e.to_string()))
}

fn toml_error(origin: &Path, message: String) -> Error {
    Error::Toml { path: origin.to_owned(), message }
}

fn template_doc() -> Result<DocumentMut, Error> {
    parse(Path::new(TEMPLATE_ORIGIN), CLIPPY_TEMPLATE)
}

fn template_entries<'a>(template: &'a DocumentMut, key: &str) -> Result<Vec<&'a Value>, Error> {
    let array = template.get(key).and_then(Item::as_array).ok_or_else(|| toml_error(Path::new(TEMPLATE_ORIGIN), format!("no `{key}` array")))?;
    Ok(array.iter().collect())
}

/// The `path` of an entry, written as a table or as a bare string.
fn entry_path(entry: &Value) -> Option<&str> {
    match entry {
        Value::String(s) => Some(s.value()),
        Value::InlineTable(t) => t.get("path")?.as_str(),
        _ => None,
    }
}

fn same_entry(have: &Value, want: &Value) -> bool {
    let field = |v: &Value, name: &str| match v {
        Value::InlineTable(t) => t.get(name).map(|x| x.to_string().trim().to_owned()),
        _ => None,
    };
    ["reason", "allow-invalid"].iter().all(|name| field(have, name) == field(want, name))
}
