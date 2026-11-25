//! Structural walk over the reference app's object tree, reporting each object's class to a [`Visitor`].
//! [`walk`] picks the layout from the header. This file holds [`Layout::Tagged`]; the other is in `compact.rs`.
//!
//! An object is `flag u8` (`0xff` = absent, `0x00` = present), `u32` end offset, class name
//! (`u32` length + ASCII), `u16` field count, then fields: name (`u32` length + bytes),
//! tag byte, payload. End offsets are absolute positions in the file. Because objects and lists
//! carry their end, a value with an unknown tag only costs the rest of its enclosing container.

use super::RECORD;
use super::compact;
use super::header::{DocumentHeader, Layout, ROOT_OFFSET};
use super::tag::{Payload, ValueTag};
use crate::container::bytes::ByteReader;
use crate::decode::DecodeError;

/// Nesting limit (objects and lists); real files reach about 50.
const MAX_DEPTH: usize = 256;
const MAX_CLASS_NAME: usize = 256;
const PRESENT: u8 = 0x00;
const ABSENT: u8 = 0xff;
/// Smallest field: empty name length and a tag with a 1-byte payload.
const MIN_FIELD_LEN: usize = 6;

/// Receives what the walk finds. Only [`Visitor::object`] and [`Visitor::unknown_tag`] are required;
/// the other callbacks let a visitor follow field names and read values.
pub trait Visitor {
    fn object(&mut self, class: &str);
    /// The object opened by the last unmatched [`Visitor::object`] call has ended.
    fn end_object(&mut self) {}
    /// A field name; the next value (scalar, object or list) belongs to it.
    fn field(&mut self, _name: &[u8]) {}
    /// A fixed-size or string value with its payload bytes (strings without their length prefix).
    fn scalar(&mut self, _tag: ValueTag, _payload: &[u8]) {}
    /// A list of `count` values starts; [`Visitor::end_list`] follows its last element.
    fn list(&mut self, _count: usize) {}
    fn end_list(&mut self) {}
    /// A tag we cannot size; the rest of the enclosing object or list was skipped.
    fn unknown_tag(&mut self, tag: u32, offset: usize);
}

/// Whether a value was read completely or its container has to be skipped.
pub(super) enum Flow {
    Done,
    Unknown,
}

/// Walks the whole tree of a file.
pub fn walk(data: &[u8], visitor: &mut impl Visitor) -> Result<(), DecodeError> {
    match DocumentHeader::parse(data)?.layout {
        Layout::Tagged => walk_tagged(data, visitor),
        Layout::Compact => compact::walk(data, visitor),
    }
}

fn walk_tagged(data: &[u8], visitor: &mut impl Visitor) -> Result<(), DecodeError> {
    let mut walker = Walker { visitor };
    let mut r = ByteReader::new(data);
    r.seek(ROOT_OFFSET).map_err(DecodeError::truncated(RECORD))?;
    match walker.tagged(&mut r, 0)? {
        Flow::Done => Ok(()),
        Flow::Unknown => Err(DecodeError::invalid(RECORD, "root value has an unknown tag")),
    }
}

struct Walker<'v, V> {
    visitor: &'v mut V,
}

