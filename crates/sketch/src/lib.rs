//! The `.sketch` container, for files saved by Sketch 48 or newer, read from the public
//! description of the format only.
//!
//! A `.sketch` file is a zip archive holding `document.json` (the document: its pages in order,
//! shared styles, swatches and library symbols copied into it), `meta.json` (the app version and
//! format version that saved it), one `pages/<id>.json` per page (the layer tree), `images/` and,
//! in newer files, `fonts/`. Every object carries its kind in `_class`. Files older than Sketch 48
//! (a SQLite database before 43, text as binary property lists until 47) are refused
//! ([`Error::TooOld`]).
//!
//! This crate is the container layer: [`Archive`] opens the zip, checks the version, and hands
//! out the document and the pages as the JSON the file holds ([`serde_json::Value`]) and the
//! images as bytes. What the layers mean is read from that JSON by the caller.
//!
//! Everything here reads untrusted bytes: member sizes, compression ratios, totals and the page
//! count are bounded; malformed input is an error or a warning, never a panic.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod rest;
#[cfg(test)]
mod tests;

use std::cell::Cell;
use std::collections::{BTreeSet, HashSet};

use serde_json::{Value, json};

/// The oldest Sketch whose files open: from 48 on, text is plain JSON too.
pub const MIN_SKETCH: u32 = 48;
/// The newest `meta.json` format version checked against real files (Sketch 83); newer files
/// open with a warning.
pub const NEWEST_TESTED_VERSION: u64 = 139;
/// What [`Error::TooOld`] says.
pub const TOO_OLD: &str = "saved by a Sketch older than 48; open it in a newer Sketch and save it again";
/// Largest `meta.json` or `document.json` read.
pub const MAX_JSON_MEMBER: u64 = 512 * 1024 * 1024;
/// Most bytes a member may unpack to, per compressed byte (real JSON packs 5–20 times and images
/// barely; a zero-filled member a thousand times), with a floor ([`MIN_ALLOWANCE`]) for small
/// members.
pub const MAX_RATIO: u64 = 100;
pub const MIN_ALLOWANCE: u64 = 16 * 1024 * 1024;
/// Most JSON read from one file in all.
#[cfg(not(target_pointer_width = "32"))]
pub const MAX_JSON_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
#[cfg(target_pointer_width = "32")]
pub const MAX_JSON_TOTAL: u64 = 512 * 1024 * 1024;
/// Most pages read from one file.
pub const MAX_PAGES: usize = 1000;
/// Most entries the archive may declare; checked before the archive is indexed.
pub const MAX_ENTRIES: u64 = 100_000;
/// The SQLite header: what Sketch wrote before version 43.
const SQLITE: &[u8] = b"SQLite format 3\0";

/// Why a `.sketch` file could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Saved by a Sketch older than [`MIN_SKETCH`].
    #[error("{TOO_OLD}")]
    TooOld,
    /// The bytes are not a readable `.sketch` file.
    #[error("Sketch file: {0}")]
    Corrupt(String),
}

fn corrupt(msg: impl Into<String>) -> Error {
    Error::Corrupt(msg.into())
}

/// What one open may read beyond the fixed caps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest page JSON read (a real page of a few thousand layers takes 33 MB); a larger page
    /// is skipped with a warning.
    pub max_page: u64,
    /// Largest image member read.
    pub max_image: u64,
    /// Most image bytes read from one file in all.
    pub max_image_total: u64,
}

impl Default for Limits {
    /// 512 MiB a page, 256 MiB an image, 1 GiB of images in all (256 MiB on 32-bit targets).
    fn default() -> Self {
        #[cfg(not(target_pointer_width = "32"))]
        let max_image_total = 1024 * 1024 * 1024;
        #[cfg(target_pointer_width = "32")]
        let max_image_total = 256 * 1024 * 1024;
        Limits { max_page: MAX_JSON_MEMBER, max_image: 256 * 1024 * 1024, max_image_total }
    }
}

/// Whether `bytes` look like a `.sketch` file: a zip holding `document.json` and `meta.json`, or
/// the SQLite database Sketch wrote before version 43 (so that it gets the "too old" message).
pub fn is_sketch(bytes: &[u8]) -> bool {
    bytes.starts_with(SQLITE) || (bytes.starts_with(b"PK") && open_zip(bytes).is_ok_and(|z| z.contains("document.json") && z.contains("meta.json")))
}

