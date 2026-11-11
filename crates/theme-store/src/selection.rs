//! `selection.json`: the suite's selection and the revisions apps reconcile against.

use std::fmt;

use craft_theme::{ContrastChoice, ModeChoice, MotionChoice, Selection, Size, SuiteChoice, TextSize, ThemeDef, ThemeId, ThemePair, Variants};
use serde_json::{Map, Value};

/// The `selection.json` format this crate writes. Version 1 (one theme for both modes) is read
/// too and upgraded by the next write.
pub const SELECTION_SCHEMA_VERSION: u64 = 2;
/// Larger selection files are refused without being parsed.
pub const MAX_SELECTION_BYTES: usize = 64 * 1024;

/// `selection.json` as last read or written by this store. With feature `serde`, apps save it in
/// their preferences (`last_suite`) and pass it back with
/// [`StoreOptions::last_accepted`](crate::StoreOptions::last_accepted).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Accepted {
    /// Empty until some app writes a choice: no file, no `suite` key, or no field in it.
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
    /// The light-mode theme.
    pub theme_light: Option<ThemeId>,
    /// The dark-mode theme.
    pub theme_dark: Option<ThemeId>,
    /// Light, dark or follow the system.
    pub mode: Option<ModeChoice>,
    /// The contrast level.
    pub contrast: Option<ContrastChoice>,
    /// The motion preference.
    pub motion: Option<MotionChoice>,
    /// The interface density.
    pub size: Option<Size>,
    /// The text size step.
    pub text_size: Option<TextSize>,
}

impl SelectionChange {
    /// A change of the light-mode theme only.
    pub fn theme_light(theme: ThemeId) -> Self {
        SelectionChange { theme_light: Some(theme), ..SelectionChange::default() }
    }

    /// A change of the dark-mode theme only.
    pub fn theme_dark(theme: ThemeId) -> Self {
        SelectionChange { theme_dark: Some(theme), ..SelectionChange::default() }
    }

    /// Both theme slots.
    pub fn themes(pair: ThemePair) -> Self {
        SelectionChange { theme_light: Some(pair.light), theme_dark: Some(pair.dark), ..SelectionChange::default() }
    }

    /// The user picked `theme`: both slots if it has both variants, else its own mode's slot, as
    /// [`SuiteChoice::pick_theme`].
    pub fn pick_theme(theme: &ThemeDef) -> Self {
        let id = || Some(theme.id.clone());
        match theme.variants {
            Variants::Both => SelectionChange { theme_light: id(), theme_dark: id(), ..SelectionChange::default() },
            Variants::LightOnly => SelectionChange { theme_light: id(), ..SelectionChange::default() },
            Variants::DarkOnly => SelectionChange { theme_dark: id(), ..SelectionChange::default() },
        }
    }

    /// A change of the mode only.
    pub fn mode(mode: ModeChoice) -> Self {
        SelectionChange { mode: Some(mode), ..SelectionChange::default() }
    }

    /// A change of the contrast only.
    pub fn contrast(contrast: ContrastChoice) -> Self {
        SelectionChange { contrast: Some(contrast), ..SelectionChange::default() }
    }

    /// A change of the motion preference only.
    pub fn motion(motion: MotionChoice) -> Self {
        SelectionChange { motion: Some(motion), ..SelectionChange::default() }
    }

    /// A change of the size only.
    pub fn size(size: Size) -> Self {
        SelectionChange { size: Some(size), ..SelectionChange::default() }
    }

    /// A change of the text size only.
    pub fn text_size(text_size: TextSize) -> Self {
        SelectionChange { text_size: Some(text_size), ..SelectionChange::default() }
    }

    /// Every field, for a control that sets the whole selection. After a conflict, "Apply mine"
    /// resends the original change (returned in the conflict) with the disk state as base.
    pub fn whole(selection: Selection) -> Self {
        SelectionChange {
            theme_light: Some(selection.theme.light),
            theme_dark: Some(selection.theme.dark),
            mode: Some(selection.mode),
            contrast: Some(selection.contrast),
            motion: Some(selection.motion),
            size: Some(selection.size),
            text_size: Some(selection.text_size),
        }
    }

    /// This change followed by `later`: `later`'s fields replace this one's where set.
    pub fn then(self, later: SelectionChange) -> Self {
        SelectionChange {
            theme_light: later.theme_light.or(self.theme_light),
            theme_dark: later.theme_dark.or(self.theme_dark),
            mode: later.mode.or(self.mode),
            contrast: later.contrast.or(self.contrast),
            motion: later.motion.or(self.motion),
            size: later.size.or(self.size),
            text_size: later.text_size.or(self.text_size),
        }
    }

