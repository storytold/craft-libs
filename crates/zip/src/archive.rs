//! An archive open for reading: entry-count guard, duplicate names, budgeted entry reads.

use std::borrow::Cow;
use std::io::{Cursor, Read};
use std::sync::Arc;

use crate::preflight::{ReadError, read_entry};
use crate::{Budget, Error, check_entry_count};

/// What an [`Archive`] refuses to exceed. Both numbers are the caller's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Most entries the archive may declare.
    pub max_entries: u64,
    /// Most bytes all the entries read from the archive may expand to together.
    pub max_total_bytes: u64,
}

/// A ZIP archive in memory. Nothing is decompressed until an entry is read.
///
/// A clone shares the parsed directory and the bytes, and carries a copy of the budget: a
/// caller that reads through a clone keeps its own count.
#[derive(Clone)]
pub struct Archive<'a> {
    zip: zip::ZipArchive<Cursor<Input<'a>>>,
    budget: Budget,
    duplicates: bool,
}

#[derive(Clone)]
enum Input<'a> {
    Borrowed(&'a [u8]),
    Owned(Arc<Vec<u8>>),
}

impl AsRef<[u8]> for Input<'_> {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Borrowed(bytes) => bytes,
            Self::Owned(bytes) => bytes.as_slice(),
        }
    }
}

const CENTRAL_HEADER: &[u8; 4] = b"PK\x01\x02";
const CENTRAL_HEADER_LEN: usize = 46;

/// The central directory headers that follow one another from `start`, counted up to `stop`.
fn count_central_headers(bytes: &[u8], start: u64, stop: usize) -> usize {
    let u16_at = |at: usize| bytes.get(at..at.checked_add(2)?).map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])));
    let Ok(mut at) = usize::try_from(start) else { return 0 };
    let mut count = 0;
    while count < stop && bytes.get(at..).is_some_and(|b| b.starts_with(CENTRAL_HEADER)) {
        let (Some(name), Some(extra), Some(comment)) = (u16_at(at + 28), u16_at(at + 30), u16_at(at + 32)) else { break };
        count += 1;
        at = at.saturating_add(CENTRAL_HEADER_LEN + name + extra + comment);
    }
    count
}

impl<'a> Archive<'a> {
    /// Indexes `bytes`. The declared entry count is checked against
    /// [`Limits::max_entries`] before `zip` parses the central directory, and again after.
    pub fn open(bytes: &'a [u8], limits: Limits) -> Result<Self, Error> {
        Self::open_input(Input::Borrowed(bytes), limits)
    }

    fn open_input(input: Input<'a>, limits: Limits) -> Result<Self, Error> {
        check_entry_count(input.as_ref(), limits.max_entries)?;
        let bytes = input.clone();
        let zip = zip::ZipArchive::new(Cursor::new(input)).map_err(|e| Error::NotZip(e.to_string()))?;
        let found = zip.len() as u64;
        if found > limits.max_entries {
            return Err(Error::TooManyEntries { declared: found, limit: limits.max_entries });
        }
        // `zip` keeps one entry per name, so a directory with more headers than `zip` has entries
        // repeats a name.
        let duplicates = count_central_headers(bytes.as_ref(), zip.central_directory_start(), zip.len().saturating_add(1)) > zip.len();
        Ok(Archive { zip, budget: Budget::new("archive", limits.max_total_bytes), duplicates })
    }

    /// The number of distinct entry names.
    pub fn len(&self) -> usize {
        self.zip.len()
    }

    pub fn is_empty(&self) -> bool {
        self.zip.is_empty()
    }

    /// The name at `index`, copied on demand so callers can process hostile directories without
    /// retaining a second copy of every potentially large name.
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

    /// Do two entries share a name? `zip` then keeps one of them, and which one a writer meant
    /// is not known: a format that must not guess refuses the archive.
    pub fn has_duplicate_names(&self) -> bool {
        self.duplicates
    }

    /// Do two entries' compressed data ranges overlap? A format that stores many files in one
    /// archive can refuse aliases so one payload cannot be expanded and retained repeatedly.
    pub fn has_overlapping_files(&mut self) -> Result<bool, Error> {
        self.zip.has_overlapping_files().map_err(|e| Error::NotZip(e.to_string()))
    }

    /// The entry names in the archive's order, folders included. A name that is not text is
    /// left out.
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

    /// The size entry `name` takes in the archive, compressed. With [`Archive::declared_size`] it
    /// gives the compression ratio the entry claims.
    pub fn compressed_size(&mut self, name: &str) -> Option<u64> {
        let index = self.zip.index_for_name(name)?;
        self.zip.by_index_raw(index).ok().map(|f| f.compressed_size())
    }

    /// Is `name` stored without compression? The answer lets a format with in-memory source bytes
    /// apply an output-size cap to compressed entries while bounding stored reads by source size.
    pub fn is_stored(&mut self, name: &str) -> Option<bool> {
        let index = self.zip.index_for_name(name)?;
        self.zip.by_index_raw(index).ok().map(|f| f.compression() == zip::CompressionMethod::Stored)
    }

