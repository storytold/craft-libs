#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! ZIP archives (also APK, JAR, IPA, MSIX, HAP…) read leniently from the raw bytes, for
//! inspectors that must list and describe damaged, prepended, split or oddly encoded archives
//! rather than refuse them (feature `lenient`). The rest of this crate is the strict side: it
//! refuses what fails [`crate::preflight`] and reads through the `zip` crate; nothing here uses
//! `zip`.
//!
//! Only the central directory is trusted for the listing. Entry data is located through the local
//! header, decompressed with a hard output cap, and checked against the recorded CRC-32. Stored,
//! DEFLATE, bzip2, ZIP LZMA, XZ and Zstandard entries are supported; other methods and encrypted entries are listed.
//!
//! The bzip2, LZMA/XZ and Zstandard methods sit behind the `lenient-bzip2`, `lenient-lzma` and
//! `lenient-zstd` features; without one, entries using it are listed and reading them is
//! [`ZipError::Unsupported`]. [`file::read_named_entry`] reads one entry from a seekable file
//! without loading the archive. With the `serde` feature the listing types are `Serialize`.
//!
//! From Craft Anything (`crates/formats/src/zip.rs` and `zip_file.rs`), with its hardening:
//! directory and extra-field bounds, ZIP64 end-record recovery after a prepended stub, refusal of
//! multi-volume archives, Info-ZIP Unicode Path names checked by CRC, decoded sizes checked
//! against the directory, and CRC-32 verification.

mod bytes;
#[cfg(any(feature = "lenient-bzip2", feature = "lenient-lzma", feature = "lenient-zstd"))]
mod decode;
pub mod file;

use crate::lenient::bytes::Reader;

/// Biggest tail we search for the end-of-central-directory record (64 KiB comment + record).
pub const EOCD_SEARCH: usize = 22 + 0xffff;
const ZIP64_FIXED_SIZE: usize = 56;
/// Bound the search for a record shifted by a prepended stub, including extensible data.
const ZIP64_RECOVERY_SEARCH: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ZipError {
    #[error("not a ZIP archive (no end-of-central-directory record)")]
    NotZip,
    #[error("ZIP is truncated: {0}")]
    Truncated(&'static str),
    #[error("ZIP is corrupt: {0}")]
    Corrupt(String),
    #[error("unsupported ZIP feature: {0}")]
    Unsupported(String),
    #[error("entry is larger than the {0}-byte limit")]
    TooLarge(u64),
    #[error("CRC-32 mismatch (expected {expected:08x}, found {found:08x})")]
    CrcMismatch { expected: u32, found: u32 },
}

/// One central-directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ZipEntry {
    /// Entry name as stored (forward slashes). Bit 11 selects lossy UTF-8; otherwise
    /// a valid Unicode Path extra field takes precedence over the UTF-8/CP437 fallback.
    pub name: String,
    pub method: u16,
    pub flags: u16,
    pub crc32: u32,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    /// Offset of the local header, already adjusted for any prepended data.
    pub local_header_offset: u64,
    pub is_dir: bool,
    pub encrypted: bool,
    /// Unix mode from the external attributes (when stored by Unix or macOS).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub unix_mode: Option<u32>,
}

impl ZipEntry {
    /// Why this entry is not a regular file or directory (symlink, device, FIFO, socket), if so.
    pub fn special_kind(&self) -> Option<&'static str> {
        match self.unix_mode? & 0o170000 {
            0o120000 => Some("symlink"),
            0o020000 => Some("character device"),
            0o060000 => Some("block device"),
            0o010000 => Some("FIFO"),
            0o140000 => Some("socket"),
            _ => None,
        }
    }

    /// Human name of the compression method.
    pub fn method_name(&self) -> &'static str {
        match self.method {
            0 => "stored",
            8 => "deflate",
            9 => "deflate64",
            12 => "bzip2",
            14 => "lzma",
            93 => "zstd",
            95 => "xz",
            99 => "aes",
            _ => "other",
        }
    }
}

