//! Checks and streaming reader for seekable ZIP archives.

use std::borrow::Cow;
use std::io::{self, Read, Seek};

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
    /// Two entries carry the same name: `Archive` refuses such an archive and its readers must
    /// see the same answer here.
    duplicates: bool,
}

impl<R: Read + Seek> SeekableArchive<R> {
    /// Indexes the trailing metadata of `reader`. The declared entry count is checked before `zip`
    /// allocates for the central directory, and indexing is bounded by
    /// [`SeekableLimits::max_metadata_bytes`].
    ///
    /// `zip` keeps one entry per name, so an archive that declares a name twice resolves to one of the two
    /// entries; [`Self::has_duplicate_names`] reports it, as [`crate::Archive::has_duplicate_names`] does, and
    /// a caller that must not read the wrong one of two conflicting entries refuses the archive on it.
    pub fn open(mut reader: R, limits: SeekableLimits) -> Result<Self, Error> {
        check_entry_count_seekable(&mut reader, limits.max_entries)?;
        let declared_headers = central_header_count_seekable(&mut reader, limits.max_metadata_bytes);
        let (meta_reader, indexing) = MetadataReader::new(reader, limits.max_metadata_bytes).map_err(|e| Error::NotZip(e.to_string()))?;
        let zip = zip::ZipArchive::new(meta_reader).map_err(|e| Error::NotZip(e.to_string()))?;
        indexing.finish();
        let found = zip.len() as u64;
        if found > limits.max_entries {
            return Err(Error::TooManyEntries { declared: found, limit: limits.max_entries });
        }
        // `zip` keeps one entry per name, so a directory with more headers than entries repeats one.
        let duplicates = declared_headers > zip.len();
        Ok(Self { zip, budget: Budget::new("archive", limits.max_total_bytes), duplicates })
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
        self.duplicates || self.zip.has_duplicate_names()
    }

    /// Locate a surviving entry by the absolute offset of its central directory record,
    /// without decoding its name or reading the local entry header.
    pub fn index_for_central_record(&self, offset: u64) -> Option<usize> {
        (0..self.zip.len()).find(|&i| self.zip.by_index_data(i).is_ok_and(|e| e.central_header_start() == offset))
    }

    /// The name at `index`, copied on demand.
    pub fn entry_name(&mut self, index: usize) -> Result<Option<String>, Error> {
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
        Ok(Some(file.is_dir()))
    }

    /// Declared decoded and compressed sizes at `index`, without name lookup.
    pub fn entry_sizes(&mut self, index: usize) -> Result<Option<(u64, u64)>, Error> {
        if index >= self.zip.len() {
            return Ok(None);
        }
        let file = self.zip.by_index_raw(index).map_err(|e| Error::NotZip(e.to_string()))?;
        Ok(Some((file.size(), file.compressed_size())))
    }

    /// Do two entries' compressed data ranges overlap?
    pub fn has_overlapping_files(&mut self) -> Result<bool, Error> {
        self.zip.has_overlapping_files().map_err(|e| Error::NotZip(e.to_string()))
    }

