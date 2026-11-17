#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! PE/COFF headers (`.exe`, `.dll`, `.sys`, `.efi`…) from untrusted bytes.
//!
//! [`headers`] reads the DOS stub pointer, the COFF header, the PE32 or PE32+ optional header,
//! its data directories and the section table; [`PeHeaders::rva_to_offset`] maps addresses
//! through it. [`rich_header`] decodes the linker's Rich header. Nothing here panics on hostile
//! input: required fields that are missing are errors, optional ones read as zero, the
//! directory count is bounded by the complete entries in the optional header, and at most
//! [`MAX_SECTIONS`] sections are read.
//!
//! From Craft Anything (`crates/formats/src/pe.rs`), where the import, export, resource and
//! program-model code built on these headers stays. With the `serde` feature, [`PeSection`] is
//! `Serialize`.

mod bytes;

use crate::bytes::Reader;

/// Sections read from the section table; the rest are ignored (the loader allows 96).
pub const MAX_SECTIONS: usize = 96;

/// The names of the sixteen optional-header data directories, by index.
pub const DATA_DIRECTORY_NAMES: [&str; 16] = [
    "export",
    "import",
    "resource",
    "exception",
    "security",
    "basereloc",
    "debug",
    "architecture",
    "globalptr",
    "tls",
    "load_config",
    "bound_import",
    "iat",
    "delay_import",
    "clr_runtime",
    "reserved",
];

/// Why [`headers`] gave up.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PeError {
    /// A required header ends before the file does.
    #[error("the file ends early ({0})")]
    Truncated(&'static str),
    /// A signature or magic number is wrong.
    #[error("the file is malformed ({0})")]
    Malformed(String),
}

/// One section table entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PeSection {
    /// The 8-byte name up to its first NUL, decoded lossily.
    pub name: String,
    pub virtual_size: u32,
    pub rva: u32,
    pub raw_size: u32,
    pub raw_offset: u32,
    pub characteristics: u32,
}

/// The COFF and optional header fields and the section table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeHeaders {
    /// File offset of the `PE\0\0` signature (`e_lfanew`).
    pub pe_offset: usize,
    pub machine: u16,
    pub timestamp: u32,
    pub characteristics: u16,
    pub pe32_plus: bool,
    pub image_base: u64,
    pub entry_rva: u32,
    pub size_of_headers: u32,
    pub subsystem: u16,
    pub dll_characteristics: u16,
    /// Major and minor linker version.
    pub linker: (u8, u8),
    /// `(rva, size)` for each data directory present, at most 16.
    pub data_dirs: Vec<(u32, u32)>,
    pub sections: Vec<PeSection>,
}

impl PeHeaders {
    /// RVA → file offset through the section table (headers map 1:1).
    pub fn rva_to_offset(&self, rva: u32) -> Option<usize> {
        for s in &self.sections {
            let size = s.virtual_size.max(s.raw_size);
            if rva >= s.rva && rva - s.rva < size {
                let delta = rva - s.rva;
                if delta >= s.raw_size {
                    return None;
                }
                return usize::try_from(s.raw_offset.checked_add(delta)?).ok();
            }
        }
        if rva < self.size_of_headers { usize::try_from(rva).ok() } else { None }
    }

    /// The virtual address of `rva` in the loaded image, or `None` when it overflows (or, for
    /// PE32, leaves the 32-bit address space).
    pub fn image_va(&self, rva: u64) -> Option<u64> {
        let rva = u32::try_from(rva).ok()?;
        let address = self.image_base.checked_add(u64::from(rva))?;
        (self.pe32_plus || address <= u64::from(u32::MAX)).then_some(address)
    }

    /// Bytes remaining in the selected RVA mapping before file offsets stop being contiguous.
    pub fn rva_file_len(&self, rva: u32) -> Option<usize> {
        let base = self.rva_to_offset(rva)?;
        let next_section_distance = |count: usize| {
            self.sections
                .iter()
                .take(count)
                .filter(|s| s.virtual_size.max(s.raw_size) > 0)
                .filter_map(|s| s.rva.checked_sub(rva))
                .filter(|&distance| distance > 0)
                .min()
                .unwrap_or(u32::MAX)
        };
        match self.sections.iter().enumerate().find(|(_, s)| rva.checked_sub(s.rva).is_some_and(|delta| delta < s.virtual_size.max(s.raw_size))) {
            Some((index, s)) => {
                rva.checked_sub(s.rva).and_then(|delta| s.raw_size.checked_sub(delta)).map(|len| len.min(next_section_distance(index)))
            }
            None => self.size_of_headers.checked_sub(rva).map(|len| len.min(next_section_distance(self.sections.len()))),
        }
        .map(u64::from)
        .and_then(|len| {
            let address_end = u64::from(u32::MAX) + 1;
            let remaining_rvas = address_end.checked_sub(u64::from(rva))?;
            let remaining_offsets = address_end.checked_sub(u64::try_from(base).ok()?)?;
            usize::try_from(len.min(remaining_rvas).min(remaining_offsets)).ok()
        })
    }

    /// Data directory `i` (see [`DATA_DIRECTORY_NAMES`]) when present and non-empty.
    pub fn dir(&self, i: usize) -> Option<(u32, u32)> {
        self.data_dirs.get(i).copied().filter(|(rva, size)| *rva != 0 && *size != 0)
    }
}