/// The parsed central directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ZipArchive {
    pub entries: Vec<ZipEntry>,
    /// Entries the directory declares (may exceed `entries.len()` when truncated).
    pub declared_entries: u64,
    /// True when `max_entries` stopped the listing early.
    pub truncated: bool,
    pub zip64: bool,
    /// Bytes before the archive proper (self-extracting stubs).
    pub prefix_bytes: u64,
    /// Inconsistent end-record metadata retained without hiding a readable directory.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
    pub warnings: Vec<String>,
}

/// True when the bytes start like a ZIP (local header or empty archive).
pub fn looks_like_zip(data: &[u8]) -> bool {
    data.starts_with(b"PK\x03\x04") || data.starts_with(b"PK\x05\x06") || data.starts_with(b"PK\x07\x08")
}

/// Recover a shifted end record only when its fixed fields still describe the classic
/// directory. Longer records must frame their extension exactly; the adjacent fixed
/// record can still report an inconsistent size through the normal warning path.
fn recover_zip64_record(data: &[u8], loc: usize, disk_entries: u64, total: u64, cd_size: u64, cd_offset: u64) -> Option<usize> {
    let r = Reader::new(data);
    let last = loc.checked_sub(ZIP64_FIXED_SIZE)?;
    let first = loc.saturating_sub(ZIP64_RECOVERY_SEARCH);
    let candidates = data.get(first..last.checked_add(4)?)?;
    candidates.windows(4).enumerate().rev().find_map(|(relative, signature)| {
        if signature != b"PK\x06\x06" {
            return None;
        }
        let rec = first.checked_add(relative)?;
        let fixed = Reader::new(r.slice(rec, ZIP64_FIXED_SIZE)?);
        let payload_size = fixed.u64_le(4)?;
        if payload_size < 44 {
            return None;
        }
        let declared_end = (rec as u64).checked_add(12)?.checked_add(payload_size)?;
        if rec != last && declared_end != loc as u64 {
            return None;
        }
        let record_disk_entries = fixed.u64_le(24)?;
        let record_total = fixed.u64_le(32)?;
        let record_cd_size = fixed.u64_le(40)?;
        let record_cd_offset = fixed.u64_le(48)?;
        if fixed.u32_le(16)? != 0
            || fixed.u32_le(20)? != 0
            || record_disk_entries != record_total
            || (disk_entries != 0xffff && disk_entries != record_disk_entries)
            || (total != 0xffff && total != record_total)
            || (cd_size != 0xffff_ffff && cd_size != record_cd_size)
            || (cd_offset != 0xffff_ffff && cd_offset != record_cd_offset)
            || record_cd_offset.checked_add(record_cd_size)? > rec as u64
        {
            return None;
        }
        if record_total > 0 {
            let directory = rec.checked_sub(usize::try_from(record_cd_size).ok()?)?;
            if record_cd_size < 46 || r.slice(directory, 4) != Some(b"PK\x01\x02") {
                return None;
            }
        }
        Some(rec)
    })
}

/// Parse the central directory, keeping at most `max_entries`.
pub fn parse(data: &[u8], max_entries: usize) -> Result<ZipArchive, ZipError> {
    parse_directory(data, max_entries, None)
}

/// Search the validated directory while retaining only the requested entry.
pub fn has_entry(data: &[u8], name: &str) -> bool {
    parse_directory(data, 1, Some(name)).is_ok_and(|archive| !archive.entries.is_empty())
}

