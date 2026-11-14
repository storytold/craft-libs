//! Test support: packs an XFL package into `.fla` (zip) archives in memory, in each of the ways
//! zip writers lay one out (ADR 0054), and writes hand-made entries for hostile archives.
//!
//! Written from the published format (PKWARE APPNOTE 6.3), independently of the reader in
//! `src/zip.rs` (it has its own checksum), so a misreading in one is caught by the other.
//! Shared by the xfl crate's unit and corpus tests and the engine's golden tests (`#[path]`).
#![allow(dead_code)]

use std::collections::BTreeMap;

/// How a variant compresses its entries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Compression {
    Deflate,
    Stored,
    /// XML deflated, images stored (as most writers do), the rest alternating.
    Mixed,
}

/// How a variant writes entry names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Names {
    /// UTF-8 with flag bit 11 set.
    Utf8Flag,
    /// UTF-8 without the flag (common, though the format says CP437).
    Utf8Unflagged,
    /// CP437 bytes without the flag, as the format specifies (names CP437 can't spell are
    /// written as UTF-8 with the flag).
    Cp437,
    /// An ASCII stand-in name with the real one in the Info-ZIP Unicode path field.
    UnicodeExtra,
}

/// One way of packing a package.
#[derive(Clone, Copy, Debug)]
pub struct Variant {
    pub name: &'static str,
    pub compression: Compression,
    /// An entry for every folder, before the files in it.
    pub folders: bool,
    /// The `mimetype` entry first, stored, as Animate writes it.
    pub mimetype: bool,
    /// `\` between folders, as some Windows tools write.
    pub backslashes: bool,
    pub names: Names,
    /// Sizes and checksums after the data (bit 3), as streaming writers do.
    pub descriptors: bool,
    /// ZIP64 sizes on every entry and a ZIP64 end record.
    pub zip64: bool,
}

const BASE: Variant = Variant {
    name: "",
    compression: Compression::Deflate,
    folders: false,
    mimetype: false,
    backslashes: false,
    names: Names::Utf8Flag,
    descriptors: false,
    zip64: false,
};

/// Every variant each corpus package is checked in.
pub const VARIANTS: &[Variant] = &[
    Variant { name: "as Animate writes it (mimetype first, deflated)", mimetype: true, ..BASE },
    Variant { name: "stored", compression: Compression::Stored, ..BASE },
    Variant { name: "mixed, with folder entries", compression: Compression::Mixed, folders: true, mimetype: true, ..BASE },
    Variant { name: "Windows separators, unflagged UTF-8 names", backslashes: true, folders: true, names: Names::Utf8Unflagged, ..BASE },
    Variant { name: "CP437 names", names: Names::Cp437, compression: Compression::Mixed, ..BASE },
    Variant { name: "Unicode path fields", names: Names::UnicodeExtra, ..BASE },
    Variant { name: "data descriptors", descriptors: true, compression: Compression::Mixed, ..BASE },
    Variant { name: "ZIP64", zip64: true, compression: Compression::Mixed, ..BASE },
    Variant {
        name: "everything at once",
        compression: Compression::Mixed,
        folders: true,
        mimetype: true,
        backslashes: true,
        names: Names::Cp437,
        descriptors: true,
        zip64: true,
    },
];

/// One entry as written. Fields a hostile test may change are public.
#[derive(Clone, Debug, Default)]
pub struct Raw {
    /// The name's bytes as stored.
    pub name: Vec<u8>,
    /// The uncompressed bytes.
    pub body: Vec<u8>,
    pub deflate: bool,
    /// General purpose flag bits besides 3 and 11 (bit 0: encrypted).
    pub flags: u16,
    pub utf8: bool,
    pub descriptor: bool,
    pub zip64: bool,
    /// Overrides: the method, the uncompressed size and the checksum the headers give.
    pub method: Option<u16>,
    pub size: Option<u64>,
    pub crc: Option<u32>,
    /// Extra field bytes in the central directory (besides ZIP64's).
    pub extra: Vec<u8>,
    /// Points this entry's central record at entry `n`'s local header instead of its own.
    pub share: Option<usize>,
    /// Stored as compressed bytes: these instead of compressing `body`.
    pub compressed: Option<Vec<u8>>,
}

impl Raw {
    pub fn new(name: &str, body: &[u8], deflate: bool) -> Raw {
        Raw { name: name.as_bytes().to_vec(), body: body.to_vec(), deflate, utf8: true, ..Raw::default() }
    }
}

