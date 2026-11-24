use std::collections::BTreeMap;
use std::fmt;

use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use super::{DropReason, MAX_BINDING_TEXT, MAX_BINDINGS, MAX_COMMANDS, Problem};
use crate::command::MAX_KEY;

// Load notes are kept for `Keymap::problems`; a hostile file must not grow them without bound.
const MAX_NOTES: usize = 64;

/// The user's changes to the default bindings: binding key → the full list of binding texts
/// that replaces the command's defaults (`[]` = no shortcut).
///
/// Stored in key order as a map of one text per command (`{ "<key>": "Cmd+K" }`, `""` for no
/// shortcut) while no entry has more than one text, and otherwise as an array of
/// `{ command, keys }` items. [`list`] always writes the array, for craft-settings files. Both
/// forms load, and in the map form a list of texts loads too. Loading never fails:
/// entries for unknown commands and texts that do not parse are kept verbatim; values that are not
/// text, overlong keys and texts, repeated keys (the first entry stays) and entries past the caps
/// are dropped, and a value that is neither a list nor a map loads as no overrides. All of these
/// are reported by [`Keymap::problems`](super::Keymap::problems).
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    map: BTreeMap<String, Vec<String>>,
    notes: Vec<Problem>,
}

impl PartialEq for Overrides {
    fn eq(&self, other: &Overrides) -> bool {
        self.map == other.map
    }
}

impl Eq for Overrides {}

impl Overrides {
    pub const fn new() -> Overrides {
        Overrides { map: BTreeMap::new(), notes: Vec::new() }
    }

    pub fn get(&self, key: &str) -> Option<&[String]> {
        self.map.get(key).map(Vec::as_slice)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.map.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Sets an entry verbatim. `false` (and nothing stored) for a key over [`MAX_KEY`] bytes, a
    /// list over [`MAX_BINDINGS`], a text over [`MAX_BINDING_TEXT`] bytes, or a new key past
    /// [`MAX_COMMANDS`] entries.
    pub fn insert(&mut self, key: impl Into<String>, texts: Vec<String>) -> bool {
        let key = key.into();
        let fits = key.len() <= MAX_KEY
            && texts.len() <= MAX_BINDINGS
            && texts.iter().all(|t| t.len() <= MAX_BINDING_TEXT)
            && (self.map.contains_key(&key) || self.map.len() < MAX_COMMANDS);
        if fits {
            self.map.insert(key, texts);
        }
        fits
    }

    pub fn remove(&mut self, key: &str) -> Option<Vec<String>> {
        self.map.remove(key)
    }

    /// Removes every entry: all commands get their defaults.
    pub fn clear(&mut self) {
        self.map.clear();
        self.notes.clear();
    }

    /// What loading dropped.
    pub fn load_notes(&self) -> &[Problem] {
        &self.notes
    }

    fn note(&mut self, problem: Problem) {
        if self.notes.len() < MAX_NOTES {
            self.notes.push(problem);
        }
    }
}

// The map form is what apps that store one text per command read, so a file stays readable by
// their older builds while no command has more than one binding.
impl Serialize for Overrides {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.map.values().all(|keys| single(keys).is_some()) {
            serializer.collect_map(self.map.iter().map(|(command, keys)| (command, single(keys).unwrap_or_default())))
        } else {
            list::serialize(self, serializer)
        }
    }
}

/// An entry's text in the map form: `""` for no shortcut. `None` for several texts, and for one
/// blank text, which the map form reads as no shortcut.
fn single(keys: &[String]) -> Option<&str> {
    match keys {
        [] => Some(""),
        [text] if !text.trim().is_empty() => Some(text.as_str()),
        _ => None,
    }
}

/// Serializes [`Overrides`] always as an array of `{ command, keys }` items, for
/// `#[serde(with = "craft_keymap::keymap::overrides_list")]`. craft-settings merges a table key
/// by key with what the file holds but replaces an array whole, so in a craft-settings file only
/// this form lets a reset command (an entry removed) stay removed after saving.
pub mod list {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::{Item, Overrides};

    pub fn serialize<S: Serializer>(overrides: &Overrides, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(overrides.map.iter().map(|(command, keys)| Item { command, keys }))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Overrides, D::Error> {
        Overrides::deserialize(deserializer)
    }
}

#[derive(Serialize)]
struct Item<'a> {
    command: &'a str,
    keys: &'a [String],
}

impl<'de> Deserialize<'de> for Overrides {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Overrides, D::Error> {
        deserializer.deserialize_any(OverridesVisitor)
    }
}

struct OverridesVisitor;

/// Collects entries with the caps applied.
#[derive(Default)]
struct Builder {
    out: Overrides,
    ignored: usize,
}

impl Builder {
    /// Whether an entry for `key` can be stored; notes why not.
    fn admits(&mut self, key: &str) -> bool {
        if key.len() > MAX_KEY {
            self.out.note(Problem::Dropped { key: clip(key, MAX_KEY), reason: DropReason::KeyTooLong });
            return false;
        }
        if self.out.map.len() >= MAX_COMMANDS && !self.out.map.contains_key(key) {
            self.ignored += 1;
            return false;
        }
        true
    }

