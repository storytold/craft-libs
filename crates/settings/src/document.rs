//! TOML document handling: `schema_version`, per-section parsing and comment-preserving merges.

use toml_edit::{DocumentMut, Item, Table, TableLike, Value};

use crate::Settings;
use crate::error::SectionError;

/// The key written first in every `config.toml`.
pub const SCHEMA_VERSION_KEY: &str = "schema_version";

/// Parses `text` as TOML, keeping comments and formatting.
pub fn parse(text: &str) -> Result<DocumentMut, toml_edit::TomlError> {
    text.parse()
}

/// Reads `schema_version`; `None` when the key is missing or not a non-negative integer.
pub fn schema_version(doc: &DocumentMut) -> Option<u32> {
    doc.get(SCHEMA_VERSION_KEY)?.as_integer().and_then(|v| u32::try_from(v).ok())
}

/// Sets `schema_version`, keeping its position if it already exists.
pub fn set_schema_version(doc: &mut DocumentMut, version: u32) {
    doc[SCHEMA_VERSION_KEY] = toml_edit::value(i64::from(version));
}

/// An empty document carrying only `schema_version`.
pub fn new_document(version: u32) -> DocumentMut {
    let mut doc = DocumentMut::new();
    set_schema_version(&mut doc, version);
    doc
}

/// Turns `doc` into `T`, one value at a time.
///
/// A value that does not fit `T` is reported with its dotted key path (`general.autosave_minutes`, a map's
/// `shortcuts.file.open`) and read as its default; every other value is kept. This is what keeps one hand-edited
/// value, or a renamed enum variant, from resetting its neighbours.
pub fn deserialize_sections<T: Settings>(doc: &DocumentMut) -> (T, Vec<SectionError>) {
    let (value, resets) = read::<T>(doc);
    (value, resets.into_iter().map(Reset::into_error).collect())
}

/// [`deserialize_sections`] for a document the store will save: every value it reset is also removed from `doc`,
/// so the next save writes the default in its place (or, for a map's entry, leaves it out).
pub fn repair_sections<T: Settings>(doc: &mut DocumentMut) -> (T, Vec<SectionError>) {
    let (value, resets) = read::<T>(doc);
    for reset in &resets {
        remove_at(doc.as_table_mut(), &reset.path);
    }
    (value, resets.into_iter().map(Reset::into_error).collect())
}

/// A value of the document that `T` could not read: its key path and the deserialization error.
struct Reset {
    path: Vec<String>,
    message: String,
}

impl Reset {
    fn into_error(self) -> SectionError {
        SectionError { section: self.path.join("."), message: self.message }
    }
}

fn read<T: Settings>(doc: &DocumentMut) -> (T, Vec<Reset>) {
    if let Ok(value) = toml_edit::de::from_document::<T>(doc.clone()) {
        return (value, Vec::new());
    }
    let defaults = T::default();
    let Ok(mut base) = toml_edit::ser::to_document(&defaults) else {
        return (defaults, Vec::new());
    };
    expand_inline_tables(base.as_table_mut());
    let mut merged = base.clone();
    let mut resets = Vec::new();
    for (key, item) in doc.iter() {
        if key == SCHEMA_VERSION_KEY {
            continue;
        }
        let mut found = Vec::new();
        accept::<T>(&base, &mut merged, &mut vec![key.to_owned()], item, &mut found);
        // The values of a section are tried one by one; together they must still fit, or the section is reset whole.
        let mut candidate = base.clone();
        if let Some(kept) = merged.get(key) {
            candidate[key] = kept.clone();
        }
        if let Err(e) = toml_edit::de::from_document::<T>(candidate) {
            match base.get(key) {
                Some(default) => merged[key] = default.clone(),
                None => {
                    merged.remove(key);
                }
            }
            found = vec![Reset { path: vec![key.to_owned()], message: e.to_string() }];
        }
        resets.extend(found);
    }
    let value = toml_edit::de::from_document::<T>(merged).unwrap_or(defaults);
    (value, resets)
}

/// Puts `item` at `path` in `merged` when the defaults with only that value in place still read as `T`. A table that
/// does not is tried key by key the same way; any other value is recorded as reset.
///
/// Each value is tried against the defaults alone, never against the values accepted before it, so a damaged
/// section costs the defaults plus one value per value.
fn accept<T: Settings>(base: &DocumentMut, merged: &mut DocumentMut, path: &mut Vec<String>, item: &Item, resets: &mut Vec<Reset>) {
    let mut candidate = base.clone();
    place(candidate.as_table_mut(), path, item.clone());
    let error = match toml_edit::de::from_document::<T>(candidate) {
        Ok(_) => {
            place(merged.as_table_mut(), path, item.clone());
            return;
        }
        Err(e) => e,
    };
    match item.as_table_like() {
        Some(table) if !table.is_empty() && holds_tables(base.as_table(), path) => {
            for (key, sub) in table.iter() {
                path.push(key.to_owned());
                accept::<T>(base, merged, path, sub, resets);
                path.pop();
            }
        }
        _ => resets.push(Reset { path: path.clone(), message: error.to_string() }),
    }
}

