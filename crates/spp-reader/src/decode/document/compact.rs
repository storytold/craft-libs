//! The compact layout of the reference app's object serialisation ([`super::Layout::Compact`], header word 1).
//!
//! Each class's field table is stored once, with its first object; later objects give the table's index and hold
//! values only. After the 12-byte header:
//!
//! - root: `u32` tag, `u32` number of field tables in the file, then the root value;
//! - object (pointer, tag 0x12, or inline value, tag 0x14): `u32` end offset, 0 meaning an absent object; then a
//!   class reference and the values of the class's fields, in table order;
//! - class reference: `i32` index of an earlier table in file order (0 = first), or -1 followed by a new table:
//!   class name (`u32` length + ASCII), `u32` field count, and per field its name (`u32` length + bytes) and `u32` tag;
//! - pointer list (0x13): `u32` count, then that many objects;
//! - value list (0x11): `u32` count; if not 0, one class reference and then each element's values, with no end offset;
//! - scalars: bare and unaligned. Sizes as in the tagged layout except enums, which take 4 bytes, and the
//!   128-bit channel mask (0x16); strings are `u32` length + bytes.
//!
//! An unknown tag can't be skipped on its own: since values carry no size, the walk resumes at the end of the
//! enclosing object, which is the first place with a stored position. The header word and the field table
//! layout are read off files saved by version 12.2 (format 92); other format numbers with this layout were not seen.

use super::RECORD;
use super::header::ROOT_OFFSET;
use super::tag::{Payload, ValueTag};
use super::walker::{Flow, Visitor, enter, finish, read_class, read_end, read_u32};
use crate::container::bytes::ByteReader;
use crate::decode::DecodeError;

/// A class reference asking for a new field table.
const NEW_TABLE: u32 = u32::MAX;
/// Smallest field table entry: empty name length and a tag.
const MIN_TABLE_ENTRY: usize = 8;
/// Smallest pointer list element: the end offset of an absent object.
const MIN_POINTER: usize = 4;
const MAX_FIELDLESS: usize = 4096;
const ENUM_FOR_VISITORS: usize = 8;

pub(super) fn walk(data: &[u8], visitor: &mut impl Visitor) -> Result<(), DecodeError> {
    let mut walker = Walker { visitor, tables: Vec::new() };
    let mut r = ByteReader::new(data);
    r.seek(ROOT_OFFSET).map_err(DecodeError::truncated(RECORD))?;
    let at = r.pos();
    let tag = read_u32(&mut r)?;
    let _tables = read_u32(&mut r)?;
    match walker.value(&mut r, tag, at, 0)? {
        Flow::Done => Ok(()),
        Flow::Unknown => Err(DecodeError::invalid(RECORD, "root value has an unknown tag")),
    }
}

struct Table<'d> {
    class: String,
    fields: Vec<(&'d [u8], u32)>,
}

struct Walker<'v, 'd, V> {
    visitor: &'v mut V,
    tables: Vec<Table<'d>>,
}