/// List directory entries, optionally retaining only a matching decoded name.
fn parse_directory(data: &[u8], max_entries: usize, name_filter: Option<&str>) -> Result<ZipArchive, ZipError> {
    let r = Reader::new(data);
    let eocd = find_eocd(data).ok_or(ZipError::NotZip)?;
    let mut disk = u32::from(r.u16_le(eocd + 4).ok_or(ZipError::Truncated("end record"))?);
    let mut directory_disk = u32::from(r.u16_le(eocd + 6).ok_or(ZipError::Truncated("end record"))?);
    let mut disk_entries = u64::from(r.u16_le(eocd + 8).ok_or(ZipError::Truncated("end record"))?);
    if !matches!(disk, 0 | 0xffff) || !matches!(directory_disk, 0 | 0xffff) {
        return Err(ZipError::Unsupported("multi-volume ZIP archives".into()));
    }
    let mut total = u64::from(r.u16_le(eocd + 10).ok_or(ZipError::Truncated("end record"))?);
    let mut cd_size = u64::from(r.u32_le(eocd + 12).ok_or(ZipError::Truncated("end record"))?);
    let mut cd_offset = u64::from(r.u32_le(eocd + 16).ok_or(ZipError::Truncated("end record"))?);
    let mut zip64 = false;
    let mut warnings = Vec::new();
    let mut eocd_record_start = eocd as u64;
    if total == 0xffff || disk_entries == 0xffff || disk == 0xffff || directory_disk == 0xffff || cd_size == 0xffff_ffff || cd_offset == 0xffff_ffff {
        // ZIP64 locator sits right before the classic record.
        if let Some(loc) = eocd.checked_sub(20)
            && r.slice(loc, 4) == Some(b"PK\x06\x07")
        {
            if r.u32_le(loc + 4) != Some(0) || r.u32_le(loc + 16) != Some(1) {
                return Err(ZipError::Unsupported("multi-volume ZIP archives".into()));
            }
            let rec = r.u64_le(loc + 8).ok_or(ZipError::Truncated("zip64 locator"))?;
            let rec = usize::try_from(rec).ok().filter(|&rec| r.slice(rec, 4) == Some(b"PK\x06\x06"));
            let rec = rec.or_else(|| {
                let recovered = recover_zip64_record(data, loc, disk_entries, total, cd_size, cd_offset);
                if recovered.is_some() {
                    warnings.push("ZIP64 locator offset does not identify its end record; recovered the record before the locator".into());
                }
                recovered
            });
            if let Some(rec) = rec {
                let payload_size = r.u64_le(rec + 4).ok_or(ZipError::Truncated("zip64 record size"))?;
                if payload_size < 44 {
                    return Err(ZipError::Corrupt("ZIP64 end record is smaller than its fixed fields".into()));
                }
                let declared_end = (rec as u64)
                    .checked_add(12)
                    .and_then(|start| start.checked_add(payload_size))
                    .ok_or_else(|| ZipError::Corrupt("ZIP64 end record extent overflows".into()))?;
                // Fixed fields must be present before the locator. A bad declared extension
                // length cannot make the reader borrow locator or classic end-record bytes.
                if rec.checked_add(ZIP64_FIXED_SIZE).is_none_or(|end| end > loc) {
                    return Err(ZipError::Truncated("zip64 record fixed fields"));
                }
                if declared_end != loc as u64 {
                    warnings.push("ZIP64 end record's declared extent does not end at its locator; using bounded fixed fields".into());
                }
                disk = r.u32_le(rec + 16).ok_or(ZipError::Truncated("zip64 record"))?;
                directory_disk = r.u32_le(rec + 20).ok_or(ZipError::Truncated("zip64 record"))?;
                let zip64_disk_entries = r.u64_le(rec + 24).ok_or(ZipError::Truncated("zip64 record"))?;
                let zip64_total = r.u64_le(rec + 32).ok_or(ZipError::Truncated("zip64 record"))?;
                if (disk_entries != 0xffff && disk_entries != zip64_disk_entries) || (total != 0xffff && total != zip64_total) {
                    return Err(ZipError::Unsupported("multi-volume ZIP archives".into()));
                }
                disk_entries = zip64_disk_entries;
                total = zip64_total;
                let zip64_cd_size = r.u64_le(rec + 40).ok_or(ZipError::Truncated("zip64 record"))?;
                let zip64_cd_offset = r.u64_le(rec + 48).ok_or(ZipError::Truncated("zip64 record"))?;
                if (cd_size != 0xffff_ffff && cd_size != zip64_cd_size) || (cd_offset != 0xffff_ffff && cd_offset != zip64_cd_offset) {
                    return Err(ZipError::Corrupt("ZIP64 and classic directory bounds disagree".into()));
                }
                cd_size = zip64_cd_size;
                cd_offset = zip64_cd_offset;
                zip64 = true;
                eocd_record_start = rec as u64;
            } else {
                // A prepended stub can invalidate the locator's absolute offset. Keep the
                // classic-directory fallback, including its existing prefix adjustment.
                warnings.push("ZIP64 locator does not point to an end record; using the classic end record".into());
            }
        }
    }
    if disk != 0 || directory_disk != 0 || disk_entries != total {
        return Err(ZipError::Unsupported("multi-volume ZIP archives".into()));
    }
    // Self-extracting archives: the directory sits later than it claims by the stub length.
    let expected_end = cd_offset.checked_add(cd_size).ok_or_else(|| ZipError::Corrupt("directory bounds overflow".into()))?;
    let prefix = eocd_record_start.saturating_sub(expected_end);
    let start = usize::try_from(cd_offset.saturating_add(prefix)).map_err(|_| ZipError::Corrupt("directory offset".into()))?;
    let directory_end = (start as u64)
        .checked_add(cd_size)
        .filter(|&end| end <= eocd_record_start)
        .and_then(|end| usize::try_from(end).ok())
        .ok_or_else(|| ZipError::Corrupt("directory extends into its end record".into()))?;
    let mut entries = Vec::new();
    let mut pos = start;
    let mut truncated = false;
    let mut seen = 0u64;
    while seen < total {
        if entries.len() >= max_entries {
            truncated = true;
            break;
        }
        if pos.checked_add(46).is_none_or(|end| end > directory_end) {
            return Err(ZipError::Truncated("directory entry"));
        }
        if r.slice(pos, 4) != Some(b"PK\x01\x02") {
            if entries.is_empty() {
                return Err(ZipError::Corrupt(format!("no central directory header at offset {pos}")));
            }
            truncated = true;
            break;
        }
        let flags = r.u16_le(pos + 8).ok_or(ZipError::Truncated("directory entry"))?;
        let method = r.u16_le(pos + 10).ok_or(ZipError::Truncated("directory entry"))?;
        let crc32 = r.u32_le(pos + 16).ok_or(ZipError::Truncated("directory entry"))?;
        let mut csize = u64::from(r.u32_le(pos + 20).ok_or(ZipError::Truncated("directory entry"))?);
        let mut usize_ = u64::from(r.u32_le(pos + 24).ok_or(ZipError::Truncated("directory entry"))?);
        let name_len = usize::from(r.u16_le(pos + 28).ok_or(ZipError::Truncated("directory entry"))?);
        let extra_len = usize::from(r.u16_le(pos + 30).ok_or(ZipError::Truncated("directory entry"))?);
        let comment_len = usize::from(r.u16_le(pos + 32).ok_or(ZipError::Truncated("directory entry"))?);
        let entry_end = pos
            .checked_add(46 + name_len + extra_len + comment_len)
            .filter(|&end| end <= directory_end)
            .ok_or(ZipError::Truncated("directory entry fields"))?;
        let host = r.slice(pos + 5, 1).and_then(|b| b.first().copied()).unwrap_or(0);
        let attrs = r.u32_le(pos + 38).ok_or(ZipError::Truncated("entry attributes"))?;
        let unix_mode = matches!(host, 3 | 19).then_some(attrs >> 16);
        let mut local = u64::from(r.u32_le(pos + 42).ok_or(ZipError::Truncated("directory entry"))?);
        let mut entry_disk = u32::from(r.u16_le(pos + 34).ok_or(ZipError::Truncated("directory entry"))?);
        let name_bytes = r.slice(pos + 46, name_len).ok_or(ZipError::Truncated("entry name"))?;
        let extra = r.slice(pos + 46 + name_len, extra_len).ok_or(ZipError::Truncated("entry extra fields"))?;
        // Each extra field owns its payload: ZIP64 values cannot borrow a following field's bytes.
        let needs_zip64 = usize_ == 0xffff_ffff || csize == 0xffff_ffff || local == 0xffff_ffff || entry_disk == 0xffff;
        let mut found_zip64 = false;
        let mut unicode_name = None;
        let mut name_crc = None;
        let mut x = 0usize;
        let er = Reader::new(extra);
        // Trailing bytes too short for a field header are padding, not truncation.
        while x.saturating_add(4) <= extra.len() {
            let id = er.u16_le(x).ok_or(ZipError::Truncated("extra field header"))?;
            let len = usize::from(er.u16_le(x + 2).ok_or(ZipError::Truncated("extra field header"))?);
            let body = x + 4;
            let field = er.slice(body, len).ok_or(ZipError::Truncated("extra field payload"))?;
            if id == 0x0001 && !found_zip64 {
                let values = Reader::new(field);
                let mut f = 0;
                if usize_ == 0xffff_ffff {
                    usize_ = values.u64_le(f).ok_or(ZipError::Truncated("zip64 uncompressed size"))?;
                    f += 8;
                }
                if csize == 0xffff_ffff {
                    csize = values.u64_le(f).ok_or(ZipError::Truncated("zip64 compressed size"))?;
                    f += 8;
                }
                if local == 0xffff_ffff {
                    local = values.u64_le(f).ok_or(ZipError::Truncated("zip64 local header offset"))?;
                    f += 8;
                }
                if entry_disk == 0xffff {
                    entry_disk = values.u32_le(f).ok_or(ZipError::Truncated("zip64 entry disk"))?;
                }
                found_zip64 = true;
            } else if id == 0x7075 && flags & (1 << 11) == 0 && unicode_name.is_none() {
                let field = Reader::new(field);
                if field.u8(0) == Some(1) && field.u32_le(1) == Some(*name_crc.get_or_insert_with(|| self::crc32(name_bytes))) {
                    unicode_name = field.data().get(5..).and_then(|bytes| std::str::from_utf8(bytes).ok()).filter(|s| !s.is_empty());
                }
            }
            x = body + len;
        }
        if needs_zip64 && !found_zip64 {
            return Err(ZipError::Corrupt("missing ZIP64 extended information".into()));
        }
        if entry_disk != 0 {
            return Err(ZipError::Unsupported("multi-volume ZIP archives".into()));
        }
        let name = unicode_name.map(str::to_owned).unwrap_or_else(|| decode_name(name_bytes, flags)).replace('\\', "/");
        if name_filter.is_none_or(|wanted| wanted == name) {
            entries.push(ZipEntry {
                is_dir: name.ends_with('/')
                    || unix_mode.is_some_and(|m| m & 0o170000 == 0o040000)
                    || (matches!(host, 0 | 10 | 14) && attrs & 0x10 != 0),
                name,
                method,
                flags,
                crc32,
                compressed_size: csize,
                uncompressed_size: usize_,
                local_header_offset: local.saturating_add(prefix),
                encrypted: flags & 1 != 0,
                unix_mode,
            });
        }
        seen += 1;
        pos = entry_end;
    }
    Ok(ZipArchive { entries, declared_entries: total, truncated, zip64, prefix_bytes: prefix, warnings })
}