/// CRC-32 (ISO-HDLC), bit by bit.
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// `s` in CP437, if every character is in it.
pub fn cp437(s: &str) -> Option<Vec<u8>> {
    const HIGH: &str =
        "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\u{a0}";
    s.chars().map(|c| if c.is_ascii() { Some(c as u8) } else { HIGH.chars().position(|h| h == c).map(|i| 0x80 + i as u8) }).collect()
}

fn le16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn le64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Writes `entries` as a zip archive, with a ZIP64 end record if `zip64_end`.
pub fn write(entries: &[Raw], zip64_end: bool) -> Vec<u8> {
    write_at(entries, zip64_end, 0)
}

/// Writes an archive whose offsets count from `base` bytes before the ZIP data.
/// This models a self-extracting archive that has been adjusted to count from file start.
pub fn write_at(entries: &[Raw], zip64_end: bool, base: u64) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let mut offsets = Vec::new();
    // (time, date): 2025-11-11 12:00.
    let (time, date) = (0x6000u16, 0x5B6Bu16);
    for e in entries {
        let data = match &e.compressed {
            Some(c) => c.clone(),
            None if e.deflate => miniz_oxide::deflate::compress_to_vec(&e.body, 6),
            None => e.body.clone(),
        };
        let method = e.method.unwrap_or(if e.deflate { 8 } else { 0 });
        let crc = e.crc.unwrap_or_else(|| crc32(&e.body));
        let size = e.size.unwrap_or(e.body.len() as u64);
        let comp = data.len() as u64;
        let flags = e.flags | if e.descriptor { 8 } else { 0 } | if e.utf8 { 0x800 } else { 0 };
        let version: u16 = if e.zip64 { 45 } else { 20 };
        let offset = base + out.len() as u64;
        offsets.push(offset);
        // The local header.
        le32(&mut out, 0x0403_4b50);
        le16(&mut out, version);
        le16(&mut out, flags);
        le16(&mut out, method);
        le16(&mut out, time);
        le16(&mut out, date);
        let local_extra = if e.zip64 {
            let mut x = Vec::new();
            le16(&mut x, 1);
            le16(&mut x, 16);
            le64(&mut x, if e.descriptor { 0 } else { size });
            le64(&mut x, if e.descriptor { 0 } else { comp });
            x
        } else {
            Vec::new()
        };
        if e.descriptor {
            out.extend_from_slice(&[0; 12]);
        } else {
            le32(&mut out, crc);
            le32(&mut out, if e.zip64 { u32::MAX } else { comp as u32 });
            le32(&mut out, if e.zip64 { u32::MAX } else { size as u32 });
        }
        le16(&mut out, e.name.len() as u16);
        le16(&mut out, local_extra.len() as u16);
        out.extend_from_slice(&e.name);
        out.extend_from_slice(&local_extra);
        out.extend_from_slice(&data);
        if e.descriptor {
            le32(&mut out, 0x0807_4b50);
            le32(&mut out, crc);
            if e.zip64 {
                le64(&mut out, comp);
                le64(&mut out, size);
            } else {
                le32(&mut out, comp as u32);
                le32(&mut out, size as u32);
            }
        }
        // Its central record.
        let at = e.share.and_then(|n| offsets.get(n).copied()).unwrap_or(offset);
        let mut extra = Vec::new();
        if e.zip64 {
            le16(&mut extra, 1);
            le16(&mut extra, 24);
            le64(&mut extra, size);
            le64(&mut extra, comp);
            le64(&mut extra, at);
        }
        extra.extend_from_slice(&e.extra);
        le32(&mut central, 0x0201_4b50);
        le16(&mut central, version);
        le16(&mut central, version);
        le16(&mut central, flags);
        le16(&mut central, method);
        le16(&mut central, time);
        le16(&mut central, date);
        le32(&mut central, crc);
        le32(&mut central, if e.zip64 { u32::MAX } else { comp as u32 });
        le32(&mut central, if e.zip64 { u32::MAX } else { size as u32 });
        le16(&mut central, e.name.len() as u16);
        le16(&mut central, extra.len() as u16);
        le16(&mut central, 0); // comment
        le16(&mut central, 0); // disk
        le16(&mut central, 0); // internal attributes
        let folder = e.name.last().is_some_and(|&c| c == b'/' || c == b'\\');
        le32(&mut central, if folder { 0x10 } else { 0 });
        le32(&mut central, if e.zip64 { u32::MAX } else { at as u32 });
        central.extend_from_slice(&e.name);
        central.extend_from_slice(&extra);
    }
    let dir_at = base + out.len() as u64;
    out.extend_from_slice(&central);
    let n = entries.len() as u64;
    if zip64_end {
        let record = base + out.len() as u64;
        le32(&mut out, 0x0606_4b50);
        le64(&mut out, 44);
        le16(&mut out, 45);
        le16(&mut out, 45);
        le32(&mut out, 0);
        le32(&mut out, 0);
        le64(&mut out, n);
        le64(&mut out, n);
        le64(&mut out, central.len() as u64);
        le64(&mut out, dir_at);
        le32(&mut out, 0x0706_4b50);
        le32(&mut out, 0);
        le64(&mut out, record);
        le32(&mut out, 1);
    }
    le32(&mut out, 0x0605_4b50);
    le32(&mut out, 0); // disks
    let (count, size, at) = if zip64_end { (u16::MAX, u32::MAX, u32::MAX) } else { (n as u16, central.len() as u32, dir_at as u32) };
    le16(&mut out, count);
    le16(&mut out, count);
    le32(&mut out, size);
    le32(&mut out, at);
    le16(&mut out, 0);
    out
}

