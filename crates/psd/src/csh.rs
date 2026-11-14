//! Photoshop custom shape sets (`.csh`, version 2).
//!
//! Container layout: Michel Mariani's public format research,
//! <https://tonton-pixel.codeberg.page/photoshop-file-formats/custom-shapes-file-format.html>.
//! Geometry uses Adobe's path record format. No external implementation or shape assets copied.

use crate::error::{PsdError, Result};
use crate::io::Reader;
use crate::path::{PathData, RECORD_LEN};

/// Maximum encoded file size accepted by this reader.
pub const MAX_FILE_BYTES: usize = 64 << 20;
/// Maximum presets in a set.
pub const MAX_SHAPES: usize = 10_000;
/// Maximum path records across all presets, not per shape.
pub const MAX_RECORDS: usize = 1_000_000;
/// Maximum UTF-16 units in a preset name, including its optional trailing null.
pub const MAX_NAME_UNITS: usize = 32_768;

/// A named shape with its reference bounds and normalized path records.
#[derive(Debug, Clone, PartialEq)]
pub struct CshShape {
    /// Display name.
    pub name: String,
    /// Source shape identifier (usually a UUID).
    pub id: String,
    /// Reference rectangle: top, left, bottom, right. Coordinates may be negative.
    pub bounds: [i32; 4],
    /// Coordinates are fractions of the reference rectangle's width and height.
    pub path: PathData,
}

/// Read a complete set before callers modify their preset libraries.
/// Unsupported versions, truncated records and excessive aggregate sizes return errors.
pub fn parse(bytes: &[u8]) -> Result<Vec<CshShape>> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(PsdError::LimitExceeded("CSH file bytes"));
    }
    let mut r = Reader::new(bytes);
    let signature = r.array()?;
    if &signature != b"cush" {
        return Err(PsdError::InvalidSignature { expected: "cush", found: signature });
    }
    if r.u32()? != 2 {
        return Err(PsdError::Unsupported("CSH file version (expected 2)".into()));
    }
    let count = r.u32()? as usize;
    if count > MAX_SHAPES {
        return Err(PsdError::LimitExceeded("CSH shape count"));
    }
    r.check_count(count as u64, 32)?;
    let mut shapes = Vec::new();
    let mut remaining_records = MAX_RECORDS;
    for _ in 0..count {
        let units = r.u32()? as usize;
        if units > MAX_NAME_UNITS {
            return Err(PsdError::LimitExceeded("CSH name length"));
        }
        let raw = r.bytes(units * 2)?;
        let mut name: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        if name.last() == Some(&0) {
            name.pop();
        }
        let name = String::from_utf16(&name).map_err(|_| PsdError::invalid("CSH name is not valid UTF-16"))?;
        let padding = (4 - r.pos() % 4) % 4;
        zero_padding(r.bytes(padding)?)?;
        if r.u32()? != 1 {
            return Err(PsdError::Unsupported("CSH shape version (expected 1)".into()));
        }
        let length = r.u32()?;
        if !length.is_multiple_of(4) {
            return Err(PsdError::invalid("CSH shape length must be aligned to four bytes"));
        }
        let mut shape = r.sub(u64::from(length))?;
        let id_len = usize::from(shape.u8()?);
        let id = std::str::from_utf8(shape.bytes(id_len)?).map_err(|_| PsdError::invalid("CSH identifier is not UTF-8"))?.to_owned();
        let bounds = [shape.i32()?, shape.i32()?, shape.i32()?, shape.i32()?];
        if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
            return Err(PsdError::invalid("CSH reference bounds are empty or reversed"));
        }
        let records = shape.remaining() / RECORD_LEN;
        remaining_records = remaining_records.checked_sub(records).ok_or(PsdError::LimitExceeded("CSH aggregate path records"))?;
        let path = PathData::from_bytes(shape.bytes(records * RECORD_LEN)?)?;
        if shape.remaining() > 3 {
            return Err(PsdError::invalid("CSH incomplete path record"));
        }
        zero_padding(shape.peek_rest())?;
        validate_path(&path)?;
        shapes.push(CshShape { name, id, bounds, path });
    }
    if !r.is_empty() {
        return Err(PsdError::invalid("CSH trailing data"));
    }
    Ok(shapes)
}

/// Refuse nonzero alignment bytes rather than silently discarding extra payload data as padding.
fn zero_padding(bytes: &[u8]) -> Result<()> {
    if bytes.iter().any(|&b| b != 0) {
        return Err(PsdError::invalid("CSH nonzero padding"));
    }
    Ok(())
}

/// Enforce subpath structure more strictly than the generic PSD path reader.
/// Each length record must be followed by exactly its declared number of knots, with matching
/// open/closed selectors. Fill and clipboard records are accepted only between subpaths.
/// Rejecting inconsistent records prevents a damaged preset from silently changing geometry.
fn validate_path(path: &PathData) -> Result<()> {
    let mut pending = 0u16;
    let mut closed = false;
    for record in &path.records {
        match record.selector {
            0 | 3 if pending == 0 => {
                pending = u16::from_be_bytes([record.data[0], record.data[1]]);
                closed = record.selector == 0;
            }
            1 | 2 | 4 | 5 if pending > 0 && closed == (record.selector <= 2) => pending -= 1,
            6..=8 if pending == 0 => {}
            _ => return Err(PsdError::invalid("CSH path selectors or knot counts are inconsistent")),
        }
    }
    if pending != 0 {
        return Err(PsdError::invalid("CSH subpath is missing knots"));
    }
    Ok(())
}
