//! The Penpot 2.x export container (`.penpot`: `penpot/export-files`, version 1), read and
//! written offline.
//!
//! The archive is a ZIP of JSON members and blobs:
//!
//! - `manifest.json`: `type` (`penpot/export-files`), `version` (1) and `files`, each with its
//!   `id` (a UUID) and `name`.
//! - `files/<file>.json`: the file's header (features, version, migrations, options).
//! - `files/<file>/pages/<page>.json`: a page (`id`, `name`, `index`, background, guides, flows).
//! - `files/<file>/pages/<page>/<shape>.json`: one shape each; the page's root frame has the nil
//!   UUID ([`ROOT_ID`]) and lists its children in `shapes`.
//! - `files/<file>/components/`, `colors/`, `typographies/`, `media/`: one `<uuid>.json` each.
//! - `files/<file>/tokens.json`: design tokens.
//! - `objects/<uuid>.json` and `objects/<uuid>.<ext>`: stored objects (images, fonts).
//!
//! This crate is the container layer: [`Archive`] validates the ZIP, checks the manifest and
//! hands out members as the JSON the file holds ([`serde_json::Value`]) and media as bytes;
//! [`write_archive`] writes members back. What the shapes mean is read from that JSON by the
//! caller.
//!
//! Everything here reads untrusted bytes: the archive size, the member count, member names, the
//! bytes actually unpacked and the amount of JSON structure are bounded; malformed input is an
//! error, never a panic.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod model;
#[cfg(test)]
mod model_tests;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// Largest JSON member read.
pub const MAX_JSON: u64 = 16 * 1024 * 1024;
/// Most JSON values and object keys parsed from one archive, every member together.
pub const MAX_JSON_VALUES: usize = 4_000_000;
/// Most shape JSON read for one page.
pub const MAX_PAGE_SHAPES_BYTES: u64 = 128 * 1024 * 1024;
/// Largest media object read.
pub const MAX_IMAGE: u64 = 128 * 1024 * 1024;
/// Largest archive opened.
pub const MAX_ARCHIVE: usize = 512 * 1024 * 1024;
/// Most bytes unpacked from one archive: checked against the declared sizes when it is opened
/// and against the bytes actually read after that.
pub const MAX_DECOMPRESSED: u64 = 1024 * 1024 * 1024;
/// Most ZIP members.
pub const MAX_ENTRIES: usize = 100_000;
/// Most files in the manifest.
pub const MAX_FILES: usize = 50;
/// Most pages in one file.
pub const MAX_PAGES: usize = 500;
/// Most shapes on one page.
pub const MAX_PAGE_SHAPES: usize = 50_000;
/// The id of a page's root frame.
pub const ROOT_ID: &str = "00000000-0000-0000-0000-000000000000";
/// The manifest's `type`.
pub const MANIFEST_TYPE: &str = "penpot/export-files";

/// Why a Penpot archive could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes are not a readable Penpot export.
    #[error("Penpot: {0}")]
    Corrupt(String),
    /// Writing failed.
    #[error("{0}")]
    Io(String),
}

fn corrupt(msg: impl Into<String>) -> Error {
    Error::Corrupt(msg.into())
}

/// Whether `s` has the shape of a UUID. Ids are checked before they are used in member paths.
pub fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i, b)| if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() })
}

/// Whether a member name is safe to look up: relative, `/`-separated, no `.` or `..` parts and no
/// control characters. Nothing is ever extracted to disk, but an ambiguous name can make one
/// member be inspected and another be read.
fn safe_name(name: &str) -> bool {
    !(name.starts_with('/') || name.contains('\\') || name.split('/').any(|part| part == "." || part == "..") || name.chars().any(char::is_control))
}

/// Whether one archive's byte budget (counted by the archive, of bytes actually read) has run out,
/// and its JSON structure budget, independent of the serialized size of dense arrays and objects.
struct Budget {
    exceeded: bool,
    json_values: usize,
    json_exceeded: bool,
}

impl Budget {
    fn check(&self) -> Result<(), Error> {
        if self.exceeded {
            Err(corrupt("actual decompressed ZIP contents exceed 1 GiB"))
        } else if self.json_exceeded {
            Err(corrupt("JSON structure exceeds import value budget"))
        } else {
            Ok(())
        }
    }