/// The parts of a `.sketch` archive: the document, the pages in document order and the images.
pub struct Archive<'b> {
    /// `document.json`. When it can't be read (nested too deep, damaged) this is `{"pages": […]}`
    /// listing the archive's pages in archive order, and a warning says so.
    pub document: Value,
    /// `meta.json`.
    pub meta: Value,
    /// `meta.json`'s format version (0 when missing).
    pub version: u64,
    /// `(id, page JSON)` in the document's order.
    pub pages: Vec<(String, Value)>,
    /// Problems met while reading (pages skipped, a newer format than tested).
    pub warnings: Vec<String>,
    zip: craft_zip::Archive<'b>,
    limits: Limits,
    /// Image bytes read so far.
    image_total: Cell<u64>,
}

impl<'b> Archive<'b> {
    /// Open `bytes` with the default [`Limits`].
    pub fn open(bytes: &'b [u8]) -> Result<Self, Error> {
        Self::open_with(bytes, Limits::default())
    }

    pub fn open_with(bytes: &'b [u8], limits: Limits) -> Result<Self, Error> {
        if bytes.starts_with(SQLITE) {
            return Err(Error::TooOld);
        }
        let mut z = open_zip(bytes).map_err(|e| corrupt(e.to_string()))?;
        // `zip` keeps one entry of a repeated name; which one the writer meant is not known.
        if z.has_duplicate_names() {
            return Err(corrupt("two entries of the archive share a name"));
        }
        let mut total = 0u64;
        let meta = json_member(&mut z, "meta.json", MAX_JSON_MEMBER, &mut total)?;
        if !meta.is_object() {
            return Err(corrupt("meta.json is not an object"));
        }
        if app_major(&meta).is_some_and(|v| v < MIN_SKETCH) {
            return Err(Error::TooOld);
        }
        let mut warnings = Vec::new();
        if meta.get("version").and_then(Value::as_u64).is_some_and(|v| v > NEWEST_TESTED_VERSION) {
            warnings.push("the file was saved by a newer Sketch than the import was checked with; some layers may differ".to_string());
        }
        // A document.json that can't be read (nested too deep, damaged) loses its shared styles and
        // library copies, not the pages: they are then taken in archive order.
        let document = match json_member(&mut z, "document.json", MAX_JSON_MEMBER, &mut total) {
            Ok(d) if d.is_object() => d,
            other => {
                let why = other.err().map_or_else(|| "not an object".to_string(), |e| e.to_string());
                warnings.push(format!("the document's shared styles and library symbols were skipped: {why}"));
                let pages: Vec<Value> = z
                    .names()
                    .filter_map(|n| {
                        let id = n.strip_prefix("pages/")?.strip_suffix(".json")?;
                        Some(json!({"_ref": format!("pages/{id}")}))
                    })
                    .collect();
                json!({"pages": pages})
            }
        };
        let refs = document.get("pages").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let mut pages = Vec::new();
        let mut seen = HashSet::new();
        for r in refs.iter().take(MAX_PAGES) {
            let Some(id) = r.get("_ref").and_then(Value::as_str).and_then(|s| s.strip_prefix("pages/")).filter(|id| valid_id(id)) else {
                warnings.push("pages the document lists without a valid reference were skipped".into());
                continue;
            };
            // A page listed twice is read once: a repeated reference must not repeat the unpacking.
            if !seen.insert(id) {
                continue;
            }
            match json_member(&mut z, &format!("pages/{id}.json"), limits.max_page, &mut total) {
                Ok(page) if page.is_object() => pages.push((id.to_string(), page)),
                Ok(_) => warnings.push("pages that aren't JSON objects were skipped".into()),
                Err(e) => warnings.push(format!("a page was skipped: {e}")),
            }
        }
        if refs.len() > MAX_PAGES {
            warnings.push(format!("the file has more than {MAX_PAGES} pages; the rest were skipped"));
        }
        let version = meta.get("version").and_then(Value::as_u64).unwrap_or(0);
        Ok(Archive { document, meta, version, pages, warnings, zip: z, limits, image_total: Cell::new(0) })
    }

