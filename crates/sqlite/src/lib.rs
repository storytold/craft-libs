#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Wire helpers for the [SQLite database file format](https://www.sqlite.org/fileformat.html).
//!
//! The caller owns cursor advancement, field bounds, and interpretation of stored values.

use core::fmt;

const MAX_VARINT_BYTES: usize = 9;

/// A failure to decode part of a SQLite record or page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Input ended before the integer was complete.
    TruncatedVarint,
    /// The serial type is reserved and has no defined field width.
    ReservedSerialType(u64),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedVarint => f.write_str("truncated SQLite varint"),
            Self::ReservedSerialType(code) => write!(f, "reserved SQLite serial type {code}"),
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
