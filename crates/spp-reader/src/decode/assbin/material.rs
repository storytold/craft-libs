//! One material block. Only `?mat.name` (the mesh's material name) and the reference app's `$mat.uid` (the texture set's
//! `sceneMaterialUID` in `document.bin`, also the texture-cache set id) are read.
//!
//! Layout (after the block size): `u32` property count, then one block per property:
//! key string, `u32` semantic, `u32` index, `u32` type, `u32` data length, data.
//! Note: the reference app writes type before length; upstream Assimp writes length before type.

use super::RECORD;
use super::block::{read_block, read_count, read_string, read_u32};
use crate::container::bytes::ByteReader;
use crate::decode::DecodeError;

const NAME_KEY: &str = "?mat.name";
const UID_KEY: &str = "$mat.uid";
/// Assimp's `aiPTI_String`: data is a `u32` length, the bytes and a NUL.
const TYPE_STRING: u32 = 3;
/// Assimp's `aiPTI_Buffer`: raw bytes.
const TYPE_BUFFER: u32 = 5;
/// Smallest possible property block: size field, empty key and four `u32`s.
const MIN_PROPERTY_LEN: usize = 8 + 4 + 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialSummary {
    /// Assimp `?mat.name`; in the reference app this is the texture set name. Empty if the material has none.
    pub name: String,
    /// The reference app's `$mat.uid` (little-endian u64), if present.
    pub uid: Option<u64>,
}

impl MaterialSummary {
    /// Parses the content of one material block.
    pub fn parse(block: &[u8]) -> Result<Self, DecodeError> {
        let mut r = ByteReader::new(block);
        let count = read_count(&mut r, MIN_PROPERTY_LEN)?;
        let mut summary = MaterialSummary { name: String::new(), uid: None };
        for _ in 0..count {
            let property = read_block(&mut r)?;
            summary.read_property(property)?;
        }
        Ok(summary)
    }

    /// Takes the name or uid from a property block; other properties are ignored.
    fn read_property(&mut self, block: &[u8]) -> Result<(), DecodeError> {
        let mut r = ByteReader::new(block);
        let key = read_string(&mut r)?;
        if key != NAME_KEY && key != UID_KEY {
            return Ok(());
        }
        let _semantic = read_u32(&mut r)?;
        let _index = read_u32(&mut r)?;
        let kind = read_u32(&mut r)?;
        let len = read_u32(&mut r)? as usize;
        let data = r.bytes(len).map_err(DecodeError::truncated(RECORD))?;
        match (key.as_str(), kind) {
            (NAME_KEY, TYPE_STRING) => self.name = read_string(&mut ByteReader::new(data))?,
            (UID_KEY, TYPE_BUFFER) if data.len() == 8 => self.uid = data.try_into().ok().map(u64::from_le_bytes),
            _ => {
                return Err(DecodeError::invalid(RECORD, format!("{key} has type {kind} and {len} bytes")));
            }
        }
        Ok(())
    }
}
