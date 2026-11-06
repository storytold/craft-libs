//! Named sets of overrides: the app's built-in sets (read only) and the user's own.
//!
//! ```
//! use craft_keymap::{Overrides, Profiles};
//!
//! let mut classic = Overrides::new();
//! classic.insert("edit.copy", vec!["Cmd+Shift+C".into()]);
//! let builtins = [("Craft", Overrides::new()), ("Classic", classic)];
//! let mut sets = Profiles::default();
//! assert_eq!(sets.active_name(&builtins), "Craft");
//!
//! sets.duplicate("Classic", "Mine", &builtins).unwrap();
//! let text = sets.export("Mine", &builtins).unwrap();
//! assert_eq!(sets.import(&text, &builtins), Ok("Mine 2".into()));
//! assert_eq!(sets.active_overrides(&builtins), sets.get("Mine", &builtins).unwrap());
//! ```

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};

use crate::import::{ImportPlan, ImportReport};
use crate::keymap::{Keymap, Overrides};

/// Most user sets.
pub const MAX_SETS: usize = 64;
/// Longest set name, in chars.
pub const MAX_SET_NAME: usize = 64;
/// Largest export document [`Profiles::import`] reads, in bytes.
pub const MAX_EXPORT_BYTES: usize = 4 << 20;

const FORMAT: &str = "craft-keymap";
const VERSION: u64 = 1;
/// The version stored [`Profiles`] carry; stored profiles without one read as version 1.
const STORE_VERSION: u64 = 1;
// Load notes are kept for `Profiles::problems`; a hostile file must not grow them without bound.
const MAX_NOTES: usize = 64;
const IMPORTED: &str = "Imported";

/// The app's built-in sets: name and overrides against the catalog defaults.
pub type Builtins<'a> = [(&'a str, Overrides)];

/// Why a set operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ProfileError {
    /// Empty, over [`MAX_SET_NAME`] chars, or with control characters.
    BadName,
    NameTaken,
    NotFound,
    /// Built-in sets cannot be changed, renamed or deleted.
    BuiltIn,
    /// [`MAX_SETS`] user sets exist.
    TooMany,
    /// Not a JSON export document.
    BadFile,
    /// A `format` other than `craft-keymap`.
    WrongFormat,
    /// A `version` newer than this crate reads.
    NewerVersion(u64),
    /// Over [`MAX_EXPORT_BYTES`].
    TooLarge,
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ProfileError::BadName => "bad set name",
            ProfileError::NameTaken => "a set with this name exists",
            ProfileError::NotFound => "no such set",
            ProfileError::BuiltIn => "built-in sets are read only",
            ProfileError::TooMany => "too many sets",
            ProfileError::BadFile => "not a shortcut set file",
            ProfileError::WrongFormat => "not a craft-keymap file",
            ProfileError::NewerVersion(v) => return write!(f, "shortcut set file version {v} is newer than this app reads"),
            ProfileError::TooLarge => "shortcut set file too large",
        };
        f.write_str(s)
    }
}

impl std::error::Error for ProfileError {}

/// Something loading dropped from stored [`Profiles`]; [`Profiles::problems`] lists them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SetProblem {
    /// A set without a valid name: none, empty, over [`MAX_SET_NAME`] chars, with control
    /// characters or with spaces around it. `name` is what was stored, cut to [`MAX_SET_NAME`]
    /// chars.
    BadName { name: String },
    /// A second set of this name; the first one stays.
    Duplicate { name: String },
    /// Overrides that are neither a list nor a map.
    BadOverrides { name: String },
    /// Sets past [`MAX_SETS`].
    TooMany { ignored: usize },
    /// Stored by a newer craft-keymap; what this version reads is loaded.
    NewerVersion(u64),
}

/// The user's sets and which set is active. The built-in sets are passed to each call.
///
/// An active name that matches no set means the first built-in set (or no overrides when the app
/// has none).
///
/// A user set may carry a built-in's name, for instance after an update adds that built-in. Until
/// [`Profiles::reconcile`] renames it, the user set is the one that name reads and writes. Call
/// `reconcile` after loading; every method that changes the profiles calls it first.
///
/// Stored as `{ version, active, sets }`. Loading never fails: what it drops is listed by
/// [`Profiles::problems`], and the next save writes the profiles without it.
#[derive(Clone, Debug, Default)]
pub struct Profiles {
    active: String,
    sets: BTreeMap<String, Overrides>,
    notes: Vec<SetProblem>,
}

