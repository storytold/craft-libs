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
//! flattening rule. Its generic form lets an application use its typed domain values directly,
//! while keeping the wire schema and JSON encoding here. The crate does not open files or set a
//! file-size policy; callers handle bounded file I/O. [`File::read_from`] parses from a reader.

use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::fmt;
use std::io::Read;

/// The extension, without the leading dot.
pub const EXTENSION: &str = "stencilcraft";
/// The format name stored in the header.
pub const FORMAT_NAME: &str = "stencilcraft";
/// The newest document format version this crate reads and writes.
pub const FORMAT_VERSION: u64 = 1;
/// The top-level member containing the format header.
pub const HEADER_KEY: &str = "stencilcraft";

/// Wire fields in the flattened native document object.
///
/// The type parameters are the consuming app's typed model values. Field names, required fields,
/// version-1 defaults, and omission of default canvas values remain owned by this crate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(bound(
    serialize = "Board: Serialize, Kerf: Serialize, Layers: Serialize, Image: Serialize, BoardSetup: Serialize + Default + PartialEq, Pages: Serialize + Default + PartialEq, Guides: Serialize + Default + PartialEq, Grid: Serialize + Default + PartialEq, Snap: Serialize + Default + PartialEq, Groups: Serialize + Default + PartialEq, Meta: Serialize + Default + PartialEq",
    deserialize = "Board: Deserialize<'de>, Kerf: Deserialize<'de>, Layers: Deserialize<'de>, Image: Deserialize<'de>, BoardSetup: Deserialize<'de> + Default, Pages: Deserialize<'de> + Default, Guides: Deserialize<'de> + Default, Grid: Deserialize<'de> + Default, Snap: Deserialize<'de> + Default, Groups: Deserialize<'de> + Default, Meta: Deserialize<'de> + Default"
))]
pub struct DocumentFields<
    Board = Value,
    Kerf = Value,
    Layers = Value,
    Image = Value,
    BoardSetup = Option<Value>,
    Pages = Option<Value>,
    Guides = Option<Value>,
    Grid = Option<Value>,
    Snap = Option<Value>,
    Groups = Option<Value>,
    Meta = Option<Value>,
> {
    /// Required millimetre artboard dimensions.
    pub board: Board,
    /// Required kerf settings.
    pub kerf: Kerf,
    /// Required drawing layers.
    pub layers: Layers,
    /// Next document object id; absent in early version 1 files, where it defaults to 1.
    #[serde(rename = "next-shape-id", default = "default_next_shape_id")]
    pub next_shape_id: u64,
    /// Embedded raster images; absent in early version 1 files, where it defaults to empty.
    #[serde(default)]
    pub images: Vec<Image>,
    /// Optional canvas page setup, omitted when absent.
    #[serde(rename = "board-setup", default, skip_serializing_if = "is_default")]
    pub board_setup: BoardSetup,
    /// Optional pages, omitted when absent.
    #[serde(default, skip_serializing_if = "is_default")]
    pub pages: Pages,
    /// Optional guides, omitted when absent.
    #[serde(default, skip_serializing_if = "is_default")]
    pub guides: Guides,
    /// Optional grid settings, omitted when absent.
    #[serde(default, skip_serializing_if = "is_default")]
    pub grid: Grid,
    /// Optional snap settings, omitted when absent.
    #[serde(default, skip_serializing_if = "is_default")]
    pub snap: Snap,
    /// Optional groups, omitted when absent.
    #[serde(default, skip_serializing_if = "is_default")]
    pub groups: Groups,
    /// Optional user metadata, omitted when absent.
    #[serde(default, skip_serializing_if = "is_default")]
    pub meta: Meta,
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    value == &T::default()
}

fn default_next_shape_id() -> u64 {
    1
}

/// A native document and its flattened, typed wire fields.
#[derive(Debug, Clone, PartialEq)]
pub struct File<T = DocumentFields> {
    /// Typed document fields after the shared codec has checked the header and schema.
    pub document: T,
}

impl<T> File<T> {
    /// Builds a native document from its wire fields.
    pub fn new(document: T) -> Self {
        Self { document }
    }

    /// Writes the current format header and flattened wire fields as indented JSON.
    pub fn to_json_pretty(&self) -> Result<String, Error>
    where
        T: Serialize,
    {
        let raw = RawFileRef { header: HeaderRef { name: FORMAT_NAME, version: FORMAT_VERSION }, document: &self.document };
        let value = serde_json::to_value(raw).map_err(Error::Json)?;
        serde_json::to_string_pretty(&value).map_err(Error::Json)
    }
}

impl File<DocumentFields> {
    /// Parses JSON, validates the format header and version, then reads the untyped wire fields.
    pub fn read(text: &str) -> Result<Self, Error> {
        let wire: RawWire = serde_json::from_str(text).map_err(Error::Json)?;
        Self::from_wire(wire)
    }

