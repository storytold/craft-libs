//! Checks and streaming reader for seekable ZIP archives.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};

use crate::entry::{has_traversal_component, has_traversal_component_bytes};
use crate::{Budget, Error, MetadataReader, check_entry_count_seekable};

/// Why an indexed seekable archive exceeded its caller's limits.
#[derive(Debug, thiserror::Error)]
pub enum InspectError {
    #[error("the archive has {actual} entries, more than {limit}")]
    TooManyEntries { actual: usize, limit: usize },
    #[error("declared entry sizes exceed {limit} bytes")]
    DeclaredBytesExceeded { limit: u64 },
    #[error("ZIP entries overlap")]
    OverlappingEntries,
    #[error("ZIP index: {0}")]
    Zip(#[from] zip::result::ZipError),
}

/// Check every declared payload, including entries the caller may ignore, and reject overlapping
/// entry headers and compressed ranges. This is a post-index check; it does not replace a pre-index count guard or
/// bound bytes actually decompressed by a caller.
pub fn inspect_seekable<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, max_entries: usize, max_declared_bytes: u64) -> Result<(), InspectError> {
    if archive.len() > max_entries {
        return Err(InspectError::TooManyEntries { actual: archive.len(), limit: max_entries });
    }
    let mut remaining = max_declared_bytes;
    for index in 0..archive.len() {
        remaining =
            remaining.checked_sub(archive.by_index_raw(index)?.size()).ok_or(InspectError::DeclaredBytesExceeded { limit: max_declared_bytes })?;
    }
    if archive.has_overlapping_entries()? {
        return Err(InspectError::OverlappingEntries);
    }
    Ok(())
}

/// What a [`SeekableArchive`] refuses to exceed. All numbers are the caller's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeekableLimits {
    /// Most entries the archive may declare.
    pub max_entries: u64,
    /// Most bytes of trailing metadata (central directory and end records) read while indexing.
    pub max_metadata_bytes: u64,
    /// Most bytes all the entries read from the archive may expand to together.
    pub max_total_bytes: u64,
}

/// A seekable ZIP archive that indexes only the trailing metadata and streams entry payloads
/// without buffering the archive or large entries in memory.
pub struct SeekableArchive<R: Read + Seek> {
    zip: zip::ZipArchive<MetadataReader<R>>,
    budget: Budget,
    duplicate_names: Vec<String>,
}

const CENTRAL_HEADER: [u8; 4] = *b"PK\x01\x02";
const CENTRAL_HEADER_LEN: usize = 46;

/// Scan central directory records from `cd_start` (bounded by `max_metadata_bytes`) and return
/// each distinct name that appears more than once, lossily decoded as UTF-8.
fn collect_duplicate_names<R: Read + Seek>(reader: &mut R, cd_start: u64, max_metadata_bytes: u64) -> Result<Vec<String>, Error> {
    reader.seek(SeekFrom::Start(cd_start)).map_err(|e| Error::NotZip(e.to_string()))?;
    let mut remaining = max_metadata_bytes;
    let mut seen: HashMap<Vec<u8>, bool> = HashMap::new();
    let mut duplicates = Vec::new();
    loop {
        let mut sig = [0u8; 4];
        match reader.read_exact(&mut sig) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(Error::NotZip(e.to_string())),
        }
        if sig != CENTRAL_HEADER {
            break;
        }
        let mut rest = [0u8; CENTRAL_HEADER_LEN - 4];
        reader.read_exact(&mut rest).map_err(|e| Error::NotZip(e.to_string()))?;
        let name_len = usize::from(u16::from_le_bytes([rest[24], rest[25]]));
        let extra_len = usize::from(u16::from_le_bytes([rest[26], rest[27]]));
        let comment_len = usize::from(u16::from_le_bytes([rest[28], rest[29]]));
        let record_bytes = (CENTRAL_HEADER_LEN + name_len + extra_len + comment_len) as u64;
        remaining = remaining.checked_sub(record_bytes).ok_or_else(|| Error::NotZip("ZIP metadata exceeds the indexing limit".into()))?;
        let mut name_buf = vec![0u8; name_len];
        reader.read_exact(&mut name_buf).map_err(|e| Error::NotZip(e.to_string()))?;
        let skip = i64::try_from(extra_len + comment_len).map_err(|_| Error::NotZip("central directory record too large".into()))?;
        if skip > 0 {
            reader.seek(SeekFrom::Current(skip)).map_err(|e| Error::NotZip(e.to_string()))?;
        }
        match seen.get_mut(&name_buf) {
            None => {
                seen.insert(name_buf, false);
            }
            Some(recorded) if !*recorded => {
                *recorded = true;
                duplicates.push(String::from_utf8_lossy(&name_buf).into_owned());
            }
            Some(_) => {}
        }
    }
    Ok(duplicates)
}

