//! The file container: a ZIP package of named parts (the file's JSON, binary payloads, optional
//! caches), never extracted to filesystem paths. Payloads are content-addressed blobs,
//! `blobs/<sha256>.<extension>`, stored once however often they are referred to. Every size is
//! bounded by the app's [`Limits`]: the file, each part, all parts expanded, the payloads resolved,
//! and the number of entries; a part is read no further than its declared size (a decompression
//! bomb stops there). What a part holds, and how a payload is encoded, is the app's.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::FormatError;

/// How hard the payloads are compressed: Fast favours interactive file operations; Compact trades
/// CPU time for space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compression {
    #[default]
    Fast,
    Compact,
}

/// An app's budgets for its containers. The functions are read each time they are needed, so a
/// limit the app lets the user raise takes effect at once.
#[derive(Clone, Copy)]
pub struct Limits {
    /// The most entries a container holds.
    pub max_entries: usize,
    /// The most bytes all parts may expand to, and all payloads may resolve to.
    pub max_expanded: fn() -> usize,
    /// The most bytes a part of this name may expand to.
    pub part_limit: fn(&str) -> usize,
    /// The Deflate level of a compressed part of this name and size.
    pub level: fn(&str, usize) -> i64,
    /// Whether a container of this many bytes may be read, or why not.
    pub check_size: fn(u64) -> Result<(), String>,
    /// Whether the work is cancelled (a save the person stopped): checked before each part, and
    /// a long part is written a piece at a time and stops before the next piece when this says
    /// so ([`FormatError::Cancelled`]). `None`: it cannot be cancelled here.
    pub cancelled: Option<fn() -> bool>,
}

/// The most bytes of a part written between two cancellation checks.
const PIECE: usize = 1 << 20;

fn invalid(message: impl std::fmt::Display) -> FormatError {
    FormatError::Invalid(message.to_string())
}

/// Whether the bytes start as a container does, whatever the file is called.
pub fn is_container(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

/// The SHA-256 of the bytes.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn hex(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for byte in digest {
        // Writing to a `String` cannot fail.
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

/// The SHA-256 of the bytes, in lowercase hexadecimal: a blob's name, and how a cache is keyed to
/// the file it was written for.
pub fn digest(bytes: &[u8]) -> String {
    hex(&sha256(bytes))
}

/// One part a [`Writer`] wrote, with the SHA-256 of its uncompressed content. A caller that signs
/// a container uses these to bind the parts without reading the file back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartDigest {
    pub name: String,
    pub len: u64,
    pub digest: [u8; 32],
}

/// A container being written.
pub struct Writer {
    zip: craft_zip::Writer,
    names: BTreeSet<String>,
    expanded: usize,
    resolved: usize,
    limits: Limits,
    /// Which written parts to record for a signature, and the records so far.
    tracked: Option<TrackedParts>,
}

/// A tracking closure and the part digests collected so far.
type TrackedParts = (fn(&str) -> bool, Vec<PartDigest>);

impl Writer {
    pub fn new(limits: Limits) -> Writer {
        Writer { zip: craft_zip::Writer::new(), names: BTreeSet::new(), expanded: 0, resolved: 0, limits, tracked: None }
    }

    /// Record the parts written from now on whose name `include` accepts, with their SHA-256, for
    /// [`Writer::take_parts`]. Used by a caller that signs the container.
    pub fn track_parts(&mut self, include: fn(&str) -> bool) {
        self.tracked = Some((include, Vec::new()));
    }

    /// The tracked parts written so far, oldest first, and stop tracking.
    pub fn take_parts(&mut self) -> Option<Vec<PartDigest>> {
        self.tracked.take().map(|(_, parts)| parts)
    }

    /// The container's bytes.
    pub fn finish(self) -> Result<Vec<u8>, FormatError> {
        self.zip.finish().map_err(invalid)
    }

    /// Add a part, compressed or stored. A name already written is written once.
    pub fn part(&mut self, name: &str, bytes: &[u8], compressed: bool) -> Result<(), FormatError> {
        self.part_with_digest(name, bytes, compressed, None)
    }

