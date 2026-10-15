//! TOML document handling: `schema_version`, per-section parsing and comment-preserving merges.

use toml_edit::{DocumentMut, Item, Table, TableLike};

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
    let Ok(mut merged) = toml_edit::ser::to_document(&defaults) else {
        return (defaults, Vec::new());
    };
    let mut errors = Vec::new();
    for (key, item) in doc.iter() {
        if key == SCHEMA_VERSION_KEY {
            continue;
        }
        let mut candidate = merged.clone();
        candidate[key] = item.clone();
        match toml_edit::de::from_document::<T>(candidate.clone()) {
            Ok(_) => merged = candidate,
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
fn expand_inline_tables(table: &mut Table) {
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

/// Renders `doc` as text.
pub fn render(doc: &DocumentMut) -> String {
    doc.to_string()
}