impl<R: Read + Seek> SeekableArchive<R> {
    /// Indexes the trailing metadata of `reader`. The declared entry count is checked before `zip`
    /// allocates for the central directory, and indexing is bounded by
    /// [`SeekableLimits::max_metadata_bytes`].
    pub fn open(mut reader: R, limits: SeekableLimits) -> Result<Self, Error> {
        check_entry_count_seekable(&mut reader, limits.max_entries)?;
        let (meta_reader, indexing) = MetadataReader::new(reader, limits.max_metadata_bytes).map_err(|e| Error::NotZip(e.to_string()))?;
        let zip = zip::ZipArchive::new(meta_reader).map_err(|e| Error::NotZip(e.to_string()))?;
        indexing.finish();
        let found = zip.len() as u64;
        if found > limits.max_entries {
            return Err(Error::TooManyEntries { declared: found, limit: limits.max_entries });
        }
        let (zip, duplicate_names) = if zip.has_duplicate_names() {
            let cd_start = zip.central_directory_start();
            let archive_offset = zip.offset();
            let mut inner = zip.into_inner().into_inner();
            let duplicates = collect_duplicate_names(&mut inner, cd_start, limits.max_metadata_bytes)?;
            let (meta_reader, indexing) = MetadataReader::new(inner, limits.max_metadata_bytes).map_err(|e| Error::NotZip(e.to_string()))?;
            let zip =
                zip::ZipArchive::with_config(zip::read::Config { archive_offset: zip::read::ArchiveOffset::Known(archive_offset) }, meta_reader)
                    .map_err(|e| Error::NotZip(e.to_string()))?;
            indexing.finish();
            (zip, duplicates)
        } else {
            (zip, Vec::new())
        };
        Ok(Self { zip, budget: Budget::new("archive", limits.max_total_bytes), duplicate_names })
    }

    /// The number of distinct entry names.
    pub fn len(&self) -> usize {
        self.zip.len()
    }

    pub fn is_empty(&self) -> bool {
        self.zip.is_empty()
    }

    /// Whether the central directory contains repeated raw names. The strict index keeps one
    /// entry per raw name, so `len` alone cannot distinguish these from unique records.
    pub fn has_duplicate_names(&self) -> bool {
        self.zip.has_duplicate_names()
    }

    /// Whether any name matching `predicate` appears more than once in the central directory.
    pub fn has_duplicate_names_matching(&self, mut predicate: impl FnMut(&str) -> bool) -> bool {
        self.duplicate_names.iter().any(|name| predicate(name))
    }

    /// Distinct names that appear more than once in the central directory.
    pub fn duplicate_names(&self) -> impl Iterator<Item = &str> {
        self.duplicate_names.iter().map(String::as_str)
    }

    /// Whether any entry in the archive contains path-traversal components (`..`, absolute paths,
    /// Windows drive prefixes, null bytes).
    pub fn has_traversal_component(&self) -> bool {
        self.zip.file_names_raw().any(has_traversal_component_bytes)
    }

    /// Locate a surviving entry by the absolute offset of its central directory record,
    /// without decoding its name or reading the local entry header.
    pub fn index_for_central_record(&self, offset: u64) -> Option<usize> {
        (0..self.zip.len()).find(|&i| self.zip.by_index_data(i).is_ok_and(|e| e.central_header_start() == offset))
    }

    /// The name at `index`, copied on demand. Names containing path-traversal components (`..`,
    /// absolute paths, Windows drive prefixes, null bytes) return `None`.
    pub fn entry_name(&mut self, index: usize) -> Result<Option<String>, Error> {
        if index >= self.zip.len() {
            return Ok(None);
        }
        let file = self.zip.by_index_raw(index).map_err(|error| Error::NotZip(error.to_string()))?;
        if has_traversal_component_bytes(file.name_raw()) {
            return Ok(None);
        }
        let name = file.name().map_err(|error| Error::NotZip(error.to_string()))?;
        if has_traversal_component(&name) {
            return Ok(None);
        }
        Ok(Some(name.into_owned()))
    }