/// True when every key along `path` in `table` is a table or missing, so values can be put under it.
fn holds_tables(table: &dyn TableLike, path: &[String]) -> bool {
    let Some((key, rest)) = path.split_first() else { return true };
    match table.get(key) {
        None => true,
        Some(item) => item.as_table_like().is_some_and(|sub| holds_tables(sub, rest)),
    }
}

/// Sets `item` at `path` in `table`, adding the tables along it that are missing.
fn place(table: &mut dyn TableLike, path: &[String], item: Item) {
    let Some((key, rest)) = path.split_first() else { return };
    if rest.is_empty() {
        table.insert(key, item);
        return;
    }
    if !table.contains_key(key) {
        table.insert(key, Item::Table(Table::new()));
    }
    if let Some(sub) = table.get_mut(key).and_then(Item::as_table_like_mut) {
        place(sub, rest, item);
    }
}

fn remove_at(table: &mut dyn TableLike, path: &[String]) {
    match path {
        [] => {}
        [key] => {
            table.remove(key);
        }
        [key, rest @ ..] => {
            if let Some(sub) = table.get_mut(key).and_then(Item::as_table_like_mut) {
                remove_at(sub, rest);
            }
        }
    }
}

/// What `T` reads from `doc`, written back as a document: the keys it knows, each with the value it took (a value it
/// reset as its default, a key `doc` lacks as its default). Keys `T` does not know are not in it.
fn known<T: Settings>(doc: &DocumentMut) -> Option<DocumentMut> {
    let (value, _) = read::<T>(doc);
    let mut known = toml_edit::ser::to_document(&value).ok()?;
    expand_inline_tables(known.as_table_mut());
    Some(known)
}

/// Writes `value` into `doc`, keeping comments, key order and keys `T` does not know.
///
/// Tables merge key by key; arrays, arrays of tables and scalars are replaced whole. A key `T` reads from `doc` that
/// `value` no longer writes (a map's removed entry, an option set to none) is removed.
pub fn merge_into<T: Settings>(doc: &mut DocumentMut, value: &T) -> Result<(), toml_edit::ser::Error> {
    let mut fresh = toml_edit::ser::to_document(value)?;
    expand_inline_tables(fresh.as_table_mut());
    if let Some(known) = known::<T>(doc) {
        remove_dropped(doc.as_table_mut(), known.as_table(), fresh.as_table());
    }
    merge_tables(doc.as_table_mut(), fresh.as_table());
    Ok(())
}

/// Removes from `doc` each key of `known` that `fresh` lacks, table by table.
fn remove_dropped(doc: &mut dyn TableLike, known: &dyn TableLike, fresh: &dyn TableLike) {
    for (key, known_item) in known.iter() {
        let Some(fresh_item) = fresh.get(key) else {
            doc.remove(key);
            continue;
        };
        if let (Some(known), Some(fresh), Some(doc)) =
            (known_item.as_table_like(), fresh_item.as_table_like(), doc.get_mut(key).and_then(Item::as_table_like_mut))
        {
            remove_dropped(doc, known, fresh);
        }
    }
}

/// Serde writes nested structs as inline tables (`general = { … }`); files are nicer to read
/// and edit as `[general]` sections, so new sections are converted before merging.
pub(crate) fn expand_inline_tables(table: &mut Table) {
    let keys: Vec<String> = table.iter().map(|(k, _)| k.to_owned()).collect();
    for key in keys {
        let Some(item) = table.get_mut(&key) else { continue };
        if let Some(inline) = item.as_inline_table() {
            *item = Item::Table(inline.clone().into_table());
        }
        if let Some(sub) = item.as_table_mut() {
            expand_inline_tables(sub);
        }
    }
}

fn merge_tables(into: &mut dyn TableLike, from: &dyn TableLike) {
    for (key, item) in from.iter() {
        match into.get_mut(key) {
            Some(existing) if existing.is_table_like() && item.is_table_like() => {
                if let (Some(existing), Some(fresh)) = (existing.as_table_like_mut(), item.as_table_like()) {
                    merge_tables(existing, fresh);
                }
            }
            Some(existing) => {
                // Keep the spacing and trailing comment of the line being replaced.
                let mut fresh = item.clone();
                if let (Some(old), Some(new)) = (existing.as_value(), fresh.as_value_mut()) {
                    *new.decor_mut() = old.decor().clone();
                }
                *existing = fresh;
            }
            None => {
                into.insert(key, item.clone());
            }
        }
    }
}

