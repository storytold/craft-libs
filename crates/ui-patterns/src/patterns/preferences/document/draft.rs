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
    /// The value at `key` (`"tools.pressureCurve"`), when it is there and fits `V`. A part of the key may name a list
    /// entry: `"performance.scratchDisks[1].path"`.
    pub fn get<V: DeserializeOwned>(&self, key: &str) -> Option<V> {
        lookup(&self.table, key)?.clone().try_into().ok()
    }

    /// The whole draft as `V`: the struct the document edits, or its JSON.
    pub fn to<V: DeserializeOwned>(&self) -> Option<V> {
        toml::Value::Table(self.table.clone()).try_into().ok()
    }

    /// Replaces the whole draft with `value`'s table; `Err` with the reason when `value` has no table form.
    pub fn replace<V: Serialize>(&mut self, value: &V) -> Result<(), String> {
        match toml::Value::try_from(value) {
            Ok(toml::Value::Table(table)) => {
                self.table = table;
                Ok(())
            }
            Ok(_) => Err("the values are not a table".to_owned()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// Writes `value` at `key`; a value that doesn't serialize, or a list entry that is not there, is ignored.
    pub fn set<V: Serialize>(&mut self, key: &str, value: &V) {
        if let Ok(value) = toml::Value::try_from(value) {
            insert(&mut self.table, key, value);
        }
    }

    /// Removes the value at `key`, for a row whose off state stores no value (an `Option` that is `None`); Apply then
    /// reports the key with no value. A missing key removes nothing.
    pub fn remove(&mut self, key: &str) {
        remove(&mut self.table, key);
    }

    /// Writes the colour row `key` (the `#rrggbb` text a [`SettingRule::color`] row holds), for the colour a picker
    /// opened from [`CustomSettings::nested`] returned.
    pub fn set_color(&mut self, key: &str, color: Srgba) {
        self.set(key, &Rgba::from(color).to_hex());
    }
}

/// One part of a dotted key: a table entry, then the list indices after it (`disks[2]`). `None` for a malformed part.
fn part(text: &str) -> Option<(&str, Vec<usize>)> {
    let Some(open) = text.find('[') else { return Some((text, Vec::new())) };
    let (name, mut rest) = text.split_at(open);
    let mut indices = Vec::new();
    while let Some(inner) = rest.strip_prefix('[') {
        let close = inner.find(']')?;
        indices.push(inner[..close].parse().ok()?);
        rest = &inner[close + 1..];
    }
    rest.is_empty().then_some((name, indices))
}

/// The entry of `value` at the list `indices`, one list deep per index.
fn entry_mut<'v>(mut value: &'v mut toml::Value, indices: &[usize]) -> Option<&'v mut toml::Value> {
    for &i in indices {
        value = value.as_array_mut()?.get_mut(i)?;
    }
    Some(value)
}

/// The value at the dotted `key` of `table`; a part may name a list entry (`scratchDisks.disks[2].path`).
pub(super) fn lookup<'t>(table: &'t toml::Table, key: &str) -> Option<&'t toml::Value> {
    let (head, rest) = match key.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (key, None),
    };
    let (name, indices) = part(head)?;
    let mut value = table.get(name)?;
    for i in indices {
        value = value.as_array()?.get(i)?;
    }
    match rest {
        None => Some(value),
        Some(rest) => lookup(value.as_table()?, rest),
    }
}

/// Writes `value` at the dotted `key`, creating missing tables on the way; a path through a value that is not a table,
/// or to a list entry that is not there, writes nothing.
pub(super) fn insert(table: &mut toml::Table, key: &str, value: toml::Value) {
    let (head, rest) = match key.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (key, None),
    };
    let Some((name, indices)) = part(head) else { return };
    if indices.is_empty() && rest.is_none() {
        table.insert(name.to_owned(), value);
        return;
    }
    let slot = if indices.is_empty() {
        table.entry(name).or_insert_with(|| toml::Value::Table(toml::Table::new()))
    } else {
        let Some(list) = table.get_mut(name) else { return };
        let Some(entry) = entry_mut(list, &indices) else { return };
        entry
    };
    match rest {
        None => *slot = value,
        Some(rest) => {
            if let toml::Value::Table(inner) = slot {
                insert(inner, rest, value);
            }
        }
    }
}

/// Removes the value at the dotted `key`; a key ending in a list entry removes that entry. A missing path removes
/// nothing.
pub(super) fn remove(table: &mut toml::Table, key: &str) {
    let (head, rest) = match key.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (key, None),
    };
    let Some((name, indices)) = part(head) else { return };
    let Some((&last, outer)) = indices.split_last() else {
        match rest {
            None => {
                table.remove(name);
            }
            Some(rest) => {
                if let Some(toml::Value::Table(inner)) = table.get_mut(name) {
                    remove(inner, rest);
                }
            }
        }
        return;
    };
    let Some(list) = table.get_mut(name).and_then(|list| entry_mut(list, outer)).and_then(toml::Value::as_array_mut) else { return };
    match rest {
        None if last < list.len() => {
            list.remove(last);
        }
        None => {}
        Some(rest) => {
            if let Some(toml::Value::Table(inner)) = list.get_mut(last) {
                remove(inner, rest);
            }
        }
    }
}