    /// The name at `index`, copied on demand without path validation.
    pub fn raw_entry_name(&mut self, index: usize) -> Result<Option<String>, Error> {
        if index >= self.zip.len() {
            return Ok(None);
        }
        let file = self.zip.by_index_raw(index).map_err(|error| Error::NotZip(error.to_string()))?;
        let name = file.name().map_err(|error| Error::NotZip(error.to_string()))?;
        Ok(Some(name.into_owned()))
    }

    /// Whether the entry at `index` is a directory according to ZIP filename conventions.
    pub fn entry_is_dir(&mut self, index: usize) -> Result<Option<bool>, Error> {
        if index >= self.zip.len() {
            return Ok(None);
        }
        let file = self.zip.by_index_raw(index).map_err(|error| Error::NotZip(error.to_string()))?;
        if has_traversal_component_bytes(file.name_raw()) {
            return Ok(None);
        }
        Ok(Some(file.is_dir()))
    }

    /// Declared decoded and compressed sizes at `index`, without name lookup.
    pub fn entry_sizes(&mut self, index: usize) -> Result<Option<(u64, u64)>, Error> {
        if index >= self.zip.len() {
            return Ok(None);
        }
        let file = self.zip.by_index_raw(index).map_err(|e| Error::NotZip(e.to_string()))?;
        if has_traversal_component_bytes(file.name_raw()) {
            return Ok(None);
        }
        Ok(Some((file.size(), file.compressed_size())))
    }

    /// Do two entries' compressed data ranges overlap?
    pub fn has_overlapping_files(&mut self) -> Result<bool, Error> {
        self.zip.has_overlapping_files().map_err(|e| Error::NotZip(e.to_string()))
    }

