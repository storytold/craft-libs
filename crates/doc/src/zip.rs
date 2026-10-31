//! A ZIP reader for `.pcraft` bundles: STORE and DEFLATE entries, found through the central
//! directory, CRC-checked. No ZIP64, no encryption, no archives split across disks.

use std::io::Read;
use std::ops::Range;

use crate::{Budget, Error, malformed};

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const EOCD_LEN: usize = 22;
const CENTRAL_LEN: usize = 46;
const LOCAL_LEN: usize = 30;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([*s.first()?, *s.get(1)?]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([*s.first()?, *s.get(1)?, *s.get(2)?, *s.get(3)?]))
}

fn truncated() -> Error {
    malformed("truncated ZIP archive")
}

/// One entry of the central directory.
pub(crate) struct Entry {
    name: Range<usize>,
    flags: u16,
    method: u16,
    crc: u32,
    compressed: usize,
    size: u64,
    /// Where the entry's data starts.
    data: usize,
}

pub(crate) struct Zip<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry>,
}

impl<'a> Zip<'a> {
    pub(crate) fn open(bytes: &'a [u8]) -> Result<Self, Error> {
        let last = bytes.len().checked_sub(EOCD_LEN).ok_or_else(|| malformed("not a ZIP archive (too short)"))?;
        // The comment may itself hold the signature: accept only a record whose comment ends the
        // file and whose central directory ends where the record starts.
        let eocd = (last.saturating_sub(65535)..=last)
            .rev()
            .find(|&i| {
                let comment_end = u16_at(bytes, i + 20).and_then(|n| i.checked_add(EOCD_LEN)?.checked_add(usize::from(n)));
                let dir_end = u32_at(bytes, i + 16)
                    .zip(u32_at(bytes, i + 12))
                    .and_then(|(off, size)| usize::try_from(off).ok()?.checked_add(usize::try_from(size).ok()?));
                u32_at(bytes, i) == Some(EOCD_SIG) && comment_end == Some(bytes.len()) && dir_end == Some(i)
            })
            .ok_or_else(|| {
                let zip64 = (last.saturating_sub(65535 + 20)..=last.saturating_sub(20)).any(|i| u32_at(bytes, i) == Some(ZIP64_LOCATOR_SIG));
                if zip64 {
                    Error::Unsupported("ZIP64 archives are not supported".into())
                } else {
                    malformed("ZIP end of central directory not found")
                }
            })?;
        let disk = u16_at(bytes, eocd + 4).ok_or_else(truncated)?;
        let dir_disk = u16_at(bytes, eocd + 6).ok_or_else(truncated)?;
        let count = usize::from(u16_at(bytes, eocd + 10).ok_or_else(truncated)?);
        if disk != 0 || dir_disk != 0 {
            return Err(Error::Unsupported("ZIP archives split across disks are not supported".into()));
        }
        let dir_size = u32_at(bytes, eocd + 12).ok_or_else(truncated)? as usize;
        let dir = eocd - dir_size;
        let mut entries = Vec::with_capacity(count.min(dir_size / CENTRAL_LEN));
        let mut p = dir;
        for _ in 0..count {
            if u32_at(bytes, p) != Some(CENTRAL_SIG) || p + CENTRAL_LEN > eocd {
                return Err(malformed("bad ZIP central directory entry"));
            }
            let field16 = |o: usize| u16_at(bytes, p + o).ok_or_else(truncated);
            let field32 = |o: usize| u32_at(bytes, p + o).ok_or_else(truncated);
            let (flags, method, crc) = (field16(8)?, field16(10)?, field32(16)?);
            let (compressed, size) = (field32(20)?, field32(24)?);
            let (name_len, extra_len, comment_len) = (usize::from(field16(28)?), usize::from(field16(30)?), usize::from(field16(32)?));
            let local = field32(42)? as usize;
            if compressed == u32::MAX || size == u32::MAX || local == u32::MAX as usize {
                return Err(Error::Unsupported("ZIP64 entries are not supported".into()));
            }
            let name_start = p + CENTRAL_LEN;
            let name_end = name_start + name_len;
            let next = name_end + extra_len + comment_len;
            if next > eocd {
                return Err(malformed("ZIP central directory entry out of range"));
            }
            if u32_at(bytes, local) != Some(LOCAL_SIG) {
                return Err(malformed("bad ZIP local header"));
            }
            let local_name = usize::from(u16_at(bytes, local + 26).ok_or_else(truncated)?);
            let local_extra = usize::from(u16_at(bytes, local + 28).ok_or_else(truncated)?);
            let data = local.checked_add(LOCAL_LEN + local_name + local_extra).ok_or_else(truncated)?;
            let compressed = compressed as usize;
            if data.checked_add(compressed).is_none_or(|end| end > bytes.len()) {
                return Err(malformed("ZIP entry data out of range"));
            }
            entries.push(Entry { name: name_start..name_end, flags, method, crc, compressed, size: u64::from(size), data });
            p = next;
        }
        Ok(Zip { bytes, entries })
    }

