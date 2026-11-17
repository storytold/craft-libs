#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! ELF program headers and dynamic section entries from untrusted bytes, 32- or 64-bit in either
//! byte order: [`program_headers`], [`dynamic_entries`], [`va_to_off`], and the segment type,
//! permission and dynamic tag names.
//!
//! From Craft Anything (`crates/formats/src/elf.rs`), where the mitigations, dynamic symbols,
//! symbol versions and core-file notes built on these stay. With the `serde` feature,
//! [`ProgramHeader`] is `Serialize` with `vaddr` as a `0x…` string.

mod bytes;
#[cfg(feature = "serde")]
mod hex;

use crate::bytes::Reader;

/// Program headers read from one table.
pub const MAX_PHDRS: usize = 4096;

/// Dynamic entries read from one segment.
pub const MAX_DYN: usize = 65_536;

/// Why a read gave up.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ElfError {
    /// The ELF header ends before the file does.
    #[error("the file ends early ({0})")]
    Truncated(&'static str),
    /// No ELF magic, or an invalid class, encoding or field.
    #[error("the file is malformed ({0})")]
    Malformed(String),
}

/// Endian/class-aware reads over an ELF image.
#[derive(Clone, Copy)]
struct E<'a> {
    r: Reader<'a>,
    is64: bool,
    le: bool,
}

impl<'a> E<'a> {
    fn new(data: &'a [u8]) -> Result<Self, ElfError> {
        if !data.starts_with(b"\x7fELF") {
            return Err(ElfError::Malformed("no ELF magic".into()));
        }
        let r = Reader::new(data);
        let class = r.u8(4).ok_or(ElfError::Truncated("ELF ident"))?;
        let enc = r.u8(5).ok_or(ElfError::Truncated("ELF ident"))?;
        if !matches!(class, 1 | 2) || !matches!(enc, 1 | 2) {
            return Err(ElfError::Malformed("bad ELF class or data encoding".into()));
        }
        Ok(E { r, is64: class == 2, le: enc == 1 })
    }

    fn u16(&self, off: usize) -> Option<u16> {
        if self.le { self.r.u16_le(off) } else { self.r.u16_be(off) }
    }

    fn u32(&self, off: usize) -> Option<u32> {
        if self.le { self.r.u32_le(off) } else { self.r.u32_be(off) }
    }

    fn u64(&self, off: usize) -> Option<u64> {
        if self.le { self.r.u64_le(off) } else { self.r.u64_be(off) }
    }

    fn word(&self, off: usize) -> Option<u64> {
        if self.is64 { self.u64(off) } else { self.u32(off).map(u64::from) }
    }

    fn word_size(&self) -> usize {
        if self.is64 { 8 } else { 4 }
    }
}

/// One program header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProgramHeader {
    /// [`segment_type_name`] of `p_type`.
    pub kind: String,
    /// [`permissions`] of `p_flags`.
    pub flags: String,
    pub offset: u64,
    #[cfg_attr(feature = "serde", serde(serialize_with = "hex::serialize"))]
    pub vaddr: u64,
    pub file_size: u64,
    pub mem_size: u64,
    pub align: u64,
    /// The raw segment type.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub p_type: u32,
    /// The raw `PF_R`/`PF_W`/`PF_X` flags.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub p_flags: u32,
}

/// A segment type's `PT_` name without the prefix (`LOAD`, `GNU_RELRO`…), or its number in hex.
pub fn segment_type_name(t: u32) -> String {
    match t {
        0 => "NULL".into(),
        1 => "LOAD".into(),
        2 => "DYNAMIC".into(),
        3 => "INTERP".into(),
        4 => "NOTE".into(),
        5 => "SHLIB".into(),
        6 => "PHDR".into(),
        7 => "TLS".into(),
        0x6474_e550 => "GNU_EH_FRAME".into(),
        0x6474_e551 => "GNU_STACK".into(),
        0x6474_e552 => "GNU_RELRO".into(),
        0x6474_e553 => "GNU_PROPERTY".into(),
        0x7000_0001 => "ARM_EXIDX".into(),
        other => format!("{other:#x}"),
    }
}

/// `p_flags` as `rwx`, with `-` for each permission missing.
pub fn permissions(f: u32) -> String {
    format!("{}{}{}", if f & 4 != 0 { 'r' } else { '-' }, if f & 2 != 0 { 'w' } else { '-' }, if f & 1 != 0 { 'x' } else { '-' })
}

