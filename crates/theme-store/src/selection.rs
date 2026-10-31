//! `selection.json`: the suite's selection and the revisions apps reconcile against.

use std::fmt;

use craft_theme::{AppDefaults, Mode, Selection, ThemeId};
use serde_json::{Map, Value};

/// The `selection.json` format this crate reads and writes.
pub const SELECTION_SCHEMA_VERSION: u64 = 1;
/// Larger selection files are refused without being parsed.
pub const MAX_SELECTION_BYTES: usize = 64 * 1024;

/// The suite's choice as `selection.json` holds it. Each field is set once a user picks it in
/// some app; until then every app shows its own default for that field.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct SuiteChoice {
    pub theme: Option<ThemeId>,
    pub mode: Option<Mode>,
}

impl SuiteChoice {
    /// Nothing chosen yet.
    pub fn is_empty(&self) -> bool {
        self.theme.is_none() && self.mode.is_none()
    }

    /// What an app that follows the suite shows: the chosen fields, and the app's defaults for
    /// the rest. With nothing chosen this is `defaults.selection()`, so
    /// `choice.effective(Some(&suite.with_defaults(&defaults)), &defaults)` is the same as
    /// passing `None` for an empty choice.
    pub fn with_defaults(&self, defaults: &AppDefaults) -> Selection {
        Selection { theme: self.theme.clone().unwrap_or_else(|| defaults.theme.clone()), mode: self.mode.unwrap_or(defaults.mode) }
    }
}

impl From<Selection> for SuiteChoice {
    fn from(s: Selection) -> Self {
        SuiteChoice { theme: Some(s.theme), mode: Some(s.mode) }
    }
}

/// `selection.json` as last read or written by this store. With feature `serde`, apps save it in
/// their preferences (`last_suite`) and pass it back with
/// [`StoreOptions::last_accepted`](crate::StoreOptions::last_accepted).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(default))]
pub struct Accepted {
    /// Empty until some app writes a choice: no file, no `suite` key, or neither field in it.
    pub suite: SuiteChoice,
    /// Goes up by one with every write of `selection.json`.
    pub revision: u64,
    /// Goes up by one with every library change (import, delete).
    pub library_revision: u64,
}

impl Accepted {
    /// A state known from elsewhere, such as the copy an app keeps in its preferences.
    pub fn from_selection(suite: Selection) -> Self {
        Accepted { suite: suite.into(), revision: 0, library_revision: 0 }
    }
}

/// The fields of the suite choice the user changed. Only these are written; the fields it leaves
/// out keep what `selection.json` holds, chosen or not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectionChange {
    pub theme: Option<ThemeId>,
    pub mode: Option<Mode>,
}

impl SelectionChange {
    pub fn theme(theme: ThemeId) -> Self {
        SelectionChange { theme: Some(theme), mode: None }
    }

    pub fn mode(mode: Mode) -> Self {
        SelectionChange { theme: None, mode: Some(mode) }
    }

    pub fn with_theme(mut self, theme: ThemeId) -> Self {
        self.theme = Some(theme);
        self
    }

    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Both fields, for a control that sets theme and mode together. After a conflict, "Apply
    /// mine" resends the original change (returned in the conflict) with the disk state as base.
    pub fn whole(selection: Selection) -> Self {
        SelectionChange { theme: Some(selection.theme), mode: Some(selection.mode) }
    }

    pub fn is_empty(&self) -> bool {
        self.theme.is_none() && self.mode.is_none()
    }

    /// `choice` with this change applied: what the app shows until the write returns.
    pub fn applied_to(&self, choice: &SuiteChoice) -> SuiteChoice {
        SuiteChoice { theme: self.theme.clone().or_else(|| choice.theme.clone()), mode: self.mode.or(choice.mode) }
    }
}

/// The choice to write, from `base` (the state the caller's change was made on), what is on
/// disk now and the change. `None` is a conflict.
///
/// A field the change leaves alone keeps disk's value, chosen or not, and never conflicts. A
/// changed field conflicts only when disk holds a chosen value other than mine and base's:
/// another app chose that field after the caller's read. A field unchosen on disk takes mine.
pub(crate) fn merge(base: &SuiteChoice, disk: &SuiteChoice, change: &SelectionChange) -> Option<SuiteChoice> {
    Some(SuiteChoice {
        theme: merge_field(base.theme.as_ref(), disk.theme.as_ref(), change.theme.as_ref())?,
        mode: merge_field(base.mode.as_ref(), disk.mode.as_ref(), change.mode.as_ref())?,
    })
}

