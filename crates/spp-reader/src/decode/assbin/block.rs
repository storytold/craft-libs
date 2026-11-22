//! Primitives shared by all parts of the reference app's assbin scene.
//!
//! The reference app's dumps differ from upstream Assimp: instead of `u32 magic + u32 size` chunks, every
//! scene, node, mesh, material and material property is a block that starts with a `u64` size
//! which counts the size field itself. Strings are `u32` length + bytes, without a terminator.

use super::RECORD;
use crate::container::bytes::ByteReader;
use crate::decode::DecodeError;

const SIZE_FIELD: usize = 8;

/// Reads one size-prefixed block and returns its content (after the size field).
pub fn read_block<'a>(r: &mut ByteReader<'a>) -> Result<&'a [u8], DecodeError> {
    let size = r.u64_le().map_err(DecodeError::truncated(RECORD))?;
    let size =
        usize::try_from(size).ok().filter(|&s| s >= SIZE_FIELD).ok_or_else(|| DecodeError::invalid(RECORD, format!("bad block size {size}")))?;
    r.bytes(size - SIZE_FIELD).map_err(DecodeError::truncated(RECORD))
}

/// Reads an Assimp string (`u32` length + bytes); invalid UTF-8 is replaced, not rejected.
pub fn read_string(r: &mut ByteReader) -> Result<String, DecodeError> {
    let len = read_u32(r)? as usize;
    let bytes = r.bytes(len).map_err(DecodeError::truncated(RECORD))?;
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

pub fn read_u32(r: &mut ByteReader) -> Result<u32, DecodeError> {
    r.u32_le().map_err(DecodeError::truncated(RECORD))
}

/// Reads a count of items that each take at least `min_item_len` bytes, rejecting counts the
/// remaining data cannot hold, so corrupt counts fail early instead of driving long loops.
pub fn read_count(r: &mut ByteReader, min_item_len: usize) -> Result<usize, DecodeError> {
    let at = r.pos();
    let count = read_u32(r)? as usize;
    match count.checked_mul(min_item_len.max(1)) {
        Some(needed) if needed <= r.remaining() => Ok(count),
        _ => Err(DecodeError::invalid(RECORD, format!("count {count} at offset {at} exceeds the remaining data"))),
    }
}
