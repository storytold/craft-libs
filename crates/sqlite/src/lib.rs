#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Wire helpers for the [SQLite database file format](https://www.sqlite.org/fileformat.html).
//!
//! The caller owns cursor advancement, field bounds, and interpretation of stored values.

use core::fmt;

const MAX_VARINT_BYTES: usize = 9;

/// The b-tree cell layouts that carry payload bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadKind {
    /// A table leaf cell; table interior cells have no payload.
    TableLeaf,
    /// An index leaf or interior cell, which share the same payload thresholds.
    Index,
}

/// Return the number of payload bytes stored in the b-tree page before overflow.
///
/// `usable` is the page size minus its reserved bytes, not the space remaining after the
/// cell header. It may be odd. Sizes outside 480..=65,536 return `None`; validating the
/// database header and bounding payload reads remain the caller's responsibility.
///
/// This implements the [SQLite b-tree cell payload formula](https://www.sqlite.org/fileformat.html#b_tree_pages).
/// The calculation allocates nothing and accepts the full `u64` payload length without
/// implying that the payload fits in the database or in memory.
pub fn local_payload_len(usable: u32, payload_len: u64, kind: PayloadKind) -> Option<u32> {
    // SQLite requires at least 480 usable bytes; this also keeps every divisor and
    // subtraction below valid even when the caller supplies an untrusted header.
    if !(480..=65_536).contains(&usable) {
        return None;
    }
    let usable = u64::from(usable);
    let maximum = match kind {
        PayloadKind::TableLeaf => usable - 35,
        PayloadKind::Index => (usable - 12) * 64 / 255 - 23,
    };
    let local = if payload_len <= maximum {
        payload_len
    } else {
        let minimum = (usable - 12) * 32 / 255 - 23;
        let candidate = minimum + (payload_len - minimum) % (usable - 4);
        if candidate <= maximum { candidate } else { minimum }
    };
    // Both branches bound the result by the supported page's maximum local payload.
    Some(local as u32)
}

/// A failure to decode part of a SQLite record or page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Input ended before the integer was complete.
    TruncatedVarint,
    /// The record's declared header length excludes its prefix or extends beyond the payload.
    InvalidRecordHeaderLength(u64),
    /// The serial type is reserved and has no defined field width.
    ReservedSerialType(u64),
    /// The decoded database page size is not a power of two in `512..=65_536`.
    InvalidPageSize(u32),
    /// The usable page size (`page_size - reserved`) is below 480 bytes.
    UsablePageSizeTooSmall(u32),
    /// A B-tree page ended before its header or cell-pointer array completed.
    TruncatedPage,
    /// The B-tree page type byte is not a table or index interior/leaf page.
    UnsupportedBtreePageType(u8),
    /// A cell pointer falls outside `[pointers_end, usable)`.
    InvalidCellPointer { offset: usize, min: usize, usable: usize },
    /// The requested cell index is outside the page's cell count.
    CellIndexOutOfBounds { index: usize, cells: usize },
    /// A signed big-endian integer slice is empty or wider than 8 bytes.
    InvalidIntegerWidth(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedVarint => f.write_str("truncated SQLite varint"),
            Self::InvalidRecordHeaderLength(length) => write!(f, "invalid SQLite record header length {length}"),
            Self::ReservedSerialType(code) => write!(f, "reserved SQLite serial type {code}"),
            Self::InvalidPageSize(size) => write!(f, "invalid SQLite page size {size}"),
            Self::UsablePageSizeTooSmall(usable) => write!(f, "usable SQLite page size {usable} is too small"),
            Self::TruncatedPage => f.write_str("truncated SQLite B-tree page"),
            Self::UnsupportedBtreePageType(kind) => write!(f, "unsupported SQLite B-tree page type 0x{kind:02x}"),
            Self::InvalidCellPointer { offset, min, usable } => write!(f, "invalid SQLite cell pointer {offset} outside {min}..{usable}"),
            Self::CellIndexOutOfBounds { index, cells } => write!(f, "SQLite cell index {index} out of bounds for {cells} cells"),
            Self::InvalidIntegerWidth(width) => write!(f, "invalid SQLite signed integer width {width}"),
        }
    }
}

impl core::error::Error for Error {}