    /// The first entry named `name`.
    pub(crate) fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| self.bytes.get(e.name.clone()) == Some(name.as_bytes()))
    }

    /// The entry's bytes, decompressed and CRC-checked; their size is checked against `max` and
    /// counted against `budget` first.
    pub(crate) fn read(&self, e: &Entry, max: u64, budget: &mut Budget, what: &str) -> Result<Vec<u8>, Error> {
        if e.flags & 1 != 0 {
            return Err(Error::Unsupported(format!("{what} is encrypted")));
        }
        if !matches!(e.method, 0 | 8) {
            return Err(Error::Unsupported(format!("{what} uses ZIP compression method {}", e.method)));
        }
        if e.size > max {
            return Err(Error::LimitExceeded(format!("{what} is {} bytes, more than the limit ({max})", e.size)));
        }
        budget.take(e.size, what)?;
        let raw = self.bytes.get(e.data..e.data + e.compressed).ok_or_else(truncated)?;
        let size = usize::try_from(e.size).map_err(|_| Error::LimitExceeded(format!("{what} does not fit in memory")))?;
        let data = if e.method == 0 {
            if e.compressed != size {
                return Err(malformed(format!("{what} has the wrong size")));
            }
            raw.to_vec()
        } else {
            // DEFLATE expands at most 1032 times.
            let mut out = Vec::with_capacity(size.min(raw.len().saturating_mul(1032)));
            flate2::read::DeflateDecoder::new(raw).take(e.size + 1).read_to_end(&mut out).map_err(|err| malformed(format!("{what}: {err}")))?;
            if out.len() != size {
                return Err(malformed(format!("{what} has the wrong size")));
            }
            out
        };
        let mut crc = flate2::Crc::new();
        crc.update(&data);
        if crc.sum() != e.crc {
            return Err(malformed(format!("{what} failed its CRC check")));
        }
        Ok(data)
    }
}

/// Does a local header in `head` name `name`? Walks the local headers from the start, for a
/// head too short to reach the central directory; stops at an entry whose size is only in a
/// data descriptor.
pub(crate) fn local_names_include(head: &[u8], name: &str) -> bool {
    let mut p = 0usize;
    while u32_at(head, p) == Some(LOCAL_SIG) {
        let (Some(flags), Some(compressed), Some(name_len), Some(extra_len)) =
            (u16_at(head, p + 6), u32_at(head, p + 18), u16_at(head, p + 26), u16_at(head, p + 28))
        else {
            return false;
        };
        let start = p + LOCAL_LEN;
        let end = start + usize::from(name_len);
        if head.get(start..end) == Some(name.as_bytes()) {
            return true;
        }
        if flags & 8 != 0 {
            return false;
        }
        p = match end.checked_add(usize::from(extra_len)).and_then(|q| q.checked_add(compressed as usize)) {
            Some(q) => q,
            None => return false,
        };
    }
    false
}