impl PartialEq for Profiles {
    fn eq(&self, other: &Profiles) -> bool {
        self.active == other.active && self.sets == other.sets
    }
}

impl Eq for Profiles {}

static NO_OVERRIDES: Overrides = Overrides::new();

/// A name as given, trimmed; `None` when empty, too long or with control characters.
pub fn set_name(name: &str) -> Option<String> {
    let n = name.trim();
    let ok = !n.is_empty() && n.chars().count() <= MAX_SET_NAME && !n.chars().any(char::is_control);
    ok.then(|| n.to_string())
}

impl Profiles {
    /// The stored active name, which may match no set; [`active_name`](Self::active_name) is the
    /// set in use.
    pub fn active(&self) -> &str {
        &self.active
    }

    /// The user's sets.
    pub fn sets(&self) -> &BTreeMap<String, Overrides> {
        &self.sets
    }

    /// What loading dropped.
    pub fn problems(&self) -> &[SetProblem] {
        &self.notes
    }

    /// Built-in names first, then the user's sets.
    pub fn names(&self, builtins: &Builtins) -> Vec<String> {
        builtins.iter().map(|(n, _)| n.to_string()).chain(self.sets.keys().cloned()).collect()
    }

    pub fn is_builtin(name: &str, builtins: &Builtins) -> bool {
        builtins.iter().any(|(n, _)| *n == name)
    }

    /// The name of the set in use.
    pub fn active_name<'a>(&'a self, builtins: &'a Builtins) -> &'a str {
        if self.sets.contains_key(&self.active) || Self::is_builtin(&self.active, builtins) {
            &self.active
        } else {
            builtins.first().map_or("", |(n, _)| n)
        }
    }

    /// The overrides of the set in use, to hand to [`Keymap::set_overrides`](crate::Keymap::set_overrides).
    pub fn active_overrides<'a>(&'a self, builtins: &'a Builtins) -> &'a Overrides {
        self.get(self.active_name(builtins), builtins).unwrap_or(&NO_OVERRIDES)
    }

    pub fn get<'a>(&'a self, name: &str, builtins: &'a Builtins) -> Option<&'a Overrides> {
        self.sets.get(name).or_else(|| builtins.iter().find(|(n, _)| *n == name).map(|(_, o)| o))
    }

    /// Renames user sets whose name matches a built-in's (ignoring case) to a free name; `active`
    /// follows. Returns whether anything was renamed.
    pub fn reconcile(&mut self, builtins: &Builtins) -> bool {
        let clashing: Vec<String> = self.sets.keys().filter(|k| self.builtin_like(k, builtins)).cloned().collect();
        let mut renamed = false;
        for old in clashing {
            let Some(set) = self.sets.remove(&old) else { continue };
            // `unique` cannot fail here: a set was just removed, so there is room and the
            // candidate names outnumber the sets.
            let name = self.unique(&old, builtins).unwrap_or_else(|_| old.clone());
            if self.active == old {
                self.active = name.clone();
            }
            renamed |= name != old;
            self.sets.insert(name, set);
        }
        renamed
    }

    pub fn select(&mut self, name: &str, builtins: &Builtins) -> Result<(), ProfileError> {
        self.reconcile(builtins);
        if self.get(name, builtins).is_none() {
            return Err(ProfileError::NotFound);
        }
        self.active = name.to_string();
        Ok(())
    }

    /// Saves edited overrides to the set in use. A built-in set is not changed: the edit goes to a
    /// new set named after `copy_name` (made unique), which becomes active and is returned. The
    /// new name is returned too when [`Profiles::reconcile`] renamed the set in use.
    pub fn store(&mut self, overrides: Overrides, builtins: &Builtins, copy_name: &str) -> Result<Option<String>, ProfileError> {
        let was = self.active_name(builtins).to_string();
        self.reconcile(builtins);
        let active = self.active_name(builtins).to_string();
        if let Some(set) = self.sets.get_mut(&active) {
            *set = overrides;
            return Ok((active != was).then_some(active));
        }
        if self.get(&active, builtins) == Some(&overrides) || (active.is_empty() && overrides.is_empty()) {
            return Ok(None);
        }
        let base = set_name(copy_name).ok_or(ProfileError::BadName)?;
        self.add(&base, overrides, builtins).map(Some)
    }