fn decode_name(bytes: &[u8], flags: u16) -> String {
    if flags & 0x0800 != 0 || bytes.is_ascii() {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    if let Ok(name) = std::str::from_utf8(bytes) {
        return name.to_owned();
    }
    const CP437: [char; 128] = [
        'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', 'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢',
        '£', '¥', '₧', 'ƒ', 'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', '░', '▒', '▓', '│', '┤', '╡', '╢', '╖',
        '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', '╨', '╤', '╥', '╙',
        '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', 'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩',
        '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{00a0}',
    ];
    bytes
        .iter()
        .map(|&byte| if byte.is_ascii() { char::from(byte) } else { CP437.get(usize::from(byte - 0x80)).copied().unwrap_or('\u{fffd}') })
        .collect()
}

/// Where the end-of-central-directory record starts, searching backwards through the last
/// [`EOCD_SEARCH`] bytes.
pub fn find_eocd(data: &[u8]) -> Option<usize> {
    if data.len() < 22 {
        return None;
    }
    let from = data.len().saturating_sub(EOCD_SEARCH);
    let tail = data.get(from..)?;
    // Search backwards so a stray signature in the comment can't shadow the real record.
    let mut i = tail.len().checked_sub(22)?;
    let mut found = None;
    let mut last_signature = None;
    loop {
        if tail.get(i..i + 4) == Some(b"PK\x05\x06") {
            last_signature.get_or_insert(from + i);
            if let Some(comment_len) = tail.get(i + 20..i + 22).and_then(|bytes| <[u8; 2]>::try_from(bytes).ok()).map(u16::from_le_bytes)
                && i + 22 + usize::from(comment_len) == tail.len()
            {
                found = Some(from + i);
            }
        }
        if i == 0 {
            return found.or(last_signature);
        }
        i -= 1;
    }
}

/// The raw (possibly compressed) bytes of an entry.
pub fn raw_entry_data<'a>(data: &'a [u8], e: &ZipEntry) -> Result<&'a [u8], ZipError> {
    let r = Reader::new(data);
    let off = usize::try_from(e.local_header_offset).map_err(|_| ZipError::Corrupt("local header offset".into()))?;
    if r.slice(off, 4) != Some(b"PK\x03\x04") {
        return Err(ZipError::Corrupt(format!("no local header for {}", e.name)));
    }
    let n = usize::from(r.u16_le(off + 26).ok_or(ZipError::Truncated("local header"))?);
    let x = usize::from(r.u16_le(off + 28).ok_or(ZipError::Truncated("local header"))?);
    let start = off.checked_add(30 + n + x).ok_or(ZipError::Truncated("local header"))?;
    let len = usize::try_from(e.compressed_size).map_err(|_| ZipError::TooLarge(e.compressed_size))?;
    r.slice(start, len).ok_or(ZipError::Truncated("entry data"))
}