    /// The bytes of the image a layer references (`images/<name>`, with or without the folder).
    /// `None` when the name is not a plain member name, there is no such member, or it is past
    /// [`Limits::max_image`], [`MAX_RATIO`] or what is left of [`Limits::max_image_total`].
    pub fn image(&self, r: &str) -> Option<Vec<u8>> {
        let name = r.strip_prefix("images/").unwrap_or(r);
        if !valid_id(name.split('.').next().unwrap_or("")) || name.split('.').count() > 2 {
            return None;
        }
        // A clone shares the parsed directory and the bytes; the resolver only has shared access.
        let mut z = self.zip.clone();
        let mut total = self.image_total.get();
        let data = capped_member(&mut z, &format!("images/{name}"), self.limits.max_image, &mut total, self.limits.max_image_total).ok();
        self.image_total.set(total);
        data
    }
}

/// The families of the fonts the document carries (Sketch 7x and newer can embed them, as
/// `fontReferences` with `fontData`), sorted, at most 20, each cut at 64 characters. A reference
/// without font data only names a font the document uses and is left out.
pub fn embedded_font_families(document: &Value) -> Vec<String> {
    let Some(refs) = document.get("fontReferences").and_then(Value::as_array) else { return Vec::new() };
    let families: BTreeSet<String> = refs
        .iter()
        .filter(|r| r.get("fontData").is_some())
        .filter_map(|r| r.get("fontFamilyName").and_then(Value::as_str))
        .map(|f| f.chars().take(64).collect())
        .take(20)
        .collect();
    families.into_iter().collect()
}

/// Member `name`, refused before unpacking when it would be larger than `limit`, than
/// [`MAX_RATIO`] times what it takes in the archive (past a floor), or than what is left of
/// `max_total` (`total`: read so far, counted up).
fn capped_member(z: &mut craft_zip::Archive<'_>, name: &str, limit: u64, total: &mut u64, max_total: u64) -> Result<Vec<u8>, Error> {
    let not_found = || corrupt(format!("{name}: not in the archive"));
    let (size, packed) = (z.declared_size(name).ok_or_else(not_found)?, z.compressed_size(name).ok_or_else(not_found)?);
    let allowed = limit.min(packed.saturating_mul(MAX_RATIO).max(MIN_ALLOWANCE)).min(max_total.saturating_sub(*total));
    if size > allowed {
        return Err(corrupt(format!("{name} is too large")));
    }
    // A failed read (a bad checksum at the end, say) unpacked all the same: charged in full, so
    // a page list naming the member again can't repeat it within the budget. The read stops one
    // byte past the declared size, so the declared size is what it cost.
    let result = match z.read(name, allowed) {
        Ok(Some(data)) => Ok(data),
        Ok(None) => Err(not_found()),
        Err(craft_zip::Error::EntryTooLarge { .. }) => Err(corrupt(format!("{name} is too large"))),
        Err(e) => Err(corrupt(e.to_string())),
    };
    *total = total.saturating_add(result.as_ref().map_or(size, |d| d.len() as u64));
    result
}

/// The archive, its declared entry count checked before it is indexed. Every budget is this
/// crate's own ([`Limits`], [`MAX_JSON_TOTAL`]), so the archive's total is not used.
fn open_zip(bytes: &[u8]) -> Result<craft_zip::Archive<'_>, craft_zip::Error> {
    craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: MAX_ENTRIES, max_total_bytes: u64::MAX })
}

/// JSON member `name`, read by [`capped_member`] within the file's [`MAX_JSON_TOTAL`].
fn json_member(z: &mut craft_zip::Archive<'_>, name: &str, limit: u64, total: &mut u64) -> Result<Value, Error> {
    let data = capped_member(z, name, limit, total, MAX_JSON_TOTAL)?;
    serde_json::from_slice(&data).map_err(|e| corrupt(format!("{name}: {e}")))
}

/// The major version of the Sketch that saved the file (`meta.json` `appVersion`, "50.2" → 50).
pub fn app_major(meta: &Value) -> Option<u32> {
    meta.get("appVersion").and_then(Value::as_str)?.split('.').next()?.trim().parse().ok()
}

/// Ids name archive members: letters, digits, `-` and `_` only, never a path.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
