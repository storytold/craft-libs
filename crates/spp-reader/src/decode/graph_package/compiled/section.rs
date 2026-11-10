//! Bounds-checked reads at section offsets.

use crate::container::bytes::ByteReader;
use crate::decode::DecodeError;

const RECORD: &str = "compiled graph";

#[derive(Debug, Clone, Copy)]
pub struct Section<'a> {
    data: &'a [u8],
    base: usize,
}

impl<'a> Section<'a> {
    pub fn new(data: &'a [u8], base: usize) -> Self {
        Section { data, base }
    }

    /// Length of the section in bytes.
    pub fn len(&self) -> u32 {
        u32::try_from(self.data.len().saturating_sub(self.base)).unwrap_or(u32::MAX)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn reader(&self, off: u32) -> Result<ByteReader<'a>, DecodeError> {
        let mut r = ByteReader::new(self.data);
        r.seek(self.base.saturating_add(off as usize)).map_err(DecodeError::truncated(RECORD))?;
        Ok(r)
    }

    pub fn u32(&self, off: u32) -> Result<u32, DecodeError> {
        self.reader(off)?.u32_le().map_err(DecodeError::truncated(RECORD))
    }

    pub fn u16(&self, off: u32) -> Result<u16, DecodeError> {
        self.reader(off)?.u16_le().map_err(DecodeError::truncated(RECORD))
    }

    pub fn f32(&self, off: u32) -> Result<f32, DecodeError> {
        self.reader(off)?.f32_le().map_err(DecodeError::truncated(RECORD))
    }

    /// `n` raw bytes from `off`, or `None` when they would run past the end.
    pub fn bytes(&self, off: u32, n: u32) -> Option<&'a [u8]> {
        let start = self.base.checked_add(off as usize)?;
        self.data.get(start..start.checked_add(n as usize)?)
    }

    /// `n` u32 words from `off`.
    pub fn words(&self, off: u32, n: u32) -> Result<Vec<u32>, DecodeError> {
        let mut r = self.reader(off)?;
        (0..n).map(|_| r.u32_le().map_err(DecodeError::truncated(RECORD))).collect()
    }
}
