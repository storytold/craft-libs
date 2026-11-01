#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! The envelope of EffectCraft `.ecproj` project files, for EffectCraft, which writes them, and
//! for the apps that read them.
//!
//! A project file is one UTF-8 JSON object. EffectCraft indents it with two spaces, or writes it
//! on one line when Settings ▸ Project ▸ Save Compact Project Files is on:
//!
//! ```text
//! {
//!   "savedBy": "0.12.0",
//!   "activeComp": 1,
//!   "openComps": [1],
//!   "schema": 2,
//!   "settings": { ... },
//!   "items": { "1": { ... } },
//!   "next_id": 2
//! }
//! ```
//!
//! - `savedBy` is the version of EffectCraft that wrote the file. Projects saved before it was
//!   added start with `schema`.
//! - The saved view (`activeComp`, `openComps`, `compTimes`) and the project's own members follow
//!   in the same object. Both are type parameters of [`File`]; their types stay in EffectCraft.
//! - `schema` is the project schema. [`check_schema`] refuses a schema newer than
//!   [`SCHEMA_VERSION`].
//!
//! [`File`] writes the object: [`File::to_json`] indented, [`File::to_json_compact`] on one line.
//! [`Head`] reads `savedBy` and `schema` from either form, and [`saved_by`] reads `savedBy` alone.
//!
//! ```
//! use std::collections::BTreeMap;
//!
//! #[derive(serde::Serialize)]
//! struct Project {
//!     schema: u32,
//!     items: BTreeMap<u64, String>,
//! }
//!
//! let project = Project { schema: craft_ecproj_format::SCHEMA_VERSION, items: BTreeMap::from([(1, "Comp 1".to_string())]) };
//! craft_ecproj_format::check_schema(project.schema)?;
//! let view = BTreeMap::<String, u64>::new();
//! let file = craft_ecproj_format::File::new("0.12.0", &view, &project);
//! let text = file.to_json()?;
//! assert_eq!(text, "{\n  \"savedBy\": \"0.12.0\",\n  \"schema\": 2,\n  \"items\": {\n    \"1\": \"Comp 1\"\n  }\n}");
//! assert_eq!(file.to_json_compact()?, r#"{"savedBy":"0.12.0","schema":2,"items":{"1":"Comp 1"}}"#);
//!
//! let head = craft_ecproj_format::Head::read(&text)?;
//! assert_eq!((head.saved_by.as_deref(), head.schema), (Some("0.12.0"), 2));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! An `.ecprojx` file holds the same object encoded as XML. This crate reads and writes the JSON
//! form only.

use std::borrow::Cow;
use std::fmt;

use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

/// The newest project schema. Schema 2 measures the positions of effects on layers without a
/// source rectangle (shape, text) from the top-left of their comp-sized bounds. EffectCraft
/// converts schema 1 projects when it opens them.
pub const SCHEMA_VERSION: u32 = 2;

/// A project's schema is newer than [`SCHEMA_VERSION`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("project schema {schema} is newer than this version supports ({supported})")]
pub struct TooNew {
    /// The schema the project declares.
    pub schema: u32,
    /// [`SCHEMA_VERSION`].
    pub supported: u32,
}

/// Refuses a project whose `schema` is newer than [`SCHEMA_VERSION`], before it is read or saved.
pub fn check_schema(schema: u32) -> Result<(), TooNew> {
    if schema > SCHEMA_VERSION { Err(TooNew { schema, supported: SCHEMA_VERSION }) } else { Ok(()) }
}

/// The object a project file holds: `savedBy`, then the members of `view`, then the members of
/// `project`.
///
/// `view` and `project` must each serialize as a map or a struct, and their member names must not
/// repeat `savedBy` or each other.
///
/// The values are written as they are, without a check: JSON has no NaN or infinity, and
/// `serde_json` writes either as `null`, which may not read back as the project. The caller
/// checks its values before saving.
#[derive(Debug, Serialize)]
pub struct File<'a, V, P> {
    /// The version of the app that writes the file.
    #[serde(rename = "savedBy")]
    pub saved_by: &'a str,
    /// The compositions that were open, and their times.
    #[serde(flatten)]
    pub view: &'a V,
    /// The project.
    #[serde(flatten)]
    pub project: &'a P,
}

impl<'a, V: Serialize, P: Serialize> File<'a, V, P> {
    pub fn new(saved_by: &'a str, view: &'a V, project: &'a P) -> Self {
        File { saved_by, view, project }
    }