impl<'d, V: Visitor> Walker<'_, 'd, V> {
    /// `at` is the position reported if the tag is unknown.
    fn value(&mut self, r: &mut ByteReader<'d>, raw_tag: u32, at: usize, depth: usize) -> Result<Flow, DecodeError> {
        let Some(tag) = ValueTag::from_compact(raw_tag) else {
            self.visitor.unknown_tag(raw_tag, at);
            return Ok(Flow::Unknown);
        };
        match tag.compact_payload() {
            Payload::Fixed(n) if tag == ValueTag::Enum => {
                // Visitors get an enum as in the tagged layout: 8 bytes.
                let mut wide = [0; ENUM_FOR_VISITORS];
                let payload = r.bytes(n).map_err(DecodeError::truncated(RECORD))?;
                for (to, from) in wide.iter_mut().zip(payload) {
                    *to = *from;
                }
                self.visitor.scalar(tag, &wide);
            }
            Payload::Fixed(n) => {
                let payload = r.bytes(n).map_err(DecodeError::truncated(RECORD))?;
                self.visitor.scalar(tag, payload);
            }
            Payload::String => {
                let len = read_u32(r)? as usize;
                let payload = r.bytes(len).map_err(DecodeError::truncated(RECORD))?;
                self.visitor.scalar(tag, payload);
            }
            Payload::List if tag == ValueTag::PointerList => self.pointer_list(r, enter(depth)?)?,
            Payload::List => return self.value_list(r, enter(depth)?),
            Payload::Object => self.object(r, enter(depth)?)?,
        }
        Ok(Flow::Done)
    }

    fn object(&mut self, r: &mut ByteReader<'d>, depth: usize) -> Result<(), DecodeError> {
        let at = r.pos();
        if read_u32(r)? == 0 {
            return Ok(());
        }
        r.seek(at).map_err(DecodeError::truncated(RECORD))?;
        let end = read_end(r)?;
        let table = self.class_reference(r)?;
        self.element(r, table, depth)?;
        finish(r, end)?;
        self.visitor.end_object();
        Ok(())
    }

    fn pointer_list(&mut self, r: &mut ByteReader<'d>, depth: usize) -> Result<(), DecodeError> {
        let count = read_u32(r)? as usize;
        check_count(r, count, MIN_POINTER)?;
        self.visitor.list(count);
        for _ in 0..count {
            self.object(r, enter(depth)?)?;
        }
        self.visitor.end_list();
        Ok(())
    }

    fn value_list(&mut self, r: &mut ByteReader<'d>, depth: usize) -> Result<Flow, DecodeError> {
        let count = read_u32(r)? as usize;
        if count == 0 {
            self.visitor.list(0);
            self.visitor.end_list();
            return Ok(Flow::Done);
        }
        let table = self.class_reference(r)?;
        // Elements without fields take no bytes; nothing but this limit bounds the loop for them.
        let fieldless = self.tables.get(table).is_some_and(|t| t.fields.is_empty());
        check_count(r, count, 1)?;
        if fieldless && count > MAX_FIELDLESS {
            return Err(DecodeError::invalid(RECORD, format!("{count} elements without fields")));
        }
        self.visitor.list(count);
        let mut flow = Flow::Done;
        for _ in 0..count {
            let element = self.element(r, table, enter(depth)?)?;
            self.visitor.end_object();
            if let Flow::Unknown = element {
                flow = Flow::Unknown;
                break;
            }
        }
        self.visitor.end_list();
        Ok(flow)
    }

    /// Announces the object and reads its field values; the caller closes it.
    fn element(&mut self, r: &mut ByteReader<'d>, table: usize, depth: usize) -> Result<Flow, DecodeError> {
        let Some(found) = self.tables.get(table) else {
            return Err(DecodeError::invalid(RECORD, format!("class index {table} out of range")));
        };
        self.visitor.object(&found.class);
        let count = found.fields.len();
        for n in 0..count {
            let Some(&(name, tag)) = self.tables.get(table).and_then(|t| t.fields.get(n)) else {
                break;
            };
            self.visitor.field(name);
            if let Flow::Unknown = self.value(r, tag, r.pos(), depth)? {
                return Ok(Flow::Unknown);
            }
        }
        Ok(Flow::Done)
    }

    /// Reads a class reference and gives the index of its field table.
    fn class_reference(&mut self, r: &mut ByteReader<'d>) -> Result<usize, DecodeError> {
        let at = r.pos();
        let reference = read_u32(r)?;
        if reference != NEW_TABLE {
            let index = reference as usize;
            if index >= self.tables.len() {
                return Err(DecodeError::invalid(RECORD, format!("class index {index} at {at} is out of range")));
            }
            return Ok(index);
        }
        let class = read_class(r)?;
        let count = read_u32(r)? as usize;
        check_count(r, count, MIN_TABLE_ENTRY)?;
        let mut fields = Vec::with_capacity(count);
        for _ in 0..count {
            let name_len = read_u32(r)? as usize;
            let name = r.bytes(name_len).map_err(DecodeError::truncated(RECORD))?;
            fields.push((name, read_u32(r)?));
        }
        self.tables.push(Table { class, fields });
        Ok(self.tables.len() - 1)
    }
}

fn check_count(r: &ByteReader, count: usize, min_item_len: usize) -> Result<(), DecodeError> {
    if count.saturating_mul(min_item_len) > r.remaining() {
        return Err(DecodeError::invalid(RECORD, format!("count {count} does not fit before the end of the data")));
    }
    Ok(())
}
