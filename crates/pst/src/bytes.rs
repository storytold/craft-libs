//! Little-endian reads out of a byte slice that fail instead of panicking.

use crate::error::{Error, Result};

const SHORT: Error = Error::Corrupt("a structure runs past its end");

pub(crate) fn slice(b: &[u8], at: usize, len: usize) -> Result<&[u8]> {
    let end = at.checked_add(len).ok_or(SHORT)?;
    b.get(at..end).ok_or(SHORT)
}

pub(crate) fn u8_at(b: &[u8], at: usize) -> Result<u8> {
    b.get(at).copied().ok_or(SHORT)
}

pub(crate) fn u16_at(b: &[u8], at: usize) -> Result<u16> {
    let s = slice(b, at, 2)?;
    Ok(u16::from_le_bytes([s.first().copied().unwrap_or(0), s.get(1).copied().unwrap_or(0)]))
}

pub(crate) fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    let s = slice(b, at, 4)?;
    let mut a = [0u8; 4];
    a.copy_from_slice(s);
    Ok(u32::from_le_bytes(a))
}

pub(crate) fn u64_at(b: &[u8], at: usize) -> Result<u64> {
    let s = slice(b, at, 8)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(s);
    Ok(u64::from_le_bytes(a))
}

pub(crate) fn array16(b: &[u8], at: usize) -> Result<[u8; 16]> {
    let s = slice(b, at, 16)?;
    let mut a = [0u8; 16];
    a.copy_from_slice(s);
    Ok(a)
}

/// UTF-16LE text, lossy; an odd last byte and trailing NULs are dropped.
pub(crate) fn utf16(b: &[u8]) -> String {
    let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let mut s = String::from_utf16_lossy(&units);
    while s.ends_with('\0') {
        s.pop();
    }
    s
}
