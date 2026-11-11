//! SettingsDraft: the draft values by dotted key.

#[cfg(doc)]
use super::{CustomSettings, SettingRule};
use craft_theme::{Rgba, Srgba};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The draft values by dotted key, for custom rows and pages.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsDraft {
    pub(super) table: toml::Table,
}

impl SettingsDraft {
    /// The value at `key` (`"tools.pressureCurve"`), when it is there and fits `V`.
    pub fn get<V: DeserializeOwned>(&self, key: &str) -> Option<V> {
        lookup(&self.table, key)?.clone().try_into().ok()
    }

    /// Writes `value` at `key`; a value that doesn't serialize is ignored.
    pub fn set<V: Serialize>(&mut self, key: &str, value: &V) {
        if let Ok(value) = toml::Value::try_from(value) {
            insert(&mut self.table, key, value);
        }
    }

    /// Writes the colour row `key` (the `#rrggbb` text a [`SettingRule::color`] row holds), for the colour a picker
    /// opened from [`CustomSettings::nested`] returned.
    pub fn set_color(&mut self, key: &str, color: Srgba) {
        self.set(key, &Rgba::from(color).to_hex());
    }
}

/// The value at the dotted `key` of `table`.
pub(super) fn lookup<'t>(table: &'t toml::Table, key: &str) -> Option<&'t toml::Value> {
    let (head, rest) = match key.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (key, None),
    };
    let value = table.get(head)?;
    match rest {
        None => Some(value),
        Some(rest) => lookup(value.as_table()?, rest),
    }
}

/// Writes `value` at the dotted `key`, creating missing tables on the way; a path through a value that is not a table
/// writes nothing.
pub(super) fn insert(table: &mut toml::Table, key: &str, value: toml::Value) {
    match key.split_once('.') {
        None => {
            table.insert(key.to_owned(), value);
        }
        Some((head, rest)) => {
            if let toml::Value::Table(inner) = table.entry(head).or_insert_with(|| toml::Value::Table(toml::Table::new())) {
                insert(inner, rest, value);
            }
        }
    }
}
