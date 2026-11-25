//! Box-level access to ISO BMFF data: a box header parser, an iterator over the boxes in a byte
//! slice, and a bounds-checked big-endian [`Reader`] for box payloads.
//!
//! The same box format is used by MP4, QuickTime, HEIF/AVIF, CR3, JPEG 2000 (JP2, JPX, JPM, MJ2)
//! and 3GP. Use this module to read boxes that [`open`](super::open) does not model, such as
//! `uuid` boxes or HEIF `meta` items.
//!
//! ```
//! use craft_media::isobmff::boxes::{BoxIter, Reader};
//!
//! // `ftyp` with major brand `crx `, then a `uuid` box with one child.
//! let mut file = vec![0, 0, 0, 16, b'f', b't', b'y', b'p', b'c', b'r', b'x', b' ', 0, 0, 0, 1];
//! file.extend_from_slice(&[0, 0, 0, 36, b'u', b'u', b'i', b'd']);
//! file.extend_from_slice(&[0x85; 16]);
//! file.extend_from_slice(&[0, 0, 0, 12, b'C', b'M', b'T', b'1', 1, 2, 3, 4]);
//!
//! for b in BoxIter::new(&file) {
//!     let b = b?;
//!     if b.kind == b"ftyp" {
//!         assert_eq!(Reader::new(b.payload).fourcc()?, b"crx ");
//!     }
//!     if b.kind == b"uuid" && b.user_type == Some([0x85; 16]) {
//!         // Iterate the children in place; offsets stay relative to `file`.
//!         for c in BoxIter::within(&file, b.payload_range()) {
//!             let c = c?;
//!             assert_eq!((c.kind, c.offset, c.payload), (b"CMT1".into(), 40, &[1u8, 2, 3, 4][..]));
//!         }
//!     }
//! }
//! # Ok::<(), craft_media::isobmff::Error>(())
//! ```
//!
//! Nothing here recurses. A caller that walks nested boxes bounds its own depth and box count.

use crate::isobmff::bytes::FourCc;
use crate::isobmff::error::{Error, Result};
use std::ops::Range;

/// Bounds-checked big-endian cursor over a byte slice.
///
/// Every read checks the remaining length and returns [`Error::Truncated`] with the reader's label
/// when the data ends early. A failed read leaves the position unchanged.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    what: &'static str,
}

impl<'a> Reader<'a> {
    /// A reader at the start of `data`. Errors name the data `"box payload"`.
    pub fn new(data: &'a [u8]) -> Self {
        Self::labeled(data, "box payload")
    }
    /// A reader whose [`Error::Truncated`] errors name the data `what`.
    pub fn labeled(data: &'a [u8], what: &'static str) -> Self {
        Reader { data, pos: 0, what }
    }
    /// The whole slice the reader was created with.
    pub fn data(&self) -> &'a [u8] {
        self.data
    }
    /// Bytes read so far.
    pub fn position(&self) -> usize {
        self.pos
    }
    /// Bytes left to read.
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }
    /// True when no bytes are left.
    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }
    /// The unread bytes, without consuming them.
    pub fn rest(&self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or(&[])
    }
    /// Read `n` bytes.
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(Error::Truncated(self.what))?;
        let s = self.data.get(self.pos..end).ok_or(Error::Truncated(self.what))?;
        self.pos = end;
        Ok(s)
    }
    /// Skip `n` bytes.
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }
    /// Read `N` bytes into an array.
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let s = self.bytes(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Ok(a)
    }
    /// Reads a `u8`.
    pub fn u8(&mut self) -> Result<u8> {
        let [b] = self.array::<1>()?;
        Ok(b)
    }
    /// Reads a big-endian `u16`.
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    /// Reads a big-endian `i16`.
    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.array()?))
    }
    /// Reads a big-endian 24-bit unsigned integer (box flags, for example) as `u32`.
    pub fn u24(&mut self) -> Result<u32> {
        let [a, b, c] = self.array::<3>()?;
        Ok(u32::from_be_bytes([0, a, b, c]))
    }
    /// Reads a big-endian `u32`.
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    /// Reads a big-endian `i32`.
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.array()?))
    }
    /// Reads a big-endian `u64`.
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    /// Reads a big-endian `i64`.
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.array()?))
    }
    /// Reads a big-endian IEEE 754 `f64`.
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_be_bytes(self.array()?))
    }
    /// Reads a four-character code.
    pub fn fourcc(&mut self) -> Result<FourCc> {
        Ok(FourCc(self.array()?))
    }
    /// Full-box header: `(version, flags)`.
    pub fn full_header(&mut self) -> Result<(u8, u32)> {
        let v = self.u32()?;
        Ok(((v >> 24) as u8, v & 0x00FF_FFFF))
    }
    /// Read a 32-bit entry count and check that `count * entry_size` bytes remain, so a hostile
    /// count cannot drive a large allocation.
    pub fn count(&mut self, entry_size: usize) -> Result<usize> {
        let n = self.u32()? as usize;
        self.check_count(n, entry_size)?;
        Ok(n)
    }
    /// Check that `n * entry_size` bytes remain.
    pub fn check_count(&self, n: usize, entry_size: usize) -> Result<()> {
        match n.checked_mul(entry_size) {
            Some(b) if b <= self.remaining() => Ok(()),
            _ => Err(Error::Truncated(self.what)),
        }
    }
}