    /// Stores an entry; a second entry for the same key is ignored and noted.
    fn add(&mut self, key: String, value: Lenient) {
        let out = &mut self.out;
        if out.map.contains_key(&key) {
            out.note(Problem::DuplicateKey { key });
            return;
        }
        let texts = match value {
            Lenient::List { texts, other, extra } => {
                if other {
                    out.note(Problem::Dropped { key: key.clone(), reason: DropReason::NotText });
                }
                if extra > 0 {
                    out.note(Problem::Dropped { key: key.clone(), reason: DropReason::TooManyBindings });
                }
                texts
            }
            Lenient::Text(t) => vec![t],
            Lenient::Other => {
                out.note(Problem::Dropped { key, reason: DropReason::NotAList });
                return;
            }
        };
        let (texts, long): (Vec<String>, Vec<String>) = texts.into_iter().partition(|t| t.len() <= MAX_BINDING_TEXT);
        if !long.is_empty() {
            out.note(Problem::Dropped { key: key.clone(), reason: DropReason::TextTooLong });
        }
        out.map.insert(key, texts);
    }

    fn finish(mut self) -> Overrides {
        if self.ignored > 0 {
            let ignored = self.ignored;
            self.out.note(Problem::TooManyCommands { ignored });
        }
        self.out
    }
}

/// No overrides, with a note that the value was not a list or map.
fn not_a_list() -> Overrides {
    let mut out = Overrides::new();
    out.note(Problem::Dropped { key: String::new(), reason: DropReason::NotAList });
    out
}

macro_rules! not_a_list {
    ($($name:ident: $t:ty),*) => {
        $(fn $name<E: de::Error>(self, _: $t) -> Result<Overrides, E> {
            Ok(not_a_list())
        })*
    };
}

impl<'de> Visitor<'de> for OverridesVisitor {
    type Value = Overrides;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a list of `{ command, keys }` items or a map of binding keys to binding texts")
    }

    not_a_list!(visit_bool: bool, visit_i64: i64, visit_u64: u64, visit_i128: i128, visit_u128: u128, visit_f64: f64, visit_str: &str, visit_bytes: &[u8], visit_char: char);

    fn visit_unit<E: de::Error>(self) -> Result<Overrides, E> {
        Ok(not_a_list())
    }

    fn visit_none<E: de::Error>(self) -> Result<Overrides, E> {
        Ok(not_a_list())
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Overrides, D::Error> {
        deserializer.deserialize_any(OverridesVisitor)
    }

    /// The array form, `[{ command = "…", keys = […] }, …]`.
    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Overrides, A::Error> {
        let mut b = Builder::default();
        while let Some(item) = access.next_element::<Value>()? {
            let command = item.get("command").and_then(Value::as_str);
            let keys = item.get("keys").map(Lenient::deserialize).and_then(Result::ok);
            match (command, keys) {
                (Some(command), Some(keys)) => {
                    if b.admits(command) {
                        b.add(command.to_string(), keys);
                    }
                }
                (command, _) => {
                    let key = clip(command.unwrap_or_default(), MAX_KEY);
                    b.out.note(Problem::Dropped { key, reason: DropReason::NotAList });
                }
            }
        }
        Ok(b.finish())
    }

    /// The map form, binding key → texts (hand edits, older files). An empty text means no
    /// shortcut, as in the apps that store one text per command and `""` for a removed shortcut.
    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Overrides, A::Error> {
        let mut b = Builder::default();
        while let Some(key) = access.next_key::<String>()? {
            if b.admits(&key) {
                let value = match access.next_value::<Lenient>()? {
                    Lenient::Text(t) if t.trim().is_empty() => Lenient::List { texts: Vec::new(), other: false, extra: 0 },
                    Lenient::List { mut texts, other, extra } => {
                        texts.retain(|t| !t.trim().is_empty());
                        Lenient::List { texts, other, extra }
                    }
                    value => value,
                };
                b.add(key, value);
            } else {
                access.next_value::<IgnoredAny>()?;
            }
        }
        Ok(b.finish())
    }
}

/// Cuts `s` to at most `max` bytes on a char boundary.
pub(crate) fn clip(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// An entry's value as found: a list (texts kept up to the cap), one text, or anything else.
enum Lenient {
    List { texts: Vec<String>, other: bool, extra: usize },
    Text(String),
    Other,
}

impl<'de> Deserialize<'de> for Lenient {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Lenient, D::Error> {
        deserializer.deserialize_any(LenientVisitor)
    }
}

struct LenientVisitor;

macro_rules! other {
    ($($name:ident: $t:ty),*) => {
        $(fn $name<E: de::Error>(self, _: $t) -> Result<Lenient, E> {
            Ok(Lenient::Other)
        })*
    };
}

impl<'de> Visitor<'de> for LenientVisitor {
    type Value = Lenient;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a list of binding texts")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Lenient, E> {
        Ok(Lenient::Text(v.to_string()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Lenient, E> {
        Ok(Lenient::Text(v))
    }

    other!(visit_bool: bool, visit_i64: i64, visit_u64: u64, visit_i128: i128, visit_u128: u128, visit_f64: f64, visit_bytes: &[u8], visit_char: char);

    fn visit_unit<E: de::Error>(self) -> Result<Lenient, E> {
        Ok(Lenient::Other)
    }

    fn visit_none<E: de::Error>(self) -> Result<Lenient, E> {
        Ok(Lenient::Other)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Lenient, D::Error> {
        deserializer.deserialize_any(LenientVisitor)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Lenient, A::Error> {
        while access.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(Lenient::Other)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Lenient, A::Error> {
        let mut texts = Vec::new();
        let mut other = false;
        let mut extra = 0usize;
        while let Some(item) = access.next_element::<Lenient>()? {
            match item {
                Lenient::Text(t) if texts.len() < MAX_BINDINGS => texts.push(t),
                Lenient::Text(_) => extra += 1,
                _ => other = true,
            }
        }
        Ok(Lenient::List { texts, other, extra })
    }
}