    /// Saves `overrides` as a user set (replacing one of that name) and makes it active.
    pub fn save_as(&mut self, name: &str, overrides: Overrides, builtins: &Builtins) -> Result<String, ProfileError> {
        self.reconcile(builtins);
        let name = set_name(name).ok_or(ProfileError::BadName)?;
        if self.builtin_like(&name, builtins) {
            return Err(ProfileError::BuiltIn);
        }
        if !self.sets.contains_key(&name) {
            if self.sets.keys().any(|k| k.to_lowercase() == name.to_lowercase()) {
                return Err(ProfileError::NameTaken);
            }
            if self.sets.len() >= MAX_SETS {
                return Err(ProfileError::TooMany);
            }
        }
        self.sets.insert(name.clone(), overrides);
        self.active = name.clone();
        Ok(name)
    }

    pub fn rename(&mut self, from: &str, to: &str, builtins: &Builtins) -> Result<String, ProfileError> {
        self.reconcile(builtins);
        if Self::is_builtin(from, builtins) {
            return Err(ProfileError::BuiltIn);
        }
        let to = set_name(to).ok_or(ProfileError::BadName)?;
        if !self.sets.contains_key(from) {
            return Err(ProfileError::NotFound);
        }
        // A settings file can hold names that differ only in case, so a case-only rename still
        // checks the other sets.
        let lower = to.to_lowercase();
        let clash = self.sets.keys().any(|k| k != from && k.to_lowercase() == lower);
        if to != from && (clash || (self.builtin_like(&to, builtins) && lower != from.to_lowercase())) {
            return Err(ProfileError::NameTaken);
        }
        if let Some(set) = self.sets.remove(from) {
            self.sets.insert(to.clone(), set);
        }
        if self.active == from {
            self.active = to.clone();
        }
        Ok(to)
    }

    pub fn delete(&mut self, name: &str, builtins: &Builtins) -> Result<(), ProfileError> {
        self.reconcile(builtins);
        if Self::is_builtin(name, builtins) {
            return Err(ProfileError::BuiltIn);
        }
        self.sets.remove(name).ok_or(ProfileError::NotFound)?;
        if self.active == name {
            self.active = builtins.first().map_or(String::new(), |(n, _)| n.to_string());
        }
        Ok(())
    }

    /// Copies a set (built-in or user) to a new user set named `to` (made unique), which becomes
    /// active. Returns its name.
    pub fn duplicate(&mut self, from: &str, to: &str, builtins: &Builtins) -> Result<String, ProfileError> {
        self.reconcile(builtins);
        let overrides = self.get(from, builtins).ok_or(ProfileError::NotFound)?.clone();
        let base = set_name(to).ok_or(ProfileError::BadName)?;
        self.add(&base, overrides, builtins)
    }

    /// A set as a JSON document: `{"format":"craft-keymap","version":1,"name":…,"overrides":…}`,
    /// overrides as [`Overrides`] serializes them.
    pub fn export(&self, name: &str, builtins: &Builtins) -> Result<String, ProfileError> {
        let overrides = self.get(name, builtins).ok_or(ProfileError::NotFound)?;
        let doc = json!({"format": FORMAT, "version": VERSION, "name": name, "overrides": overrides});
        serde_json::to_string_pretty(&doc).map_err(|_| ProfileError::BadFile)
    }