    /// Parses JSON from a reader, without first buffering the source bytes in a string.
    ///
    /// Parsed document fields are still materialized as JSON values. Callers that impose a
    /// total file-size policy should enforce it on the reader before calling this method.
    pub fn read_from(reader: impl Read) -> Result<Self, Error> {
        let wire: RawWire = serde_json::from_reader(reader).map_err(Error::Json)?;
        Self::from_wire(wire)
    }

    fn from_wire(wire: RawWire) -> Result<Self, Error> {
        let header = wire.fields.get(HEADER_KEY).and_then(Value::as_object).ok_or(Error::NotStencilCraft)?;
        if header.get("name").and_then(Value::as_str) != Some(FORMAT_NAME) {
            return Err(Error::NotStencilCraft);
        }
        let Some(version) = header.get("version").and_then(Value::as_u64) else {
            return Err(Error::NotStencilCraft);
        };
        if version != FORMAT_VERSION {
            return Err(Error::Version { found: version, supported: FORMAT_VERSION });
        }
        let raw: RawFile<DocumentFields> = serde_json::from_value(Value::Object(wire.fields)).map_err(|source| Error::Document { source })?;
        Ok(Self { document: raw.document })
    }
}

impl<T> File<T>
where
    T: for<'de> Deserialize<'de>,
{
    /// Parses JSON into a typed document model after validating the shared format header.
    pub fn read_typed(text: &str) -> Result<Self, Error> {
        let wire: RawWire = serde_json::from_str(text).map_err(Error::Json)?;
        Self::from_typed_wire(wire)
    }

    /// Parses JSON from a reader into a typed document model.
    pub fn read_typed_from(reader: impl Read) -> Result<Self, Error> {
        let wire: RawWire = serde_json::from_reader(reader).map_err(Error::Json)?;
        Self::from_typed_wire(wire)
    }

    fn from_typed_wire(wire: RawWire) -> Result<Self, Error> {
        let header = wire.fields.get(HEADER_KEY).and_then(Value::as_object).ok_or(Error::NotStencilCraft)?;
        if header.get("name").and_then(Value::as_str) != Some(FORMAT_NAME) {
            return Err(Error::NotStencilCraft);
        }
        let Some(version) = header.get("version").and_then(Value::as_u64) else {
            return Err(Error::NotStencilCraft);
        };
        if version != FORMAT_VERSION {
            return Err(Error::Version { found: version, supported: FORMAT_VERSION });
        }
        let raw: RawFile<T> = serde_json::from_value(Value::Object(wire.fields)).map_err(|source| Error::Document { source })?;
        Ok(Self { document: raw.document })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Header {
    name: String,
    version: u64,
}

#[derive(Debug, Deserialize)]
struct RawFile<T = DocumentFields> {
    #[serde(rename = "stencilcraft")]
    _header: Header,
    #[serde(flatten)]
    document: T,
}

/// A top-level streaming visitor that stores only fields owned by this wire schema.
///
/// Values are kept as `Value` because consumers own their document models. Unknown fields are
/// skipped, duplicate fields retain serde_json's last-value-wins behavior, and header checks run
/// only after the full JSON input has been parsed.
struct RawWire {
    fields: Map<String, Value>,
}

impl<'de> Deserialize<'de> for RawWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RawWireVisitor;

        impl<'de> Visitor<'de> for RawWireVisitor {
            type Value = RawWire;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON value containing a StencilCraft document")
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut fields = Map::new();
                while let Some(key) = access.next_key::<String>()? {
                    if is_wire_field(&key) {
                        fields.insert(key, access.next_value()?);
                    } else {
                        let _ = access.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(RawWire { fields })
            }

            fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                while access.next_element::<IgnoredAny>()?.is_some() {}
                Ok(RawWire { fields: Map::new() })
            }

            fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(RawWire { fields: Map::new() })
            }

            fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(RawWire { fields: Map::new() })
            }

            fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(RawWire { fields: Map::new() })
            }

            fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(RawWire { fields: Map::new() })
            }

            fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(RawWire { fields: Map::new() })
            }

            fn visit_string<E>(self, _value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(RawWire { fields: Map::new() })
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(RawWire { fields: Map::new() })
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(RawWire { fields: Map::new() })
            }
        }

        deserializer.deserialize_any(RawWireVisitor)
    }
}

fn is_wire_field(name: &str) -> bool {
    matches!(
        name,
        HEADER_KEY
            | "board"
            | "kerf"
            | "layers"
            | "next-shape-id"
            | "images"
            | "board-setup"
            | "pages"
            | "guides"
            | "grid"
            | "snap"
            | "groups"
            | "meta"
    )
}

#[derive(Serialize)]
struct HeaderRef<'a> {
    name: &'a str,
    version: u64,
}

