//! GridCraft's native workbook file: the workbook as one pretty-printed UTF-8 JSON object,
//! saved as `.json` (or `.scjson`, its older name).
//!
//! GridCraft owns the workbook model and what happens after a read: rebuilding its indexes,
//! dropping cells outside the grid, and reading its macros. This crate owns the bytes: which
//! extensions name the format, how the JSON is read and written, and how entries kept from a
//! newer version are written back into a list as the file held them.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// The extensions of the format, lower case. `scjson` is the name older builds saved.
pub const EXTENSIONS: [&str; 2] = ["json", "scjson"];

/// Whether `extension` (without the dot, any case) names the format.
pub fn is_extension(extension: &str) -> bool {
    EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(extension))
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The bytes are not a workbook: not JSON, or JSON of another shape.
    #[error("not a GridCraft JSON workbook: {0}")]
    Read(serde_json::Error),
    /// The workbook could not be written as JSON.
    #[error("{0}")]
    Write(serde_json::Error),
}

/// Read a workbook. Unknown fields are left to the model's own serde rules.
pub fn read<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(bytes).map_err(Error::Read)
}

/// Write a workbook as pretty-printed JSON (two-space indent, no newline at the end).
pub fn write<T: Serialize>(workbook: &T) -> Result<Vec<u8>, Error> {
    serde_json::to_vec_pretty(workbook).map_err(Error::Write)
}

/// [`fn@write`], with `kept` appended to the top-level list `field` (made if absent): entries a
/// newer version wrote that this build kept without reading. With nothing kept this is exactly
/// [`fn@write`]. Otherwise the workbook goes through a [`Value`] first, so its object keys are
/// written in `serde_json`'s map order.
pub fn write_with_kept<T: Serialize>(workbook: &T, field: &str, kept: &[Value]) -> Result<Vec<u8>, Error> {
    if kept.is_empty() {
        return write(workbook);
    }
    let mut v = serde_json::to_value(workbook).map_err(Error::Write)?;
    if let Some(list) = v.as_object_mut().map(|o| o.entry(field).or_insert_with(|| Value::Array(vec![]))).and_then(Value::as_array_mut) {
        list.extend(kept.iter().cloned());
    }
    serde_json::to_vec_pretty(&v).map_err(Error::Write)
}

#[cfg(test)]
mod tests;
