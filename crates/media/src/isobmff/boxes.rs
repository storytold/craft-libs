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
    pub fn u8(&mut self) -> Result<u8> {
        let [b] = self.array::<1>()?;
        Ok(b)
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.array()?))
    }
    pub fn u24(&mut self) -> Result<u32> {
        let [a, b, c] = self.array::<3>()?;
        Ok(u32::from_be_bytes([0, a, b, c]))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.array()?))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.array()?))
    }
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_be_bytes(self.array()?))
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isobmff::bytes::BoxBuf;

    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn header_forms() {
        let h = BoxHeader::parse(&[0, 0, 0, 16, b'f', b'r', b'e', b'e']).unwrap();
        assert_eq!((h.kind, h.header_len, h.size, h.payload_size()), (FourCc(*b"free"), 8, Some(16), Some(8)));
        let h = BoxHeader::parse(&[0, 0, 0, 0, b'm', b'd', b'a', b't']).unwrap();
        assert_eq!((h.size, h.payload_size()), (None, None));
        let mut v = vec![0, 0, 0, 1, b'm', b'd', b'a', b't'];
        v.extend_from_slice(&100u64.to_be_bytes());
        let h = BoxHeader::parse(&v).unwrap();
        assert_eq!((h.header_len, h.size), (16, Some(100)));
        let mut v = vec![0, 0, 0, 30, b'u', b'u', b'i', b'd'];
        v.extend_from_slice(&[7u8; 16]);
        let h = BoxHeader::parse(&v).unwrap();
        assert_eq!((h.header_len, h.user_type), (24, Some([7u8; 16])));
        assert!(BoxHeader::parse(&[0, 0, 0, 4, b'f', b'r', b'e', b'e']).is_err());
        assert!(BoxHeader::parse(&[0, 0, 0]).is_err());
        // A uuid box whose size does not cover its user type.
        let mut v = vec![0, 0, 0, 20, b'u', b'u', b'i', b'd'];
        v.extend_from_slice(&[7u8; 16]);
        assert!(BoxHeader::parse(&v).is_err());
    }

    #[test]
    fn iterate_children() {
        let mut b = BoxBuf::new();
        b.leaf(b"aaaa", &[1, 2]);
        let m = b.start(b"bbbb");
        b.leaf(b"cccc", &[]);
        b.end(m);
        b.u32(0); // QT terminator
        let kids: Vec<_> = boxes(&b.buf).collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(kids.len(), 2);
        assert_eq!(kids[0].payload, &[1, 2]);
        assert_eq!((kids[1].offset, kids[1].payload_range(), kids[1].range()), (10, 18..26, 10..26));
        assert_eq!(find(&b.buf, b"bbbb").map(|p| p.len()), Some(8));
        let inner: Vec<_> = BoxIter::within(&b.buf, kids[1].payload_range()).map(|r| r.unwrap().offset).collect();
        assert_eq!(inner, [18]);
        // Oversized child → one error, then the iterator stops.
        let bad = [0u8, 0, 0, 99, b'x', b'x', b'x', b'x', 0];
        let mut it = boxes(&bad);
        assert!(it.next().unwrap().is_err());
        assert!(it.next().is_none());
    }

    #[test]
    fn large_size_zero_size_and_uuid() {
        let mut large = vec![0, 0, 0, 1, b'm', b'd', b'a', b't'];
        large.extend_from_slice(&19u64.to_be_bytes());
        large.extend_from_slice(&[9, 9, 9]);
        let mut uuid = vec![0, 0, 0, 26, b'u', b'u', b'i', b'd'];
        uuid.extend_from_slice(&[0xAB; 16]);
        uuid.extend_from_slice(&[1, 2]);
        let tail = [0, 0, 0, 0, b'm', b'd', b'a', b't', 5, 6, 7];
        let file = [large, uuid, tail.to_vec()].concat();
        let all: Vec<_> = boxes(&file).collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!((all[0].header_len, all[0].payload), (16, &[9u8, 9, 9][..]));
        assert_eq!((all[1].user_type, all[1].header_len, all[1].payload), (Some([0xAB; 16]), 24, &[1u8, 2][..]));
        assert_eq!((all[2].offset, all[2].extends_to_end, all[2].payload), (45, true, &[5u8, 6, 7][..]));
        // Size 0 inside a range runs to the end of the range, not of the slice.
        let in_range: Vec<_> = BoxIter::within(&file, 45..53).map(|r| r.unwrap().payload.len()).collect();
        assert_eq!(in_range, [0]);
    }

    #[test]
    fn truncated_last_box() {
        let mut file = bx(b"ftyp", b"jp2 ");
        let mut cut = bx(b"jp2c", &[0xFF, 0x4F, 0xFF, 0x51, 0, 0]);
        cut.truncate(10);
        file.extend_from_slice(&cut);
        assert!(boxes(&file).nth(1).unwrap().is_err());
        let all: Vec<_> = boxes(&file).allow_truncated().collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(all.len(), 2);
        assert!(!all[0].truncated);
        assert_eq!((all[1].truncated, all[1].payload), (true, &[0xFF, 0x4F][..]));
    }

    #[test]
    fn hostile_sizes_never_panic() {
        let headers: [&[u8]; 6] = [
            &[0xFF, 0xFF, 0xFF, 0xFF, b'a', b'b', b'c', b'd'],
            &[0, 0, 0, 1, b'a', b'b', b'c', b'd', 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
            &[0, 0, 0, 1, b'a', b'b', b'c', b'd', 0, 0, 0, 0, 0, 0, 0, 3],
            &[0, 0, 0, 1, b'u', b'u', b'i', b'd', 0, 0, 0, 0, 0, 0, 0, 30],
            &[0, 0, 0, 0, b'u', b'u', b'i', b'd', 1, 2, 3],
            &[0, 0, 0, 7, b'a', b'b', b'c', b'd'],
        ];
        for h in headers {
            for lenient in [false, true] {
                let mut it = BoxIter::new(h);
                if lenient {
                    it = it.allow_truncated();
                }
                assert!(it.count() <= 1);
            }
        }
        // Ranges outside the data are clipped.
        assert_eq!(BoxIter::within(&[0, 0, 0, 8, 1, 2, 3, 4], 4..100).count(), 1);
        #[allow(clippy::reversed_empty_ranges)]
        let backwards = 6..2;
        assert_eq!(BoxIter::within(&[0; 8], backwards).count(), 0);
        assert_eq!(BoxIter::within(&[0; 8], 50..60).count(), 0);
    }

    #[test]
    fn reader_bounds() {
        let mut c = Reader::labeled(&[0, 0, 0, 5, 1], "t");
        assert!(c.count(1).is_err());
        let mut c = Reader::new(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert!(c.count(usize::MAX).is_err());
        let mut c = Reader::new(&[1, 2, 3, 4, 5]);
        assert_eq!(c.u24().unwrap(), 0x010203);
        assert!(c.u32().is_err());
        assert_eq!((c.position(), c.rest()), (3, &[4u8, 5][..]));
        assert!(matches!(Reader::labeled(&[], "x").u8(), Err(Error::Truncated("x"))));
    }
}
