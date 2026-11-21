//! The record framing [MS-PPT] shares with [MS-ODRAW], from `craft-officeart`: an 8-byte header
//! (version, instance, type, length) followed by `len` bytes, which for a container (version
//! `0xF`) are more records. PowerPoint's own records add a reader for their variable-length bodies.

pub use craft_officeart::record::{Rec, Records, u16_at, u32_at};

/// Containers nest at most this deep; deeper input is treated as damaged and skipped.
pub const MAX_DEPTH: usize = 32;

/// A little-endian reader over a record body for the variable-length structures (text
/// exceptions, style runs): every read is checked and a short body ends the walk.
pub struct Cursor<'a> {
    pub b: &'a [u8],
    pub at: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Cursor { b, at: 0 }
    }
    /// Bytes left to read.
    pub fn remaining(&self) -> usize {
        self.b.len().saturating_sub(self.at)
    }
    pub fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.at)?;
        self.at += 1;
        Some(v)
    }
    pub fn u16(&mut self) -> Option<u16> {
        let v = u16_at(self.b, self.at)?;
        self.at += 2;
        Some(v)
    }
    pub fn i16(&mut self) -> Option<i16> {
        self.u16().map(|v| v as i16)
    }
    pub fn u32(&mut self) -> Option<u32> {
        let v = u32_at(self.b, self.at)?;
        self.at += 4;
        Some(v)
    }
}

#[cfg(test)]
pub(crate) mod build {
    //! Writing records, for tests that build `.ppt` files.

    pub fn atom(kind: u16, instance: u16, body: &[u8]) -> Vec<u8> {
        head(0, kind, instance, body)
    }

    pub fn container(kind: u16, instance: u16, children: &[Vec<u8>]) -> Vec<u8> {
        head(0xF, kind, instance, &children.concat())
    }

    fn head(ver: u16, kind: u16, instance: u16, body: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(body.len() + 8);
        v.extend_from_slice(&(ver | (instance << 4)).to_le_bytes());
        v.extend_from_slice(&kind.to_le_bytes());
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        v
    }
}