    /// The entry names in the archive's order, folders included. Non-UTF-8 names are omitted.
    pub fn names(&self) -> impl Iterator<Item = Cow<'_, str>> {
        self.zip.file_names().filter_map(Result::ok)
    }

    /// Is there an entry `name` (exactly as stored)?
    pub fn contains(&self, name: &str) -> bool {
        self.zip.index_for_name(name).is_some()
    }

    /// The size entry `name` declares. It is a claim, not a measurement.
    pub fn declared_size(&mut self, name: &str) -> Option<u64> {
        let index = self.zip.index_for_name(name)?;
        self.zip.by_index_raw(index).ok().map(|f| f.size())
    }

    /// The size entry `name` takes in the archive, compressed.
    pub fn compressed_size(&mut self, name: &str) -> Option<u64> {
        let index = self.zip.index_for_name(name)?;
        self.zip.by_index_raw(index).ok().map(|f| f.compressed_size())
    }

    /// Is `name` stored without compression?
    pub fn is_stored(&mut self, name: &str) -> Option<bool> {
        let index = self.zip.index_for_name(name)?;
        self.zip.by_index_raw(index).ok().map(|f| f.compression() == zip::CompressionMethod::Stored)
    }

    /// Open entry `name` for streaming reads, or `None` when there is no such entry.
    ///
    /// The entry may declare and expand to at most `max` bytes, and at most what is left of
    /// [`SeekableLimits::max_total_bytes`]. The returned [`EntryReader`] enforces the size cap and
    /// verifies the CRC-32 on EOF, charging [`Self::budget`] only when the full entry is read.
    pub fn open_entry(&mut self, name: &str, max: u64) -> Result<Option<EntryReader<'_, R>>, Error> {
        let Some(index) = self.zip.index_for_name(name) else { return Ok(None) };
        self.open_entry_index_named(index, name, max)
    }

    /// Open the entry at `index` for streaming reads under the same rules as [`Self::open_entry`].
    pub fn open_entry_index(&mut self, index: usize, max: u64) -> Result<Option<EntryReader<'_, R>>, Error> {
        if index >= self.zip.len() {
            return Ok(None);
        }
        let name = self.entry_name(index)?.unwrap_or_default();
        self.open_entry_index_named(index, &name, max)
    }

    fn open_entry_index_named(&mut self, index: usize, name: &str, max: u64) -> Result<Option<EntryReader<'_, R>>, Error> {
        let file = self.zip.by_index(index).map_err(|e| Error::Entry { name: name.to_string(), msg: e.to_string() })?;
        let declared = file.size();
        if declared > max {
            return Err(Error::EntryTooLarge { name: name.to_string(), limit: max });
        }
        if declared > self.budget.remaining() {
            return Err(Error::BudgetSpent { name: name.to_string(), limit: self.budget.limit() });
        }
        Ok(Some(EntryReader {
            file,
            budget: &mut self.budget,
            name: name.to_string(),
            declared,
            limit: declared,
            read: 0,
            finished: false,
            fail: None,
        }))
    }

    /// Read all bytes of entry `name` into memory, or `None` when there is no such entry.
    pub fn read(&mut self, name: &str, max: u64) -> Result<Option<Vec<u8>>, Error> {
        let Some(index) = self.zip.index_for_name(name) else { return Ok(None) };
        self.read_index_named(index, name, max)
    }

    /// Read all bytes of the entry at `index` into memory.
    pub fn read_index(&mut self, index: usize, max: u64) -> Result<Option<Vec<u8>>, Error> {
        if index >= self.zip.len() {
            return Ok(None);
        }
        let name = self.entry_name(index)?.unwrap_or_default();
        self.read_index_named(index, &name, max)
    }

    fn read_index_named(&mut self, index: usize, name: &str, max: u64) -> Result<Option<Vec<u8>>, Error> {
        let Some(mut reader) = self.open_entry_index_named(index, name, max)? else { return Ok(None) };
        let mut out = Vec::with_capacity(usize::try_from(reader.declared).unwrap_or(64 * 1024).min(64 * 1024));
        if let Err(e) = reader.read_to_end(&mut out) {
            return Err(reader.take_error().unwrap_or_else(|| Error::Entry { name: name.to_string(), msg: e.to_string() }));
        }
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

/// Reads from a seekable source while the caller's metadata allowance lasts, so counting the
/// directory cannot read more than indexing may.
struct MetadataWindow<R> {
    reader: R,
    remaining: u64,
}

impl<R: Read + Seek> MetadataWindow<R> {
    fn read_at(&mut self, at: u64, len: usize) -> Option<Vec<u8>> {
        let wanted = u64::try_from(len).ok()?;
        if wanted > self.remaining {
            return None;
        }
        self.remaining -= wanted;
        let mut out = vec![0u8; len];
        self.reader.seek(io::SeekFrom::Start(at)).ok()?;
        self.reader.read_exact(&mut out).ok()?;
        Some(out)
    }
}

/// The central-directory headers a seekable archive declares, counted from the directory itself the way
/// `Archive` counts them. `zip` keeps one entry per name, so more headers than entries repeats a name,
/// and this reader refuses what the in-memory guard refuses. Zero means the count could not be read.
fn central_header_count_seekable<R: Read + Seek>(reader: &mut R, max_metadata_bytes: u64) -> usize {
    let end = match reader.seek(io::SeekFrom::End(0)) {
        Ok(end) => end,
        Err(_) => return 0,
    };
    let mut window = MetadataWindow { reader, remaining: max_metadata_bytes };
    let tail_len = usize::try_from(end.min(max_metadata_bytes.max(22))).unwrap_or(0);
    let tail_start = end.saturating_sub(tail_len as u64);
    let Some(tail) = window.read_at(tail_start, tail_len) else { return 0 };
    // The end record is the last 22 bytes, followed only by its own comment.
    let Some(end_record) = (0..tail.len().saturating_sub(21)).rev().find(|&at| {
        tail.get(at..at + 4) == Some(b"PK\x05\x06")
            && tail
                .get(at + 20..at + 22)
                .is_some_and(|b| at + 22 + usize::from(u16::from_le_bytes([b[0], b[1]])) == tail.len())
    }) else {
        return 0;
    };
    let field = |at: usize, width: usize| -> u64 {
        let mut value = 0u64;
        for (shift, byte) in tail.get(at..at + width).unwrap_or_default().iter().enumerate() {
            value |= u64::from(*byte) << (8 * shift);
        }
        value
    };
    let entries = field(end_record + 10, 2) as usize;
    let start = field(end_record + 16, 4);
    if start == u64::from(u32::MAX) || start < tail_start {
        return entries;
    }

    let mut at = start;
    let mut count = 0;
    while count <= entries {
        let Some(header) = window.read_at(at, 46) else { break };
        if header.get(..4) != Some(b"PK\x01\x02") {
            break;
        }
        let length = |at: usize| usize::from(u16::from_le_bytes([header[at], header[at + 1]]));
        count += 1;
        at = at.saturating_add(46 + length(28) as u64 + length(30) as u64 + length(32) as u64);
    }
    count
}

/// Streaming reader for a single entry opened from [`SeekableArchive`].
///
/// Enforces the declared and caller-provided byte limits and charges the archive's [`Budget`]
/// once the entry has been read to its declared size and its CRC-32 verified.
///
/// An entry abandoned part-read is charged nothing, even though its bytes were decompressed: a caller that
/// stops early charges what it consumed through [`SeekableArchive::budget_mut`] when that matters.
pub struct EntryReader<'a, R: Read> {
    file: zip::read::ZipFile<'a, MetadataReader<R>>,
    budget: &'a mut Budget,
    name: String,
    declared: u64,
    limit: u64,
    read: u64,
    finished: bool,
    /// The sized error behind an `io::Error` this reader returned, so its caller gets the same
    /// `Error` variant `Archive` reports for the same archive.
    fail: Option<Error>,
}