/// True when bytes start with MZ and point at a `PE\0\0` signature.
pub fn has_pe_header(data: &[u8]) -> bool {
    let r = Reader::new(data);
    r.u32_le(0x3c).and_then(|o| r.slice(o as usize, 4)) == Some(b"PE\0\0")
}

/// Parse the DOS stub pointer, COFF header, optional header and section table.
pub fn headers(data: &[u8]) -> Result<PeHeaders, PeError> {
    let r = Reader::new(data);
    if !data.starts_with(b"MZ") {
        return Err(PeError::Malformed("no MZ signature".into()));
    }
    let pe = r.u32_le(0x3c).ok_or(PeError::Truncated("DOS header"))? as usize;
    if r.slice(pe, 4) != Some(b"PE\0\0") {
        return Err(PeError::Malformed("no PE signature".into()));
    }
    let coff = pe + 4;
    let t = |o: Option<u16>| o.ok_or(PeError::Truncated("COFF header"));
    let machine = t(r.u16_le(coff))?;
    let nsections = usize::from(t(r.u16_le(coff + 2))?);
    let timestamp = r.u32_le(coff + 4).unwrap_or(0);
    let opt_size = usize::from(t(r.u16_le(coff + 16))?);
    let characteristics = t(r.u16_le(coff + 18))?;
    let opt = coff + 20;
    let optional = Reader::new(r.slice(opt, opt_size).ok_or(PeError::Truncated("optional header"))?);
    let magic = optional.u16_le(0).ok_or(PeError::Truncated("optional header"))?;
    let pe32_plus = match magic {
        0x20b => true,
        0x10b => false,
        m => return Err(PeError::Malformed(format!("optional header magic {m:#x}"))),
    };
    let (dd, count_off) = if pe32_plus { (112, 108) } else { (96, 92) };
    if opt_size < dd {
        return Err(PeError::Truncated("fixed optional header fields"));
    }
    let image_base = if pe32_plus { optional.u64_le(24) } else { optional.u32_le(28).map(u64::from) }.unwrap_or(0);
    let count = (optional.u32_le(count_off).unwrap_or(0) as usize).min(16).min((opt_size - dd) / 8);
    let data_dirs = (0..count).map(|i| (optional.u32_le(dd + i * 8).unwrap_or(0), optional.u32_le(dd + i * 8 + 4).unwrap_or(0))).collect();
    let sh = opt + opt_size;
    let mut sections = Vec::new();
    for i in 0..nsections.min(MAX_SECTIONS) {
        let o = sh + i * 40;
        let Some(name) = r.slice(o, 8) else { break };
        let name = String::from_utf8_lossy(name.split(|&c| c == 0).next().unwrap_or(&[])).into_owned();
        sections.push(PeSection {
            name,
            virtual_size: r.u32_le(o + 8).unwrap_or(0),
            rva: r.u32_le(o + 12).unwrap_or(0),
            raw_size: r.u32_le(o + 16).unwrap_or(0),
            raw_offset: r.u32_le(o + 20).unwrap_or(0),
            characteristics: r.u32_le(o + 36).unwrap_or(0),
        });
    }
    Ok(PeHeaders {
        pe_offset: pe,
        machine,
        timestamp,
        characteristics,
        pe32_plus,
        image_base,
        entry_rva: optional.u32_le(16).unwrap_or(0),
        size_of_headers: optional.u32_le(60).unwrap_or(0),
        subsystem: optional.u16_le(68).unwrap_or(0),
        dll_characteristics: optional.u16_le(70).unwrap_or(0),
        linker: (optional.u8(2).unwrap_or(0), optional.u8(3).unwrap_or(0)),
        data_dirs,
        sections,
    })
}

/// The optional header's subsystem as a short name (`windows-gui`, `efi-application`…), or
/// `unknown`.
pub fn subsystem_name(s: u16) -> &'static str {
    match s {
        1 => "native",
        2 => "windows-gui",
        3 => "windows-console",
        7 => "posix-console",
        9 => "windows-ce",
        10 => "efi-application",
        11 => "efi-boot-driver",
        12 => "efi-runtime-driver",
        14 => "xbox",
        16 => "windows-boot",
        _ => "unknown",
    }
}

/// Decoded Rich header entries between the DOS stub and `pe_offset`: (product id, build, count).
/// At most 256 entries.
pub fn rich_header(data: &[u8], pe_offset: usize) -> Option<Vec<(u16, u16, u32)>> {
    let r = Reader::new(data);
    let region = data.get(0x40..pe_offset.min(data.len()))?;
    let rich = region.windows(4).position(|w| w == b"Rich")? + 0x40;
    let key = r.u32_le(rich + 4)?;
    // Walk back to "DanS".
    let mut pos = rich;
    let mut start = None;
    while pos >= 0x44 {
        pos -= 4;
        if r.u32_le(pos)? ^ key == 0x536e_6144 {
            start = Some(pos);
            break;
        }
    }
    let start = start?;
    let mut out = Vec::new();
    let mut o = start + 16;
    while o + 8 <= rich && out.len() < 256 {
        let id = r.u32_le(o)? ^ key;
        let count = r.u32_le(o + 4)? ^ key;
        out.push(((id >> 16) as u16, (id & 0xffff) as u16, count));
        o += 8;
    }
    Some(out)
}

#[cfg(test)]
mod tests;