/// The program headers of an ELF file, at most [`MAX_PHDRS`]. A table that runs past the file
/// ends at the last complete entry.
pub fn program_headers(data: &[u8]) -> Result<Vec<ProgramHeader>, ElfError> {
    let e = E::new(data)?;
    let (phoff, phentsize, phnum) = if e.is64 { (e.u64(32), e.u16(54), e.u16(56)) } else { (e.u32(28).map(u64::from), e.u16(42), e.u16(44)) };
    let phoff = usize::try_from(phoff.ok_or(ElfError::Truncated("ELF header"))?).map_err(|_| ElfError::Malformed("phoff".into()))?;
    let phentsize = usize::from(phentsize.unwrap_or(0));
    let phnum = usize::from(phnum.unwrap_or(0)).min(MAX_PHDRS);
    let min = if e.is64 { 56 } else { 32 };
    if phnum > 0 && phentsize < min {
        return Err(ElfError::Malformed("program header entry too small".into()));
    }
    let mut out = Vec::new();
    for i in 0..phnum {
        // The fields below sit within `phentsize` bytes of `o`, so their offsets can't overflow either.
        let Some(o) = i.checked_mul(phentsize).and_then(|x| x.checked_add(phoff)).filter(|&o| o.checked_add(phentsize).is_some()) else { break };
        let ph = if e.is64 {
            (e.u32(o), e.u32(o + 4), e.u64(o + 8), e.u64(o + 16), e.u64(o + 32), e.u64(o + 40), e.u64(o + 48))
        } else {
            (
                e.u32(o),
                e.u32(o + 24),
                e.u32(o + 4).map(u64::from),
                e.u32(o + 8).map(u64::from),
                e.u32(o + 16).map(u64::from),
                e.u32(o + 20).map(u64::from),
                e.u32(o + 28).map(u64::from),
            )
        };
        let (Some(t), Some(f), Some(off), Some(va), Some(fs), Some(ms), Some(al)) = ph else { break };
        out.push(ProgramHeader {
            kind: segment_type_name(t),
            flags: permissions(f),
            offset: off,
            vaddr: va,
            file_size: fs,
            mem_size: ms,
            align: al,
            p_type: t,
            p_flags: f,
        });
    }
    Ok(out)
}

/// A dynamic tag's `DT_` name without the prefix (`NEEDED`, `GNU_HASH`…), or its number in hex.
pub fn dynamic_tag_name(tag: u64) -> String {
    match tag {
        0 => "NULL",
        1 => "NEEDED",
        2 => "PLTRELSZ",
        3 => "PLTGOT",
        4 => "HASH",
        5 => "STRTAB",
        6 => "SYMTAB",
        7 => "RELA",
        8 => "RELASZ",
        9 => "RELAENT",
        10 => "STRSZ",
        11 => "SYMENT",
        12 => "INIT",
        13 => "FINI",
        14 => "SONAME",
        15 => "RPATH",
        16 => "SYMBOLIC",
        17 => "REL",
        18 => "RELSZ",
        19 => "RELENT",
        20 => "PLTREL",
        21 => "DEBUG",
        22 => "TEXTREL",
        23 => "JMPREL",
        24 => "BIND_NOW",
        25 => "INIT_ARRAY",
        26 => "FINI_ARRAY",
        27 => "INIT_ARRAYSZ",
        28 => "FINI_ARRAYSZ",
        29 => "RUNPATH",
        30 => "FLAGS",
        0x6fff_fef5 => "GNU_HASH",
        0x6fff_fffb => "FLAGS_1",
        0x6fff_fff0 => "VERSYM",
        0x6fff_fffe => "VERNEED",
        0x6fff_ffff => "VERNEEDNUM",
        _ => return format!("{tag:#x}"),
    }
    .into()
}

/// Map a virtual address to a file offset using `PT_LOAD` headers. Addresses in a segment's
/// zero-filled tail, or whose offset would overflow, map to nothing.
pub fn va_to_off(phdrs: &[ProgramHeader], va: u64) -> Option<u64> {
    phdrs.iter().filter(|p| p.p_type == 1).find(|p| va >= p.vaddr && va - p.vaddr < p.file_size).and_then(|p| p.offset.checked_add(va - p.vaddr))
}

/// The (tag, value) pairs of the `PT_DYNAMIC` segment in `phdrs`, up to its `DT_NULL`, at most
/// [`MAX_DYN`]. Empty without a dynamic segment.
pub fn dynamic_entries(data: &[u8], phdrs: &[ProgramHeader]) -> Result<Vec<(u64, u64)>, ElfError> {
    Ok(entries(&E::new(data)?, phdrs))
}

fn entries(e: &E<'_>, phdrs: &[ProgramHeader]) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    let Some(d) = phdrs.iter().find(|p| p.p_type == 2) else { return out };
    let ent = 2 * e.word_size();
    let n = usize::try_from(d.file_size).unwrap_or(0) / ent;
    let start = usize::try_from(d.offset).unwrap_or(usize::MAX);
    for i in 0..n.min(MAX_DYN) {
        let Some(o) = i.checked_mul(ent).and_then(|x| start.checked_add(x)) else { break };
        let (Some(tag), Some(val)) = (e.word(o), o.checked_add(e.word_size()).and_then(|v| e.word(v))) else { break };
        if tag == 0 {
            break;
        }
        out.push((tag, val));
    }
    out
}

#[cfg(test)]
mod tests;