    /// Whether no field is set.
    pub fn is_empty(&self) -> bool {
        *self == SelectionChange::default()
    }

    /// `choice` with this change applied: what the app shows until the write returns.
    pub fn applied_to(&self, choice: &SuiteChoice) -> SuiteChoice {
        SuiteChoice {
            theme_light: self.theme_light.clone().or_else(|| choice.theme_light.clone()),
            theme_dark: self.theme_dark.clone().or_else(|| choice.theme_dark.clone()),
            mode: self.mode.or(choice.mode),
            contrast: self.contrast.or(choice.contrast),
            motion: self.motion.or(choice.motion),
            size: self.size.or(choice.size),
            text_size: self.text_size.or(choice.text_size),
        }
    }

    /// The themes this change picks, light slot first.
    pub(crate) fn picked(&self) -> impl Iterator<Item = &ThemeId> {
        self.theme_light.iter().chain(&self.theme_dark)
    }
}

/// The choice to write, from `base` (the state the caller's change was made on), what is on
/// disk now and the change. `None` is a conflict.
///
/// A field the change leaves alone keeps disk's value, chosen or not, and never conflicts. A
/// changed field conflicts only when disk holds a chosen value other than mine and base's:
/// another app chose that field after the caller's read. A field unchosen on disk takes mine.
/// Each theme slot is a field of its own.
pub(crate) fn merge(base: &SuiteChoice, disk: &SuiteChoice, change: &SelectionChange) -> Option<SuiteChoice> {
    Some(SuiteChoice {
        theme_light: merge_field(base.theme_light.as_ref(), disk.theme_light.as_ref(), change.theme_light.as_ref())?,
        theme_dark: merge_field(base.theme_dark.as_ref(), disk.theme_dark.as_ref(), change.theme_dark.as_ref())?,
        mode: merge_field(base.mode.as_ref(), disk.mode.as_ref(), change.mode.as_ref())?,
        contrast: merge_field(base.contrast.as_ref(), disk.contrast.as_ref(), change.contrast.as_ref())?,
        motion: merge_field(base.motion.as_ref(), disk.motion.as_ref(), change.motion.as_ref())?,
        size: merge_field(base.size.as_ref(), disk.size.as_ref(), change.size.as_ref())?,
        text_size: merge_field(base.text_size.as_ref(), disk.text_size.as_ref(), change.text_size.as_ref())?,
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
        /// The schema version the file declares.
        schema_version: u64,
    },
    /// The file is larger than the store reads.
    TooLarge {
        /// The file's length in bytes.
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
    let version = match raw.get("schema_version").and_then(Value::as_u64) {
        Some(v @ 1..=SELECTION_SCHEMA_VERSION) => v,
        Some(v) if v > SELECTION_SCHEMA_VERSION => return Err(SelectionError::TooNew { schema_version: v }),
        _ => return Err(corrupt("\"schema_version\" is missing or not 1 or 2")),
    };
    let revision = counter(&raw, "revision")?;
    let library_revision = counter(&raw, "library_revision")?;
    let suite = match raw.get("suite") {
        None | Some(Value::Null) => SuiteChoice::default(),
        Some(Value::Object(suite)) => parse_suite(suite, version)?,
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

/// Version 1 has one theme id for both modes; version 2 an object with a `light` and a `dark`
/// slot, each optional.
fn parse_suite(suite: &Map<String, Value>, version: u64) -> Result<SuiteChoice, SelectionError> {
    let (theme_light, theme_dark) = match suite.get("theme") {
        None | Some(Value::Null) => (None, None),
        Some(Value::String(s)) if version == 1 => {
            let id = theme_id(s, "suite.theme")?;
            (Some(id.clone()), Some(id))
        }
        Some(_) if version == 1 => return Err(corrupt("\"suite.theme\" is not a string")),
        Some(Value::Object(pair)) => (theme_slot(pair, "light")?, theme_slot(pair, "dark")?),
        Some(_) => return Err(corrupt("\"suite.theme\" is not an object")),
    };
    Ok(SuiteChoice {
        theme_light,
        theme_dark,
        mode: choice_field(suite, "mode", &MODES)?,
        contrast: choice_field(suite, "contrast", &CONTRASTS)?,
        motion: choice_field(suite, "motion", &MOTIONS)?,
        size: choice_field(suite, "size", &SIZES)?,
        text_size: choice_field(suite, "text_size", &TEXT_SIZES_READ)?,
    })
}

fn theme_slot(pair: &Map<String, Value>, key: &str) -> Result<Option<ThemeId>, SelectionError> {
    match pair.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => theme_id(s, &format!("suite.theme.{key}")).map(Some),
        Some(_) => Err(corrupt(&format!("\"suite.theme.{key}\" is not a string"))),
    }
}

fn theme_id(s: &str, at: &str) -> Result<ThemeId, SelectionError> {
    ThemeId::parse(s).map_err(|e| corrupt(&format!("\"{at}\": {e}")))
}

/// Each choice's name in the file, as craft-theme's serde writes it.
const MODES: [(ModeChoice, &str); 3] = [(ModeChoice::System, "system"), (ModeChoice::Light, "light"), (ModeChoice::Dark, "dark")];
const CONTRASTS: [(ContrastChoice, &str); 3] =
    [(ContrastChoice::System, "system"), (ContrastChoice::Standard, "standard"), (ContrastChoice::High, "high")];
const MOTIONS: [(MotionChoice, &str); 3] = [(MotionChoice::System, "system"), (MotionChoice::Full, "full"), (MotionChoice::Reduced, "reduced")];
const SIZES: [(Size, &str); 2] = [(Size::Compact, "compact"), (Size::Comfortable, "comfortable")];
const TEXT_SIZES: [(TextSize, &str); 4] =
    [(TextSize::Tiny, "tiny"), (TextSize::Small, "small"), (TextSize::Medium, "medium"), (TextSize::Large, "large")];
/// [`TEXT_SIZES`] plus the names of the former steps (`default`, `larger`) that no new name
/// shares, so a file written before the rename still loads.
const TEXT_SIZES_READ: [(TextSize, &str); 6] = [
    (TextSize::Tiny, "tiny"),
    (TextSize::Small, "small"),
    (TextSize::Medium, "medium"),
    (TextSize::Large, "large"),
    (TextSize::Small, "default"),
    (TextSize::Large, "larger"),
];

fn choice_field<T: Copy>(suite: &Map<String, Value>, key: &str, names: &[(T, &str)]) -> Result<Option<T>, SelectionError> {
    match suite.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().and_then(|s| names.iter().find(|(_, n)| *n == s)).map(|(c, _)| Some(*c)).ok_or_else(|| {
            let list: Vec<&str> = names.iter().map(|(_, n)| *n).collect();
            corrupt(&format!("\"suite.{key}\" is not {}", list.join(", ")))
        }),
    }
}

fn choice_name<T: PartialEq>(names: &[(T, &'static str)], value: &T) -> Value {
    names.iter().find(|(c, _)| c == value).map_or(Value::Null, |(_, n)| Value::from(*n))
}

fn corrupt(why: &str) -> SelectionError {
    SelectionError::Corrupt(why.to_owned())
}

/// `raw` (the object last read from disk, or empty) with the known fields set to `next`, as
/// version 2. Fields nobody chose are left out; with none chosen, `suite` stays as it was.
pub(crate) fn render_selection(raw: &mut Map<String, Value>, next: &Accepted, written_by: &str) -> Vec<u8> {
    raw.insert("schema_version".into(), SELECTION_SCHEMA_VERSION.into());
    raw.insert("revision".into(), next.revision.into());
    raw.insert("library_revision".into(), next.library_revision.into());
    let s = &next.suite;
    if !s.is_empty() {
        let mut suite = match raw.remove("suite") {
            Some(Value::Object(suite)) => suite,
            _ => Map::new(),
        };
        let theme = (s.theme_light.is_some() || s.theme_dark.is_some()).then(|| {
            // A version 1 string becomes the object; unknown keys in an object stay.
            let mut pair = match suite.remove("theme") {
                Some(Value::Object(pair)) => pair,
                _ => Map::new(),
            };
            set_or_remove(&mut pair, "light", s.theme_light.as_ref().map(|t| t.as_str().into()));
            set_or_remove(&mut pair, "dark", s.theme_dark.as_ref().map(|t| t.as_str().into()));
            Value::Object(pair)
        });
        set_or_remove(&mut suite, "theme", theme);
        set_or_remove(&mut suite, "mode", s.mode.map(|c| choice_name(&MODES, &c)));
        set_or_remove(&mut suite, "contrast", s.contrast.map(|c| choice_name(&CONTRASTS, &c)));
        set_or_remove(&mut suite, "motion", s.motion.map(|c| choice_name(&MOTIONS, &c)));
        set_or_remove(&mut suite, "size", s.size.map(|c| choice_name(&SIZES, &c)));
        set_or_remove(&mut suite, "text_size", s.text_size.map(|c| choice_name(&TEXT_SIZES, &c)));
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
    use craft_ui::testing::trial;
    use serde_json::json;

    use super::*;

    fn id(s: &str) -> ThemeId {
        ThemeId::parse(s).unwrap()
    }

    fn choice(theme: Option<&str>, mode: Option<ModeChoice>) -> SuiteChoice {
        SuiteChoice { theme_light: theme.map(id), theme_dark: theme.map(id), mode, ..SuiteChoice::default() }
    }

    fn both(theme: &str) -> SelectionChange {
        SelectionChange::themes(ThemePair::same(id(theme)))
    }

    #[trial]
    fn merge_table() {
        let base = choice(Some("craft.artcraft"), Some(ModeChoice::System));
        let dark_disk = choice(Some("craft.artcraft"), Some(ModeChoice::Dark));
        // Only mine changed / another app changed the other field.
        assert_eq!(merge(&base, &base, &both("craft.graphite")), Some(choice(Some("craft.graphite"), Some(ModeChoice::System))));
        assert_eq!(merge(&base, &dark_disk, &both("craft.graphite")), Some(choice(Some("craft.graphite"), Some(ModeChoice::Dark))));
        // Same field: equal value is fine; a different value written elsewhere conflicts.
        assert_eq!(merge(&base, &dark_disk, &SelectionChange::mode(ModeChoice::Dark)), Some(dark_disk.clone()));
        assert_eq!(merge(&base, &dark_disk, &SelectionChange::mode(ModeChoice::Light)), None);
        // "Apply mine" resends the change on the disk state as base.
        assert_eq!(
            merge(&dark_disk, &dark_disk, &SelectionChange::mode(ModeChoice::Light)),
            Some(choice(Some("craft.artcraft"), Some(ModeChoice::Light)))
        );
        // Nothing chosen: a change writes only its own field.
        let none = SuiteChoice::default();
        assert_eq!(merge(&none, &none, &both("craft.graphite")), Some(choice(Some("craft.graphite"), None)));
        // Two apps on a fresh install, one choosing the theme, the other the mode: no conflict.
        let theme_only = choice(Some("craft.graphite"), None);
        assert_eq!(
            merge(&none, &theme_only, &SelectionChange::mode(ModeChoice::Light)),
            Some(choice(Some("craft.graphite"), Some(ModeChoice::Light)))
        );
        // Another app chose the same field first: equal is fine, different conflicts.
        assert_eq!(merge(&none, &theme_only, &both("craft.graphite")), Some(theme_only.clone()));
        assert_eq!(merge(&none, &theme_only, &both("craft.artcraft")), None);
        // A file that lost a field: the change writes into it.
        assert_eq!(merge(&base, &none, &SelectionChange::mode(ModeChoice::Dark)), Some(choice(None, Some(ModeChoice::Dark))));
    }

    #[trial]
    fn version_1_is_read_into_both_slots_and_upgraded_on_write() {
        let v1 = br#"{"schema_version":1,"revision":6,"library_revision":1,"suite":{"theme":"user.navy","mode":"system","x":2}}"#;
        let doc = parse_selection(v1).unwrap();
        assert_eq!(doc.accepted.suite, choice(Some("user.navy"), Some(ModeChoice::System)));
        let next = Accepted { suite: SelectionChange::theme_dark(id("craft.pro")).applied_to(&doc.accepted.suite), revision: 7, library_revision: 1 };
        let mut raw = doc.raw;
        let out = render_selection(&mut raw, &next, "t");
        let back: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(back["schema_version"], 2);
        assert_eq!(back["suite"], json!({"theme": {"light": "user.navy", "dark": "craft.pro"}, "mode": "system", "x": 2}));
        assert_eq!(parse_selection(&out).unwrap().accepted, next);
        // Each version's own theme form only.
        assert!(matches!(parse_selection(br#"{"schema_version":1,"suite":{"theme":{"light":"user.navy"}}}"#), Err(SelectionError::Corrupt(_))));
        assert!(matches!(parse_selection(br#"{"schema_version":2,"suite":{"theme":"user.navy"}}"#), Err(SelectionError::Corrupt(_))));
    }
}