    /// The entry names in the archive's order, folders included. Non-UTF-8 names and names
    /// containing path-traversal components are omitted.
    pub fn names(&self) -> impl Iterator<Item = Cow<'_, str>> {
        self.zip.file_names().filter_map(Result::ok).filter(|name| !has_traversal_component(name))
    }

    /// Is there an entry `name` (exactly as stored)? Paths containing traversal components
    /// return `false`.
    pub fn contains(&self, name: &str) -> bool {
        if has_traversal_component(name) {
            return false;
        }
        self.zip.index_for_name(name).is_some()
    }

    /// The size entry `name` declares. It is a claim, not a measurement.
    pub fn declared_size(&mut self, name: &str) -> Option<u64> {
        if has_traversal_component(name) {
            return None;
        }
        let index = self.zip.index_for_name(name)?;
        self.zip.by_index_raw(index).ok().map(|f| f.size())
    }

    /// The size entry `name` takes in the archive, compressed.
    pub fn compressed_size(&mut self, name: &str) -> Option<u64> {
        if has_traversal_component(name) {
            return None;
        }
        let index = self.zip.index_for_name(name)?;
        self.zip.by_index_raw(index).ok().map(|f| f.compressed_size())
    }

    /// Is `name` stored without compression?
    pub fn is_stored(&mut self, name: &str) -> Option<bool> {
        if has_traversal_component(name) {
            return None;
        }
        let index = self.zip.index_for_name(name)?;
        self.zip.by_index_raw(index).ok().map(|f| f.compression() == zip::CompressionMethod::Stored)
    }

    /// Open entry `name` for streaming reads, or `None` when there is no such entry.
    ///
    /// The entry may declare and expand to at most `max` bytes, and at most what is left of
    /// [`SeekableLimits::max_total_bytes`]. The returned [`EntryReader`] enforces the size cap and
    /// verifies the CRC-32 on EOF, charging [`Self::budget`] only when the full entry is read.
    pub fn open_entry(&mut self, name: &str, max: u64) -> Result<Option<EntryReader<'_, R>>, Error> {
        if has_traversal_component(name) {
            return Ok(None);
        }
        let Some(index) = self.zip.index_for_name(name) else { return Ok(None) };
        self.open_entry_index_named(index, name, max)
    }

    /// Open the entry at `index` for streaming reads under the same rules as [`Self::open_entry`].
    pub fn open_entry_index(&mut self, index: usize, max: u64) -> Result<Option<EntryReader<'_, R>>, Error> {
        if index >= self.zip.len() {
            return Ok(None);
        }
        let Some(name) = self.entry_name(index)? else {
            return Ok(None);
        };
        self.open_entry_index_named(index, &name, max)
    }

    fn open_entry_index_named(&mut self, index: usize, name: &str, max: u64) -> Result<Option<EntryReader<'_, R>>, Error> {
        if has_traversal_component(name) {
            return Ok(None);
        }
        let file = self.zip.by_index(index).map_err(|e| Error::Entry { name: name.to_string(), msg: e.to_string() })?;
        let declared = file.size();
        if declared > max {
            return Err(Error::EntryTooLarge { name: name.to_string(), limit: max });
        }
        if declared > self.budget.remaining() {
            return Err(Error::BudgetSpent { name: name.to_string(), limit: self.budget.limit() });
        }
        Ok(Some(EntryReader { file, budget: &mut self.budget, declared, limit: declared, read: 0, finished: false }))
    }

    /// Read all bytes of entry `name` into memory, or `None` when there is no such entry.
    pub fn read(&mut self, name: &str, max: u64) -> Result<Option<Vec<u8>>, Error> {
        if has_traversal_component(name) {
            return Ok(None);
        }
        let Some(index) = self.zip.index_for_name(name) else { return Ok(None) };
        self.read_index_named(index, name, max)
    }

    /// Read all bytes of the entry at `index` into memory.
    pub fn read_index(&mut self, index: usize, max: u64) -> Result<Option<Vec<u8>>, Error> {
        if index >= self.zip.len() {
            return Ok(None);
        }
        let Some(name) = self.entry_name(index)? else {
            return Ok(None);
        };
        self.read_index_named(index, &name, max)
    }

    fn read_index_named(&mut self, index: usize, name: &str, max: u64) -> Result<Option<Vec<u8>>, Error> {
        let Some(mut reader) = self.open_entry_index_named(index, name, max)? else { return Ok(None) };
        let mut out = Vec::with_capacity(usize::try_from(reader.declared).unwrap_or(64 * 1024).min(64 * 1024));
        reader.read_to_end(&mut out).map_err(|e| Error::Entry { name: name.to_string(), msg: e.to_string() })?;
        Ok(Some(out))
    }

    /// The total budget: what the entries read so far have used, and what is left.
    pub fn budget(&self) -> &Budget {
        &self.budget
    }

    /// The total budget, for a caller with its own charging rule.
    pub fn budget_mut(&mut self) -> &mut Budget {
        &mut self.budget
    }
}

/// Streaming reader for a single entry opened from [`SeekableArchive`].
///
/// Enforces the declared and caller-provided byte limits and charges the archive's [`Budget`]
/// once the full entry has been read and its CRC-32 verified.
pub struct EntryReader<'a, R: Read> {
    file: zip::read::ZipFile<'a, MetadataReader<R>>,
    budget: &'a mut Budget,
    declared: u64,
    limit: u64,
    read: u64,
    finished: bool,
}

impl<R: Read> EntryReader<'_, R> {
    /// The uncompressed size declared by the entry header.
    pub fn declared_size(&self) -> u64 {
        self.declared
    }
}