/// Decode the leading SQLite varint and return its raw bit pattern and byte count.
///
/// The first eight bytes each contribute seven bits. If all eight have the high bit set,
/// the ninth contributes all eight bits. The caller interprets the bits as signed when needed.
/// Decoding ends at the first complete representation, including non-minimal encodings.
///
/// # Errors
///
/// Returns [`Error::TruncatedVarint`] if the slice ends before the integer is complete.
///
/// ```
/// use craft_sqlite::read_varint;
///
/// assert_eq!(read_varint(&[0x81, 0x00, 0xff]), Ok((128, 2)));
/// ```
pub fn read_varint(input: &[u8]) -> Result<(u64, usize), Error> {
    let mut value = 0;
    for (index, &byte) in input.iter().take(MAX_VARINT_BYTES).enumerate() {
        if index == MAX_VARINT_BYTES - 1 {
            return Ok(((value << 8) | u64::from(byte), MAX_VARINT_BYTES));
        }
        value = (value << 7) | u64::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    Err(Error::TruncatedVarint)
}

/// Split a SQLite record payload into its serial-type bytes and body.
///
/// The [record header length](https://www.sqlite.org/fileformat.html#record_format) includes
/// the leading length varint. The returned serial-type slice excludes that varint and ends
/// before the body. The caller decodes serial types from that slice and checks field lengths.
///
/// # Errors
///
/// Returns [`Error::TruncatedVarint`] for an incomplete length and
/// [`Error::InvalidRecordHeaderLength`] for a size outside the supplied payload's bounds.
///
/// ```
/// use craft_sqlite::split_record;
///
/// assert_eq!(split_record(&[2, 1, 42]), Ok((&[1][..], &[42][..])));
/// ```
pub fn split_record(payload: &[u8]) -> Result<(&[u8], &[u8]), Error> {
    let (length, length_bytes) = read_varint(payload)?;
    let invalid_length = Error::InvalidRecordHeaderLength(length);
    let header_end = usize::try_from(length).map_err(|_| invalid_length)?;
    let (header, body) = payload.split_at_checked(header_end).ok_or(invalid_length)?;
    let serial_types = header.get(length_bytes..).ok_or(invalid_length)?;
    Ok((serial_types, body))
}

/// Return the encoded body width in bytes for a SQLite record serial type.
///
/// The width is independent of the host's pointer size. The caller checks it against the
/// available body bytes before reading a field.
///
/// # Errors
///
/// Returns [`Error::ReservedSerialType`] for codes 10 and 11.
///
/// ```
/// use craft_sqlite::serial_type_len;
///
/// assert_eq!(serial_type_len(3), Ok(3));
/// assert_eq!(serial_type_len(17), Ok(2));
/// ```
pub const fn serial_type_len(serial_type: u64) -> Result<u64, Error> {
    match serial_type {
        0 | 8 | 9 => Ok(0),
        1..=4 => Ok(serial_type),
        5 => Ok(6),
        6 | 7 => Ok(8),
        10 | 11 => Err(Error::ReservedSerialType(serial_type)),
        // Integer division accounts for the one-code difference between BLOB and text.
        _ => Ok((serial_type - 12) / 2),
    }
}

/// Decode a raw big-endian 16-bit SQLite header page size (`1` represents `65_536`).
///
/// # Errors
///
/// Returns [`Error::InvalidPageSize`] when the decoded size is not a power of two in `512..=65_536`.
///
/// ```
/// use craft_sqlite::page_size_from_raw;
///
/// assert_eq!(page_size_from_raw(1), Ok(65_536));
/// assert_eq!(page_size_from_raw(4096), Ok(4096));
/// ```
pub const fn page_size_from_raw(raw: u16) -> Result<u32, Error> {
    let size = if raw == 1 { 65_536 } else { raw as u32 };
    if size < 512 || !size.is_power_of_two() { Err(Error::InvalidPageSize(size)) } else { Ok(size) }
}

/// Compute the usable byte count of a SQLite page after subtracting `reserved` trailing bytes.
///
/// # Errors
///
/// Returns [`Error::UsablePageSizeTooSmall`] when `page_size - reserved < 480`.
pub const fn usable_page_size(page_size: u32, reserved: u8) -> Result<u32, Error> {
    let Some(usable) = page_size.checked_sub(reserved as u32) else {
        return Err(Error::UsablePageSizeTooSmall(0));
    };
    if usable < 480 { Err(Error::UsablePageSizeTooSmall(usable)) } else { Ok(usable) }
}

/// Decode a 1..=8-byte big-endian signed two's-complement integer into `i64`.
///
/// # Errors
///
/// Returns [`Error::InvalidIntegerWidth`] if `bytes` is empty or longer than 8 bytes.
///
/// ```
/// use craft_sqlite::decode_signed_be_int;
///
/// assert_eq!(decode_signed_be_int(&[0x80, 0x00, 0x01]), Ok(-8_388_607));
/// ```
pub fn decode_signed_be_int(bytes: &[u8]) -> Result<i64, Error> {
    let Some(&first) = bytes.first() else {
        return Err(Error::InvalidIntegerWidth(0));
    };
    if bytes.len() > 8 {
        return Err(Error::InvalidIntegerWidth(bytes.len()));
    }
    let mut value: i64 = if first & 0x80 != 0 { -1 } else { 0 };
    for &byte in bytes {
        value = (value << 8) | i64::from(byte);
    }
    Ok(value)
}

/// Parsed B-tree page header and cell-pointer array bounds within a usable page slice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BtreePageHeader {
    /// B-tree page type (`0x02`, `0x05`, `0x0a`, or `0x0d`).
    pub kind: u8,
    /// Offset where the B-tree page header starts (`100` on page 1, `0` otherwise).
    pub header_offset: usize,
    /// Number of cells declared in the B-tree header.
    pub cells: usize,
    /// Start offset of the 2-byte big-endian cell-pointer array.
    pub pointers_offset: usize,
    /// Exclusive end offset of the cell-pointer array.
    pub pointers_end: usize,
}

