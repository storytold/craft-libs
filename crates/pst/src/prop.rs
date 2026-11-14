//! Property values: the types of [MS-OXCDATA] 2.11.1 as a .pst file stores them, single and
//! multi-valued ([MS-PST] 2.3.3.4).

use crate::bytes::{slice, u16_at, u32_at, u64_at, utf16};
use crate::error::{Error, Result};

/// Property types ([MS-OXCDATA] 2.11.1).
pub mod types {
    pub const NULL: u16 = 0x0001;
    pub const INT16: u16 = 0x0002;
    pub const INT32: u16 = 0x0003;
    pub const FLOAT32: u16 = 0x0004;
    pub const FLOAT64: u16 = 0x0005;
    pub const CURRENCY: u16 = 0x0006;
    pub const APP_TIME: u16 = 0x0007;
    pub const ERROR: u16 = 0x000A;
    pub const BOOLEAN: u16 = 0x000B;
    pub const OBJECT: u16 = 0x000D;
    pub const INT64: u16 = 0x0014;
    pub const STRING8: u16 = 0x001E;
    pub const STRING: u16 = 0x001F;
    pub const TIME: u16 = 0x0040;
    pub const GUID: u16 = 0x0048;
    pub const SERVER_ID: u16 = 0x00FB;
    pub const RESTRICTION: u16 = 0x00FD;
    pub const RULE_ACTION: u16 = 0x00FE;
    pub const BINARY: u16 = 0x0102;
    /// Set on a type for its multi-valued form.
    pub const MULTIPLE: u16 = 0x1000;
}

use types::{
    APP_TIME, BINARY, BOOLEAN, CURRENCY, ERROR, FLOAT32, FLOAT64, GUID, INT16, INT32, INT64, MULTIPLE, NULL, OBJECT, RESTRICTION, RULE_ACTION,
    SERVER_ID, STRING, STRING8, TIME,
};

/// A property's value.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PropValue {
    Null,
    Int16(i16),
    Int32(i32),
    Float32(f32),
    Float64(f64),
    /// Currency, in ten-thousandths.
    Currency(i64),
    /// An OLE automation date.
    AppTime(f64),
    Error(u32),
    Bool(bool),
    Int64(i64),
    /// A FILETIME: 100-nanosecond intervals since 1601-01-01 UTC.
    Time(u64),
    Guid([u8; 16]),
    String(String),
    /// Text in the item's code page, as stored (ANSI files, and some properties of Unicode ones).
    String8(Vec<u8>),
    Binary(Vec<u8>),
    /// An object in a subnode (an attached message's data): its subnode id and size.
    Object {
        nid: u32,
        size: u32,
    },
    MultiInt16(Vec<i16>),
    MultiInt32(Vec<i32>),
    MultiFloat32(Vec<f32>),
    MultiFloat64(Vec<f64>),
    MultiCurrency(Vec<i64>),
    MultiAppTime(Vec<f64>),
    MultiInt64(Vec<i64>),
    MultiTime(Vec<u64>),
    MultiGuid(Vec<[u8; 16]>),
    MultiString(Vec<String>),
    MultiString8(Vec<Vec<u8>>),
    MultiBinary(Vec<Vec<u8>>),
    /// Larger than [`crate::Limits::max_inline_value`]: not read with the rest; read it with
    /// [`crate::Pst::read_property`].
    Large {
        size: u64,
    },
    /// A type this reader doesn't know, as stored.
    Unknown {
        ty: u16,
        data: Vec<u8>,
    },
    /// A value that could not be read, and why: kept in its place so that it is accounted for,
    /// and named in the `left_out` of whatever holds it.
    Damaged {
        ty: u16,
        reason: crate::Error,
    },
}

/// One property of an object.
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub id: u16,
    pub ty: u16,
    pub value: PropValue,
}

impl PropValue {
    /// The bytes it holds outside itself (its text, bytes or values).
    pub(crate) fn held(&self) -> usize {
        use std::mem::size_of_val;
        let vecs = |v: &[Vec<u8>]| v.iter().map(|b| size_of_val(b).saturating_add(b.capacity())).sum::<usize>();
        match self {
            PropValue::String(s) => s.capacity(),
            PropValue::String8(b) | PropValue::Binary(b) | PropValue::Unknown { data: b, .. } => b.capacity(),
            PropValue::MultiInt16(v) => size_of_val(v.as_slice()),
            PropValue::MultiInt32(v) => size_of_val(v.as_slice()),
            PropValue::MultiFloat32(v) => size_of_val(v.as_slice()),
            PropValue::MultiFloat64(v) | PropValue::MultiAppTime(v) => size_of_val(v.as_slice()),
            PropValue::MultiCurrency(v) | PropValue::MultiInt64(v) => size_of_val(v.as_slice()),
            PropValue::MultiTime(v) => size_of_val(v.as_slice()),
            PropValue::MultiGuid(v) => size_of_val(v.as_slice()),
            PropValue::MultiString(v) => v.iter().map(|s| size_of_val(s).saturating_add(s.capacity())).sum(),
            PropValue::MultiString8(v) | PropValue::MultiBinary(v) => vecs(v),
            _ => 0,
        }
    }
}