impl<R: Read> EntryReader<'_, R> {
    /// The uncompressed size declared by the entry header.
    pub fn declared_size(&self) -> u64 {
        self.declared
    }

    /// The sized error behind the last `io::Error` this reader returned, if it had one.
    pub fn take_error(&mut self) -> Option<Error> {
        self.fail.take()
    }
}

impl<R: Read> Read for EntryReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.finished || buf.is_empty() {
            return Ok(0);
        }
        let remaining = self.limit.saturating_sub(self.read);
        let max_read = usize::try_from(remaining.saturating_add(1)).unwrap_or(usize::MAX).min(buf.len());
        let n = self.file.read(&mut buf[..max_read]).inspect_err(|e| {
            self.fail = Some(Error::Entry { name: self.name.clone(), msg: e.to_string() });
        })?;
        if n == 0 {
            if self.read < self.declared {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("entry ended after {} bytes, declared {}", self.read, self.declared),
                ));
            }
            self.budget.charge(self.read).map_err(|e| {
                self.fail = Some(Error::BudgetSpent { name: self.name.clone(), limit: e.limit });
                io::Error::new(io::ErrorKind::InvalidData, e.to_string())
            })?;
            self.finished = true;
            return Ok(0);
        }
        let next = self.read.saturating_add(n as u64);
        if next > self.limit {
            self.fail = Some(Error::EntryTooLarge { name: self.name.clone(), limit: self.limit });
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("entry exceeds declared size of {} bytes", self.limit)));
        }
        self.read = next;
        if self.read == self.declared {
            // A caller that reads exactly the declared size never asks again, so the entry is finished
            // here: one more read lets `zip` verify the CRC-32, and the budget is charged now rather than
            // at a zero-length read that may never come.
            let mut probe = [0u8; 1];
            match self.file.read(&mut probe) {
                Ok(0) => {
                    self.budget.charge(self.read).map_err(|e| {
                        self.fail = Some(Error::BudgetSpent { name: self.name.clone(), limit: e.limit });
                        io::Error::new(io::ErrorKind::InvalidData, e.to_string())
                    })?;
                    self.finished = true;
                }
                Ok(_) => {
                    self.fail = Some(Error::EntryTooLarge { name: self.name.clone(), limit: self.limit });
                    return Err(io::Error::new(io::ErrorKind::InvalidData, format!("entry exceeds declared size of {} bytes", self.limit)));
                }
                Err(e) => {
                    self.fail = Some(Error::Entry { name: self.name.clone(), msg: e.to_string() });
                    return Err(e);
                }
            }
        }
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
    fn a_full_read_charges_the_budget_without_another_read() {
        use std::io::{Cursor, Read, Write};

        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        writer.start_file("a", options).unwrap();
        writer.write_all(b"abc").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let limits = SeekableLimits { max_entries: 4, max_metadata_bytes: 64 * 1024, max_total_bytes: 16 };
        let mut archive = SeekableArchive::open(Cursor::new(bytes), limits).unwrap();
        {
            let mut reader = archive.open_entry("a", 3).unwrap().unwrap();
            let mut buf = [0u8; 3];
            reader.read_exact(&mut buf).unwrap();
        }
        // The caller read exactly the declared size and never asked again: the charge is already made.
        assert_eq!(archive.budget().used(), 3);
    }

    #[test]
    fn a_budget_refusal_is_the_variant_the_in_memory_archive_returns() {
        let mut writer = crate::Writer::new();
        writer.add("a", b"one", crate::Compression::Stored).unwrap();
        writer.add("b", b"two", crate::Compression::Stored).unwrap();
        let bytes = writer.finish().unwrap();

        let mut in_memory = crate::Archive::open(&bytes, crate::Limits { max_entries: 4, max_total_bytes: 3 }).unwrap();
        assert!(in_memory.read("a", 3).unwrap().is_some());
        let memory_err = in_memory.read("b", 3).unwrap_err();

        let limits = SeekableLimits { max_entries: 4, max_metadata_bytes: 64 * 1024, max_total_bytes: 3 };
        let mut seekable = SeekableArchive::open(Cursor::new(&bytes), limits).unwrap();
        assert!(seekable.read("a", 3).unwrap().is_some());
        let seekable_err = seekable.read("b", 3).unwrap_err();

        assert!(matches!(memory_err, Error::BudgetSpent { limit: 3, .. }), "{memory_err:?}");
        assert!(matches!(seekable_err, Error::BudgetSpent { limit: 3, .. }), "{seekable_err:?}");
    }    #[test]
    fn a_repeated_entry_name_is_reported_as_the_in_memory_archive_reports_it() {
        let mut writer = crate::Writer::new();
        writer.add("a", b"one", crate::Compression::Stored).unwrap();
        let sealed = writer.finish().unwrap();

        // A second central-directory record for the same entry: two names, one local header. The writer
        // refuses a duplicate, so the archive is assembled from the one it wrote.
        let eocd = (0..sealed.len().saturating_sub(21))
            .rev()
            .find(|&at| sealed.get(at..at + 4) == Some([0x50, 0x4b, 0x05, 0x06].as_slice()))
            .unwrap();
        let field = |at: usize, width: usize| -> u32 {
            let mut value = 0u32;
            for (shift, byte) in sealed.get(at..at + width).unwrap().iter().enumerate() {
                value |= u32::from(*byte) << (8 * shift);
            }
            value
        };
        let entries = field(eocd + 10, 2);
        let directory_size = field(eocd + 12, 4) as usize;
        let directory_start = field(eocd + 16, 4) as usize;
        let directory = sealed.get(directory_start..directory_start + directory_size).unwrap().to_vec();

        let mut bytes = sealed.get(..eocd).unwrap().to_vec();
        bytes.extend_from_slice(&directory);
        bytes.extend_from_slice(sealed.get(eocd..).unwrap());
        let trailer = eocd + directory.len();
        bytes.get_mut(trailer + 8..trailer + 10).unwrap().copy_from_slice(&((entries + 1) as u16).to_le_bytes());
        bytes.get_mut(trailer + 10..trailer + 12).unwrap().copy_from_slice(&((entries + 1) as u16).to_le_bytes());
        bytes.get_mut(trailer + 12..trailer + 16).unwrap().copy_from_slice(&((directory_size * 2) as u32).to_le_bytes());

        let limits = SeekableLimits { max_entries: 4, max_metadata_bytes: 64 * 1024, max_total_bytes: 1024 };
        let in_memory = crate::Archive::open(&bytes, crate::Limits { max_entries: 4, max_total_bytes: 1024 }).unwrap();
        let seekable = SeekableArchive::open(Cursor::new(&bytes), limits).unwrap();
        assert_eq!(seekable.len(), in_memory.len());
        assert!(in_memory.has_duplicate_names(), "two central-directory records name one entry");
        assert_eq!(
            seekable.has_duplicate_names(),
            in_memory.has_duplicate_names(),
            "the streaming reader answers as the in-memory archive does"
        );
    }



}