/// Merges this instance's changes into the file another instance wrote (gap 31, PhotoCraft's
/// multi-instance merge): `base` is the document as this store last read or wrote it, `ours` what
/// it would write now, `theirs` the file on disk, which becomes the result.
///
/// - `ours == base`: nothing changes; `theirs` is kept.
/// - `base` counts as `T` reads it: a key it lacks holds its default, so a section an older file
///   did not have is not taken as all of this instance's change.
/// - Tables merge key by key: a key in both `base` and `theirs` recurses; a key removed on disk
///   and unchanged here stays removed; any other key of `ours` is written (`ours` wins at the
///   leaf, scalars and arrays replaced whole); a key `base` had and `ours` dropped is removed.
/// - Comments and formatting of `theirs` stay, as in [`merge_into`].
///
/// When the disk's `schema_version` is newer than `base`'s, `base` is migrated to it with
/// [`Settings::migrate`] first, so a migration this instance ran is not written over another
/// instance's choice (PhotoCraft's `without_stale_migration`).
// pc: crates/ui-egui/src/prefs/prefs_ui/save.rs:151 merge_changes, :134 without_stale_migration, :115 write_prefs
pub fn merge_changes<T: Settings>(base: &DocumentMut, ours: &DocumentMut, theirs: &mut DocumentMut) {
    if same_table(base.as_table(), ours.as_table()) {
        return;
    }
    let base = as_read::<T>(&migrated_base::<T>(base, theirs));
    merge_changed_tables(base.as_table(), ours.as_table(), theirs.as_table_mut());
}

/// `doc` with each key `T` reads and `doc` lacks filled in as `T` reads it.
fn as_read<T: Settings>(doc: &DocumentMut) -> DocumentMut {
    let Some(mut filled) = known::<T>(doc) else {
        return doc.clone();
    };
    merge_tables(filled.as_table_mut(), doc.as_table());
    filled
}

/// `base` migrated up to the version `theirs` has, when `theirs` is newer; otherwise `base`.
fn migrated_base<T: Settings>(base: &DocumentMut, theirs: &DocumentMut) -> DocumentMut {
    let (Some(from), Some(to)) = (schema_version(base), schema_version(theirs)) else {
        return base.clone();
    };
    let mut migrated = base.clone();
    for step in from..to.min(T::SCHEMA_VERSION) {
        if T::migrate(step, &mut migrated).is_err() {
            return base.clone();
        }
        set_schema_version(&mut migrated, step + 1);
    }
    migrated
}

fn merge_changed_tables(base: &dyn TableLike, ours: &dyn TableLike, theirs: &mut dyn TableLike) {
    for (key, our_item) in ours.iter() {
        match (base.get(key), theirs.get_mut(key)) {
            (Some(base_item), Some(their_item)) => merge_changed_items(base_item, our_item, their_item),
            // Removed on disk and unchanged here: it stays removed.
            (Some(base_item), None) if same_item(base_item, our_item) => {}
            // A key neither `base` nor `T`'s defaults have (a map entry both instances added): every leaf of ours is ours.
            (None, Some(their_item)) => match (their_item.as_table_like_mut(), our_item.as_table_like()) {
                (Some(their_table), Some(our_table)) => merge_changed_tables(&Table::new(), our_table, their_table),
                _ => replace_item(their_item, our_item),
            },
            (_, None) => {
                theirs.insert(key, our_item.clone());
            }
        }
    }
    let removed: Vec<String> = base.iter().map(|(k, _)| k).filter(|k| !ours.contains_key(k)).map(str::to_owned).collect();
    for key in removed {
        theirs.remove(&key);
    }
}

fn merge_changed_items(base: &Item, ours: &Item, theirs: &mut Item) {
    if same_item(base, ours) {
        return;
    }
    match (base.as_table_like(), ours.as_table_like(), theirs.as_table_like_mut()) {
        (Some(base), Some(ours), Some(theirs)) => merge_changed_tables(base, ours, theirs),
        _ => replace_item(theirs, ours),
    }
}

/// Replaces `theirs` with `ours`, keeping the spacing and trailing comment of a replaced value.
fn replace_item(theirs: &mut Item, ours: &Item) {
    let mut fresh = ours.clone();
    if let (Some(old), Some(new)) = (theirs.as_value(), fresh.as_value_mut()) {
        *new.decor_mut() = old.decor().clone();
    }
    *theirs = fresh;
}

/// Equal in content, ignoring comments, spacing and key order.
fn same_table(a: &dyn TableLike, b: &dyn TableLike) -> bool {
    a.len() == b.len() && a.iter().all(|(k, ai)| b.get(k).is_some_and(|bi| same_item(ai, bi)))
}

fn same_item(a: &Item, b: &Item) -> bool {
    if let (Some(a), Some(b)) = (a.as_table_like(), b.as_table_like()) {
        return same_table(a, b);
    }
    match (a, b) {
        (Item::Value(a), Item::Value(b)) => same_value(a, b),
        (Item::ArrayOfTables(a), Item::ArrayOfTables(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| same_table(a, b)),
        _ => false,
    }
}

fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(a), Value::String(b)) => a.value() == b.value(),
        (Value::Integer(a), Value::Integer(b)) => a.value() == b.value(),
        (Value::Float(a), Value::Float(b)) => a.value() == b.value(),
        (Value::Boolean(a), Value::Boolean(b)) => a.value() == b.value(),
        (Value::Datetime(a), Value::Datetime(b)) => a.value() == b.value(),
        (Value::Array(a), Value::Array(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| same_value(a, b)),
        (Value::InlineTable(a), Value::InlineTable(b)) => same_table(a, b),
        _ => false,
    }
}

/// Renders `doc` as text.
pub fn render(doc: &DocumentMut) -> String {
    doc.to_string()
}
