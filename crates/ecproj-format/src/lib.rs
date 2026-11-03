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
//! [`Head::read`] reads `savedBy` and `schema` from either, and refuses a schema newer than
//! [`SCHEMA_VERSION`].
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
///
/// Other crates read its fields but do not build one:
///
/// ```compile_fail
/// let _ = craft_ecproj_format::TooNew { schema: 3, supported: 2 };
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("project schema {schema} is newer than this version supports ({supported})")]
#[non_exhaustive]
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

/// `savedBy` and `schema`, read from a project file by [`Head::read`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Head {
    /// The version of the app that wrote the file. `None` for projects saved before `savedBy` was
    /// added, and when `savedBy` is not a string or appears more than once.
    pub saved_by: Option<String>,
    /// The project schema, no newer than [`SCHEMA_VERSION`].
    pub schema: u32,
}

impl Head {
    /// Reads `text`, a project file: one JSON object with a `schema` that is a `u32`. Other
    /// members are skipped, and two `schema` members are refused. A schema newer than
    /// [`SCHEMA_VERSION`] is refused with [`ReadError::TooNew`], which holds the file's `savedBy`.
    pub fn read(text: &str) -> Result<Head, ReadError> {
        let mut deserializer = serde_json::Deserializer::from_str(text);
        let head = deserializer.deserialize_map(HeadVisitor)?;
        deserializer.end()?;
        if let Err(too_new) = check_schema(head.schema) {
            return Err(ReadError::TooNew { too_new, saved_by: head.saved_by });
        }
        Ok(head)
    }
}

/// [`Head::read`] could not read a project file.
///
/// Other crates match it with a wildcard arm, and match [`ReadError::TooNew`] with `..`:
///
/// ```
/// use craft_ecproj_format::{Head, ReadError};
///
/// let text = r#"{"savedBy": "0.13.0", "schema": 3}"#;
/// let message = match Head::read(text) {
///     Ok(_) => String::new(),
///     Err(ReadError::TooNew { too_new, saved_by, .. }) => format!("saved by {}: {too_new}", saved_by.unwrap_or_default()),
///     Err(e) => e.to_string(),
/// };
/// assert_eq!(message, "saved by 0.13.0: project schema 3 is newer than this version supports (2)");
/// ```
///
/// ```compile_fail
/// fn name(e: craft_ecproj_format::ReadError) -> &'static str {
///     match e {
///         craft_ecproj_format::ReadError::Json(_) => "json",
///         craft_ecproj_format::ReadError::TooNew { .. } => "too new",
///     }
/// }
/// ```
///
/// ```compile_fail
/// fn saved_by(e: craft_ecproj_format::ReadError) -> Option<String> {
///     match e {
///         craft_ecproj_format::ReadError::TooNew { too_new: _, saved_by } => saved_by,
///         _ => None,
///     }
/// }
/// ```
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReadError {
    /// The text is not one JSON object with a `schema` that is a `u32`.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// The schema is newer than [`SCHEMA_VERSION`]. Reads as [`TooNew`] does.
    #[error("{too_new}")]
    #[non_exhaustive]
    TooNew {
        /// The schema the file declares, and [`SCHEMA_VERSION`].
        too_new: TooNew,
        /// The file's `savedBy`, read as [`Head::saved_by`] is.
        saved_by: Option<String>,
    },
}

/// Reads a [`Head`] from a JSON object. Other members are skipped.
struct HeadVisitor;

impl<'de> Visitor<'de> for HeadVisitor {
    type Value = Head;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a project file object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Head, A::Error> {
        let mut saved_by = None;
        let mut seen_saved_by = false;
        let mut schema = None;
        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            match &*key {
                "savedBy" if seen_saved_by => {
                    map.next_value::<IgnoredAny>()?;
                    saved_by = None;
                }
                "savedBy" => {
                    saved_by = map.next_value::<SavedBy>()?.0;
                    seen_saved_by = true;
                }
                "schema" if schema.is_some() => return Err(de::Error::duplicate_field("schema")),
                "schema" => schema = Some(map.next_value()?),
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        let schema = schema.ok_or_else(|| de::Error::missing_field("schema"))?;
        Ok(Head { saved_by, schema })
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