    /// Admit JSON before serde allocates its tree. Outside strings, each container, comma and
    /// colon covers at least one value or object key; counting one root as well deliberately
    /// overestimates structure. Serde still validates syntax.
    fn parse_json(&mut self, data: &[u8], path: &str) -> Result<Value, Error> {
        self.check()?;
        let mut values = 1;
        let (mut quoted, mut escaped) = (false, false);
        for &byte in data {
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
            } else if byte == b'"' {
                quoted = true;
            } else if matches!(byte, b'[' | b'{' | b',' | b':') {
                values += 1;
            }
        }
        if values > self.json_values {
            self.json_exceeded = true;
            return Err(corrupt("JSON structure exceeds import value budget"));
        }
        self.json_values -= values;
        serde_json::from_slice(data).map_err(|e| corrupt(format!("{path}: {e}")))
    }
}

/// One entry of the manifest's `files`.
#[derive(Clone, Debug, PartialEq)]
pub struct FileEntry {
    /// The file's UUID: its members are under `files/<id>`.
    pub id: String,
    pub name: Option<String>,
}

/// An opened Penpot export.
pub struct Archive<'b> {
    /// Counts the bytes read against [`MAX_DECOMPRESSED`].
    zip: craft_zip::Archive<'b>,
    /// Member names in archive order.
    names: Vec<String>,
    budget: Budget,
    /// `manifest.json`.
    pub manifest: Value,
    /// The manifest's files, in its order (1 to [`MAX_FILES`]).
    pub files: Vec<FileEntry>,
}