/// Read and decompress one entry, refusing to produce more than `max_bytes`.
pub fn read_entry(data: &[u8], e: &ZipEntry, max_bytes: u64) -> Result<Vec<u8>, ZipError> {
    let out = read_entry_unverified(data, e, max_bytes)?;
    let found = crc32(&out);
    if found != e.crc32 {
        return Err(ZipError::CrcMismatch { expected: e.crc32, found });
    }
    Ok(out)
}

/// Like [`read_entry`] but without comparing the stored CRC-32, so a caller that accepts a mismatch can still get the bytes.
pub fn read_entry_unverified(data: &[u8], e: &ZipEntry, max_bytes: u64) -> Result<Vec<u8>, ZipError> {
    if e.encrypted {
        return Err(ZipError::Unsupported(format!("{} is encrypted", e.name)));
    }
    if e.uncompressed_size > max_bytes {
        return Err(ZipError::TooLarge(max_bytes));
    }
    let raw = raw_entry_data(data, e)?;
    let cap = usize::try_from(max_bytes).unwrap_or(usize::MAX);
    let out = match e.method {
        0 if raw.len() as u64 > max_bytes => return Err(ZipError::TooLarge(max_bytes)),
        0 => raw.to_vec(),
        8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, cap).map_err(|err| match err.status {
            miniz_oxide::inflate::TINFLStatus::HasMoreOutput => ZipError::TooLarge(max_bytes),
            _ => ZipError::Corrupt(format!("deflate stream for {} is damaged", e.name)),
        })?,
        12 | 95 => decode_bzip2_or_xz(raw, e, max_bytes)?,
        14 => decode_lzma(raw, e, max_bytes)?,
        93 => decode_zstd(raw, e, max_bytes)?,
        m => return Err(ZipError::Unsupported(format!("compression method {m} ({}) for {}", e.method_name(), e.name))),
    };
    if out.len() as u64 != e.uncompressed_size {
        return Err(ZipError::Corrupt(format!(
            "decoded size for {} is {}, but its directory entry declares {}",
            e.name,
            out.len(),
            e.uncompressed_size
        )));
    }
    Ok(out)
}

