//! What the reader does with every field of `document.bin`: read it, ignore it on purpose (with the reason), or
//! know it as a setting it does not handle yet (with the value a fresh object holds). The model builder checks
//! every object it walks against [`table`]: a field that is not handled is kept on the nearest layer, effect, texture
//! set or document as an [`UnhandledSetting`] with its value, and a field the table does not list at all (a newer
//! version, a class never seen) is kept as an unknown one. Nothing the file holds is dropped silently.
//!
//! Defaults are the value at which ignoring the field changes nothing: the value a fresh object holds (the usual value
//! over the 158-project corpus, `examples/field_census`, checked against a fresh version 12.2 project), except where
//! the reference app writes the value from the resource (a graph's alpha input, its random draws): there the default is the value
//! meaning "none", so the importer flags what it doesn't do. The rows after the corpus's fields are version 12.2 fields
//! no corpus project holds yet (found by the census of the fresh project).

mod table;

use super::raw::{RawObject, RawValue};
use std::borrow::Cow;

/// What the reader does with one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// The model builder reads it.
    Read,
    /// Read by another part of the reader (named).
    ReadElsewhere(&'static str),
    /// An object not read as such whose own fields are checked one by one (what it is).
    Container(&'static str),
    /// Cannot change pixels (UI state, labels, identities, defaults for objects created later): why.
    Ignored(&'static str),
    /// A field of an object reached only through a field that is classified as a whole (named).
    Within(&'static str),
    /// The reference app writes it from the resource itself; there is no user default to compare with. Kept, never flagged.
    Derived(&'static str),
    /// Could change pixels and is not handled yet. `default`: the value of a fresh object, in [`RawValue::describe`]'s
    /// notation; `what`: what the setting does.
    Unhandled { default: &'static str, what: &'static str },
}

impl Rule {
    /// The one-word kind used in the TSV.
    pub fn kind(&self) -> &'static str {
        match self {
            Rule::Read => "read",
            Rule::ReadElsewhere(_) => "read elsewhere",
            Rule::Container(_) => "container",
            Rule::Ignored(_) => "ignored",
            Rule::Within(_) => "within",
            Rule::Derived(_) => "derived",
            Rule::Unhandled { .. } => "unhandled",
        }
    }

    /// The reason, place or meaning that goes with the rule (empty for [`Rule::Read`]).
    pub fn note(&self) -> &'static str {
        match self {
            Rule::Read => "",
            Rule::ReadElsewhere(s) | Rule::Container(s) | Rule::Ignored(s) | Rule::Within(s) | Rule::Derived(s) => s,
            Rule::Unhandled { what, .. } => what,
        }
    }

    /// True when the builder should look inside the field's objects: their fields are the reader's business too.
    fn descends(&self) -> bool {
        matches!(self, Rule::Read | Rule::ReadElsewhere(_) | Rule::Container(_))
    }
}

/// One row of the field table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldEntry {
    pub class: &'static str,
    pub field: &'static str,
    pub rule: Rule,
}

/// Every row, sorted by class and field.
pub fn table() -> impl Iterator<Item = FieldEntry> {
    table::TABLE.iter().map(|&(class, field, rule)| FieldEntry { class, field, rule })
}

/// The row of `class.field`, if the table has one.
pub fn lookup(class: &str, field: &str) -> Option<FieldEntry> {
    let i = table::TABLE.binary_search_by(|(c, f, ..)| (*c, *f).cmp(&(class, field))).ok()?;
    table().nth(i)
}

/// What kind of unhandled field a setting is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    /// Listed as [`Rule::Unhandled`].
    Unhandled { default: &'static str, what: &'static str },
    /// Listed as [`Rule::Derived`]: never flagged.
    Derived,
    /// Not in the table: a field this reader has never seen.
    Unknown,
}

