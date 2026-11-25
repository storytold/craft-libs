//! The 12-byte header of the reference app's object serialisation (`document.bin`, `default_material.bin`,
//! `layersstackstate.bin`, `layersstackstate2.bin`): magic, a `u32` layout word, and a `u32` format version.
//! The layout word selects how the rest is written: 0 for [`Layout::Tagged`], 1 for [`Layout::Compact`].

use super::RECORD;
use crate::container::bytes::ByteReader;
use crate::decode::DecodeError;

const MAGIC: [u8; 4] = [0xdd, 0x2f, 0x7c, 0x1b];
/// Offset of the root value, right after the header.
pub const ROOT_OFFSET: usize = 12;

/// How a file lays out its objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Field names and one-byte tags stored with every object. Seen up to format 78 (2023).
    Tagged,
    /// Each class's field table stored once. Seen at format 92 (version 12.2); see `compact.rs`.
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentHeader {
    pub layout: Layout,
    /// Grows with the reference app releases: 51 (2019) … 78 (version 9, 2023) … 92 (version 12.2).
    /// `layersstackstate.bin` and `layersstackstate2.bin` use 2.
    pub format_version: u32,
}

impl DocumentHeader {
    pub fn parse(data: &[u8]) -> Result<Self, DecodeError> {
        let mut r = ByteReader::new(data);
        let magic = r.bytes(MAGIC.len()).map_err(DecodeError::truncated(RECORD))?;
        if magic != MAGIC {
            return Err(DecodeError::BadMagic { record: RECORD });
        }
        let layout = match r.u32_le().map_err(DecodeError::truncated(RECORD))? {
            0 => Layout::Tagged,
            1 => Layout::Compact,
            word => return Err(DecodeError::invalid(RECORD, format!("unknown layout word {word}"))),
        };
        let format_version = r.u32_le().map_err(DecodeError::truncated(RECORD))?;
        Ok(DocumentHeader { layout, format_version })
    }
}