impl<'b> Archive<'b> {
    /// Open `bytes`: the ZIP is validated whole (size, member count, duplicate and unsafe names,
    /// declared unpacked total) before any JSON is decoded, then the manifest is read and checked.
    pub fn open(bytes: &'b [u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_ARCHIVE {
            return Err(corrupt("archive exceeds 512 MiB"));
        }
        let mut zip = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: MAX_ENTRIES as u64, max_total_bytes: MAX_DECOMPRESSED })
            .map_err(|e| match e {
                craft_zip::Error::TooManyEntries { .. } | craft_zip::Error::TooManyEndRecords(_) => corrupt("too many ZIP members"),
                e => corrupt(format!("invalid ZIP: {e}")),
            })?;
        // The ZIP reader keeps one member per name: reject duplicates instead of guessing which
        // version is real.
        if zip.has_duplicate_names() {
            return Err(corrupt("duplicate ZIP member names"));
        }
        // Reject oversized logical archives before decoding any JSON. This guards against ZIP bombs
        // whose compressed size is innocuous but whose advertised uncompressed size is enormous.
        let names: Vec<String> = zip.names().map(|n| n.into_owned()).collect();
        let mut expanded = 0_u64;
        for name in &names {
            if !safe_name(name) {
                return Err(corrupt("unsafe ZIP member path"));
            }
            expanded = expanded.saturating_add(zip.declared_size(name).unwrap_or(0));
            if expanded > MAX_DECOMPRESSED {
                return Err(corrupt("ZIP contents exceed 1 GiB"));
            }
        }
        let budget = Budget { exceeded: false, json_values: MAX_JSON_VALUES, json_exceeded: false };
        let mut archive = Archive { zip, names, budget, manifest: Value::Null, files: Vec::new() };
        let manifest = archive.json("manifest.json")?;
        if manifest.get("type").and_then(Value::as_str) != Some(MANIFEST_TYPE) {
            return Err(corrupt("not a Penpot export-files archive"));
        }
        if manifest.get("version").and_then(Value::as_u64) != Some(1) {
            return Err(corrupt("unsupported Penpot export version"));
        }
        let files = manifest.get("files").and_then(Value::as_array).ok_or_else(|| corrupt("no files in manifest"))?;
        if files.is_empty() || files.len() > MAX_FILES {
            return Err(corrupt("invalid number of Penpot files"));
        }
        for file in files {
            let Some(id) = file.get("id").and_then(Value::as_str).filter(|id| is_uuid(id)) else {
                return Err(corrupt("invalid file UUID"));
            };
            archive.files.push(FileEntry { id: id.to_owned(), name: file.get("name").and_then(Value::as_str).map(str::to_owned) });
        }
        archive.manifest = manifest;
        Ok(archive)
    }

    /// Fails once a read ran past [`MAX_DECOMPRESSED`] or [`MAX_JSON_VALUES`]. A caller that
    /// treats some members as optional (and so drops their errors) calls this afterwards, so that
    /// an exhausted budget is never swallowed.
    pub fn check(&self) -> Result<(), Error> {
        self.budget.check()
    }

    /// Bytes this archive may still unpack.
    pub fn bytes_left(&self) -> u64 {
        self.zip.budget().remaining()
    }

    /// Whether the archive has member `path`.
    pub fn has(&self, path: &str) -> bool {
        self.zip.contains(path)
    }

    /// Every member name, in archive order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The members `<dir>/<uuid>.json`, in archive order, at most `max` of them. `Err` when there
    /// are more: the caller decides whether that fails the file or truncates the list.
    pub fn uuid_members(&self, dir: &str, max: usize) -> Result<Vec<String>, Vec<String>> {
        let found: Vec<String> = self
            .names
            .iter()
            .filter(|name| {
                name.strip_prefix(dir)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .is_some_and(|part| part.len() == 41 && part.ends_with(".json") && part.get(..36).is_some_and(is_uuid))
            })
            .take(max.saturating_add(1))
            .cloned()
            .collect();
        if found.len() > max { Err(found.into_iter().take(max).collect()) } else { Ok(found) }
    }

    /// JSON member `path` read as a record of [`model`] (or any type serde reads), within the
    /// same budgets as [`Self::json`]. A field of the wrong type fails the read.
    pub fn typed<T: serde::de::DeserializeOwned>(&mut self, path: &str) -> Result<T, Error> {
        let value = self.json(path)?;
        serde_json::from_value(value).map_err(|e| corrupt(format!("{path}: {e}")))
    }

    /// JSON member `path`, within [`MAX_JSON`] and the archive's budgets.
    pub fn json(&mut self, path: &str) -> Result<Value, Error> {
        let data = self.read(path, MAX_JSON, MAX_JSON, || format!("JSON member too large: {path}"))?;
        self.budget.parse_json(&data, path)
    }

    /// Member `path` read whole when it is at most `declared_max` bytes by its header and at most
    /// `max` bytes read.
    fn read(&mut self, path: &str, declared_max: u64, max: u64, too_large: impl Fn() -> String) -> Result<Vec<u8>, Error> {
        self.budget.check()?;
        let declared = self.zip.declared_size(path).ok_or_else(|| corrupt(format!("{path}: not in the archive")))?;
        if declared > declared_max {
            return Err(corrupt(too_large()));
        }
        self.read_charged(path, declared, max, || corrupt(too_large()))
    }

    /// Member `path` (declaring `declared` bytes) up to `max` bytes. Running out of the archive's
    /// byte budget fails every later [`Archive::check`]; a read that fails otherwise is charged
    /// its declared size (the read stops one byte past it), so it is never free to repeat.
    fn read_charged(&mut self, path: &str, declared: u64, max: u64, too_large: impl Fn() -> Error) -> Result<Vec<u8>, Error> {
        match self.zip.read(path, max) {
            Ok(Some(data)) => Ok(data),
            Ok(None) => Err(corrupt(format!("{path}: not in the archive"))),
            Err(craft_zip::Error::EntryTooLarge { .. }) => Err(too_large()),
            Err(craft_zip::Error::BudgetSpent { .. }) => {
                self.budget.exceeded = true;
                Err(corrupt("Penpot decompression budget exhausted"))
            }
            Err(e) => {
                let charge = declared.min(self.zip.budget().remaining());
                let _ = self.zip.budget_mut().charge(charge);
                Err(corrupt(e.to_string()))
            }
        }
    }

    /// Shape JSON member `path`, within [`MAX_JSON`] and what is left of `page_budget` (bytes of
    /// shape JSON for its page, counted down).
    pub fn shape_json(&mut self, path: &str, page_budget: &mut u64) -> Result<Value, Error> {
        self.budget.check()?;
        let declared = self.zip.declared_size(path).ok_or_else(|| corrupt(format!("{path}: not in the archive")))?;
        if declared > MAX_JSON {
            return Err(corrupt(format!("JSON member too large: {path}")));
        }
        if declared > *page_budget {
            return Err(corrupt("page shapes exceed byte budget"));
        }
        let data = self.read_charged(path, declared, MAX_JSON.min(*page_budget), || corrupt(format!("JSON member too large: {path}")))?;
        *page_budget -= data.len() as u64;
        self.budget.parse_json(&data, path)
    }

    /// The pages of file `file_id` (`files/<file>/pages/<uuid>.json`), sorted by their `index`
    /// (pages without one last, ties in archive order). Fails past [`MAX_PAGES`].
    pub fn pages(&mut self, file_id: &str) -> Result<Vec<Value>, Error> {
        let paths = self.uuid_members(&format!("files/{file_id}/pages"), MAX_PAGES).map_err(|_| corrupt("too many pages"))?;
        let mut pages = Vec::with_capacity(paths.len());
        for path in paths {
            pages.push(self.json(&path)?);
        }
        pages.sort_by_key(|p| p.get("index").and_then(Value::as_i64).unwrap_or(i64::MAX));
        Ok(pages)
    }

    /// The shapes of a page (`files/<file>/pages/<page>/<uuid>.json`) by their `id`; a shape
    /// without a UUID `id` is left out. Fails past [`MAX_PAGE_SHAPES`] shapes or
    /// [`MAX_PAGE_SHAPES_BYTES`] of shape JSON.
    pub fn page_shapes(&mut self, file_id: &str, page_id: &str) -> Result<BTreeMap<String, Value>, Error> {
        let paths =
            self.uuid_members(&format!("files/{file_id}/pages/{page_id}"), MAX_PAGE_SHAPES).map_err(|_| corrupt("too many layers on a page"))?;
        let mut shapes = BTreeMap::new();
        let mut page_budget = MAX_PAGE_SHAPES_BYTES;
        for path in paths {
            let shape = self.shape_json(&path, &mut page_budget)?;
            if let Some(id) = shape.get("id").and_then(Value::as_str).filter(|id| is_uuid(id)) {
                shapes.insert(id.to_owned(), shape);
            }
        }
        Ok(shapes)
    }

    /// The bytes of media `media_id` of file `file_id`: `files/<file>/media/<id>.json` names the
    /// stored object (`mediaId`) and its type (`mtype`), and the bytes are
    /// `objects/<object>.<ext>` (`jpg`, `webp` or `png`). `None` when either is missing, the ids
    /// are not UUIDs, or the object is larger than `max` bytes or [`MAX_IMAGE`].
    pub fn media(&mut self, file_id: &str, media_id: &str, max: u64) -> Option<Vec<u8>> {
        if !is_uuid(file_id) || !is_uuid(media_id) {
            return None;
        }
        let meta = self.json(&format!("files/{file_id}/media/{media_id}.json")).ok()?;
        let object = meta.get("mediaId").and_then(Value::as_str).filter(|id| is_uuid(id))?;
        let extension = match meta.get("mtype").and_then(Value::as_str) {
            Some("image/jpeg") => "jpg",
            Some("image/webp") => "webp",
            Some("image/png") => "png",
            _ => "unknown",
        };
        self.read(&format!("objects/{object}.{extension}"), MAX_IMAGE, max.min(MAX_IMAGE), String::new).ok()
    }

    /// How many members under `objects/`, `fonts/` and `assets/fonts/` start with a font
    /// signature (TrueType, OpenType, a collection, WOFF or WOFF2). Four bytes of each are read;
    /// no payload is decoded.
    pub fn embedded_fonts(&mut self) -> usize {
        let names: Vec<String> =
            self.names.iter().filter(|n| n.starts_with("objects/") || n.starts_with("fonts/") || n.starts_with("assets/fonts/")).cloned().collect();
        let mut count = 0;
        for name in names {
            match self.zip.read_head(&name, 4) {
                Ok(Some(header)) if matches!(header.as_slice(), b"OTTO" | b"true" | b"ttcf" | b"wOFF" | b"wOF2" | [0, 1, 0, 0]) => count += 1,
                Err(craft_zip::Error::BudgetSpent { .. }) => self.budget.exceeded = true,
                _ => {}
            }
        }
        count
    }
}

/// A `manifest.json` for `files` (id, name, the file's `features`), saying which program wrote it.
pub fn manifest(generated_by: &str, files: &[(&str, &str, Value)]) -> Value {
    let files: Vec<Value> = files.iter().map(|(id, name, features)| serde_json::json!({"id": id, "name": name, "features": features})).collect();
    serde_json::json!({"type": MANIFEST_TYPE, "version": 1, "generatedBy": generated_by, "refer": "penpot", "files": files, "relations": []})
}

/// A Penpot archive of `members` (path, bytes), deflated, in the order given. Paths must be safe
/// and distinct, as [`Archive::open`] requires of what it reads.
pub fn write_archive<'a>(members: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Result<Vec<u8>, Error> {
    let mut z = craft_zip::Writer::new();
    let mut seen = BTreeSet::new();
    for (path, bytes) in members {
        if path.is_empty() || !safe_name(path) || !seen.insert(path) {
            return Err(Error::Io(format!("unsafe or repeated member path: {}", path.chars().take(200).collect::<String>())));
        }
        z.add(path, bytes, craft_zip::Compression::Level(3)).map_err(|e| Error::Io(e.to_string()))?;
    }
    z.finish().map_err(|e| Error::Io(e.to_string()))
}
