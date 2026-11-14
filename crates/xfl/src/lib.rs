//! Reads the hostile ZIP package used by an Animate `.fla` (an archive of an XFL folder),
//! including its XFL-specific path rules and resource limits.
//!
//! Implements the parts of the published ZIP format (PKWARE APPNOTE 6.3) that archive tools
//! write: the end-of-central-directory record (and its ZIP64 form), central directory entries
//! with ZIP64 sizes, local headers, data descriptors, "stored" and "deflate" entries (inflated
//! by `miniz_oxide`), UTF-8 names (flag bit 11 or the Info-ZIP Unicode path field) and CP437
//! names. Self-extracting archives with ZIP data after a program stub are supported whether
//! their offsets count from the ZIP data or from the start of the file. Encryption, other
//! compression methods and split archives are refused with a clear error.
//!
//! The archive is treated as hostile. Every offset and size is checked against the bytes it
//! comes from; entries may not overlap each other or the central directory; a name that would
//! leave the package (`..`, absolute or drive paths), is listed twice or is nested deeper than
//! [`MAX_DEPTH`] refuses the archive; an entry may not claim more than deflate can produce from
//! its bytes ([`MAX_RATIO`]), more than [`MAX_ENTRY`], or a size or checksum it doesn't have;
//! and the archive hands out at most [`MAX_TOTAL`] bytes in all. Nothing is ever written to
//! disk. Never panics.
//!
//! [`edges`] decodes native shape-edge attributes into neutral points and line/quadratic
//! segments for consumers to map into their own geometry models.

use std::cell::Cell;
use std::collections::HashMap;

/// XFL's edge-attribute grammar, independent of any consuming application's geometry types.
pub mod edges;

/// Largest single entry inflated (bytes).
pub const MAX_ENTRY: usize = 256 * 1024 * 1024;
/// Most bytes one archive hands out, over every read (the import's memory budget).
pub const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
/// Most entries an archive may list (files and folders).
pub const MAX_ENTRIES: usize = 100_000;
/// Most folders a path may be inside (Animate's library nests a few deep).
pub const MAX_DEPTH: usize = 32;
/// Largest size-to-compressed-size ratio believed: deflate itself can't pass about 1032 : 1,
/// so an entry claiming more is a zip bomb or a damaged size, refused before inflating.
pub const MAX_RATIO: u64 = 1024;

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum ZipError {
    #[error("not a zip archive (no end-of-directory record)")]
    NotZip,
    #[error("the archive is damaged: {0}")]
    Damaged(&'static str),
    #[error("the archive is damaged: '{0}' {1}")]
    DamagedEntry(String, &'static str),
    #[error("the archive uses a zip feature this XFL reader can't read ({0})")]
    UnsupportedArchive(&'static str),
    #[error("'{0}' uses a zip feature this XFL reader can't read ({1})")]
    Unsupported(String, &'static str),
    #[error("'{0}' is too large to read")]
    TooLarge(String),
    #[error("the archive asks for more than {} MB in all", MAX_TOTAL / (1024 * 1024))]
    TooMuch,
    #[error("the archive lists more than {MAX_ENTRIES} files")]
    TooMany,
    #[error("'{0}' claims to expand more than compression allows (a zip bomb or a damaged archive)")]
    Bomb(String),
    #[error("'{0}' and '{1}' share bytes in the archive (a zip bomb or a damaged archive)")]
    Overlap(String, String),
    #[error("'{0}' points outside the document")]
    Unsafe(String),
    #[error("'{0}' is in the archive twice")]
    Duplicate(String),
    #[error("'{0}' is inside more than {MAX_DEPTH} folders")]
    TooDeep(String),
}

#[derive(Debug, Clone)]
struct Entry {
    method: u16,
    flags: u16,
    crc: u32,
    comp: usize,
    size: usize,
    /// Where its (compressed) bytes start.
    data: usize,
}

pub struct Archive<'a> {
    data: &'a [u8],
    entries: HashMap<String, Entry>,
    folders: usize,
    /// Bytes still to hand out ([`MAX_TOTAL`] at first).
    budget: Cell<u64>,
}

const END: u32 = 0x0605_4b50;
const END64: u32 = 0x0606_4b50;
const LOCATOR64: u32 = 0x0706_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

fn get(d: &[u8], at: usize, len: usize) -> Option<&[u8]> {
    d.get(at..at.checked_add(len)?)
}
fn u16_at(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(get(d, i, 2)?.try_into().ok()?))
}
fn u32_at(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(get(d, i, 4)?.try_into().ok()?))
}
fn u64_at(d: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(get(d, i, 8)?.try_into().ok()?))
}