    /// Reads an exported set into a new user set (renamed when the name is taken) and makes it
    /// active. Returns its name.
    pub fn import(&mut self, text: &str, builtins: &Builtins) -> Result<String, ProfileError> {
        if text.len() > MAX_EXPORT_BYTES {
            return Err(ProfileError::TooLarge);
        }
        let doc: Value = serde_json::from_str(text).map_err(|_| ProfileError::BadFile)?;
        if doc.get("format").and_then(Value::as_str) != Some(FORMAT) {
            return Err(ProfileError::WrongFormat);
        }
        match doc.get("version").and_then(Value::as_u64) {
            Some(VERSION) => {}
            Some(v) if v > VERSION => return Err(ProfileError::NewerVersion(v)),
            _ => return Err(ProfileError::BadFile),
        }
        let overrides = match doc.get("overrides") {
            Some(v @ (Value::Object(_) | Value::Array(_))) => Overrides::deserialize(v).map_err(|_| ProfileError::BadFile)?,
            None => Overrides::new(),
            Some(_) => return Err(ProfileError::BadFile),
        };
        self.reconcile(builtins);
        let base = doc.get("name").and_then(Value::as_str).and_then(set_name).unwrap_or_else(|| IMPORTED.to_string());
        self.add(&base, overrides, builtins)
    }

    /// Applies an import plan to a copy of `keymap` (the keymap the plan was made from) and keeps
    /// the resulting overrides as a new user set, which becomes active; `keymap` is not changed.
    /// The set is named `name`, else the foreign set's name, else `Imported`, made unique.
    /// Returns its name and the report of [`ImportPlan::apply`].
    pub fn import_plan(
        &mut self,
        plan: &ImportPlan,
        keymap: &Keymap,
        name: Option<&str>,
        builtins: &Builtins,
    ) -> Result<(String, ImportReport), ProfileError> {
        self.reconcile(builtins);
        if self.sets.len() >= MAX_SETS {
            return Err(ProfileError::TooMany);
        }
        let mut copy = keymap.clone();
        let report = plan.apply(&mut copy);
        let base = name.or(plan.report.set.as_deref()).and_then(set_name).unwrap_or_else(|| IMPORTED.to_string());
        let name = self.add(&base, copy.overrides().clone(), builtins)?;
        Ok((name, report))
    }

    /// Adds a user set under a unique name from `base` and makes it active.
    fn add(&mut self, base: &str, overrides: Overrides, builtins: &Builtins) -> Result<String, ProfileError> {
        let name = self.unique(base, builtins)?;
        self.sets.insert(name.clone(), overrides);
        self.active = name.clone();
        Ok(name)
    }

    fn builtin_like(&self, name: &str, builtins: &Builtins) -> bool {
        let lower = name.to_lowercase();
        builtins.iter().any(|(n, _)| n.to_lowercase() == lower)
    }

    /// Whether `name` matches a built-in or user set, ignoring case.
    pub(crate) fn taken(&self, name: &str, builtins: &Builtins) -> bool {
        let lower = name.to_lowercase();
        self.builtin_like(name, builtins) || self.sets.keys().any(|k| k.to_lowercase() == lower)
    }

    /// `base`, or `base 2`, `base 3`… cut to fit [`MAX_SET_NAME`].
    fn unique(&self, base: &str, builtins: &Builtins) -> Result<String, ProfileError> {
        if self.sets.len() >= MAX_SETS {
            return Err(ProfileError::TooMany);
        }
        if !self.taken(base, builtins) {
            return Ok(base.to_string());
        }
        // `Custom 2` taken: try `Custom 3`, not `Custom 2 2`.
        let base = match base.rsplit_once(' ') {
            Some((stem, n)) if !stem.is_empty() && (1..=3).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit()) => stem,
            _ => base,
        };
        for n in 2..(MAX_SETS + builtins.len() + 3) {
            let suffix = format!(" {n}");
            let keep = MAX_SET_NAME.saturating_sub(suffix.chars().count());
            let name: String = base.chars().take(keep).collect::<String>().trim_end().to_string() + &suffix;
            if !self.taken(&name, builtins) {
                return Ok(name);
            }
        }
        Err(ProfileError::NameTaken)
    }
}

impl Serialize for Profiles {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("Profiles", 3)?;
        s.serialize_field("version", &STORE_VERSION)?;
        s.serialize_field("active", &self.active)?;
        s.serialize_field("sets", &SetList(&self.sets))?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for Profiles {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Profiles, D::Error> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Stored {
            #[serde(deserialize_with = "lenient_version")]
            version: Option<u64>,
            #[serde(deserialize_with = "lenient_name")]
            active: String,
            #[serde(deserialize_with = "capped_sets")]
            sets: Loaded,
        }
        let Stored { version, active, sets: Loaded { sets, mut notes, .. } } = Stored::deserialize(deserializer)?;
        if let Some(v) = version.filter(|v| *v > STORE_VERSION) {
            notes.insert(0, SetProblem::NewerVersion(v));
            notes.truncate(MAX_NOTES);
        }
        Ok(Profiles { active, sets, notes })
    }
}

