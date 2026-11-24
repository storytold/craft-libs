//! Four-character codes and a box builder for the muxers. Box reading lives in
//! [`boxes`](super::boxes).

pub(crate) use crate::isobmff::boxes::{BoxHeader, Reader as Cur, boxes, find};
use std::fmt;

/// A four-character code (box type, sample entry format, brand, handler type…).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct FourCc(pub [u8; 4]);

impl FourCc {
    pub const fn new(b: &[u8; 4]) -> Self {
        FourCc(*b)
    }
    pub fn as_u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }
    /// Lossy string form; `©` (0xA9) is mapped to U+00A9, other non-printables to `?`.
    pub fn to_string_lossy(self) -> String {
        self.0
            .iter()
            .map(|&b| match b {
                0xA9 => '©',
                0x20..=0x7E => b as char,
                _ => '?',
            })
            .collect()
    }
}

impl fmt::Display for FourCc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_lossy())
    }
}
impl fmt::Debug for FourCc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FourCc({:?})", self.to_string_lossy())
    }
}
impl From<&[u8; 4]> for FourCc {
    fn from(b: &[u8; 4]) -> Self {
        FourCc(*b)
    }
}
impl PartialEq<[u8; 4]> for FourCc {
    fn eq(&self, other: &[u8; 4]) -> bool {
        &self.0 == other
    }
}
impl PartialEq<&[u8; 4]> for FourCc {
    fn eq(&self, other: &&[u8; 4]) -> bool {
        &self.0 == *other
    }
}
impl PartialEq<&&[u8; 4]> for FourCc {
    fn eq(&self, other: &&&[u8; 4]) -> bool {
        &self.0 == **other
    }
}

/// Growable buffer for building boxes.
#[derive(Default, Clone, Debug)]
pub(crate) struct BoxBuf {
    pub buf: Vec<u8>,
}

impl BoxBuf {
    pub fn new() -> Self {
        Self::default()
    }
    /// Begin a box; returns a marker to pass to [`end`](Self::end).
    pub fn start(&mut self, kind: &[u8; 4]) -> usize {
        let at = self.buf.len();
        self.buf.extend_from_slice(&[0, 0, 0, 0]);
        self.buf.extend_from_slice(kind);
        at
    }
    pub fn start_full(&mut self, kind: &[u8; 4], version: u8, flags: u32) -> usize {
        let at = self.start(kind);
        self.u32(((version as u32) << 24) | (flags & 0xFF_FFFF));
        at
    }
    pub fn end(&mut self, at: usize) {
        let size = (self.buf.len() - at) as u32;
        self.buf[at..at + 4].copy_from_slice(&size.to_be_bytes());
    }
    /// Append a complete box with the given payload.
    pub fn leaf(&mut self, kind: &[u8; 4], payload: &[u8]) {
        let at = self.start(kind);
        self.bytes(payload);
        self.end(at);
    }
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    pub fn i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    pub fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    pub fn f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    pub fn zeros(&mut self, n: usize) {
        self.buf.resize(self.buf.len() + n, 0);
    }
    pub fn len(&self) -> usize {
        self.buf.len()
    }
}