#[cfg(any(feature = "lenient-bzip2", feature = "lenient-lzma"))]
fn decode_bzip2_or_xz(raw: &[u8], e: &ZipEntry, max_bytes: u64) -> Result<Vec<u8>, ZipError> {
    let decoded = match e.method {
        #[cfg(feature = "lenient-bzip2")]
        12 => decode::bzip2(raw, max_bytes),
        #[cfg(feature = "lenient-lzma")]
        95 => decode::xz(raw, max_bytes),
        _ => return Err(not_built(e)),
    };
    decoded.map_err(|error| match error {
        decode::DecompressError::TooLarge => ZipError::TooLarge(max_bytes),
        _ => ZipError::Corrupt(format!("{} stream for {} is damaged: {error}", e.method_name(), e.name)),
    })
}

#[cfg(not(any(feature = "lenient-bzip2", feature = "lenient-lzma")))]
fn decode_bzip2_or_xz(_: &[u8], e: &ZipEntry, _: u64) -> Result<Vec<u8>, ZipError> {
    Err(not_built(e))
}

#[cfg(feature = "lenient-lzma")]
fn decode_lzma(raw: &[u8], e: &ZipEntry, max_bytes: u64) -> Result<Vec<u8>, ZipError> {
    let header = Reader::new(raw);
    let properties_size = header.u16_le(2).ok_or(ZipError::Truncated("LZMA properties header"))?;
    if properties_size != 5 {
        return Err(ZipError::Unsupported(format!("LZMA properties size {properties_size} for {}", e.name)));
    }
    let stream = raw.get(4..).ok_or(ZipError::Truncated("LZMA properties"))?;
    decode::lzma_zip(stream, e.uncompressed_size, e.flags & 2 != 0, max_bytes).map_err(|error| match error {
        decode::DecompressError::TooLarge => ZipError::TooLarge(max_bytes),
        _ => ZipError::Corrupt(format!("LZMA stream for {} is damaged: {error}", e.name)),
    })
}