impl<V: Visitor> Walker<'_, V> {
    fn tagged(&mut self, r: &mut ByteReader, depth: usize) -> Result<Flow, DecodeError> {
        let at = r.pos();
        let byte = read_u8(r)?;
        let Some(tag) = ValueTag::from_byte(byte) else {
            self.visitor.unknown_tag(u32::from(byte), at);
            return Ok(Flow::Unknown);
        };
        match tag.payload() {
            Payload::Fixed(n) => {
                let payload = r.bytes(n).map_err(DecodeError::truncated(RECORD))?;
                self.visitor.scalar(tag, payload);
            }
            Payload::String => {
                let len = read_u32(r)? as usize;
                let payload = r.bytes(len).map_err(DecodeError::truncated(RECORD))?;
                self.visitor.scalar(tag, payload);
            }
            Payload::List => self.list(r, enter(depth)?)?,
            Payload::Object => self.object(r, enter(depth)?)?,
        }
        Ok(Flow::Done)
    }

    fn object(&mut self, r: &mut ByteReader, depth: usize) -> Result<(), DecodeError> {
        match read_u8(r)? {
            ABSENT => return Ok(()),
            PRESENT => {}
            flag => {
                return Err(DecodeError::invalid(RECORD, format!("object flag {flag:#04x} at {}", r.pos() - 1)));
            }
        }
        let end = read_end(r)?;
        self.visitor.object(&read_class(r)?);
        let fields = read_u16(r)? as usize;
        check_count(r, fields, end, MIN_FIELD_LEN)?;
        for _ in 0..fields {
            let name_len = read_u32(r)? as usize;
            let name = r.bytes(name_len).map_err(DecodeError::truncated(RECORD))?;
            self.visitor.field(name);
            if let Flow::Unknown = self.tagged(r, depth)? {
                break;
            }
        }
        finish(r, end)?;
        self.visitor.end_object();
        Ok(())
    }

    fn list(&mut self, r: &mut ByteReader, depth: usize) -> Result<(), DecodeError> {
        let end = read_end(r)?;
        let count = read_u32(r)? as usize;
        check_count(r, count, end, 1)?;
        self.visitor.list(count);
        for _ in 0..count {
            if let Flow::Unknown = self.tagged(r, depth)? {
                break;
            }
        }
        finish(r, end)?;
        self.visitor.end_list();
        Ok(())
    }
}

pub(super) fn enter(depth: usize) -> Result<usize, DecodeError> {
    if depth >= MAX_DEPTH {
        return Err(DecodeError::invalid(RECORD, "objects nested too deeply"));
    }
    Ok(depth + 1)
}

/// Reads an end offset; it must lie between the current position and the end of the data.
pub(super) fn read_end(r: &mut ByteReader) -> Result<usize, DecodeError> {
    let end = read_u32(r)? as usize;
    if end < r.pos() || end > r.len() {
        return Err(DecodeError::invalid(RECORD, format!("end offset {end} at {} is out of range", r.pos() - 4)));
    }
    Ok(end)
}

/// Moves to a container's end; having read past it means the data is inconsistent.
pub(super) fn finish(r: &mut ByteReader, end: usize) -> Result<(), DecodeError> {
    if r.pos() > end {
        return Err(DecodeError::invalid(RECORD, format!("content overruns its end offset {end}")));
    }
    r.seek(end).map_err(DecodeError::truncated(RECORD))
}

fn check_count(r: &ByteReader, count: usize, end: usize, min_item_len: usize) -> Result<(), DecodeError> {
    if count.saturating_mul(min_item_len) > end.saturating_sub(r.pos()) {
        return Err(DecodeError::invalid(RECORD, format!("count {count} does not fit before offset {end}")));
    }
    Ok(())
}

/// Class names are ASCII identifiers such as `DataDocument` or `Data2DTransformation`.
pub(super) fn read_class(r: &mut ByteReader) -> Result<String, DecodeError> {
    let at = r.pos();
    let len = read_u32(r)? as usize;
    if len == 0 || len > MAX_CLASS_NAME {
        return Err(DecodeError::invalid(RECORD, format!("class name length {len} at {at}")));
    }
    let bytes = r.bytes(len).map_err(DecodeError::truncated(RECORD))?;
    if !bytes.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_') {
        return Err(DecodeError::invalid(RECORD, format!("class name at {at} is not an identifier")));
    }
    Ok(bytes.iter().map(|&b| b as char).collect())
}

fn read_u8(r: &mut ByteReader) -> Result<u8, DecodeError> {
    r.u8().map_err(DecodeError::truncated(RECORD))
}

fn read_u16(r: &mut ByteReader) -> Result<u16, DecodeError> {
    r.u16_le().map_err(DecodeError::truncated(RECORD))
}

pub(super) fn read_u32(r: &mut ByteReader) -> Result<u32, DecodeError> {
    r.u32_le().map_err(DecodeError::truncated(RECORD))
}