impl Property {
    /// What keeping it costs: itself, and what it holds.
    pub(crate) fn kept(&self) -> u64 {
        std::mem::size_of::<Property>().saturating_add(self.value.held()) as u64
    }

    /// The 32-bit property tag (id in the high half, type in the low).
    pub fn tag(&self) -> u32 {
        (u32::from(self.id) << 16) | u32::from(self.ty)
    }
}

/// An object's properties, in the order of their ids.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Properties(pub Vec<Property>);

impl Properties {
    pub fn get(&self, id: u16) -> Option<&PropValue> {
        self.0.iter().find(|p| p.id == id).map(|p| &p.value)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Property> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// A text property: Unicode, or 8-bit text read as Latin-1. Latin-1 is right only for
    /// part of the code pages an ANSI file may use; the import decodes [`PropValue::String8`]
    /// with the item's own code page (`PidTagMessageCodepage`, `PidTagInternetCodepage`)
    /// instead of calling this.
    pub fn text(&self, id: u16) -> Option<String> {
        match self.get(id)? {
            PropValue::String(s) => Some(s.clone()),
            PropValue::String8(b) => Some(b.iter().map(|&c| char::from(c)).collect()),
            _ => None,
        }
    }

    /// The properties that could not be read ([`PropValue::Damaged`]): id and reason.
    pub fn damaged(&self) -> impl Iterator<Item = (u16, &crate::Error)> {
        self.0.iter().filter_map(|p| match &p.value {
            PropValue::Damaged { reason, .. } => Some((p.id, reason)),
            _ => None,
        })
    }

    pub fn int(&self, id: u16) -> Option<i64> {
        match self.get(id)? {
            PropValue::Int16(v) => Some(i64::from(*v)),
            PropValue::Int32(v) => Some(i64::from(*v)),
            PropValue::Int64(v) => Some(*v),
            PropValue::Error(v) => Some(i64::from(*v)),
            _ => None,
        }
    }

    pub fn bool(&self, id: u16) -> Option<bool> {
        match self.get(id)? {
            PropValue::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub fn binary(&self, id: u16) -> Option<&[u8]> {
        match self.get(id)? {
            PropValue::Binary(v) => Some(v),
            _ => None,
        }
    }

    pub fn time(&self, id: u16) -> Option<u64> {
        match self.get(id)? {
            PropValue::Time(v) => Some(*v),
            _ => None,
        }
    }
}

/// The size of a fixed-size type, `None` for a variable-size one.
pub(crate) fn fixed_size(ty: u16) -> Option<usize> {
    match ty {
        NULL => Some(0),
        BOOLEAN => Some(1),
        INT16 => Some(2),
        INT32 | FLOAT32 | ERROR => Some(4),
        FLOAT64 | CURRENCY | APP_TIME | INT64 | TIME => Some(8),
        GUID => Some(16),
        _ => None,
    }
}

/// A fixed-size value from exactly its bytes (or more: the first are read).
pub(crate) fn fixed(ty: u16, b: &[u8]) -> Result<PropValue> {
    Ok(match ty {
        NULL => PropValue::Null,
        BOOLEAN => PropValue::Bool(b.first().copied().ok_or(Error::Corrupt("a short value"))? != 0),
        INT16 => PropValue::Int16(u16_at(b, 0)? as i16),
        INT32 => PropValue::Int32(u32_at(b, 0)? as i32),
        FLOAT32 => PropValue::Float32(f32::from_bits(u32_at(b, 0)?)),
        ERROR => PropValue::Error(u32_at(b, 0)?),
        FLOAT64 => PropValue::Float64(f64::from_bits(u64_at(b, 0)?)),
        CURRENCY => PropValue::Currency(u64_at(b, 0)? as i64),
        APP_TIME => PropValue::AppTime(f64::from_bits(u64_at(b, 0)?)),
        INT64 => PropValue::Int64(u64_at(b, 0)? as i64),
        TIME => PropValue::Time(u64_at(b, 0)?),
        GUID => PropValue::Guid(crate::bytes::array16(b, 0)?),
        other => PropValue::Unknown { ty: other, data: b.to_vec() },
    })
}

/// What building a value from its `b` bytes holds beyond them, at most: for a list of
/// strings or binaries, each item's own vector and its place while the list is made (an
/// empty item is four bytes in the file and far more in memory). Other values hold no more
/// than their bytes.
pub(crate) fn building_cost(ty: u16, b: &[u8]) -> usize {
    let variable = matches!(ty, t if t == STRING | MULTIPLE || t == STRING8 | MULTIPLE || t == BINARY | MULTIPLE);
    if !variable {
        return 0;
    }
    let count = u32_at(b, 0).map_or(0, |c| (c as usize).min(b.len() / 4));
    count.saturating_mul(std::mem::size_of::<Vec<u8>>().saturating_add(std::mem::size_of::<&[u8]>()))
}

/// A value stored apart (in the heap or a subnode), from all its bytes.
pub(crate) fn stored(ty: u16, b: &[u8]) -> Result<PropValue> {
    if ty & MULTIPLE != 0 {
        return multi(ty & !MULTIPLE, b);
    }
    Ok(match ty {
        STRING => PropValue::String(utf16(b)),
        STRING8 => PropValue::String8(trim_nul(b).to_vec()),
        BINARY | SERVER_ID | RESTRICTION | RULE_ACTION => PropValue::Binary(b.to_vec()),
        OBJECT => PropValue::Object { nid: u32_at(b, 0)?, size: u32_at(b, 4)? },
        t if fixed_size(t).is_some() => fixed(t, b)?,
        other => PropValue::Unknown { ty: other, data: b.to_vec() },
    })
}

fn trim_nul(b: &[u8]) -> &[u8] {
    let mut end = b.len();
    while end > 0 && b.get(end.saturating_sub(1)) == Some(&0) {
        end = end.saturating_sub(1);
    }
    b.get(..end).unwrap_or(b)
}

fn multi(base: u16, b: &[u8]) -> Result<PropValue> {
    if let Some(size) = fixed_size(base).filter(|&s| s > 1) {
        if !b.len().is_multiple_of(size) {
            return Err(Error::Corrupt("a multi-valued property's size"));
        }
        // Each value straight into its own type: no value is held twice on the way.
        return Ok(match base {
            INT16 => PropValue::MultiInt16(values(b, size, |c| Ok(u16_at(c, 0)? as i16))?),
            INT32 => PropValue::MultiInt32(values(b, size, |c| Ok(u32_at(c, 0)? as i32))?),
            FLOAT32 => PropValue::MultiFloat32(values(b, size, |c| Ok(f32::from_bits(u32_at(c, 0)?)))?),
            FLOAT64 => PropValue::MultiFloat64(values(b, size, |c| Ok(f64::from_bits(u64_at(c, 0)?)))?),
            CURRENCY => PropValue::MultiCurrency(values(b, size, |c| Ok(u64_at(c, 0)? as i64))?),
            APP_TIME => PropValue::MultiAppTime(values(b, size, |c| Ok(f64::from_bits(u64_at(c, 0)?)))?),
            INT64 => PropValue::MultiInt64(values(b, size, |c| Ok(u64_at(c, 0)? as i64))?),
            TIME => PropValue::MultiTime(values(b, size, |c| u64_at(c, 0))?),
            GUID => PropValue::MultiGuid(values(b, size, |c| crate::bytes::array16(c, 0))?),
            _ => PropValue::Unknown { ty: base | MULTIPLE, data: b.to_vec() },
        });
    }
    // Only the lists of strings and binaries have their items read; any other is kept as bytes.
    if !matches!(base, STRING | STRING8 | BINARY) {
        return Ok(PropValue::Unknown { ty: base | MULTIPLE, data: b.to_vec() });
    }
    let items = variable_items(b)?;
    Ok(match base {
        STRING => PropValue::MultiString(items.into_iter().map(utf16).collect()),
        STRING8 => PropValue::MultiString8(items.into_iter().map(|i| trim_nul(i).to_vec()).collect()),
        _ => PropValue::MultiBinary(items.into_iter().map(<[u8]>::to_vec).collect()),
    })
}

/// The items of a multi-valued property of a variable-size type (2.3.3.4.2).
/// Each `size` bytes of `b` straight into a value of its own type: none is held twice.
fn values<T>(b: &[u8], size: usize, f: impl Fn(&[u8]) -> Result<T>) -> Result<Vec<T>> {
    let mut out = Vec::with_capacity(b.len().checked_div(size).unwrap_or(0));
    for c in b.chunks_exact(size) {
        out.push(f(c)?);
    }
    Ok(out)
}

fn variable_items(b: &[u8]) -> Result<Vec<&[u8]>> {
    if b.is_empty() {
        return Ok(Vec::new());
    }
    let count = u32_at(b, 0)? as usize;
    // Each item has its 4-byte offset, so the count can't pass a quarter of the size.
    if count > b.len() / 4 {
        return Err(Error::Corrupt("a multi-valued property's count"));
    }
    let table_end = count.saturating_mul(4).saturating_add(4);
    let offset = |i: usize| -> Result<usize> {
        if i >= count {
            return Ok(b.len());
        }
        Ok(u32_at(b, i.saturating_mul(4).saturating_add(4))? as usize)
    };
    let mut items = Vec::with_capacity(count);
    for i in 0..count {
        let (start, end) = (offset(i)?, offset(i.saturating_add(1))?);
        if start < table_end || end < start || end > b.len() {
            return Err(Error::Corrupt("a multi-valued property's offsets"));
        }
        items.push(slice(b, start, end.saturating_sub(start))?);
    }
    Ok(items)
}