    /// The first `n` bytes of entry `name` (all of it when it is shorter), or `None` when there
    /// is no such entry: for telling a file by its signature without inflating it. The bytes are
    /// charged to the total; a total with less left than the bytes wanted refuses before reading.
    ///
    /// The entry's checksum is not checked: it covers the whole entry.
    pub fn read_head(&mut self, name: &str, n: usize) -> Result<Option<Vec<u8>>, Error> {
        let Some(index) = self.zip.index_for_name(name) else { return Ok(None) };
        let entry = |msg: String| Error::Entry { name: name.to_string(), msg };
        let f = self.zip.by_index(index).map_err(|e| entry(e.to_string()))?;
        // Never more than the entry declares: a shorter entry costs only what it has.
        let want = (n as u64).min(f.size());
        if self.budget.remaining() < want {
            return Err(Error::BudgetSpent { name: name.to_string(), limit: self.budget.limit() });
        }
        let mut out = Vec::with_capacity(n.min(64 * 1024));
        f.take(want).read_to_end(&mut out).map_err(|e| entry(e.to_string()))?;
        self.budget.charge(out.len() as u64).map_err(|e| Error::BudgetSpent { name: name.to_string(), limit: e.limit })?;
        Ok(Some(out))
    }

    /// The bytes of entry `name`, or `None` when there is no such entry.
    ///
    /// The entry may declare, and expand to, at most `max` bytes, and at most what is left of
    /// [`Limits::max_total_bytes`]. An entry that expands past the size it declares is refused.
    /// Only the bytes of a read that succeeds are charged to the total.
    pub fn read(&mut self, name: &str, max: u64) -> Result<Option<Vec<u8>>, Error> {
        let Some(index) = self.zip.index_for_name(name) else { return Ok(None) };
        self.read_index_named(index, name, max)
    }

    /// Read the entry at `index` under the same size, checksum and aggregate budget
    /// rules as [`Self::read`], without repeating a decoded-name lookup.
    pub fn read_index(&mut self, index: usize, max: u64) -> Result<Option<Vec<u8>>, Error> {
        if index >= self.zip.len() {
            return Ok(None);
        }
        let name = self.entry_name(index)?.unwrap_or_default();
        self.read_index_named(index, &name, max)
    }

    fn read_index_named(&mut self, index: usize, name: &str, max: u64) -> Result<Option<Vec<u8>>, Error> {
        let entry = |msg: String| Error::Entry { name: name.to_string(), msg };
        let f = self.zip.by_index(index).map_err(|e| entry(e.to_string()))?;
        let declared = f.size();
        match read_entry(f, declared, max, &mut self.budget) {
            Ok(data) => Ok(Some(data)),
            Err(ReadError::DeclaredTooLarge { limit, .. }) | Err(ReadError::Overrun { limit }) => {
                Err(Error::EntryTooLarge { name: name.to_string(), limit })
            }
            Err(ReadError::Budget(e)) => Err(Error::BudgetSpent { name: name.to_string(), limit: e.limit }),
            Err(ReadError::Io(e)) => Err(entry(e.to_string())),
        }
    }

    /// The total budget: what the entries read so far have used, and what is left.
    pub fn budget(&self) -> &Budget {
        &self.budget
    }

    /// The total budget, for a caller with its own charging rule (charging a failed read, say).
    pub fn budget_mut(&mut self) -> &mut Budget {
        &mut self.budget
    }
}

impl Archive<'static> {
    /// Indexes owned bytes without copying them. Clones share the backing storage.
    pub fn open_owned(bytes: Vec<u8>, limits: Limits) -> Result<Self, Error> {
        Self::open_input(Input::Owned(Arc::new(bytes)), limits)
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    #[test]
    fn named_read_preserves_entry_error_for_a_corrupt_local_header() {
        let mut writer = crate::Writer::new();
        writer.add("a", b"abc", crate::Compression::Stored).unwrap();
        let mut bytes = writer.finish().unwrap();
        bytes[0] = 0; // The central directory remains valid; the local signature does not.
        let mut archive = Archive::open(&bytes, Limits { max_entries: 1, max_total_bytes: 3 }).unwrap();
        assert!(matches!(archive.read("a", 3), Err(Error::Entry { name, .. }) if name == "a"));
        assert_eq!(archive.budget().remaining(), 3);
    }

    #[test]
    fn indexed_reads_charge_the_same_budget_and_enforce_entry_limits() {
        let mut writer = crate::Writer::new();
        writer.add("a", b"abc", crate::Compression::Stored).unwrap();
        writer.add("b", b"def", crate::Compression::Deflated).unwrap();
        let bytes = writer.finish().unwrap();
        let mut archive = Archive::open(&bytes, Limits { max_entries: 2, max_total_bytes: 5 }).unwrap();
        assert!(archive.read_index(0, 2).is_err());
        assert_eq!(archive.read_index(0, 3).unwrap().unwrap(), b"abc");
        assert!(archive.read_index(1, 3).is_err());
        assert_eq!(archive.read_index(2, 3).unwrap(), None);
        assert_eq!(archive.entry_sizes(0).unwrap(), Some((3, 3)));
        assert_eq!(archive.entry_sizes(2).unwrap(), None);
    }

    #[test]
    fn owned_storage_lives_until_last_archive_clone_is_dropped() {
        let bytes = crate::Writer::new().finish().unwrap();
        let bytes = Arc::new(bytes);
        let weak = Arc::downgrade(&bytes);
        let archive = Archive::open_input(Input::Owned(bytes), Limits { max_entries: 1, max_total_bytes: 1 }).unwrap();
        let clone = archive.clone();
        drop(archive);
        assert!(weak.upgrade().is_some());
        drop(clone);
        assert!(weak.upgrade().is_none());
    }
}
