//! ZIP structure checks that run before a ZIP reader indexes the archive, and bounded entry reads.
//!
//! A ZIP file ends with an end-of-central-directory record (EOCD) naming the offset and size of a
//! central directory that lists every entry, optionally preceded by a ZIP64 pair when the archive
//! is too large for the classic fields. [`preflight`] validates those records and walks the
//! directory on the raw bytes, so a hostile entry count or directory size is refused before a
//! reader can spend memory and time on it. [`read_entry`] then bounds one entry's inflation.
//!
//! Only single-disk archives are accepted: split archives and self-extracting prefixed archives
//! are rejected, because their relative offsets would make this check disagree with the bytes a
//! later reader indexes. The algorithm follows the ZIP specification (PKWARE APPNOTE, "ZIP File
//! Format"); no third-party code is involved.

use std::fmt;
use std::io::Read;

use crate::{Budget, Exceeded};

const EOCD_SIGNATURE: &[u8; 4] = b"PK\x05\x06";
const ZIP64_EOCD_SIGNATURE: &[u8; 4] = b"PK\x06\x06";
const ZIP64_LOCATOR_SIGNATURE: &[u8; 4] = b"PK\x06\x07";
const CENTRAL_FILE_SIGNATURE: &[u8; 4] = b"PK\x01\x02";
const CENTRAL_DIGITAL_SIGNATURE: &[u8; 4] = b"PK\x05\x05";

/// The classic end-of-central-directory record, fixed part.
const EOCD_LEN: usize = 22;
/// The ZIP64 end-of-central-directory locator.
const ZIP64_LOCATOR_LEN: usize = 20;
/// The fixed part of a ZIP64 end-of-central-directory record: 12 bytes of signature and record
/// size, then 44 bytes of fixed fields.
const ZIP64_EOCD_FIXED_LEN: usize = 44;
const ZIP64_EOCD_HEADER_LEN: u64 = 12;
/// The fixed part of a central-directory file header, before the name, extra field and comment.
const CENTRAL_FILE_LEN: usize = 46;
/// The largest archive comment the classic end record can describe.
const MAX_COMMENT_LEN: usize = u16::MAX as usize;
/// Central-directory file header field offsets.
const NAME_LEN_AT: usize = 28;
const EXTRA_LEN_AT: usize = 30;
const COMMENT_LEN_AT: usize = 32;
const ENTRY_DISK_AT: usize = 34;

/// Application-owned limits for [`preflight`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most central-directory entries the archive may declare.
    pub max_entries: u64,
    /// The most bytes the central directory may occupy.
    pub max_central_directory_bytes: u64,
    /// The most bytes a central-directory entry name may occupy.
    pub max_name_bytes: u16,
}

/// What [`preflight`] found: the directory's declared totals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Directory {
    /// The number of central-directory entries, counted by walking them.
    pub entries: u64,
    /// The declared size of the central directory in bytes.
    pub central_directory_bytes: u64,
}

/// Why [`preflight`] refused an archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No end-of-central-directory record spans the end of the file.
    MissingEndRecord,
    /// More than one end record could claim the end of the file, so which one a reader would use
    /// is not determined; none is trusted.
    AmbiguousEndRecords,
    /// The structure is malformed or unsupported. The message names what was wrong.
    Invalid(&'static str),
    /// The archive declares more entries than the limit admits.
    TooManyEntries {
        /// The declared entry count.
        actual: u64,
        /// The caller's limit.
        limit: u64,
    },
    /// The central directory is larger than the limit admits.
    CentralDirectoryTooLarge {
        /// The declared directory size in bytes.
        actual: u64,
        /// The caller's limit.
        limit: u64,
    },
    /// A central-directory entry name is longer than the limit admits.
    NameTooLong {
        /// The declared name length in bytes.
        actual: u64,
        /// The caller's limit.
        limit: u64,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingEndRecord => f.write_str("the ZIP end record is missing"),
            Self::AmbiguousEndRecords => f.write_str("the ZIP has more than one plausible end record"),
            Self::Invalid(message) => write!(f, "invalid ZIP structure: {message}"),
            Self::TooManyEntries { actual, limit } => write!(f, "the ZIP declares {actual} entries; the limit is {limit}"),
            Self::CentralDirectoryTooLarge { actual, limit } => write!(f, "the ZIP central directory is {actual} bytes; the limit is {limit}"),
            Self::NameTooLong { actual, limit } => write!(f, "a ZIP entry name is {actual} bytes; the limit is {limit}"),
        }
    }
}