/// A field the reader saw on an object and does not handle, with its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnhandledSetting {
    pub class: Cow<'static, str>,
    pub field: Cow<'static, str>,
    /// The value in [`RawValue::describe`]'s notation.
    pub value: String,
    pub kind: SettingKind,
}

/// The words around a flagged setting in reasons; [`UnhandledSetting::key_in`] reads them back.
const SETTING_PREFIX: &str = "setting ";
const NOT_HANDLED: &str = " not handled";

impl UnhandledSetting {
    /// `Class.field`.
    pub fn key(&self) -> String {
        format!("{}.{}", self.class, self.field)
    }

    /// True when the value is the one a fresh object holds (always false for unknown fields, never true for derived ones
    /// as there is no default).
    pub fn is_default(&self) -> bool {
        matches!(self.kind, SettingKind::Unhandled { default, .. } if default == self.value)
    }

    /// True when the result may differ from the reference app's because of it: an unhandled setting away from its default, or a
    /// field the reader doesn't know.
    pub fn may_change_pixels(&self) -> bool {
        match self.kind {
            SettingKind::Unhandled { .. } => !self.is_default(),
            SettingKind::Derived => false,
            SettingKind::Unknown => true,
        }
    }

    /// The reason shown in reports: `setting Class.field = value not handled` (`(unknown field)` added for fields the
    /// reader has never seen).
    pub fn reason(&self) -> String {
        let unknown = match self.kind {
            SettingKind::Unknown => " (unknown field)",
            _ => "",
        };
        format!("{SETTING_PREFIX}{} = {}{NOT_HANDLED}{unknown}", self.key(), self.value)
    }

    /// The `Class.field` keys of the settings named in a report text, in order (see [`Self::reason`]).
    pub fn keys_in(text: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(i) = rest.find(SETTING_PREFIX) {
            rest = &rest[i + SETTING_PREFIX.len()..];
            let Some(end) = rest.find(" = ") else { break };
            let key = &rest[..end];
            let tail = &rest[end..];
            let closes = tail.find(NOT_HANDLED).is_some_and(|n| !tail[..n].contains(SETTING_PREFIX));
            if closes && key.contains('.') && !key.contains(' ') {
                out.push(key);
            }
        }
        out
    }
}

/// The settings of `object`'s own fields that the reader does not handle, and whether to look inside each field.
/// Fields listed as handled or ignored give nothing.
pub fn check(object: &RawObject, mut found: impl FnMut(UnhandledSetting)) {
    for (name, value) in &object.fields {
        let setting = |kind, class: Cow<'static, str>, field: Cow<'static, str>| UnhandledSetting { class, field, value: value.describe(), kind };
        match lookup(&object.class, name) {
            Some(FieldEntry { class, field, rule: Rule::Unhandled { default, what }, .. }) => {
                found(setting(SettingKind::Unhandled { default, what }, class.into(), field.into()))
            }
            Some(FieldEntry { class, field, rule: Rule::Derived(_), .. }) => found(setting(SettingKind::Derived, class.into(), field.into())),
            Some(_) => {}
            None => found(setting(SettingKind::Unknown, object.class.clone().into(), name.clone().into())),
        }
    }
}

/// The objects held by `object`'s fields that the reader looks inside (fields read, or unknown classes' fields are
/// not looked into: the field itself is what gets reported).
pub fn children(object: &RawObject) -> impl Iterator<Item = &RawObject> {
    object.fields.iter().filter(|(name, _)| lookup(&object.class, name).is_some_and(|e| e.rule.descends())).flat_map(|(_, value)| objects_in(value))
}

fn objects_in(value: &RawValue) -> Box<dyn Iterator<Item = &RawObject> + '_> {
    match value {
        RawValue::Object(o) => Box::new(std::iter::once(o.as_ref())),
        RawValue::List(items) => Box::new(items.iter().flat_map(objects_in)),
        _ => Box::new(std::iter::empty()),
    }
}

#[cfg(test)]
mod tests;
