//! Named sets of overrides: the app's built-in sets (read only) and the user's own.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};

use crate::keymap::Overrides;
use crate::keymap::overrides::MaybeOverrides;

/// Most user sets.
pub const MAX_SETS: usize = 64;
/// Longest set name, in chars.
pub const MAX_SET_NAME: usize = 64;
/// Largest export document [`Profiles::import`] reads, in bytes.
pub const MAX_EXPORT_BYTES: usize = 4 << 20;

const FORMAT: &str = "craft-keymap";
const VERSION: u64 = 1;
const IMPORTED: &str = "Imported";

/// The app's built-in sets: name and overrides against the catalogue defaults.
pub type Builtins<'a> = [(&'a str, Overrides)];

/// Why a set operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// The user's sets and which set is active. The built-in sets are passed to each call.
///
/// An `active` name that matches no set means the first built-in set (or no overrides when the app
/// has none).
///
/// A user set may carry a built-in's name, for instance after an update adds that built-in. Until
/// [`Profiles::reconcile`] renames it, the user set is the one that name reads and writes. Call
/// `reconcile` after loading; every method that changes the profiles calls it first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profiles {
    #[serde(deserialize_with = "lenient_name")]
    pub active: String,
    #[serde(serialize_with = "sets_as_list", deserialize_with = "capped_sets")]
    pub sets: BTreeMap<String, Overrides>,
}

static NO_OVERRIDES: Overrides = Overrides::new();

/// A name as given, trimmed; `None` when empty, too long or with control characters.
pub fn set_name(name: &str) -> Option<String> {
    let n = name.trim();
    let ok = !n.is_empty() && n.chars().count() <= MAX_SET_NAME && !n.chars().any(char::is_control);
    ok.then(|| n.to_string())
}

impl Profiles {
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
        let name = self.unique(&base, builtins)?;
        self.sets.insert(name.clone(), overrides);
        self.active = name.clone();
        Ok(Some(name))
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
        if to != from && (self.taken(&to, builtins) && to.to_lowercase() != from.to_lowercase()) {
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

    /// A set as a JSON document: `{"format":"craft-keymap","version":1,"name":…,"overrides":[…]}`,
    /// overrides in their array form.
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
        let name = self.unique(&base, builtins)?;
        self.sets.insert(name.clone(), overrides);
        self.active = name.clone();
        Ok(name)
    }

    fn builtin_like(&self, name: &str, builtins: &Builtins) -> bool {
        let lower = name.to_lowercase();
        builtins.iter().any(|(n, _)| n.to_lowercase() == lower)
    }

    fn taken(&self, name: &str, builtins: &Builtins) -> bool {
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

fn lenient_name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let v = Value::deserialize(deserializer)?;
    Ok(v.as_str().and_then(set_name).unwrap_or_default())
}

// An array of `{ name, overrides }` rather than a table, so that craft-settings (which replaces
// arrays whole) drops a deleted set from the file.
fn sets_as_list<S: Serializer>(sets: &BTreeMap<String, Overrides>, serializer: S) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct Set<'a> {
        name: &'a str,
        overrides: &'a Overrides,
    }
    serializer.collect_seq(sets.iter().map(|(name, overrides)| Set { name, overrides }))
}

fn capped_sets<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<String, Overrides>, D::Error> {
    deserializer.deserialize_any(SetsVisitor)
}

struct SetsVisitor;

impl<'de> Visitor<'de> for SetsVisitor {
    type Value = BTreeMap<String, Overrides>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a list of `{ name, overrides }` items or a map of set names to overrides")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let mut out = BTreeMap::new();
        while let Some(item) = access.next_element::<Value>()? {
            let Some(name) = item.get("name").and_then(Value::as_str) else { continue };
            if !admits(&out, name) {
                continue;
            }
            let overrides = item.get("overrides").map(MaybeOverrides::deserialize).and_then(Result::ok);
            if let Some(MaybeOverrides(Some(o))) = overrides {
                out.insert(name.to_string(), o);
            }
        }
        Ok(out)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        let mut out = BTreeMap::new();
        while let Some(key) = access.next_key::<String>()? {
            if admits(&out, &key) {
                if let MaybeOverrides(Some(o)) = access.next_value()? {
                    out.insert(key, o);
                }
            } else {
                access.next_value::<IgnoredAny>()?;
            }
        }
        Ok(out)
    }
}

/// A valid name, stored as written, with room under [`MAX_SETS`].
fn admits(sets: &BTreeMap<String, Overrides>, name: &str) -> bool {
    set_name(name).is_some_and(|n| n == name) && (sets.len() < MAX_SETS || sets.contains_key(name))
}