/// Parse and bounds-check the B-tree page header and cell-pointer array on `usable_page`.
///
/// # Errors
///
/// Returns [`Error::TruncatedPage`] if the page ends before the header or pointer array, or
/// [`Error::UnsupportedBtreePageType`] if the page type byte is not `0x02`, `0x05`, `0x0a`, or `0x0d`.
pub fn parse_btree_page_header(usable_page: &[u8], is_page_one: bool) -> Result<BtreePageHeader, Error> {
    let header_offset = if is_page_one { 100 } else { 0 };
    let Some(&kind) = usable_page.get(header_offset) else {
        return Err(Error::TruncatedPage);
    };
    let header_size = match kind {
        0x05 | 0x02 => 12,
        0x0d | 0x0a => 8,
        _ => return Err(Error::UnsupportedBtreePageType(kind)),
    };
    let Some(cells_bytes) = header_offset
        .checked_add(3)
        .and_then(|start| start.checked_add(2).and_then(|end| usable_page.get(start..end)))
        .and_then(|slice| <[u8; 2]>::try_from(slice).ok())
    else {
        return Err(Error::TruncatedPage);
    };
    let cells = usize::from(u16::from_be_bytes(cells_bytes));
    let Some(pointers_offset) = header_offset.checked_add(header_size) else {
        return Err(Error::TruncatedPage);
    };
    let Some(pointers_bytes) = cells.checked_mul(2) else {
        return Err(Error::TruncatedPage);
    };
    let Some(pointers_end) = pointers_offset.checked_add(pointers_bytes) else {
        return Err(Error::TruncatedPage);
    };
    if pointers_end > usable_page.len() {
        return Err(Error::TruncatedPage);
    }
    Ok(BtreePageHeader { kind, header_offset, cells, pointers_offset, pointers_end })
}

/// Read and bounds-check the `index`-th cell offset from a parsed [`BtreePageHeader`].
///
/// # Errors
///
/// Returns [`Error::CellIndexOutOfBounds`] when `index >= header.cells`, [`Error::TruncatedPage`]
/// if the pointer slot is out of bounds, or [`Error::InvalidCellPointer`] if the pointer lies
/// outside `[header.pointers_end, usable_page.len())`.
pub fn btree_cell_offset(usable_page: &[u8], header: &BtreePageHeader, index: usize) -> Result<usize, Error> {
    if index >= header.cells {
        return Err(Error::CellIndexOutOfBounds { index, cells: header.cells });
    }
    let Some(bytes) = index
        .checked_mul(2)
        .and_then(|off| header.pointers_offset.checked_add(off))
        .and_then(|ptr| ptr.checked_add(2).and_then(|end| usable_page.get(ptr..end)))
        .and_then(|slice| <[u8; 2]>::try_from(slice).ok())
    else {
        return Err(Error::TruncatedPage);
    };
    let offset = usize::from(u16::from_be_bytes(bytes));
    if offset < header.pointers_end || offset >= usable_page.len() {
        return Err(Error::InvalidCellPointer { offset, min: header.pointers_end, usable: usable_page.len() });
    }
    Ok(offset)
}