fn merge_field<T: PartialEq + Clone>(base: Option<&T>, disk: Option<&T>, mine: Option<&T>) -> Option<Option<T>> {
    match mine {
        Some(mine) if disk.is_some() && disk != Some(mine) && disk != base => None,
        Some(mine) => Some(Some(mine.clone())),
        None => Some(disk.cloned()),
    }
}

/// The counters of a damaged `selection.json` that still parses as JSON, so the write that
/// replaces it never moves them backwards; `base` stands in for everything else.
pub(crate) fn salvage(bytes: Option<&[u8]>, base: &Accepted) -> Accepted {
    let value = bytes.and_then(|b| serde_json::from_slice::<Value>(b).ok());
    let counter = |key: &str| value.as_ref().and_then(|v| v.get(key)).and_then(Value::as_u64).unwrap_or(0);
    Accepted {
        suite: base.suite.clone(),
        revision: counter("revision").max(base.revision),
        library_revision: counter("library_revision").max(base.library_revision),
    }
}

/// Why `selection.json` couldn't be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionError {
    /// Not JSON, not an object, or a known key with a value of the wrong type.
    Corrupt(String),
    /// Written by a newer build; never overwritten.
    TooNew {
        schema_version: u64,
    },
    TooLarge {
        len: usize,
    },
}

/// A parsed `selection.json`: the known fields and the whole object, so that keys this build
/// doesn't know are written back.
#[derive(Clone, Debug, Default)]
pub(crate) struct SelectionDoc {
    pub raw: Map<String, Value>,
    pub accepted: Accepted,
}

pub(crate) fn parse_selection(bytes: &[u8]) -> Result<SelectionDoc, SelectionError> {
    if bytes.len() > MAX_SELECTION_BYTES {
        return Err(SelectionError::TooLarge { len: bytes.len() });
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let value: Value = serde_json::from_slice(bytes).map_err(|e| SelectionError::Corrupt(e.to_string()))?;
    let Value::Object(raw) = value else {
        return Err(corrupt("not a JSON object"));
    };
    match raw.get("schema_version").and_then(Value::as_u64) {
        Some(SELECTION_SCHEMA_VERSION) => {}
        Some(v) if v > SELECTION_SCHEMA_VERSION => return Err(SelectionError::TooNew { schema_version: v }),
        _ => return Err(corrupt("\"schema_version\" is missing or not 1")),
    }
    let revision = counter(&raw, "revision")?;
    let library_revision = counter(&raw, "library_revision")?;
    let suite = match raw.get("suite") {
        None | Some(Value::Null) => SuiteChoice::default(),
        Some(Value::Object(suite)) => parse_suite(suite)?,
        Some(_) => return Err(corrupt("\"suite\" is not an object")),
    };
    Ok(SelectionDoc { accepted: Accepted { suite, revision, library_revision }, raw })
}

fn counter(raw: &Map<String, Value>, key: &str) -> Result<u64, SelectionError> {
    match raw.get(key) {
        None => Ok(0),
        Some(v) => v.as_u64().ok_or_else(|| corrupt(&format!("\"{key}\" is not a whole number"))),
    }
}

fn parse_suite(suite: &Map<String, Value>) -> Result<SuiteChoice, SelectionError> {
    let theme = match suite.get("theme") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(ThemeId::parse(s).map_err(|e| corrupt(&format!("\"suite.theme\": {e}")))?),
        Some(_) => return Err(corrupt("\"suite.theme\" is not a string")),
    };
    let mode = match suite.get("mode") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().and_then(mode_from_key).ok_or_else(|| corrupt("\"suite.mode\" is not system, light or dark"))?),
    };
    Ok(SuiteChoice { theme, mode })
}

fn corrupt(why: &str) -> SelectionError {
    SelectionError::Corrupt(why.to_owned())
}