impl<R: Read> Read for EntryReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.finished || buf.is_empty() {
            return Ok(0);
        }
        let remaining = self.limit.saturating_sub(self.read);
        let max_read = usize::try_from(remaining.saturating_add(1)).unwrap_or(usize::MAX).min(buf.len());
        let n = self.file.read(&mut buf[..max_read])?;
        if n == 0 {
            if self.read < self.declared {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("entry ended after {} bytes, declared {}", self.read, self.declared),
                ));
            }
            self.budget.charge(self.read).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            self.finished = true;
            return Ok(0);
        }
        let next = self.read.saturating_add(n as u64);
        if next > self.limit {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("entry exceeds declared size of {} bytes", self.limit)));
        }
        self.read = next;
        Ok(n)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[test]
    fn counts_all_declared_entries_and_detects_aliases() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        writer.start_file("first", options).unwrap();
        writer.write_all(b"payload").unwrap();
        writer.shallow_copy_file("first", "alias").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert!(matches!(inspect_seekable(&mut archive, 1, 14), Err(InspectError::TooManyEntries { .. })));
        assert!(matches!(inspect_seekable(&mut archive, 2, 13), Err(InspectError::DeclaredBytesExceeded { .. })));
        assert!(matches!(inspect_seekable(&mut archive, 2, 14), Err(InspectError::OverlappingEntries)));
    }

    #[test]
    fn empty_aliases_still_overlap_at_the_header() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("empty", zip::write::SimpleFileOptions::default()).unwrap();
        writer.shallow_copy_file("empty", "alias").unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(writer.finish().unwrap().into_inner())).unwrap();
        assert!(matches!(inspect_seekable(&mut archive, 2, 0), Err(InspectError::OverlappingEntries)));
    }

    #[test]
    fn permits_exact_limits_and_empty_archive() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("a", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(b"123").unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(writer.finish().unwrap().into_inner())).unwrap();
        assert!(inspect_seekable(&mut archive, 1, 3).is_ok());
        let empty = zip::ZipWriter::new(Cursor::new(Vec::new())).finish().unwrap().into_inner();
        assert!(inspect_seekable(&mut zip::ZipArchive::new(Cursor::new(empty)).unwrap(), 0, 0).is_ok());
    }

    #[test]
    fn seekable_archive_streams_stored_and_deflated_entries_within_limits() {
        let mut writer = crate::Writer::new();
        let big: Vec<u8> = (0..50_000u32).flat_map(|i| (i % 251).to_le_bytes()).collect();
        writer.add("a.bin", &big, crate::Compression::Deflated).unwrap();
        writer.add("b.bin", b"hello stored", crate::Compression::Stored).unwrap();
        let bytes = writer.finish().unwrap();

        let limits = SeekableLimits { max_entries: 10, max_metadata_bytes: 16_384, max_total_bytes: 250_000 };
        let mut archive = SeekableArchive::open(Cursor::new(bytes), limits).unwrap();
        assert_eq!(archive.len(), 2);
        assert!(archive.contains("a.bin"));
        assert_eq!(archive.declared_size("a.bin"), Some(big.len() as u64));

        // Stream in small chunks without loading via Vec in one shot.
        let mut reader = archive.open_entry("a.bin", big.len() as u64).unwrap().unwrap();
        let mut out = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = reader.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            out.extend_from_slice(&chunk[..n]);
        }
        assert_eq!(out, big);
        drop(reader);
        assert_eq!(archive.budget().used(), big.len() as u64);

        let b = archive.read("b.bin", 100).unwrap().unwrap();
        assert_eq!(b, b"hello stored");
        assert_eq!(archive.budget().used(), big.len() as u64 + 12);
    }

    #[test]
    fn seekable_archive_enforces_entry_count_size_budget_and_crc() {
        let sample = crate::zip64_entry_count_sample(20_001);
        let err = SeekableArchive::open(
            Cursor::new(sample),
            SeekableLimits { max_entries: 20_000, max_metadata_bytes: 16 << 20, max_total_bytes: 1 << 20 },
        )
        .err()
        .unwrap();
        assert!(matches!(err, Error::TooManyEntries { declared: 20_001, limit: 20_000 }), "{err:?}");

        let mut writer = crate::Writer::new();
        writer.add("cat.lrcat", &[7u8; 1024], crate::Compression::Deflated).unwrap();
        let bytes = writer.finish().unwrap();

        // Entry larger than per-entry max is refused before reading.
        let mut archive =
            SeekableArchive::open(Cursor::new(&bytes), SeekableLimits { max_entries: 10, max_metadata_bytes: 16_384, max_total_bytes: 2048 })
                .unwrap();
        assert!(matches!(archive.open_entry("cat.lrcat", 1023), Err(Error::EntryTooLarge { limit: 1023, .. })));
        assert_eq!(archive.budget().used(), 0);

        // Entry larger than remaining budget is refused.
        let mut tight =
            SeekableArchive::open(Cursor::new(&bytes), SeekableLimits { max_entries: 10, max_metadata_bytes: 16_384, max_total_bytes: 512 }).unwrap();
        assert!(matches!(tight.open_entry("cat.lrcat", 2048), Err(Error::BudgetSpent { limit: 512, .. })));

        // Corrupted payload fails CRC check and does not charge budget.
        let mut bad = bytes.clone();
        bad[45] ^= 0xFF;
        let mut bad_archive =
            SeekableArchive::open(Cursor::new(&bad), SeekableLimits { max_entries: 10, max_metadata_bytes: 16_384, max_total_bytes: 2048 }).unwrap();
        let mut reader = bad_archive.open_entry("cat.lrcat", 2048).unwrap().unwrap();
        let mut sink = Vec::new();
        assert!(reader.read_to_end(&mut sink).is_err());
        drop(reader);
        assert_eq!(bad_archive.budget().used(), 0);
    }

    #[test]
    fn seekable_archive_reports_duplicate_names_for_matching() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        writer.start_file("notes.txt", options).unwrap();
        writer.write_all(b"first").unwrap();
        writer.start_file("notes.tx2", options).unwrap();
        writer.write_all(b"second").unwrap();
        writer.start_file("Cat.lrcat", options).unwrap();
        writer.write_all(b"catalog").unwrap();
        let mut bytes = writer.finish().unwrap().into_inner();
        for window in 0..=bytes.len() - 9 {
            if &bytes[window..window + 9] == b"notes.tx2" {
                bytes[window..window + 9].copy_from_slice(b"notes.txt");
            }
        }

        let limits = SeekableLimits { max_entries: 10, max_metadata_bytes: 16_384, max_total_bytes: 4096 };
        let mut archive = SeekableArchive::open(Cursor::new(bytes), limits).unwrap();
        assert!(archive.has_duplicate_names());
        assert_eq!(archive.duplicate_names().collect::<Vec<_>>(), vec!["notes.txt"]);
        assert!(!archive.has_duplicate_names_matching(|n| n.ends_with(".lrcat")));
        assert!(archive.has_duplicate_names_matching(|n| n == "notes.txt"));
        assert_eq!(archive.read("Cat.lrcat", 100).unwrap().as_deref(), Some(b"catalog".as_slice()));
    }

    #[test]
    fn seekable_archive_rejects_path_traversal_components() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        writer.start_file("../../etc/passwd", options).unwrap();
        writer.write_all(b"hostile").unwrap();
        writer.start_file(r"..\..\windows\system32", options).unwrap();
        writer.write_all(b"hostile").unwrap();
        writer.start_file("C:/test.txt", options).unwrap();
        writer.write_all(b"hostile").unwrap();
        writer.start_file("valid.txt", options).unwrap();
        writer.write_all(b"valid content").unwrap();
        let bytes = writer.finish().unwrap().into_inner();

        let limits = SeekableLimits { max_entries: 10, max_metadata_bytes: 16_384, max_total_bytes: 4096 };
        let mut archive = SeekableArchive::open(Cursor::new(bytes), limits).unwrap();
        assert!(archive.has_traversal_component());

        // Entry enumeration and names() iterator omit traversal vectors
        let names = (0..archive.len()).filter_map(|i| archive.entry_name(i).unwrap()).collect::<Vec<_>>();
        assert_eq!(names, vec!["valid.txt"]);
        assert_eq!(archive.names().collect::<Vec<_>>(), vec!["valid.txt"]);

        // Virtual lookups reject traversal
        assert!(!archive.contains("../../etc/passwd"));
        assert!(!archive.contains(r"..\..\windows\system32"));
        assert!(!archive.contains("C:/test.txt"));
        assert!(archive.contains("valid.txt"));
        assert_eq!(archive.declared_size("../../etc/passwd"), None);
        assert_eq!(archive.compressed_size(r"..\..\windows\system32"), None);
        assert_eq!(archive.is_stored("C:/test.txt"), None);

        // Entry reads reject traversal
        assert!(archive.open_entry("../../etc/passwd", 100).unwrap().is_none());
        assert!(archive.open_entry(r"..\..\windows\system32", 100).unwrap().is_none());
        assert!(archive.open_entry("C:/test.txt", 100).unwrap().is_none());
        assert_eq!(archive.read("../../etc/passwd", 100).unwrap(), None);
        assert_eq!(archive.read(r"..\..\windows\system32", 100).unwrap(), None);
        assert_eq!(archive.read("C:/test.txt", 100).unwrap(), None);
        assert_eq!(archive.read_index(0, 100).unwrap(), None);
        assert_eq!(archive.read_index(1, 100).unwrap(), None);
        assert_eq!(archive.read_index(2, 100).unwrap(), None);

        // Safe entry reads normally
        assert_eq!(archive.read("valid.txt", 100).unwrap().as_deref(), Some(b"valid content".as_slice()));
    }
}