/// The header of one box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BoxHeader {
    /// Box type.
    pub kind: FourCc,
    /// Header length in bytes: 8, plus 8 for a 64-bit `largesize`, plus 16 for a `uuid` user type.
    pub header_len: u64,
    /// Total box size including the header. `None` for a size field of 0: the box runs to the end
    /// of its container (or of the file at top level).
    pub size: Option<u64>,
    /// The 16-byte extended type of a `uuid` box.
    pub user_type: Option<[u8; 16]>,
}

impl BoxHeader {
    /// The longest possible header: size, type, `largesize` and a `uuid` user type.
    pub const MAX_LEN: usize = 32;

    /// Parse a box header from the start of `buf`. Pass at least [`MAX_LEN`](Self::MAX_LEN) bytes
    /// when that many are available; fewer are enough for headers without `largesize` or a user
    /// type.
    ///
    /// Errors: [`Error::Truncated`] when `buf` ends inside the header, [`Error::Invalid`] when the
    /// declared size is smaller than the header.
    pub fn parse(buf: &[u8]) -> Result<BoxHeader> {
        let mut c = Reader::labeled(buf, "box header");
        let size32 = c.u32()?;
        let kind = c.fourcc()?;
        let mut header_len = 8u64;
        let size = match size32 {
            0 => None,
            1 => {
                header_len += 8;
                Some(c.u64()?)
            }
            s => Some(u64::from(s)),
        };
        let user_type = if kind == b"uuid" {
            header_len += 16;
            Some(c.array::<16>()?)
        } else {
            None
        };
        if let Some(s) = size
            && s < header_len
        {
            return Err(Error::Invalid(format!("box '{kind}' size {s} smaller than its header")));
        }
        Ok(BoxHeader { kind, header_len, size, user_type })
    }

    /// Payload size (box size minus header), or `None` when the box runs to the end.
    pub fn payload_size(&self) -> Option<u64> {
        self.size.map(|s| s.saturating_sub(self.header_len))
    }
}

/// One box found by [`BoxIter`]. Offsets are relative to the slice the iterator was created with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BoxRef<'a> {
    /// Box type.
    pub kind: FourCc,
    /// The 16-byte extended type of a `uuid` box. The payload starts after it.
    pub user_type: Option<[u8; 16]>,
    /// Offset of the box's first header byte.
    pub offset: usize,
    /// Header length in bytes, including `largesize` and the `uuid` user type.
    pub header_len: usize,
    /// The box contents after the header.
    pub payload: &'a [u8],
    /// The size field was 0, so the box runs to the end of the iterated range.
    pub extends_to_end: bool,
    /// The declared size went past the end of the range and the payload was cut there. Only
    /// [`BoxIter::allow_truncated`] produces this.
    pub truncated: bool,
}