/// The body of the extra field `id`, if `extra` has one.
fn extra_field(mut extra: &[u8], id: u16) -> Option<&[u8]> {
    while extra.len() >= 4 {
        let (tag, len) = (u16_at(extra, 0)?, u16_at(extra, 2)? as usize);
        let body = get(extra, 4, len)?;
        if tag == id {
            return Some(body);
        }
        extra = extra.get(4 + len..)?;
    }
    None
}

/// CRC-32 tables for eight bytes at a time ("slicing by 8"): `TABLES[0]` is the classic
/// byte table, `TABLES[k][b]` the checksum of byte `b` followed by `k` zero bytes.
const TABLES: [[u32; 256]; 8] = {
    let mut t = [[0u32; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[0][i] = c;
        i += 1;
    }
    let mut k = 1;
    while k < 8 {
        let mut i = 0;
        while i < 256 {
            let prev = t[k - 1][i];
            t[k][i] = (prev >> 8) ^ t[0][(prev & 0xff) as usize];
            i += 1;
        }
        k += 1;
    }
    t
};

/// CRC-32 (ISO-HDLC, the zip checksum), eight bytes at a time.
pub fn crc32(data: &[u8]) -> u32 {
    let t = &TABLES;
    let mut c = !0u32;
    let (blocks, rest) = data.as_chunks::<8>();
    for b in blocks {
        let lo = c ^ u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        c = t[7][(lo & 0xff) as usize]
            ^ t[6][((lo >> 8) & 0xff) as usize]
            ^ t[5][((lo >> 16) & 0xff) as usize]
            ^ t[4][(lo >> 24) as usize]
            ^ t[3][b[4] as usize]
            ^ t[2][b[5] as usize]
            ^ t[1][b[6] as usize]
            ^ t[0][b[7] as usize];
    }
    for &b in rest {
        c = t[0][((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

/// Code page 437, bytes 0x80 to 0xFF: the zip format's name encoding without the UTF-8 flag.
const CP437: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', //
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', //
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', //
    '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', //
    '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', //
    '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', //
    'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', //
    '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{a0}',
];

/// An entry's name as text: the Info-ZIP Unicode path field when it matches the name, UTF-8
/// when flag bit 11 says so, else CP437 as the format specifies. Many writers store UTF-8
/// without setting the flag, so a name that is valid UTF-8 is read as UTF-8 (ASCII, which
/// nearly every name is, reads the same either way).
fn decode_name(raw: &[u8], utf8: bool, extra: &[u8]) -> Result<String, ZipError> {
    if let Some(up) = extra_field(extra, 0x7075)
        && up.first() == Some(&1)
        && u32_at(up, 1) == Some(crc32(raw))
        && let Some(Ok(s)) = up.get(5..).map(std::str::from_utf8)
    {
        return Ok(s.to_string());
    }
    match std::str::from_utf8(raw) {
        Ok(s) => Ok(s.to_string()),
        Err(_) if utf8 => Err(ZipError::Damaged("an entry's name is not the UTF-8 it says it is")),
        Err(_) => Ok(raw.iter().map(|&b| if b < 0x80 { b as char } else { CP437.get((b - 0x80) as usize).copied().unwrap_or('\u{fffd}') }).collect()),
    }
}

/// A name as a plain path inside the package: `\` read as `/`, empty and `.` parts dropped.
/// Names that would leave the package are refused, though nothing is ever extracted: every
/// lookup stays unambiguous, as the folder form's reader also refuses them.
fn clean_path(name: &str) -> Result<String, ZipError> {
    let unix = name.replace('\\', "/");
    let shown = || name.chars().take(200).collect::<String>();
    if unix.starts_with('/') || unix.contains('\0') {
        return Err(ZipError::Unsafe(shown()));
    }
    let mut parts = Vec::new();
    for part in unix.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(ZipError::Unsafe(shown())),
            // `C:`, `C:x`: a drive.
            p if parts.is_empty() && p.contains(':') => return Err(ZipError::Unsafe(shown())),
            p => parts.push(p),
        }
    }
    if parts.len() > MAX_DEPTH + 1 {
        return Err(ZipError::TooDeep(shown()));
    }
    Ok(parts.join("/"))
}

fn method_name(method: u16) -> &'static str {
    match method {
        9 => "Deflate64 compression",
        12 => "bzip2 compression",
        14 => "LZMA compression",
        93 => "Zstandard compression",
        95 => "XZ compression",
        99 => "AES encryption",
        _ => "an unknown compression method",
    }
}

/// The end of the central directory: (entry count, its size, its offset, where it must end).
///
/// For a self-extracting archive, the record's offsets may be relative to the ZIP data rather
/// than the start of the executable. The returned directory offset stays as written; its real
/// location is resolved against `dir_limit` when opening the archive.
fn find_end(data: &[u8]) -> Result<(u64, u64, u64, usize), ZipError> {
    // The end record is in the last 22 + 65535 bytes (it may carry a comment).
    let lo = data.len().saturating_sub(22 + 65_535);
    let mut eocd = None;
    // A shifted empty SFX archive has no central-directory signature with which to distinguish
    // it from an empty EOCD-shaped record inside the preceding archive's comment. Keep it as a
    // last resort while searching for an older, structurally stronger candidate.
    let mut empty_sfx = None;
    let mut fallback = None;
    // Fake EOCD records in a comment can all describe the same large directory. Cache binary
    // directory jumps so checking each candidate does not rescan every entry.
    let mut directory_jumps = HashMap::new();
    for candidate in (lo..data.len().saturating_sub(21)).rev() {
        if u32_at(data, candidate) != Some(END)
            || !u16_at(data, candidate + 20)
                .is_some_and(|comment| candidate.checked_add(22 + usize::from(comment)).is_some_and(|end| end <= data.len()))
        {
            continue;
        }
        // Keep the earliest bounds-valid candidate's eventual parse error if no structurally
        // consistent candidate is found. This preserves useful damaged-ZIP diagnostics.
        fallback = Some(candidate);

        let count = u16_at(data, candidate + 10).unwrap_or(u16::MAX);
        let size = u32_at(data, candidate + 12).unwrap_or(u32::MAX);
        let offset = u32_at(data, candidate + 16).unwrap_or(u32::MAX);
        if count == u16::MAX || size == u32::MAX || offset == u32::MAX {
            let locator = candidate.checked_sub(20).filter(|&at| u32_at(data, at) == Some(LOCATOR64));
            let valid = locator.and_then(|locator| find_zip64_end(data, locator)).and_then(|at| {
                let count = u64_at(data, at + 32)?;
                let dir_size = u64_at(data, at + 40)?;
                let dir_offset = u64_at(data, at + 48)?;
                locate_directory(data, count, dir_size, dir_offset, at)
                    .filter(|&(dir, _)| directory_is_exact(data, dir, dir_size, count, &mut directory_jumps))
                    .map(|(_, shift)| (at, count == 0 && shift > 0))
            });
            let Some((_, shifted_empty)) = valid else {
                continue;
            };
            if shifted_empty {
                empty_sfx = Some(candidate);
                continue;
            }
        } else {
            let Some((_, shift)) = locate_directory(data, count.into(), size.into(), offset.into(), candidate)
                .filter(|&(dir, _)| directory_is_exact(data, dir, size.into(), count.into(), &mut directory_jumps))
            else {
                continue;
            };
            if count == 0 && shift > 0 {
                empty_sfx = Some(candidate);
                continue;
            }
        }
        eocd = Some(candidate);
        break;
    }
    let eocd = eocd.or(empty_sfx).or(fallback).ok_or(ZipError::NotZip)?;
    let field16 = |o: usize| u16_at(data, eocd + o).ok_or(ZipError::Damaged("the end record is cut short"));
    let field32 = |o: usize| u32_at(data, eocd + o).ok_or(ZipError::Damaged("the end record is cut short"));
    let (disk, dir_disk, on_disk, count) = (field16(4)?, field16(6)?, field16(8)?, field16(10)?);
    let (size, offset) = (field32(12)?, field32(16)?);
    if count != 0xffff && size != 0xffff_ffff && offset != 0xffff_ffff {
        if disk != 0 || dir_disk != 0 || on_disk != count {
            return Err(ZipError::UnsupportedArchive("split across several files"));
        }
        return Ok((count.into(), size.into(), offset.into(), eocd));
    }
    // ZIP64: a locator just before the end record points at the ZIP64 end record.
    let locator = eocd.checked_sub(20).filter(|&l| u32_at(data, l) == Some(LOCATOR64)).ok_or(ZipError::Damaged("the ZIP64 end record is missing"))?;
    let missing = ZipError::Damaged("the ZIP64 end record is missing");
    let at = find_zip64_end(data, locator).ok_or(missing)?;
    let cut = ZipError::Damaged("the ZIP64 end record is cut short");
    let (disk, dir_disk) = (u32_at(data, at + 16).ok_or(cut.clone())?, u32_at(data, at + 20).ok_or(cut.clone())?);
    let (on_disk, count) = (u64_at(data, at + 24).ok_or(cut.clone())?, u64_at(data, at + 32).ok_or(cut.clone())?);
    if disk != 0 || dir_disk != 0 || on_disk != count {
        return Err(ZipError::UnsupportedArchive("split across several files"));
    }
    Ok((count, u64_at(data, at + 40).ok_or(cut.clone())?, u64_at(data, at + 48).ok_or(cut)?, at))
}

/// Finds the ZIP64 end record named by `locator`. For an SFX archive with unadjusted offsets,
/// the locator points to where that record was before the stub was prepended, so the record
/// itself immediately before the locator is authoritative. The shifted fallback supports the
/// no-extensible-data layout; ordinary ZIP64 records continue to use the locator's exact offset.
fn find_zip64_end(data: &[u8], locator: usize) -> Option<usize> {
    let stated = usize::try_from(u64_at(data, locator.checked_add(8)?)?).ok()?;
    let valid = |at: usize, shifted: bool| {
        if u32_at(data, at) != Some(END64) {
            return false;
        }
        let Some(record_size_at) = at.checked_add(4) else { return false };
        let Some(record_size) = u64_at(data, record_size_at).and_then(|n| usize::try_from(n).ok()) else { return false };
        if at.checked_add(12).and_then(|start| start.checked_add(record_size)) != Some(locator) || (shifted && record_size != 44) {
            return false;
        }
        let Some(size_at) = at.checked_add(40) else { return false };
        let Some(offset_at) = at.checked_add(48) else { return false };
        let Some(size) = u64_at(data, size_at).and_then(|n| usize::try_from(n).ok()) else { return false };
        let Some(offset) = u64_at(data, offset_at).and_then(|n| usize::try_from(n).ok()) else { return false };
        if !shifted {
            offset.checked_add(size) == Some(at)
        } else {
            let empty_directory = size == 0 && u64_at(data, at + 24) == Some(0) && u64_at(data, at + 32) == Some(0);
            at > stated
                && offset.checked_add(size) == Some(stated)
                && at.checked_sub(size).is_some_and(|dir| dir > offset && (empty_directory || u32_at(data, dir) == Some(CENTRAL)))
        }
    };
    if valid(stated, false) {
        return Some(stated);
    }
    let at = locator.checked_sub(56).filter(|&at| at > stated)?;
    valid(at, true).then_some(at)
}

/// Resolves a directory offset. If the stated offset already points to a central-directory
/// header and the directory ends at `limit`, it is used as is. Otherwise the only correction
/// accepted is a prefix shift that moves the directory to immediately before `limit`.
fn locate_directory(data: &[u8], count: u64, size: u64, stated: u64, limit: usize) -> Option<(usize, usize)> {
    let stated = usize::try_from(stated).ok()?;
    let size = usize::try_from(size).ok()?;
    if stated.checked_add(size) == Some(limit) && (count == 0 || u32_at(data, stated) == Some(CENTRAL)) {
        return Some((stated, 0));
    }
    let actual = limit.checked_sub(size)?;
    if actual > stated && (count == 0 || u32_at(data, actual) == Some(CENTRAL)) {
        return Some((actual, actual - stated));
    }
    None
}

fn central_entry_end(data: &[u8], cursor: usize) -> Option<usize> {
    if u32_at(data, cursor) != Some(CENTRAL) {
        return None;
    }
    let header_end = cursor.checked_add(46).filter(|&at| at <= data.len())?;
    let name_len = usize::from(u16_at(data, header_end - 18)?);
    let extra_len = usize::from(u16_at(data, header_end - 16)?);
    let comment_len = usize::from(u16_at(data, header_end - 14)?);
    header_end.checked_add(name_len)?.checked_add(extra_len)?.checked_add(comment_len).filter(|&end| end <= data.len())
}

/// Return the offset after `2^power` contiguous central-directory records, memoizing offsets
/// shared by competing EOCD candidates in untrusted comments.
fn central_directory_jump(data: &[u8], start: usize, power: u8, jumps: &mut HashMap<(usize, u8), Option<usize>>) -> Option<usize> {
    if let Some(&cached) = jumps.get(&(start, power)) {
        return cached;
    }
    let end = if power == 0 {
        central_entry_end(data, start)
    } else {
        central_directory_jump(data, start, power - 1, jumps).and_then(|middle| central_directory_jump(data, middle, power - 1, jumps))
    };
    jumps.insert((start, power), end);
    end
}

/// Checks that the declared central-directory entries consume the entire directory. This is
/// used while choosing among EOCD-shaped signatures, where comments may contain many fake records.
fn directory_is_exact(data: &[u8], start: usize, size: u64, count: u64, jumps: &mut HashMap<(usize, u8), Option<usize>>) -> bool {
    let Ok(size) = usize::try_from(size) else { return false };
    let Some(end) = start.checked_add(size).filter(|&end| end <= data.len()) else { return false };
    // Preserve the existing TooMany diagnostic in `Archive::open` without walking attacker-sized
    // entry counts here.
    if count > MAX_ENTRIES as u64 {
        return true;
    }
    let mut cursor = start;
    let mut remaining = count;
    let mut power = 0;
    while remaining != 0 {
        if remaining & 1 != 0 {
            let Some(next) = central_directory_jump(data, cursor, power, jumps) else {
                return false;
            };
            cursor = next;
        }
        remaining >>= 1;
        power += 1;
    }
    cursor == end
}

impl<'a> Archive<'a> {
    pub fn open(data: &'a [u8]) -> Result<Archive<'a>, ZipError> {
        let (count, dir_size, dir_at, dir_limit) = find_end(data)?;
        let past = ZipError::Damaged("the central directory is outside the archive");
        let (dir_at, shift) = locate_directory(data, count, dir_size, dir_at, dir_limit).ok_or(past.clone())?;
        let dir_end = usize::try_from(dir_size).ok().and_then(|s| dir_at.checked_add(s)).filter(|&e| e <= dir_limit).ok_or(past.clone())?;
        let dir = data.get(dir_at..dir_end).ok_or(past)?;
        if count > MAX_ENTRIES as u64 {
            return Err(ZipError::TooMany);
        }
        let count = count as usize;
        // Every central entry is at least 46 bytes.
        let cut = ZipError::Damaged("the central directory is cut short");
        if count.saturating_mul(46) > dir.len() {
            return Err(cut);
        }
        let mut entries = HashMap::with_capacity(count);
        let mut folders = 0;
        // (start, end, name) of every entry's local header and data.
        let mut spans: Vec<(usize, usize, String)> = Vec::with_capacity(count);
        let mut i = 0usize;
        for _ in 0..count {
            if u32_at(dir, i) != Some(CENTRAL) {
                return Err(cut);
            }
            let short = |o: usize| u16_at(dir, i + o).ok_or(cut.clone());
            let long = |o: usize| u32_at(dir, i + o).ok_or(cut.clone());
            let (flags, method) = (short(8)?, short(10)?);
            let crc = long(16)?;
            let (comp32, size32, offset32) = (long(20)?, long(24)?, long(42)?);
            let (nlen, xlen, clen) = (short(28)? as usize, short(30)? as usize, short(32)? as usize);
            let raw_name = get(dir, i + 46, nlen).ok_or(cut.clone())?;
            let extra = get(dir, i + 46 + nlen, xlen).ok_or(cut.clone())?;
            get(dir, i + 46 + nlen + xlen, clen).ok_or(cut.clone())?;
            i += 46 + nlen + xlen + clen;

            let name = decode_name(raw_name, flags & 0x0800 != 0, extra)?;
            let shown = || name.chars().take(200).collect::<String>();
            // ZIP64: each 32-bit field that overflowed is in the extra field, in this order.
            let (mut size, mut comp, mut offset) = (u64::from(size32), u64::from(comp32), u64::from(offset32));
            if size32 == u32::MAX || comp32 == u32::MAX || offset32 == u32::MAX {
                let z = extra_field(extra, 0x0001).ok_or_else(|| ZipError::DamagedEntry(shown(), "has no ZIP64 sizes"))?;
                let mut at = 0;
                for (wide, value) in [(size32, &mut size), (comp32, &mut comp), (offset32, &mut offset)] {
                    if wide == u32::MAX {
                        *value = u64_at(z, at).ok_or_else(|| ZipError::DamagedEntry(shown(), "has its ZIP64 sizes cut short"))?;
                        at += 8;
                    }
                }
            }
            let outside = || ZipError::DamagedEntry(shown(), "is outside the archive");
            let offset = usize::try_from(offset).ok().and_then(|offset| offset.checked_add(shift)).ok_or_else(outside)?;
            let comp = usize::try_from(comp).map_err(|_| outside())?;
            // An entry's size is checked when it is read: one too large to read may sit unused.
            let size = usize::try_from(size).unwrap_or(usize::MAX);

            // Its local header, then its data, all before the central directory.
            if u32_at(data, offset) != Some(LOCAL) {
                return Err(ZipError::DamagedEntry(shown(), "has no local header"));
            }
            let lnlen = u16_at(data, offset + 26).ok_or_else(outside)? as usize;
            let lxlen = u16_at(data, offset + 28).ok_or_else(outside)? as usize;
            let start = offset + 30 + lnlen + lxlen;
            let end = start.checked_add(comp).filter(|&e| e <= dir_at).ok_or_else(outside)?;
            spans.push((offset, end, shown()));

            let folder = name.ends_with('/') || name.ends_with('\\');
            let path = clean_path(&name)?;
            if folder || path.is_empty() {
                if size != 0 || comp > 2 {
                    return Err(ZipError::DamagedEntry(shown(), "is a folder with data"));
                }
                folders += 1;
                continue;
            }
            if method == 8 && size as u64 > (comp as u64).saturating_mul(MAX_RATIO).saturating_add(4096) {
                return Err(ZipError::Bomb(shown()));
            }
            if method == 0 && flags & 0x41 == 0 && size != comp {
                return Err(ZipError::DamagedEntry(shown(), "is stored with two different sizes"));
            }
            if entries.insert(path, Entry { method, flags, crc, comp, size, data: start }).is_some() {
                return Err(ZipError::Duplicate(shown()));
            }
        }
        if i != dir.len() {
            return Err(ZipError::Damaged("the central directory has trailing data"));
        }
        // No two entries may share bytes (the overlapping-entries zip bomb).
        spans.sort_unstable_by_key(|s| s.0);
        if let Some(w) = spans.windows(2).find(|w| w[0].1 > w[1].0) {
            return Err(ZipError::Overlap(w[0].2.clone(), w[1].2.clone()));
        }
        Ok(Archive { data, entries, folders, budget: Cell::new(MAX_TOTAL) })
    }

    /// The files' paths (`/`-separated, folders left out).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// How many files and folder entries the archive lists.
    pub fn counts(&self) -> (usize, usize) {
        (self.entries.len(), self.folders)
    }

    /// The bytes of entry `name` (path inside the archive, `/`-separated), checked against
    /// its size and checksum.
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>, ZipError> {
        let Some(e) = self.entries.get(name) else { return Ok(None) };
        if e.flags & 0x41 != 0 {
            return Err(ZipError::Unsupported(name.into(), "encryption"));
        }
        if !matches!(e.method, 0 | 8) {
            return Err(ZipError::Unsupported(name.into(), method_name(e.method)));
        }
        if e.size > MAX_ENTRY || e.comp > MAX_ENTRY {
            return Err(ZipError::TooLarge(name.into()));
        }
        let left = self.budget.get();
        if e.size as u64 > left {
            return Err(ZipError::TooMuch);
        }
        let raw = get(self.data, e.data, e.comp).ok_or_else(|| ZipError::DamagedEntry(name.into(), "is outside the archive"))?;
        let out = match e.method {
            0 => raw.to_vec(),
            _ => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, e.size.max(1)).map_err(|err| {
                let why = if err.status == miniz_oxide::inflate::TINFLStatus::HasMoreOutput {
                    "expands past the size it gives"
                } else {
                    "has damaged compressed data"
                };
                ZipError::DamagedEntry(name.into(), why)
            })?,
        };
        if out.len() != e.size {
            return Err(ZipError::DamagedEntry(name.into(), "is not the size it gives"));
        }
        if crc32(&out) != e.crc {
            return Err(ZipError::DamagedEntry(name.into(), "fails its checksum"));
        }
        self.budget.set(left - e.size as u64);
        Ok(Some(out))
    }
}

#[cfg(test)]
#[path = "../tests/support/fla.rs"]
pub(crate) mod pack;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