#[derive(Serialize)]
struct RawFileRef<'a, T> {
    #[serde(rename = "stencilcraft")]
    header: HeaderRef<'a>,
    #[serde(flatten)]
    document: &'a T,
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
    use std::io::{self, Cursor, Read};

    use serde::{Deserialize, Serialize};
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
    fn scalar_roots_are_not_documents() {
        for text in ["false", "-1", "1", "1.5", "\"scalar\"", "null"] {
            assert!(matches!(File::read(text), Err(Error::NotStencilCraft)));
        }
        assert!(matches!(File::read_from(Cursor::new(br#""scalar""#)), Err(Error::NotStencilCraft)));
    }

    #[test]
    fn refuses_versions_other_than_the_current_version() {
        assert!(matches!(File::read(r#"{"stencilcraft":{"name":"stencilcraft","version":2}}"#), Err(Error::Version { found: 2, supported: 1 })));
    }

    #[test]
    fn reader_preserves_duplicate_key_and_error_precedence_behavior() -> Result<(), Error> {
        let duplicate = r#"{"stencilcraft":{"name":"wrong","version":1},"stencilcraft":{"name":"stencilcraft","version":1},"board":{"first":true},"board":{"width_mm":100,"height_mm":150},"kerf":{},"layers":[]}"#;
        let file = File::read_from(Cursor::new(duplicate))?;
        assert_eq!(file.document.board, json!({"width_mm":100,"height_mm":150}));

        let malformed_after_bad_header = Cursor::new(br#"{"stencilcraft":{"name":"other","version":1},"board": "#);
        assert!(matches!(File::read_from(malformed_after_bad_header), Err(Error::Json(_))));
        assert!(matches!(File::read_from(Cursor::new(b"[]")), Err(Error::NotStencilCraft)));
        Ok(())
    }

    #[test]
    fn reader_accepts_large_input_in_short_reads() -> Result<(), Error> {
        struct ShortReads<R>(R);
        impl<R: Read> Read for ShortReads<R> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let length = buffer.len().min(1024);
                self.0.read(&mut buffer[..length])
            }
        }

        let text = format!(
            r#"{{"stencilcraft":{{"name":"stencilcraft","version":1}},"board":{{}},"kerf":{{}},"layers":[],"future-field":"{}"}}"#,
            "x".repeat(8 * 1024 * 1024)
        );
        let file = File::read_from(ShortReads(Cursor::new(text.as_bytes())))?;
        assert_eq!(file.document.layers, json!([]));
        Ok(())
    }

    #[test]
    fn separates_json_errors_from_wire_schema_errors() {
        assert!(matches!(File::read("{"), Err(Error::Json(_))));
        assert!(matches!(File::read(r#"{"stencilcraft":{"name":"stencilcraft","version":1}}"#), Err(Error::Document { .. })));
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct TypedBoard {
        width_mm: f64,
        height_mm: f64,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct TypedFields {
        board: TypedBoard,
        #[serde(default, rename = "board-setup", skip_serializing_if = "Option::is_none")]
        board_setup: Option<TypedBoard>,
    }

    fn typed_fields() -> TypedFields {
        TypedFields { board: TypedBoard { width_mm: 100.0, height_mm: 150.0 }, board_setup: None }
    }

    #[test]
    fn typed_readers_round_trip_typed_fields_and_preserve_header_errors() -> Result<(), Error> {
        let file = File::new(typed_fields());
        let text = file.to_json_pretty()?;
        assert!(!text.contains("board-setup"));
        assert_eq!(File::read_typed(&text)?, file);
        assert_eq!(File::read_typed_from(Cursor::new(text.as_bytes()))?, file);

        let malformed = r#"{"stencilcraft":{"name":"stencilcraft","version":1},"board":"wrong"}"#;
        assert!(matches!(File::<TypedFields>::read_typed(malformed), Err(Error::Document { .. })));
        assert!(matches!(File::<TypedFields>::read_typed_from(Cursor::new(malformed)), Err(Error::Document { .. })));

        let wrong_header = r#"{"stencilcraft":{"name":"other","version":1},"board":{}}"#;
        assert!(matches!(File::<TypedFields>::read_typed(wrong_header), Err(Error::NotStencilCraft)));
        assert!(matches!(File::<TypedFields>::read_typed_from(Cursor::new(wrong_header)), Err(Error::NotStencilCraft)));

        let future_version = r#"{"stencilcraft":{"name":"stencilcraft","version":2},"board":{}}"#;
        assert!(matches!(File::<TypedFields>::read_typed(future_version), Err(Error::Version { found: 2, supported: 1 })));
        assert!(matches!(File::<TypedFields>::read_typed_from(Cursor::new(future_version)), Err(Error::Version { found: 2, supported: 1 })));
        Ok(())
    }
}