fn lenient_name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let v = Value::deserialize(deserializer)?;
    Ok(v.as_str().and_then(set_name).unwrap_or_default())
}

fn lenient_version<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    Ok(Value::deserialize(deserializer)?.as_u64())
}

// An array of `{ name, overrides }` rather than a table, and each set's overrides in their array
// form, so that craft-settings (which replaces arrays whole) drops a deleted set or entry from the
// file.
struct SetList<'a>(&'a BTreeMap<String, Overrides>);

impl Serialize for SetList<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Set<'a> {
            name: &'a str,
            #[serde(serialize_with = "crate::keymap::overrides_list::serialize")]
            overrides: &'a Overrides,
        }
        serializer.collect_seq(self.0.iter().map(|(name, overrides)| Set { name, overrides }))
    }
}

/// The sets that loaded, and notes on the ones that did not.
#[derive(Default)]
struct Loaded {
    sets: BTreeMap<String, Overrides>,
    notes: Vec<SetProblem>,
    ignored: usize,
}

impl Loaded {
    fn note(&mut self, problem: SetProblem) {
        if self.notes.len() < MAX_NOTES {
            self.notes.push(problem);
        }
    }

    /// Keeps a set with a valid name, stored as written, that is new and fits under [`MAX_SETS`];
    /// notes why not otherwise.
    fn add(&mut self, name: &str, overrides: Option<Overrides>) {
        let shown = || name.chars().take(MAX_SET_NAME).collect::<String>();
        if !set_name(name).is_some_and(|n| n == name) {
            self.note(SetProblem::BadName { name: shown() });
        } else if self.sets.contains_key(name) {
            self.note(SetProblem::Duplicate { name: shown() });
        } else if self.sets.len() >= MAX_SETS {
            self.ignored += 1;
        } else if let Some(o) = overrides {
            self.sets.insert(name.to_string(), o);
        } else {
            self.note(SetProblem::BadOverrides { name: shown() });
        }
    }

    fn done(mut self) -> Loaded {
        if self.ignored > 0 {
            let ignored = std::mem::take(&mut self.ignored);
            self.note(SetProblem::TooMany { ignored });
        }
        self
    }
}

fn capped_sets<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Loaded, D::Error> {
    deserializer.deserialize_any(SetsVisitor)
}

struct SetsVisitor;

// Anything other than a list or a map loads as no sets, so the rest of `Profiles` still loads.
macro_rules! no_sets {
    ($($name:ident: $t:ty),*) => {
        $(fn $name<E: de::Error>(self, _: $t) -> Result<Self::Value, E> {
            Ok(Loaded::default())
        })*
    };
}

impl<'de> Visitor<'de> for SetsVisitor {
    type Value = Loaded;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a list of `{ name, overrides }` items or a map of set names to overrides")
    }

    no_sets!(visit_bool: bool, visit_i64: i64, visit_u64: u64, visit_i128: i128, visit_u128: u128, visit_f64: f64, visit_str: &str, visit_bytes: &[u8], visit_char: char);

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(Loaded::default())
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(Loaded::default())
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(SetsVisitor)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let mut out = Loaded::default();
        while let Some(item) = access.next_element::<Value>()? {
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
            out.add(name, item.get("overrides").and_then(overrides_of));
        }
        Ok(out.done())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let mut out = Loaded::default();
        while let Some(key) = access.next_key::<String>()? {
            if out.sets.len() >= MAX_SETS && !out.sets.contains_key(&key) {
                access.next_value::<IgnoredAny>()?;
                out.ignored += 1;
            } else {
                let overrides = overrides_of(&access.next_value::<Value>()?);
                out.add(&key, overrides);
            }
        }
        Ok(out.done())
    }
}

/// A set's overrides: a list or a map, anything else is no set.
fn overrides_of(v: &Value) -> Option<Overrides> {
    match v {
        Value::Object(_) | Value::Array(_) => Overrides::deserialize(v).ok(),
        _ => None,
    }
}