pub(crate) fn mode_from_key(s: &str) -> Option<Mode> {
    match s {
        "system" => Some(Mode::System),
        "light" => Some(Mode::Light),
        "dark" => Some(Mode::Dark),
        _ => None,
    }
}

pub(crate) fn mode_key(mode: Mode) -> &'static str {
    match mode {
        Mode::System => "system",
        Mode::Light => "light",
        Mode::Dark => "dark",
    }
}

/// `raw` (the object last read from disk, or empty) with the known fields set to `next`. Fields
/// nobody chose are left out; with none chosen, `suite` stays as it was.
pub(crate) fn render_selection(raw: &mut Map<String, Value>, next: &Accepted, written_by: &str) -> Vec<u8> {
    raw.insert("schema_version".into(), SELECTION_SCHEMA_VERSION.into());
    raw.insert("revision".into(), next.revision.into());
    raw.insert("library_revision".into(), next.library_revision.into());
    if !next.suite.is_empty() {
        let mut suite = match raw.remove("suite") {
            Some(Value::Object(suite)) => suite,
            _ => Map::new(),
        };
        set_or_remove(&mut suite, "theme", next.suite.theme.as_ref().map(|t| t.as_str().into()));
        set_or_remove(&mut suite, "mode", next.suite.mode.map(|m| mode_key(m).into()));
        raw.insert("suite".into(), Value::Object(suite));
    }
    raw.insert("written_by".into(), written_by.into());
    let mut out = serde_json::to_vec_pretty(&*raw).unwrap_or_default();
    out.push(b'\n');
    out
}

fn set_or_remove(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    match value {
        Some(v) => map.insert(key.into(), v),
        None => map.remove(key),
    };
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectionError::Corrupt(why) => write!(f, "the suite theme selection file is damaged: {why}"),
            SelectionError::TooNew { schema_version } => write!(
                f,
                "the suite theme selection was saved by a newer Craft app (format {schema_version}); this app won't change it until it is updated"
            ),
            SelectionError::TooLarge { len } => {
                write!(f, "the suite theme selection file is {} KiB; at most {} KiB is read", len.div_ceil(1024), MAX_SELECTION_BYTES / 1024)
            }
        }
    }
}