#[cfg(not(feature = "lenient-lzma"))]
fn decode_lzma(_: &[u8], e: &ZipEntry, _: u64) -> Result<Vec<u8>, ZipError> {
    Err(not_built(e))
}

#[cfg(feature = "lenient-zstd")]
fn decode_zstd(raw: &[u8], e: &ZipEntry, max_bytes: u64) -> Result<Vec<u8>, ZipError> {
    decode::zstd(raw, max_bytes).map_err(|err| ZipError::Corrupt(format!("zstd stream for {} is damaged: {err}", e.name)))
}

#[cfg(not(feature = "lenient-zstd"))]
fn decode_zstd(_: &[u8], e: &ZipEntry, _: u64) -> Result<Vec<u8>, ZipError> {
    Err(not_built(e))
}

/// An entry whose method this build leaves out (see the crate features).
#[cfg_attr(all(feature = "lenient-bzip2", feature = "lenient-lzma", feature = "lenient-zstd"), allow(dead_code))]
fn not_built(e: &ZipEntry) -> ZipError {
    ZipError::Unsupported(format!("compression method {} ({}) for {} is not built in", e.method, e.method_name(), e.name))
}

/// CRC-32 (IEEE, as used by ZIP and PNG).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod testkit;

#[cfg(test)]
mod tests;
