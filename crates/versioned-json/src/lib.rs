//! Helpers for named, versioned JSON documents whose payload is an object.
//!
//! The library checks and writes the common envelope. Each application owns its payload schema,
//! migrations and validation.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use serde_json::{Map, Value};

/// A recognized document header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub version: u64,
}

/// Header errors, kept independent of an application's schema and error type.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("missing or invalid `{0}` header")]
    Missing(String),
    #[error("expected format `{expected}`, found `{found}`")]
    WrongName { expected: String, found: String },
    #[error("format version {found} is unsupported; this build reads {expected}")]
    UnsupportedVersion { found: u64, expected: u64 },
}

/// Checks `root[header_key]` for the expected name and exact supported version.
pub fn read_header(root: &Value, header_key: &str, expected_name: &str, expected_version: u64) -> Result<Header, Error> {
    let header = root.get(header_key).ok_or_else(|| Error::Missing(header_key.to_owned()))?;
    let name = header.get("name").and_then(Value::as_str).ok_or_else(|| Error::Missing(header_key.to_owned()))?;
    let version = header.get("version").and_then(Value::as_u64).ok_or_else(|| Error::Missing(header_key.to_owned()))?;
    if name != expected_name {
        return Err(Error::WrongName { expected: expected_name.to_owned(), found: name.to_owned() });
    }
    if version != expected_version {
        return Err(Error::UnsupportedVersion { found: version, expected: expected_version });
    }
    Ok(Header { name: name.to_owned(), version })
}

/// Adds a named/versioned header to a JSON object, creating the object if needed.
pub fn write_header(root: &mut Value, header_key: &str, name: &str, version: u64) -> Result<(), Error> {
    if !root.is_object() {
        *root = Value::Object(Map::new());
    }
    let object = root.as_object_mut().ok_or_else(|| Error::Missing(header_key.to_owned()))?;
    object.insert(header_key.to_owned(), serde_json::json!({ "name": name, "version": version }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_headers() {
        let mut root = serde_json::json!({ "payload": true });
        write_header(&mut root, "format", "example", 3).unwrap();
        assert_eq!(read_header(&root, "format", "example", 3).unwrap().version, 3);
        assert_eq!(read_header(&root, "format", "example", 4), Err(Error::UnsupportedVersion { found: 3, expected: 4 }));
    }
}
