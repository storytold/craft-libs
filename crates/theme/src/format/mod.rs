//! Theme files: schema `craft-design-theme`, version 1. [`parse`] reads and validates one,
//! [`fn@write`] produces the stable text form, [`to_file`] wraps a [`ThemeDef`] that has no file,
//! and [`ThemeDef::validate`] checks a theme against what [`parse`] accepts.
//!
//! The library does no I/O: callers hand over bytes and take back text.

use std::fmt;

use serde_json::{Map, Value};

use super::theme::ThemeDef;
use crate::IdError;

mod app_sizes;
mod parse;
mod sizes;
mod validate;
mod write;

pub use parse::parse;

/// Reads an app's size overrides: [`AppSizes::parse`](crate::AppSizes::parse).
pub fn parse_app_sizes(bytes: &[u8]) -> Result<crate::AppSizes, FormatErrors> {
    app_sizes::parse(bytes)
}
pub use app_sizes::write_app_sizes;
pub use sizes::{SIZES_SCHEMA, SIZES_SCHEMA_VERSION, SizeSetFile, parse_sizes, sizes_to_file, write_sizes};
pub use write::write;

/// The `schema` value of a theme file.
pub const SCHEMA: &str = "craft-design-theme";
/// The theme file format this crate reads and writes.
pub const SCHEMA_VERSION: u64 = 1;
/// Larger theme files are refused without being parsed.
pub const MAX_THEME_BYTES: usize = 256 * 1024;

/// The expected form of a length, in [`ErrorKind::BadValue`].
const LENGTH: &str = "a length: a number of 0 or more, or \"full\"";
/// The expected form of an edge, in [`ErrorKind::BadValue`].
const OFFSET: &str = "a number";
const ROWS: &str = "\"divided\" or \"rounded\"";
const EDGE: &str = "\"flat\" or {\"bevel\": {\"width\": n}} with n above 0";

/// A parsed theme file: the theme and the JSON it came from, so that [`fn@write`] keeps unknown keys.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeFile {
    /// The theme. [`fn@write`] refuses one that [`ThemeDef::validate`] rejects.
    pub theme: ThemeDef,
    raw: Map<String, Value>,
}

/// A theme file for `theme`, as if it had been parsed from the file [`fn@write`] produces. Fails
/// when [`ThemeDef::validate`] does.
pub fn to_file(theme: &ThemeDef) -> Result<ThemeFile, FormatErrors> {
    theme.validate()?;
    Ok(ThemeFile { theme: theme.clone(), raw: Map::new() })
}

/// What is wrong at one place in a file.
#[derive(Clone, Debug, PartialEq)]
pub enum ErrorKind {
    /// The file is over [`MAX_THEME_BYTES`].
    TooLarge {
        /// Its size.
        len: usize,
    },
    /// Not JSON; the parser's message.
    Json(String),
    /// The top level is not an object.
    NotObject,
    /// A `schema` other than [`SCHEMA`], such as the pilot format's `craft-theme`.
    OtherSchema {
        /// The file's `schema`.
        schema: String,
    },
    /// Written by a newer format than this build reads.
    TooNew {
        /// The file's `schema_version`.
        schema_version: u64,
    },
    /// `schema_version` is missing or not a positive integer.
    BadSchemaVersion,
    /// A required key is missing.
    Missing,
    /// The value has the wrong JSON type.
    WrongType {
        /// What was expected.
        expected: &'static str,
    },
    /// A value outside what the key accepts.
    BadValue {
        /// What was expected.
        expected: &'static str,
    },
    /// Not `#RRGGBB` or `#RRGGBBAA`.
    BadColour,
    /// The theme `id` is malformed.
    BadId(IdError),
    /// An app id is malformed.
    BadAppId,
    /// A path that names no semantic token or component field.
    UnknownToken,
    /// A component field in a colour map: component fields are lengths.
    PinNotColor,
    /// A colour token in `components`; colours go in the mode maps.
    ColorNotPerMode,
    /// A colour map for a mode the theme's `variants` leaves out.
    NoSuchMode,
    /// `seeds` in a map that does not take them (high-contrast and app maps).
    SeedsNotAllowed,
    /// A reference such as `"{accent.solid}"` naming no token or field.
    UnknownReference {
        /// The name between the braces.
        name: String,
    },
    /// A reference to a component field that is not a colour.
    ReferenceNotColor {
        /// The name between the braces.
        name: String,
    },
    /// References that lead back to the token that starts them.
    ReferenceCycle {
        /// The token paths along the cycle, the first repeated at the end.
        chain: Vec<String>,
    },
    /// References whose values keep changing because each reads a token derived from another:
    /// a reference to a token that is derived from the token the reference pins.
    ReferenceNeverSettles {
        /// The token paths of the references that still change after the last round.
        tokens: Vec<String>,
    },
    /// A number outside the range its path allows.
    OutOfRange {
        /// The least value.
        min: f32,
        /// The greatest value.
        max: f32,
    },
    /// A value that breaks a relation to another value.
    CrossRule {
        /// What the value must be, such as "at most base.size.control.md".
        rule: &'static str,
    },
    /// A translucent colour on a surface (`surface.*` except `surface.scrim` and `surface.shadow`).
    NotOpaque,
}