    /// [`Writer::part`] with a digest the caller already worked out (a blob's name is its digest),
    /// so a signed save does not hash the same bytes twice.
    fn part_with_digest(&mut self, name: &str, bytes: &[u8], compressed: bool, digest: Option<[u8; 32]>) -> Result<(), FormatError> {
        if bytes.len() > (self.limits.part_limit)(name) {
            return Err(invalid("part too large"));
        }
        if self.names.contains(name) {
            return Ok(());
        }
        // Check the counters this part would leave, and commit them only once it is written: a
        // refused part is not remembered as written.
        let expanded = self.expanded.checked_add(bytes.len()).ok_or_else(|| invalid("expanded size overflow"))?;
        if expanded > (self.limits.max_expanded)() || self.names.len().saturating_add(1) > self.limits.max_entries {
            return Err(invalid("package too large"));
        }
        let cancelled = self.limits.cancelled;
        if cancelled.is_some_and(|c| c()) {
            return Err(FormatError::Cancelled);
        }
        // the configured level, clamped to the Deflate levels craft-zip writes (1 to 9)
        let compression = if compressed {
            craft_zip::Compression::Level((self.limits.level)(name, bytes.len()).clamp(1, 9) as u8)
        } else {
            craft_zip::Compression::Stored
        };
        match cancelled {
            Some(cancelled) if bytes.len() > PIECE => {
                let mut stopped = false;
                let written = self.zip.add_with(name, compression, |out| {
                    for piece in bytes.chunks(PIECE) {
                        if cancelled() {
                            stopped = true;
                            return Err(std::io::Error::other("cancelled"));
                        }
                        out.write_all(piece)?;
                    }
                    Ok(())
                });
                if stopped {
                    return Err(FormatError::Cancelled);
                }
                written.map_err(invalid)?;
            }
            _ => self.zip.add(name, bytes, compression).map_err(invalid)?,
        }
        self.names.insert(name.to_owned());
        self.expanded = expanded;
        if let Some((include, parts)) = &mut self.tracked
            && include(name)
        {
            let digest = digest.unwrap_or_else(|| sha256(bytes));
            parts.push(PartDigest { name: name.to_owned(), len: bytes.len() as u64, digest });
        }
        Ok(())
    }

    /// Would `entries` more optional parts of `extra` bytes, stored uncompressed, still keep the
    /// file within `budget` and its expanded size within the reader's budget? The parts already
    /// written count at their expanded size (an upper bound on what they take in the file).
    pub fn room(&self, entries: usize, extra: usize, budget: usize) -> bool {
        self.remaining(entries, budget).is_some_and(|left| extra <= left)
    }

    /// The bytes `entries` more optional parts may hold together ([`Writer::room`]).
    pub fn space(&self, entries: usize, budget: usize) -> usize {
        self.remaining(entries, budget).unwrap_or(0)
    }

    /// The bytes left for `entries` more parts, or `None` when the entries don't fit or what is
    /// written already is past `budget` or the reader's budget.
    fn remaining(&self, entries: usize, budget: usize) -> Option<usize> {
        let count = self.names.len().checked_add(entries).filter(|&n| n <= self.limits.max_entries)?;
        let overhead = count.saturating_mul(256).saturating_add(65_536);
        let in_file = budget.checked_sub(overhead.saturating_add(self.expanded))?;
        let expanded = (self.limits.max_expanded)().checked_sub(self.expanded)?;
        Some(in_file.min(expanded))
    }

    /// Add a payload as a blob, `blobs/<sha256>.<extension>`, and say its name, for the JSON that
    /// refers to it.
    pub fn blob(&mut self, bytes: &[u8], extension: &str, compressed: bool) -> Result<Value, FormatError> {
        let resolved = self
            .resolved
            .checked_add(bytes.len())
            .filter(|&n| n <= (self.limits.max_expanded)())
            .ok_or_else(|| invalid("resolved payloads too large"))?;
        let digest = sha256(bytes);
        let name = format!("blobs/{}.{extension}", hex(&digest));
        self.part_with_digest(&name, bytes, compressed, Some(digest))?;
        self.resolved = resolved;
        Ok(Value::String(name))
    }
}

/// A container being read. Opening reads all bytes into memory and checks the directory
/// (the entry count, unique and plain names, every part's
/// declared size), not the parts.
pub struct Reader {
    archive: craft_zip::Archive<'static>,
    blobs: BTreeMap<String, Vec<u8>>,
    /// The bytes of payloads resolved so far, repeats counted: bounded by the expanded budget.
    pub hydrated: usize,
    document: Option<Arc<[u8]>>,
    limits: Limits,
    /// Every byte part reads have expanded to, whatever the budget was at the time.
    read_total: u64,
}