impl BoxRef<'_> {
    /// Offset one past the box's last byte (where the next sibling starts).
    pub fn end(&self) -> usize {
        self.offset.saturating_add(self.header_len).saturating_add(self.payload.len())
    }
    /// The byte range of the whole box, header included.
    pub fn range(&self) -> Range<usize> {
        self.offset..self.end()
    }
    /// The byte range of the payload.
    pub fn payload_range(&self) -> Range<usize> {
        self.offset.saturating_add(self.header_len)..self.end()
    }
}

/// Iterator over the sibling boxes in a byte range.
///
/// Yields each complete box in order. A malformed header or a box larger than the remaining range
/// yields one `Err` and ends the iteration, so the iterator always terminates and never reads
/// outside the range. Fewer than 8 trailing bytes that are all zero (the QuickTime 32-bit
/// terminator) end the iteration silently; other short trailing data is an error.
#[derive(Clone, Debug)]
pub struct BoxIter<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
    allow_truncated: bool,
    done: bool,
}

impl<'a> BoxIter<'a> {
    /// Iterate the boxes in all of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self::within(data, 0..data.len())
    }

    /// Iterate the boxes in `data[range]`, reporting offsets relative to `data`. Use it to walk
    /// into a box's [`payload_range`](BoxRef::payload_range) (or a part of it, after a full-box
    /// header or a sample entry's fixed fields) and keep file offsets. A range that reaches past
    /// `data` is cut at its end.
    pub fn within(data: &'a [u8], range: Range<usize>) -> Self {
        let end = range.end.min(data.len());
        let pos = range.start.min(end);
        BoxIter { data, pos, end, allow_truncated: false, done: false }
    }

    /// Accept a last box whose declared size runs past the end of the range: it is yielded with
    /// the payload cut at the end and [`BoxRef::truncated`] set, and the iteration ends. For
    /// partially downloaded or cut files.
    pub fn allow_truncated(mut self) -> Self {
        self.allow_truncated = true;
        self
    }

    /// Offset where the next box would start, relative to the iterated slice.
    pub fn offset(&self) -> usize {
        self.pos
    }

    fn step(&mut self) -> Option<Result<BoxRef<'a>>> {
        let rest = self.data.get(self.pos..self.end).unwrap_or(&[]);
        if rest.is_empty() {
            return None;
        }
        if rest.len() < 8 {
            if rest.iter().all(|&b| b == 0) {
                return None;
            }
            return Some(Err(Error::Truncated("box header")));
        }
        let h = match BoxHeader::parse(rest) {
            Ok(h) => h,
            Err(e) => return Some(Err(e)),
        };
        let avail = rest.len() as u64;
        let (size, truncated) = match h.size {
            None => (avail, false),
            Some(s) if s <= avail => (s, false),
            Some(_) if self.allow_truncated => (avail, true),
            Some(s) => {
                return Some(Err(Error::Invalid(format!("box '{}' size {s} exceeds its container ({avail} bytes left)", h.kind))));
            }
        };
        // `size <= rest.len()` and `header_len <= size` (or the header could not have parsed from
        // `rest`), so both fit in usize.
        let (size, header_len) = (size as usize, h.header_len as usize);
        let payload = rest.get(header_len..size)?;
        let b = BoxRef { kind: h.kind, user_type: h.user_type, offset: self.pos, header_len, payload, extends_to_end: h.size.is_none(), truncated };
        self.pos += size;
        if truncated {
            self.done = true;
        }
        Some(Ok(b))
    }
}

impl<'a> Iterator for BoxIter<'a> {
    type Item = Result<BoxRef<'a>>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let item = self.step();
        if !matches!(item, Some(Ok(_))) {
            self.done = true;
        }
        item
    }
}

impl std::iter::FusedIterator for BoxIter<'_> {}

/// Iterate the boxes in all of `data`; the same as [`BoxIter::new`].
pub fn boxes(data: &[u8]) -> BoxIter<'_> {
    BoxIter::new(data)
}

/// The payload of the first box of type `kind` in `data`. Boxes after a malformed one are not
/// searched.
pub fn find<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data).map_while(|b| b.ok()).find(|b| b.kind == kind).map(|b| b.payload)
}