impl std::error::Error for Error {}

/// Why [`read_entry`] refused an entry.
///
/// Not `PartialEq`: an [`std::io::Error`] is not comparable, so tests compare the displayed
/// message instead.
#[derive(Debug)]
pub enum ReadError {
    /// The entry declares more bytes than the per-entry limit admits.
    DeclaredTooLarge {
        /// The declared size.
        declared: u64,
        /// The caller's per-entry limit.
        limit: u64,
    },
    /// The entry produced more bytes than the smaller of its declared size and the per-entry
    /// limit: the declared size lied, or the data is a decompression bomb.
    Overrun {
        /// The per-entry bound that was passed.
        limit: u64,
    },
    /// The entry needed more bytes than the total budget had left.
    Budget(Exceeded),
    /// The reader failed.
    Io(std::io::Error),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeclaredTooLarge { declared, limit } => write!(f, "the entry declares {declared} bytes; the limit is {limit}"),
            Self::Overrun { limit } => write!(f, "the entry produced more than the {limit} bytes it is allowed"),
            Self::Budget(exceeded) => exceeded.fmt(f),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Budget(exceeded) => Some(exceeded),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ReadError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<Exceeded> for ReadError {
    fn from(exceeded: Exceeded) -> Self {
        Self::Budget(exceeded)
    }
}

/// The most bytes [`read_entry`] reserves up front, however large the entry declares itself.
///
/// A declared size is attacker-controlled, so reserving it would let a ten-byte archive claim a
/// gigabyte; the buffer grows as real bytes arrive instead.
pub const MAX_PREALLOCATION: u64 = 64 * 1024;

/// Validate the end records and walk the declared central directory without allocating.
///
/// Checks, in order:
///
/// * the end-of-central-directory record: exactly one candidate must span the end of the file, so
///   an archive that hides a second plausible record is refused rather than guessed at;
/// * the ZIP64 pair when the classic record uses a sentinel or a locator is present: a single disk,
///   a locator immediately before the end record, the ZIP64 record ending at that locator, and no
///   disagreement between the classic and ZIP64 fields;
/// * the declared entry count, directory size and entry-name lengths against `limits`;
/// * the directory arithmetic: its offset and size must not overflow and must reach the end
///   records, so a lying offset or size cannot point the reader elsewhere;
/// * every entry header in the directory: its signature, its disk number, its variable-length
///   fields, and that the entries exactly fill the declared directory, allowing one optional
///   central-directory digital signature at its end.
///
/// Returns the declared entry count and directory size. Malformed input is an error, never a panic.
pub fn preflight(bytes: &[u8], limits: Limits) -> Result<Directory, Error> {
    let eocd_at = find_end_record(bytes)?;
    let directory = end_record_totals(bytes, eocd_at)?;

    if directory.entries > limits.max_entries {
        return Err(Error::TooManyEntries { actual: directory.entries, limit: limits.max_entries });
    }
    if directory.size > limits.max_central_directory_bytes {
        return Err(Error::CentralDirectoryTooLarge { actual: directory.size, limit: limits.max_central_directory_bytes });
    }

    let directory_end = directory.offset.checked_add(directory.size).ok_or(Error::Invalid("central-directory offset overflow"))?;
    let structure_at = u64::try_from(directory.structure_at).map_err(|_| Error::Invalid("end-record offset does not fit"))?;
    if directory_end != structure_at {
        return Err(Error::Invalid("central-directory offset and size do not reach the end records"));
    }
    let start = usize::try_from(directory.offset).map_err(|_| Error::Invalid("central-directory offset does not fit this platform"))?;
    let end = usize::try_from(directory_end).map_err(|_| Error::Invalid("central-directory end does not fit this platform"))?;

    walk_directory(bytes, start, end, directory.entries, limits.max_name_bytes)?;
    Ok(Directory { entries: directory.entries, central_directory_bytes: directory.size })
}

/// Read one entry up to the smallest of `max`, the total budget's remainder and `declared`.
///
/// The declared size is refused outright when it exceeds `max`. At most [`MAX_PREALLOCATION`] bytes
/// are reserved however large the entry claims to be. The bytes are read up to the bound and only
/// the bytes actually read are charged to `total`, so a declaration that overstates the entry
/// costs nothing extra.
///
/// More bytes arriving than the bound admits is the hostile case: [`ReadError::Overrun`] when the
/// per-entry bound or the declaration was what stopped the read (a lying size or a decompression
/// bomb), and [`ReadError::Budget`] when the total budget was already the tighter bound.
///
/// Works with any [`Read`], including a `zip` crate entry reader. An I/O failure is
/// [`ReadError::Io`]; the bytes read before it are not charged.
pub fn read_entry(r: impl Read, declared: u64, max: u64, total: &mut Budget) -> Result<Vec<u8>, ReadError> {
    if declared > max {
        return Err(ReadError::DeclaredTooLarge { declared, limit: max });
    }
    let per_entry = declared.min(max);
    let budget_left = total.remaining();
    let bound = per_entry.min(budget_left);
    // One byte past the bound has to be readable to tell "exactly the bound" from "more".
    let mut out = Vec::with_capacity(usize::try_from(bound.min(MAX_PREALLOCATION)).unwrap_or(0));
    let mut reader = r.take(bound.saturating_add(1));
    reader.read_to_end(&mut out)?;
    let read = u64::try_from(out.len()).unwrap_or(u64::MAX);
    if read > bound {
        // The budget was the tighter bound, so the total limit is what refused the entry.
        if per_entry > budget_left {
            return Err(ReadError::Budget(Exceeded { what: total.what(), limit: total.limit(), requested: read }));
        }
        return Err(ReadError::Overrun { limit: bound });
    }
    total.charge(read)?;
    Ok(out)
}

/// The declared entry count, directory size and offset, and where the end records begin.
struct EndRecord {
    entries: u64,
    size: u64,
    offset: u64,
    /// The offset of the first end record: the ZIP64 record when there is one, else the EOCD.
    structure_at: usize,
}

/// Find the single end-of-central-directory record that ends the file.
///
/// A record is a candidate when its signature is present, its declared comment length puts its end
/// exactly at the end of the file, and it lies in the last 64 KiB plus 22 bytes — the only place
/// the format allows one. The search runs backwards, so the record nearest the end is considered
/// first; finding a second candidate means the archive is ambiguous and is refused.
fn find_end_record(bytes: &[u8]) -> Result<usize, Error> {
    let last = bytes.len().checked_sub(EOCD_LEN).ok_or(Error::MissingEndRecord)?;
    let first = bytes.len().saturating_sub(EOCD_LEN.saturating_add(MAX_COMMENT_LEN));
    let mut found = None;
    for at in (first..=last).rev() {
        if !signature_at(bytes, at, EOCD_SIGNATURE) {
            continue;
        }
        let Some(comment_len) = u16_at(bytes, at + 20).map(usize::from) else { continue };
        let Some(end) = at.checked_add(EOCD_LEN).and_then(|value| value.checked_add(comment_len)) else { continue };
        if end != bytes.len() {
            continue;
        }
        if found.replace(at).is_some() {
            return Err(Error::AmbiguousEndRecords);
        }
    }
    found.ok_or(Error::MissingEndRecord)
}

/// Read the totals the end records declare, using the ZIP64 pair when one is present.
fn end_record_totals(bytes: &[u8], eocd_at: usize) -> Result<EndRecord, Error> {
    let disk = u16_at(bytes, eocd_at + 4).ok_or(Error::Invalid("truncated end record"))?;
    let directory_disk = u16_at(bytes, eocd_at + 6).ok_or(Error::Invalid("truncated end record"))?;
    let disk_entries = u16_at(bytes, eocd_at + 8).ok_or(Error::Invalid("truncated end record"))?;
    let entries = u16_at(bytes, eocd_at + 10).ok_or(Error::Invalid("truncated end record"))?;
    let directory_size = u32_at(bytes, eocd_at + 12).ok_or(Error::Invalid("truncated end record"))?;
    let directory_offset = u32_at(bytes, eocd_at + 16).ok_or(Error::Invalid("truncated end record"))?;

    let sentinel = disk == u16::MAX
        || directory_disk == u16::MAX
        || disk_entries == u16::MAX
        || entries == u16::MAX
        || directory_size == u32::MAX
        || directory_offset == u32::MAX;
    let locator_at = eocd_at.checked_sub(ZIP64_LOCATOR_LEN).filter(|at| signature_at(bytes, *at, ZIP64_LOCATOR_SIGNATURE));

    let Some(locator_at) = locator_at else {
        if sentinel {
            return Err(Error::Invalid("ZIP64 sentinel without a ZIP64 end record"));
        }
        if disk != 0 || directory_disk != 0 || disk_entries != entries {
            return Err(Error::Invalid("multi-disk ZIP archive"));
        }
        return Ok(EndRecord {
            entries: u64::from(entries),
            size: u64::from(directory_size),
            offset: u64::from(directory_offset),
            structure_at: eocd_at,
        });
    };

    let zip64_disk = u32_at(bytes, locator_at + 4).ok_or(Error::Invalid("truncated ZIP64 locator"))?;
    let zip64_offset = u64_at(bytes, locator_at + 8).ok_or(Error::Invalid("truncated ZIP64 locator"))?;
    let zip64_disks = u32_at(bytes, locator_at + 16).ok_or(Error::Invalid("truncated ZIP64 locator"))?;
    if zip64_disk != 0 || zip64_disks != 1 {
        return Err(Error::Invalid("multi-disk ZIP64 archive"));
    }
    let zip64_at = usize::try_from(zip64_offset).map_err(|_| Error::Invalid("ZIP64 end-record offset does not fit this platform"))?;
    if !signature_at(bytes, zip64_at, ZIP64_EOCD_SIGNATURE) {
        return Err(Error::Invalid("the ZIP64 end record is missing at its declared offset"));
    }
    let zip64_disk_number = u32_at(bytes, zip64_at + 16).ok_or(Error::Invalid("truncated ZIP64 end record"))?;
    let zip64_directory_disk = u32_at(bytes, zip64_at + 20).ok_or(Error::Invalid("truncated ZIP64 end record"))?;
    let zip64_disk_entries = u64_at(bytes, zip64_at + 24).ok_or(Error::Invalid("truncated ZIP64 end record"))?;
    let zip64_entries = u64_at(bytes, zip64_at + 32).ok_or(Error::Invalid("truncated ZIP64 end record"))?;
    let zip64_directory_size = u64_at(bytes, zip64_at + 40).ok_or(Error::Invalid("truncated ZIP64 end record"))?;
    let zip64_directory_offset = u64_at(bytes, zip64_at + 48).ok_or(Error::Invalid("truncated ZIP64 end record"))?;
    let zip64_record_size = u64_at(bytes, zip64_at + 4).ok_or(Error::Invalid("truncated ZIP64 end record"))?;
    if zip64_record_size < u64::try_from(ZIP64_EOCD_FIXED_LEN).unwrap_or(44) {
        return Err(Error::Invalid("the ZIP64 end record is too short"));
    }
    // The ZIP64 record ends exactly where its locator begins; anything else means the locator
    // points somewhere other than this record.
    let record_end = u64::try_from(zip64_at)
        .ok()
        .and_then(|at| at.checked_add(ZIP64_EOCD_HEADER_LEN))
        .and_then(|at| at.checked_add(zip64_record_size))
        .ok_or(Error::Invalid("ZIP64 end-record length overflow"))?;
    if record_end != u64::try_from(locator_at).unwrap_or(u64::MAX) {
        return Err(Error::Invalid("the ZIP64 end record does not end at its locator"));
    }
    if zip64_disk_number != 0 || zip64_directory_disk != 0 || zip64_disk_entries != zip64_entries {
        return Err(Error::Invalid("multi-disk ZIP64 archive"));
    }
    if (disk != u16::MAX && disk != 0)
        || (directory_disk != u16::MAX && directory_disk != 0)
        || (disk_entries != u16::MAX && u64::from(disk_entries) != zip64_entries)
        || (entries != u16::MAX && u64::from(entries) != zip64_entries)
        || (directory_size != u32::MAX && u64::from(directory_size) != zip64_directory_size)
        || (directory_offset != u32::MAX && u64::from(directory_offset) != zip64_directory_offset)
    {
        return Err(Error::Invalid("the classic and ZIP64 end records disagree"));
    }
    Ok(EndRecord { entries: zip64_entries, size: zip64_directory_size, offset: zip64_directory_offset, structure_at: zip64_at })
}

/// Walk the declared directory, checking each entry header and that the entries fill it exactly.
fn walk_directory(bytes: &[u8], start: usize, end: usize, entries: u64, max_name_bytes: u16) -> Result<(), Error> {
    if end > bytes.len() || start > end {
        return Err(Error::Invalid("the central directory is outside the archive"));
    }
    let mut at = start;
    for _ in 0..entries {
        if !signature_at(bytes, at, CENTRAL_FILE_SIGNATURE) {
            return Err(Error::Invalid("a central-directory entry signature is missing"));
        }
        let name = u16_at(bytes, at + NAME_LEN_AT).ok_or(Error::Invalid("truncated central-directory entry"))?;
        let extra = usize::from(u16_at(bytes, at + EXTRA_LEN_AT).ok_or(Error::Invalid("truncated central-directory entry"))?);
        let comment = usize::from(u16_at(bytes, at + COMMENT_LEN_AT).ok_or(Error::Invalid("truncated central-directory entry"))?);
        let disk = u16_at(bytes, at + ENTRY_DISK_AT).ok_or(Error::Invalid("truncated central-directory entry"))?;
        if disk != 0 {
            return Err(Error::Invalid("a central-directory entry refers to another disk"));
        }
        if name == 0 || name > max_name_bytes {
            return Err(Error::NameTooLong { actual: u64::from(name), limit: u64::from(max_name_bytes) });
        }
        at = at
            .checked_add(CENTRAL_FILE_LEN)
            .and_then(|value| value.checked_add(usize::from(name)))
            .and_then(|value| value.checked_add(extra))
            .and_then(|value| value.checked_add(comment))
            .ok_or(Error::Invalid("central-directory entry length overflow"))?;
        if at > end {
            return Err(Error::Invalid("a central-directory entry exceeds the declared directory size"));
        }
    }
    // The directory normally ends where its last entry does, optionally followed by one
    // central-directory digital signature record.
    if at < end && signature_at(bytes, at, CENTRAL_DIGITAL_SIGNATURE) {
        let signature_len = usize::from(u16_at(bytes, at + 4).ok_or(Error::Invalid("truncated central-directory signature"))?);
        at = at
            .checked_add(6)
            .and_then(|value| value.checked_add(signature_len))
            .ok_or(Error::Invalid("central-directory signature length overflow"))?;
    }
    if at != end {
        return Err(Error::Invalid("the central-directory entry count and size disagree"));
    }
    Ok(())
}

fn bytes_at(bytes: &[u8], at: usize, len: usize) -> Option<&[u8]> {
    bytes.get(at..at.checked_add(len)?)
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes_at(bytes, at, 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes_at(bytes, at, 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes_at(bytes, at, 8)?.try_into().ok()?))
}

fn signature_at(bytes: &[u8], at: usize, signature: &[u8; 4]) -> bool {
    bytes_at(bytes, at, signature.len()) == Some(signature.as_slice())
}

#[cfg(test)]
mod tests;