impl Reader {
    pub fn new(mut source: impl Read + Seek, limits: Limits) -> Result<Self, FormatError> {
        let length = source.seek(SeekFrom::End(0)).map_err(invalid)?;
        (limits.check_size)(length).map_err(invalid)?;

        // Read all bytes into memory for the archive, no more than the length just approved: a file
        // that grew since is refused rather than read past its budget.
        source.seek(SeekFrom::Start(0)).map_err(invalid)?;
        let capacity = usize::try_from(length).map_err(|_| invalid("file too large"))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(capacity).map_err(|_| invalid("file too large"))?;
        source.take(length.saturating_add(1)).read_to_end(&mut bytes).map_err(invalid)?;
        if bytes.len() as u64 != length {
            return Err(invalid("the file changed while it was read"));
        }

        let mut archive = craft_zip::Archive::open_owned(
            bytes,
            craft_zip::Limits { max_entries: limits.max_entries as u64, max_total_bytes: (limits.max_expanded)() as u64 },
        )
        .map_err(invalid)?;
        // the directory lists distinct names, so a repeated name is caught here
        if archive.has_duplicate_names() {
            return Err(invalid("duplicate or invalid archive entry"));
        }

        // Validate entry names match format rules: no empty, no backslashes,
        // no path components that are "." or "..", and no duplicates; and every part's declared
        // size within its configured limit.
        let mut name_set = BTreeSet::new();
        for entry_idx in 0..archive.len() {
            // an entry whose name can't be read would be left out of the package unnoticed
            let Ok(Some(name)) = archive.entry_name(entry_idx) else {
                return Err(invalid("duplicate or invalid archive entry"));
            };
            {
                if name.is_empty()
                    || name.contains('\\')
                    || name.split('/').any(|c| c.is_empty() || c == "." || c == "..")
                    || !name_set.insert(name.clone())
                {
                    return Err(invalid("duplicate or invalid archive entry"));
                }
                if archive.declared_size(&name).is_some_and(|size| size > (limits.part_limit)(&name) as u64) {
                    return Err(invalid("part too large"));
                }
            }
        }

        Ok(Self { archive, blobs: BTreeMap::new(), hydrated: 0, document: None, limits, read_total: 0 })
    }

    /// The file's own part, `document.json`, read once and kept: provisional caches and the
    /// authoritative reading are bound to the same bytes, even if another process overwrites the
    /// open file in place. Blobs remain hash-checked.
    pub fn document(&mut self, limit: usize) -> Result<Arc<[u8]>, FormatError> {
        if self.document.is_none() {
            let bytes = self.limited_part("document.json", limit)?;
            self.document = Some(bytes.into());
        }
        // a cached document still keeps the current part limit
        let limit = limit.min((self.limits.part_limit)("document.json"));
        self.document.as_ref().filter(|d| d.len() <= limit).cloned().ok_or_else(|| invalid("metadata budget"))
    }

    /// Whether the container has a part of this name.
    pub fn has_part(&self, name: &str) -> bool {
        self.archive.contains(name)
    }

    /// The names of every part, in the container's order.
    pub fn part_names(&self) -> Vec<String> {
        self.archive.names().map(|n| n.into_owned()).collect()
    }

    /// The part of this name, at most `limit` bytes and never more than its configured limit.
    pub fn limited_part(&mut self, name: &str, limit: usize) -> Result<Vec<u8>, FormatError> {
        let limit = limit.min((self.limits.part_limit)(name));
        let expected = self.archive.declared_size(name).ok_or_else(|| invalid("part not found"))?;
        if expected > limit as u64 {
            return Err(invalid("part too large"));
        }
        // the expanded budget is read each time: a raised (or lowered) one applies at once, the
        // bytes already read still counting against it
        let budget = self.archive.budget_mut();
        *budget = craft_zip::Budget::new(budget.what(), (self.limits.max_expanded)() as u64);
        if budget.charge(self.read_total).is_err() {
            return Err(invalid("package too large"));
        }
        // Read within budget. craft-zip's read() handles decompression bombs via the limit.
        let bytes = self.archive.read(name, limit as u64).map_err(invalid)?.ok_or_else(|| invalid("part not found"))?;
        if bytes.len() as u64 != expected {
            return Err(invalid("part length mismatch"));
        }
        self.read_total = self.read_total.saturating_add(expected);
        Ok(bytes)
    }

    /// The blob `value`'s `field` names, of this extension, within its part limit.
    pub fn blob(&mut self, value: &Value, field: &str, extension: &str) -> Result<&[u8], FormatError> {
        let limit = (self.limits.part_limit)(&format!("blob.{extension}"));
        self.limited_blob(value, field, extension, limit)
    }

