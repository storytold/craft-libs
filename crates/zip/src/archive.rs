//! An archive open for reading: entry-count guard, duplicate names, budgeted entry reads.

use std::borrow::Cow;
use std::io::{Cursor, Read};

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
    zip: zip::ZipArchive<Cursor<&'a [u8]>>,
    budget: Budget,
    duplicates: bool,
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
        check_entry_count(bytes, limits.max_entries)?;
        let zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::NotZip(e.to_string()))?;
        let found = zip.len() as u64;
        if found > limits.max_entries {
            return Err(Error::TooManyEntries { declared: found, limit: limits.max_entries });
        }
        // `zip` keeps one entry per name, so a directory with more headers than `zip` has entries
        // repeats a name.
        let duplicates = count_central_headers(bytes, zip.central_directory_start(), zip.len().saturating_add(1)) > zip.len();
        Ok(Archive { zip, budget: Budget::new("archive", limits.max_total_bytes), duplicates })
    }

    /// The number of distinct entry names.
    pub fn len(&self) -> usize {
        self.zip.len()
    }

    pub fn is_empty(&self) -> bool {
        self.zip.is_empty()
    }

    /// Do two entries share a name? `zip` then keeps one of them, and which one a writer meant
    /// is not known: a format that must not guess refuses the archive.
    pub fn has_duplicate_names(&self) -> bool {
        self.duplicates
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
