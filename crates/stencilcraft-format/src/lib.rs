#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! Reader and writer for the versioned JSON document format.
//!
//! The file stores a format header under `stencilcraft` and the document's wire fields at the
//! top level:
//!
//! ```json
//! {
//!   "stencilcraft": { "name": "stencilcraft", "version": 1 },
//!   "board": { "width_mm": 100.0, "height_mm": 150.0 },
//!   "kerf": { "width_mm": 0.0, "apply_to_exports": false },
//!   "layers": []
//! }
//! ```
//!
//! [`DocumentFields`] owns the JSON field names, required and defaulted fields, and the
//! flattening rule. Its values are JSON because the consuming app owns the document model,
//! domain validation, and conversion between that model and the wire fields. The crate does not
//! open files or set a file-size policy; callers handle bounded file I/O.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The extension, without the leading dot.
pub const EXTENSION: &str = "stencilcraft";
/// The format name stored in the header.
pub const FORMAT_NAME: &str = "stencilcraft";
/// The newest document format version this crate reads and writes.
pub const FORMAT_VERSION: u64 = 1;
/// The top-level member containing the format header.
pub const HEADER_KEY: &str = "stencilcraft";

/// Wire fields in the flattened native document object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentFields {
    /// Required millimetre artboard dimensions.
    pub board: Value,
    /// Required kerf settings.
    pub kerf: Value,
    /// Required drawing layers.
    pub layers: Value,
    /// Next document object id; absent in early version 1 files, where it defaults to 1.
    #[serde(rename = "next-shape-id", default = "default_next_shape_id")]
    pub next_shape_id: u64,
    /// Embedded raster images; absent in early version 1 files, where it defaults to empty.
    #[serde(default)]
    pub images: Vec<Value>,
    /// Optional canvas page setup, omitted when absent.
    #[serde(rename = "board-setup", default, skip_serializing_if = "Option::is_none")]
    pub board_setup: Option<Value>,
    /// Optional pages, omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<Value>,
    /// Optional guides, omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guides: Option<Value>,
    /// Optional grid settings, omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<Value>,
    /// Optional snap settings, omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap: Option<Value>,
    /// Optional groups, omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Value>,
    /// Optional user metadata, omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

fn default_next_shape_id() -> u64 {
    1
}

/// A native document and its flattened wire fields.
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    /// The fields after the header have been parsed, with app-domain objects still encoded as
    /// JSON values.
    pub document: DocumentFields,
}

impl File {
    /// Builds a native document from its wire fields.
    pub fn new(document: DocumentFields) -> Self {
        Self { document }
    }

    /// Parses JSON, validates the format header and version, then reads the wire fields.
    pub fn read(text: &str) -> Result<Self, Error> {
        let value: Value = serde_json::from_str(text).map_err(Error::Json)?;
        let header = value.get(HEADER_KEY).and_then(Value::as_object).ok_or(Error::NotStencilCraft)?;
        if header.get("name").and_then(Value::as_str) != Some(FORMAT_NAME) {
            return Err(Error::NotStencilCraft);
        }
        let Some(version) = header.get("version").and_then(Value::as_u64) else {
            return Err(Error::NotStencilCraft);
        };
        if version != FORMAT_VERSION {
            return Err(Error::Version { found: version, supported: FORMAT_VERSION });
        }
        let raw: RawFile = serde_json::from_value(value).map_err(|source| Error::Document { source })?;
        Ok(Self { document: raw.document })
    }

    /// Writes the current format header and flattened wire fields as indented JSON.
    pub fn to_json_pretty(&self) -> Result<String, Error> {
        let raw = RawFileRef { header: HeaderRef { name: FORMAT_NAME, version: FORMAT_VERSION }, document: &self.document };
        let value = serde_json::to_value(raw).map_err(Error::Json)?;
        serde_json::to_string_pretty(&value).map_err(Error::Json)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Header {
    name: String,
    version: u64,
}

#[derive(Debug, Deserialize)]
struct RawFile {
    #[serde(rename = "stencilcraft")]
    _header: Header,
    #[serde(flatten)]
    document: DocumentFields,
}

#[derive(Serialize)]
struct HeaderRef<'a> {
    name: &'a str,
    version: u64,
}

#[derive(Serialize)]
struct RawFileRef<'a> {
    #[serde(rename = "stencilcraft")]
    header: HeaderRef<'a>,
    #[serde(flatten)]
    document: &'a DocumentFields,
}

/// A file was malformed, belongs to another format, or uses an unsupported version.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The input is not valid JSON, or the file could not be serialized.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// The input has no valid StencilCraft format header.
    #[error("not a stencilcraft file")]
    NotStencilCraft,
    /// The input names a version other than [`FORMAT_VERSION`].
    #[error("format version {found}, this build reads {supported}")]
    Version { found: u64, supported: u64 },
    /// The JSON envelope is valid, but a required or typed wire field is malformed.
    #[error("{source}")]
    Document { source: serde_json::Error },
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{DocumentFields, Error, FORMAT_VERSION, File};

    fn fields() -> DocumentFields {
        DocumentFields {
            board: json!({"width_mm": 100.0, "height_mm": 150.0}),
            kerf: json!({"width_mm": 0.0, "apply_to_exports": false}),
            layers: json!([]),
            next_shape_id: 1,
            images: Vec::new(),
            board_setup: None,
            pages: None,
            guides: None,
            grid: None,
            snap: None,
            groups: None,
            meta: None,
        }
    }

    #[test]
    fn writes_flattened_schema_with_the_current_header_and_reads_it() -> Result<(), Error> {
        let file = File::new(fields());
        let text = file.to_json_pretty()?;
        assert!(text.starts_with("{\n  \"board\":"));
        let value: Value = serde_json::from_str(&text)?;
        assert_eq!(value["stencilcraft"]["name"], "stencilcraft");
        assert_eq!(value["stencilcraft"]["version"], FORMAT_VERSION);
        assert_eq!(value["board"], file.document.board);
        assert_eq!(value["layers"], file.document.layers);
        assert!(value.get("board-setup").is_none());
        assert_eq!(File::read(&text)?, file);
        Ok(())
    }

    #[test]
    fn reads_version_one_defaults_and_ignores_unknown_fields() -> Result<(), Error> {
        let text = r#"{"stencilcraft":{"name":"stencilcraft","version":1},"board":{},"kerf":{},"layers":[],"future-field":true}"#;
        let file = File::read(text)?;
        assert_eq!(file.document.next_shape_id, 1);
        assert!(file.document.images.is_empty());
        Ok(())
    }

    #[test]
    fn refuses_missing_or_mismatched_headers() {
        for text in [
            r#"{"board":{},"kerf":{},"layers":[]}"#,
            r#"{"stencilcraft":{"name":"other","version":1}}"#,
            r#"{"stencilcraft":{"name":"stencilcraft","version":"1"}}"#,
        ] {
            assert!(matches!(File::read(text), Err(Error::NotStencilCraft)));
        }
    }

    #[test]
    fn refuses_versions_other_than_the_current_version() {
        assert!(matches!(File::read(r#"{"stencilcraft":{"name":"stencilcraft","version":2}}"#), Err(Error::Version { found: 2, supported: 1 })));
    }

    #[test]
    fn separates_json_errors_from_wire_schema_errors() {
        assert!(matches!(File::read("{"), Err(Error::Json(_))));
        assert!(matches!(File::read(r#"{"stencilcraft":{"name":"stencilcraft","version":1}}"#), Err(Error::Document { .. })));
    }
}