    /// [`Reader::blob`] within `limit` bytes. Its hash is checked when it is first read, and every
    /// reference counts against the resolved budget.
    pub fn limited_blob(&mut self, value: &Value, field: &str, extension: &str, limit: usize) -> Result<&[u8], FormatError> {
        let name = value.get(field).and_then(Value::as_str).ok_or_else(|| invalid(format!("missing `{field}` reference")))?;
        // a cached blob still keeps the part's current limit
        let limit = limit.min((self.limits.part_limit)(name));
        let hash = name
            .strip_prefix("blobs/")
            .and_then(|n| n.strip_suffix(&format!(".{extension}")))
            .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
            .ok_or_else(|| invalid("invalid blob reference"))?;
        if !self.blobs.contains_key(name) {
            let bytes = self.limited_part(name, limit)?;
            if digest(&bytes) != hash {
                return Err(invalid(format!("blob hash mismatch: {name}")));
            }
            self.blobs.insert(name.to_owned(), bytes);
        }
        let bytes = self.blobs.get(name).ok_or_else(|| invalid("missing blob"))?;
        if bytes.len() > limit {
            return Err(invalid("blob exceeds its resource budget"));
        }
        // Count repeated references too: a tiny archive must not expand into unbounded owned
        // strings or arrays when the same blob is referenced repeatedly.
        self.hydrated = self
            .hydrated
            .checked_add(bytes.len())
            .filter(|&n| n <= (self.limits.max_expanded)())
            .ok_or_else(|| invalid("resolved payloads too large"))?;
        Ok(bytes)
    }
}

/// A small PNG picture of the file, kept in the container as a disposable cache keyed to the
/// exact file it shows: `cache/thumbnail.json` (its key) and `cache/thumbnail.png`. Reading it
/// never resolves a payload; a picture that isn't one, or is stale, is a miss.
#[derive(Clone, Copy, Debug)]
pub struct Thumbnail {
    /// The key's format tag.
    pub format: &'static str,
    /// The build that wrote it: only the same build reads it back.
    pub app: &'static str,
    /// Its width and height in pixels.
    pub size: [usize; 2],
    /// The most bytes it may have.
    pub max_bytes: usize,
}

impl Thumbnail {
    /// Whether the bytes start as a PNG of this size within the byte budget (not a decode: a
    /// consumer decodes with its own dimension and allocation limits).
    pub fn valid(&self, bytes: &[u8]) -> bool {
        bytes.len() <= self.max_bytes
            && bytes.get(..8) == Some(b"\x89PNG\r\n\x1a\n")
            && bytes.get(8..16) == Some(b"\0\0\0\rIHDR")
            && bytes.get(16..20) == Some((self.size[0] as u32).to_be_bytes().as_slice())
            && bytes.get(20..24) == Some((self.size[1] as u32).to_be_bytes().as_slice())
    }

    /// Add the picture, keyed to `document` (the file's own part), when it is one and there is
    /// room for it within `budget`; otherwise leave it out, never failing the save.
    pub fn write(&self, writer: &mut Writer, png: &[u8], document: &[u8], budget: usize) -> Result<(), FormatError> {
        if !self.valid(png) {
            return Ok(());
        }
        let key =
            serde_json::to_vec(&serde_json::json!({"format": self.format, "app": self.app, "document": digest(document), "payload": digest(png)}))
                .map_err(invalid)?;
        let fits = |name: &str, len: usize| len <= (writer.limits.part_limit)(name);
        if !fits("cache/thumbnail.json", key.len())
            || !fits("cache/thumbnail.png", png.len())
            || !writer.room(2, key.len().saturating_add(png.len()), budget)
        {
            return Ok(());
        }
        writer.part("cache/thumbnail.json", &key, false)?;
        writer.part("cache/thumbnail.png", png, false)
    }

    /// The picture, when this build wrote it for exactly the file's own part (`document.json`,
    /// read within `document_limit`).
    pub fn read(&self, reader: &mut Reader, document_limit: usize) -> Result<Vec<u8>, FormatError> {
        let key: Value = serde_json::from_slice(&reader.limited_part("cache/thumbnail.json", 4096)?).map_err(invalid)?;
        let document = reader.document(document_limit)?;
        if key.get("format").and_then(Value::as_str) != Some(self.format)
            || key.get("app").and_then(Value::as_str) != Some(self.app)
            || key.get("document").and_then(Value::as_str) != Some(digest(&document).as_str())
        {
            return Err(invalid("thumbnail key"));
        }
        let png = reader.limited_part("cache/thumbnail.png", self.max_bytes)?;
        if !self.valid(&png) || key.get("payload").and_then(Value::as_str) != Some(digest(&png).as_str()) {
            return Err(invalid("thumbnail payload"));
        }
        Ok(png)
    }
}

#[cfg(test)]
#[path = "container_tests.rs"]
mod tests;
