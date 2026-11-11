//! The HEADER at offset 0 ([MS-PST] 2.2.2.6) and the ROOT in it (2.2.2.5): which format, how
//! data is encoded, and where the two B-trees start. Both CRCs are checked.

use crate::bytes::{u8_at, u16_at, u32_at, u64_at};
use crate::crc::crc32;
use crate::crypt::Encryption;
use crate::error::{Error, Result};

/// ANSI (Outlook 97–2002: 32-bit block ids and offsets) or Unicode (64-bit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Ansi,
    Unicode,
}

impl Format {
    /// Bytes in a block id or a file offset.
    pub(crate) fn id_size(self) -> usize {
        match self {
            Format::Ansi => 4,
            Format::Unicode => 8,
        }
    }

    pub(crate) fn read_id(self, b: &[u8], at: usize) -> Result<u64> {
        match self {
            Format::Ansi => u32_at(b, at).map(u64::from),
            Format::Unicode => u64_at(b, at),
        }
    }

    /// A block reference (`BREF`): id, then offset.
    pub(crate) fn read_bref(self, b: &[u8], at: usize) -> Result<Bref> {
        let bid = self.read_id(b, at)?;
        let ib = self.read_id(b, at.checked_add(self.id_size()).ok_or(Error::OutOfBounds)?)?;
        Ok(Bref { bid, ib })
    }

    /// Bytes of a page trailer, a block trailer (the same sizes).
    pub(crate) fn trailer_size(self) -> usize {
        match self {
            Format::Ansi => 12,
            Format::Unicode => 16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Bref {
    pub bid: u64,
    pub ib: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Header {
    pub format: Format,
    pub version: u16,
    pub encryption: Encryption,
    pub file_eof: u64,
    pub nbt: Bref,
    pub bbt: Bref,
}

pub(crate) const HEADER_READ: usize = 564;

impl Header {
    pub(crate) fn parse(b: &[u8]) -> Result<Header> {
        if b.get(0..4) != Some(b"!BDN") || b.get(8..10) != Some(b"SM") {
            return Err(Error::NotPst);
        }
        let version = u16_at(b, 10)?;
        let format = match version {
            14 | 15 => Format::Ansi,
            // 36 is an offline folders file with 4 KB pages, 37 one protected by Windows
            // Information Protection: neither is read.
            23..=35 => Format::Unicode,
            v => return Err(Error::UnsupportedVersion(v)),
        };
        // dwCRCPartial: the 471 bytes from wMagicClient.
        let partial = u32_at(b, 4)?;
        if crc32(b.get(8..479).ok_or(Error::NotPst)?) != partial {
            return Err(Error::BadChecksum("the file's header"));
        }
        // ROOT (at 180, or 164 in ANSI): dwReserved, ibFileEof, ibAMapLast, cbAMapFree,
        // cbPMapFree, BREFNBT, BREFBBT; then the sentinel and bCryptMethod.
        let (eof_at, nbt_at, bbt_at, sentinel, crypt) = match format {
            Format::Unicode => (184, 216, 232, 512, 513),
            Format::Ansi => (168, 184, 192, 460, 461),
        };
        if format == Format::Unicode {
            // dwCRCFull: the 516 bytes from wMagicClient to bidNextB.
            let full = u32_at(b, 524)?;
            if crc32(b.get(8..524).ok_or(Error::NotPst)?) != full {
                return Err(Error::BadChecksum("the file's header"));
            }
        }
        if u8_at(b, sentinel)? != 0x80 {
            return Err(Error::Corrupt("the header's sentinel"));
        }
        let encryption = match u8_at(b, crypt)? {
            0 => Encryption::None,
            1 => Encryption::Permute,
            2 => Encryption::Cyclic,
            other => return Err(Error::UnsupportedEncryption(other)),
        };
        let file_eof = format.read_id(b, eof_at)?;
        let nbt = format.read_bref(b, nbt_at)?;
        let bbt = format.read_bref(b, bbt_at)?;
        Ok(Header { format, version, encryption, file_eof, nbt, bbt })
    }
}