impl std::error::Error for SelectionError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(theme: &str, mode: Mode) -> Selection {
        Selection { theme: ThemeId::parse(theme).unwrap(), mode }
    }

    fn choice(theme: Option<&str>, mode: Option<Mode>) -> SuiteChoice {
        SuiteChoice { theme: theme.map(|t| ThemeId::parse(t).unwrap()), mode }
    }

    #[test]
    fn merge_table() {
        let graphite = || SelectionChange::theme(ThemeId::parse("craft.graphite").unwrap());
        let base = choice(Some("craft.artcraft"), Some(Mode::System));
        let dark_disk = choice(Some("craft.artcraft"), Some(Mode::Dark));
        // Only mine changed / another app changed the other field.
        assert_eq!(merge(&base, &base, &graphite()), Some(choice(Some("craft.graphite"), Some(Mode::System))));
        assert_eq!(merge(&base, &dark_disk, &graphite()), Some(choice(Some("craft.graphite"), Some(Mode::Dark))));
        // Same field: equal value is fine; a different value written elsewhere conflicts.
        assert_eq!(merge(&base, &dark_disk, &SelectionChange::mode(Mode::Dark)), Some(dark_disk.clone()));
        assert_eq!(merge(&base, &dark_disk, &SelectionChange::mode(Mode::Light)), None);
        // "Apply mine" resends the change on the disk state as base.
        assert_eq!(merge(&dark_disk, &dark_disk, &SelectionChange::mode(Mode::Light)), Some(choice(Some("craft.artcraft"), Some(Mode::Light))));
        // Nothing chosen: a change writes only its own field.
        let none = SuiteChoice::default();
        assert_eq!(merge(&none, &none, &graphite()), Some(choice(Some("craft.graphite"), None)));
        // Two apps on a fresh install, one choosing the theme, the other the mode: no conflict.
        let theme_only = choice(Some("craft.graphite"), None);
        assert_eq!(merge(&none, &theme_only, &SelectionChange::mode(Mode::Light)), Some(choice(Some("craft.graphite"), Some(Mode::Light))));
        // Another app chose the same field first: equal is fine, different conflicts.
        assert_eq!(merge(&none, &theme_only, &graphite()), Some(theme_only.clone()));
        assert_eq!(merge(&none, &theme_only, &SelectionChange::theme(ThemeId::parse("craft.artcraft").unwrap())), None);
        // A file that lost a field: the change writes into it.
        assert_eq!(merge(&base, &none, &SelectionChange::mode(Mode::Dark)), Some(choice(None, Some(Mode::Dark))));
    }

    #[test]
    fn damaged_counters_are_salvaged() {
        let base = Accepted { suite: choice(Some("craft.graphite"), None), revision: 3, library_revision: 9 };
        let bytes = br#"{"schema_version":1,"revision":41,"library_revision":2,"suite":{"theme":5}}"#;
        assert_eq!(salvage(Some(bytes), &base), Accepted { suite: base.suite.clone(), revision: 41, library_revision: 9 });
        assert_eq!(salvage(Some(b"{ not json"), &base), base);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn accepted_round_trips_through_serde() {
        let accepted = Accepted { suite: choice(None, Some(Mode::Dark)), revision: 4, library_revision: 1 };
        let json = serde_json::to_string(&accepted).unwrap();
        assert_eq!(serde_json::from_str::<Accepted>(&json).unwrap(), accepted);
        assert_eq!(serde_json::from_str::<Accepted>("{}").unwrap(), Accepted::default());
    }

    #[test]
    fn unknown_keys_survive_a_rewrite() {
        let text = br#"{"schema_version":1,"revision":4,"library_revision":2,"extra":{"a":[1]},
            "suite":{"theme":"user.navy","mode":"light","later":true},"written_by":"x"}"#;
        let doc = parse_selection(text).unwrap();
        assert_eq!(doc.accepted.revision, 4);
        assert_eq!(doc.accepted.suite, sel("user.navy", Mode::Light).into());
        let next = Accepted { suite: sel("craft.graphite", Mode::Dark).into(), revision: 5, library_revision: 2 };
        let mut raw = doc.raw;
        let out = render_selection(&mut raw, &next, "test 1.0");
        let back: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(back["extra"], serde_json::json!({"a": [1]}));
        assert_eq!(back["suite"]["later"], true);
        assert_eq!(back["written_by"], "test 1.0");
        assert_eq!(parse_selection(&out).unwrap().accepted, next);
    }

    #[test]
    fn no_suite_key_means_no_selection_and_bad_fields_are_corrupt() {
        let bare = parse_selection(br#"{"schema_version":1,"library_revision":3}"#).unwrap();
        assert_eq!(bare.accepted, Accepted { suite: SuiteChoice::default(), revision: 0, library_revision: 3 });
        let mut raw = bare.raw;
        let out = render_selection(&mut raw, &Accepted { suite: SuiteChoice::default(), revision: 1, library_revision: 4 }, "t");
        assert!(serde_json::from_slice::<Value>(&out).unwrap().get("suite").is_none());
        // One field chosen: the other is left out.
        let mode_only = parse_selection(br#"{"schema_version":1,"suite":{"mode":"dark"}}"#).unwrap();
        assert_eq!(mode_only.accepted.suite, choice(None, Some(Mode::Dark)));
        let mut raw = mode_only.raw;
        let out = render_selection(&mut raw, &Accepted { suite: choice(Some("craft.graphite"), None), revision: 1, library_revision: 0 }, "t");
        let written: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(written["suite"], serde_json::json!({"theme": "craft.graphite"}));
        for bad in [
            &br#"{"revision":1}"#[..],
            b"[1]",
            b"{",
            br#"{"schema_version":1,"suite":{"theme":"craft.graphite","mode":"auto"}}"#,
            br#"{"schema_version":1,"suite":{"theme":"A","mode":"dark"}}"#,
            br#"{"schema_version":1,"suite":{"theme":7}}"#,
        ] {
            assert!(matches!(parse_selection(bad), Err(SelectionError::Corrupt(_))), "{}", String::from_utf8_lossy(bad));
        }
        assert_eq!(parse_selection(br#"{"schema_version":2,"suite":7}"#).unwrap_err(), SelectionError::TooNew { schema_version: 2 });
    }
}
