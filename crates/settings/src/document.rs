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

/// Turns `doc` into `T`, one top-level section at a time.
///
/// A section that does not fit `T` is reported and replaced by its default; every other section
/// is kept. This is what keeps a renamed enum variant from resetting the whole file.
pub fn deserialize_sections<T: Settings>(doc: &DocumentMut) -> (T, Vec<SectionError>) {
    let defaults = T::default();
    let Ok(base) = toml_edit::ser::to_document(&defaults) else {
        return (defaults, Vec::new());
    };
    let mut merged = base.clone();
    let mut errors = Vec::new();
    for (key, item) in doc.iter() {
        if key == SCHEMA_VERSION_KEY {
            continue;
        }
        // Each section is tried against the defaults alone, never against the sections accepted
        // before it, so a file of many keys costs the defaults plus that key per key instead of
        // the whole document so far.
        let mut candidate = base.clone();
        candidate[key] = item.clone();
        match toml_edit::de::from_document::<T>(candidate) {
            Ok(_) => merged[key] = item.clone(),
            Err(e) => errors.push(SectionError { section: key.to_owned(), message: e.to_string() }),
        }
    }
    let value = toml_edit::de::from_document::<T>(merged).unwrap_or(defaults);
    (value, errors)
}

/// Writes `value` into `doc`, keeping comments, key order and keys `value` does not have.
///
/// Tables merge key by key; arrays, arrays of tables and scalars are replaced whole.
pub fn merge_into<T: Settings>(doc: &mut DocumentMut, value: &T) -> Result<(), toml_edit::ser::Error> {
    let mut fresh = toml_edit::ser::to_document(value)?;
    expand_inline_tables(fresh.as_table_mut());
    merge_tables(doc.as_table_mut(), fresh.as_table());
    Ok(())
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
    let base = migrated_base::<T>(base, theirs);
    merge_changed_tables(base.as_table(), ours.as_table(), theirs.as_table_mut());
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