    /// The file's text, indented with two spaces, with no newline at the end.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// The file's text on one line, with no spaces between tokens and no newline at the end.
    pub fn to_json_compact(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// `savedBy` and `schema`, read from a project file. Other members are skipped.
///
/// A `Head` reads what EffectCraft opens: a JSON object with a `schema` that is a `u32`, or a JSON
/// array whose first element is the schema (serde's array form of the project, with no
/// `savedBy`). Two `schema` members are refused.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Head {
    /// The version of the app that wrote the file. `None` for projects saved before `savedBy` was
    /// added, and when `savedBy` is not a string or appears more than once.
    pub saved_by: Option<String>,
    /// The project schema.
    pub schema: u32,
}

impl Head {
    /// Reads `text`, a project file. The schema is not checked: call [`check_schema`] on it.
    pub fn read(text: &str) -> Result<Head, serde_json::Error> {
        serde_json::from_str(text)
    }
}

impl<'de> Deserialize<'de> for Head {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let members = deserializer.deserialize_struct("Head", &["savedBy", "schema"], MembersVisitor { schema: true })?;
        let schema = members.schema.ok_or_else(|| de::Error::missing_field("schema"))?;
        Ok(Head { saved_by: members.saved_by, schema })
    }
}

/// The `savedBy` of project file `text`. `None` when the file has none, when it is not a JSON
/// object, or when `savedBy` is not a string or appears more than once. Other members, `schema`
/// included, are skipped.
pub fn saved_by(text: &str) -> Option<String> {
    let mut de = serde_json::Deserializer::from_str(text);
    let members = de.deserialize_map(MembersVisitor { schema: false }).ok()?;
    de.end().ok()?;
    members.saved_by
}

/// The envelope members of a JSON object.
struct Members {
    saved_by: Option<String>,
    schema: Option<u32>,
}

/// Reads `savedBy`, and `schema` when `schema` is true. Other members are skipped.
struct MembersVisitor {
    schema: bool,
}

impl<'de> Visitor<'de> for MembersVisitor {
    type Value = Members;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a project file object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Members, A::Error> {
        let mut members = Members { saved_by: None, schema: None };
        let mut seen_saved_by = false;
        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            match &*key {
                "savedBy" if seen_saved_by => {
                    map.next_value::<IgnoredAny>()?;
                    members.saved_by = None;
                }
                "savedBy" => {
                    members.saved_by = map.next_value::<SavedBy>()?.0;
                    seen_saved_by = true;
                }
                "schema" if self.schema && members.schema.is_some() => return Err(de::Error::duplicate_field("schema")),
                "schema" if self.schema => members.schema = Some(map.next_value()?),
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(members)
    }

    /// A project written as an array holds its members in order, `schema` first, and no
    /// `savedBy`. Only a [`Head`] reads one.
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Members, A::Error> {
        if !self.schema {
            return Err(de::Error::invalid_type(de::Unexpected::Seq, &self));
        }
        let schema = seq.next_element::<u32>()?.ok_or_else(|| de::Error::invalid_length(0, &self))?;
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Members { saved_by: None, schema: Some(schema) })
    }
}

/// A `savedBy` value: the string, or `None` for any other value. Arrays and objects are skipped
/// element by element with [`IgnoredAny`], so a large or deeply nested value is not kept.
struct SavedBy(Option<String>);

impl<'de> Deserialize<'de> for SavedBy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(SavedByVisitor)
    }
}

struct SavedByVisitor;

impl<'de> Visitor<'de> for SavedByVisitor {
    type Value = SavedBy;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<SavedBy, E> {
        Ok(SavedBy(Some(v.to_owned())))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<SavedBy, E> {
        Ok(SavedBy(Some(v)))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<SavedBy, E> {
        Ok(SavedBy(None))
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<SavedBy, E> {
        Ok(SavedBy(None))
    }

    fn visit_i128<E: de::Error>(self, _: i128) -> Result<SavedBy, E> {
        Ok(SavedBy(None))
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<SavedBy, E> {
        Ok(SavedBy(None))
    }

    fn visit_u128<E: de::Error>(self, _: u128) -> Result<SavedBy, E> {
        Ok(SavedBy(None))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<SavedBy, E> {
        Ok(SavedBy(None))
    }

    fn visit_unit<E: de::Error>(self) -> Result<SavedBy, E> {
        Ok(SavedBy(None))
    }

    fn visit_none<E: de::Error>(self) -> Result<SavedBy, E> {
        Ok(SavedBy(None))
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<SavedBy, D::Error> {
        deserializer.deserialize_ignored_any(IgnoredAny)?;
        Ok(SavedBy(None))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<SavedBy, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(SavedBy(None))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<SavedBy, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(SavedBy(None))
    }
}