/// One problem in a file, with its JSON path.
#[derive(Clone, Debug, PartialEq)]
pub struct FormatError {
    /// The JSON path: object keys from the top. Empty for the whole file.
    pub path: Vec<String>,
    /// What is wrong.
    pub kind: ErrorKind,
}

impl FormatError {
    /// The path as displayed: dot-separated, a segment containing a dot quoted
    /// (`dark."surface.panel"`).
    pub fn path_string(&self) -> String {
        let segments: Vec<String> = self.path.iter().map(|s| if s.contains('.') || s.is_empty() { format!("\"{s}\"") } else { s.clone() }).collect();
        segments.join(".")
    }

    /// What is wrong, as a sentence fragment.
    pub fn message(&self) -> String {
        match &self.kind {
            ErrorKind::TooLarge { len } => format!("file is {len} bytes; the limit is {MAX_THEME_BYTES}"),
            ErrorKind::Json(e) => format!("not JSON: {e}"),
            ErrorKind::NotObject => "not a JSON object".to_owned(),
            ErrorKind::OtherSchema { schema } => format!("schema is {schema:?}, not {SCHEMA:?}"),
            ErrorKind::TooNew { schema_version } => format!("schema_version {schema_version} is newer than {SCHEMA_VERSION}"),
            ErrorKind::BadSchemaVersion => "not a positive integer".to_owned(),
            ErrorKind::Missing => "missing".to_owned(),
            ErrorKind::WrongType { expected } => format!("expected {expected}"),
            ErrorKind::BadValue { expected } => format!("expected {expected}"),
            ErrorKind::BadColour => "not a colour".to_owned(),
            ErrorKind::BadId(e) => e.to_string(),
            ErrorKind::BadAppId => "not an app id: use 2 to 64 of a-z, 0-9, '.', '_' or '-', starting with a-z or 0-9".to_owned(),
            ErrorKind::UnknownToken => "unknown token".to_owned(),
            ErrorKind::PinNotColor => "a component field is a length; set it in components".to_owned(),
            ErrorKind::ColorNotPerMode => "colours are set per mode, in light or dark".to_owned(),
            ErrorKind::NoSuchMode => "the theme's variants leave this mode out".to_owned(),
            ErrorKind::SeedsNotAllowed => "seeds are set in the theme's own light and dark maps only".to_owned(),
            ErrorKind::UnknownReference { name } => format!("reference {{{name}}} names no token"),
            ErrorKind::ReferenceNotColor { name } => format!("reference {{{name}}} is not a colour field"),
            ErrorKind::ReferenceNeverSettles { tokens } => {
                format!("references depend on themselves through derivation and never settle: {}", tokens.join(", "))
            }
            ErrorKind::ReferenceCycle { chain } => format!("references form a cycle: {}", chain.join(" -> ")),
            ErrorKind::OutOfRange { min, max } => format!("outside {min} to {max}"),
            ErrorKind::CrossRule { rule } => format!("must be {rule}"),
            ErrorKind::NotOpaque => "surfaces must be opaque".to_owned(),
        }
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() { f.write_str(&self.message()) } else { write!(f, "{}: {}", self.path_string(), self.message()) }
    }
}

impl std::error::Error for FormatError {}

/// Every problem found in a file. File-level errors (too large, not JSON, wrong schema) come
/// alone; field errors are all reported together.
#[derive(Clone, Debug, PartialEq)]
pub struct FormatErrors(pub Vec<FormatError>);

impl FormatErrors {
    /// The error that stopped parsing before any field was read, if any: the reason the file was not read at all.
    pub fn file_level(&self) -> Option<&ErrorKind> {
        let stops = |k: &ErrorKind| {
            matches!(
                k,
                ErrorKind::TooLarge { .. }
                    | ErrorKind::Json(_)
                    | ErrorKind::NotObject
                    | ErrorKind::OtherSchema { .. }
                    | ErrorKind::TooNew { .. }
                    | ErrorKind::BadSchemaVersion
            )
        };
        self.0.iter().map(|e| &e.kind).find(|k| stops(k))
    }
}

impl fmt::Display for FormatErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for FormatErrors {}