/// The entries `files` (paths inside the package) become in `v`.
pub fn entries(files: &BTreeMap<String, Vec<u8>>, v: &Variant) -> Vec<Raw> {
    let mut out = Vec::new();
    let named = |path: &str, body: &[u8], deflate: bool| {
        let shown = if v.backslashes { path.replace('/', "\\") } else { path.to_string() };
        let mut r = Raw::new(&shown, body, deflate);
        r.descriptor = v.descriptors;
        r.zip64 = v.zip64;
        match v.names {
            Names::Utf8Flag => {}
            Names::Utf8Unflagged => r.utf8 = false,
            Names::Cp437 => {
                if let Some(b) = cp437(&shown) {
                    (r.name, r.utf8) = (b, false);
                }
            }
            Names::UnicodeExtra => {
                let stand_in: Vec<u8> = shown.chars().map(|c| if c.is_ascii() { c as u8 } else { b'_' }).collect();
                le16(&mut r.extra, 0x7075);
                le16(&mut r.extra, (5 + shown.len()) as u16);
                r.extra.push(1);
                le32(&mut r.extra, crc32(&stand_in));
                r.extra.extend_from_slice(shown.as_bytes());
                (r.name, r.utf8) = (stand_in, false);
            }
        }
        r
    };
    if v.mimetype {
        out.push(named("mimetype", b"application/vnd.adobe.xfl", false));
    }
    let mut made = std::collections::BTreeSet::new();
    for (i, (path, body)) in files.iter().enumerate() {
        if v.folders {
            let parts: Vec<&str> = path.split('/').collect();
            for k in 1..parts.len() {
                let folder = format!("{}/", parts[..k].join("/"));
                if made.insert(folder.clone()) {
                    out.push(named(&folder, b"", false));
                }
            }
        }
        let image = [".png", ".jpg", ".jpeg", ".gif"].iter().any(|x| path.to_ascii_lowercase().ends_with(x));
        let deflate = match v.compression {
            Compression::Deflate => true,
            Compression::Stored => false,
            Compression::Mixed => path.ends_with(".xml") || (!image && i % 2 == 0),
        };
        out.push(named(path, body, deflate));
    }
    out
}

/// `files` packed as a `.fla` in variant `v`.
pub fn pack(files: &BTreeMap<String, Vec<u8>>, v: &Variant) -> Vec<u8> {
    write(&entries(files, v), v.zip64)
}

/// A package folder's files by their path inside it, leaving out its `golden/` renders.
pub fn package(dir: &std::path::Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &std::path::Path, prefix: &str, out: &mut BTreeMap<String, Vec<u8>>) {
        let mut list: Vec<_> = std::fs::read_dir(dir).map(|r| r.flatten().collect()).unwrap_or_default();
        list.sort_by_key(|e| e.file_name());
        for e in list {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if e.path().is_dir() {
                if path != "golden" {
                    walk(&e.path(), &path, out);
                }
            } else if let Ok(b) = std::fs::read(e.path()) {
                out.insert(path, b);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, "", &mut out);
    out
}
