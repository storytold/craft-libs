//! Checked little/big-endian reads over untrusted bytes. Every read returns `None` when out of range.

#[derive(Debug, Clone, Copy)]
pub struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data }
    }

    pub fn slice(&self, off: usize, len: usize) -> Option<&'a [u8]> {
        let end = off.checked_add(len)?;
        self.data.get(off..end)
    }

    pub fn u8(&self, off: usize) -> Option<u8> {
        self.data.get(off).copied()
    }

    fn arr<const N: usize>(&self, off: usize) -> Option<[u8; N]> {
        self.slice(off, N)?.try_into().ok()
    }

    pub fn u16_be(&self, off: usize) -> Option<u16> {
        self.arr(off).map(u16::from_be_bytes)
    }

    pub fn u32_be(&self, off: usize) -> Option<u32> {
        self.arr(off).map(u32::from_be_bytes)
    }

    pub fn u64_be(&self, off: usize) -> Option<u64> {
        self.arr(off).map(u64::from_be_bytes)
    }
}

/// Decode one modified UTF-8 code unit. Invalid bytes consume only themselves,
/// so a damaged sequence cannot hide valid text or borrow missing bytes.
pub(crate) fn mutf8_unit(bytes: &[u8]) -> (u16, usize) {
    match bytes {
        [a @ 0x00..=0x7f, ..] => (u16::from(*a), 1),
        [0xc0, 0x80, ..] => (0, 2),
        [a @ 0xc2..=0xdf, b @ 0x80..=0xbf, ..] => ((u16::from(*a & 0x1f) << 6) | u16::from(*b & 0x3f), 2),
        [a @ 0xe0..=0xef, b @ 0x80..=0xbf, c @ 0x80..=0xbf, ..] if *a != 0xe0 || *b >= 0xa0 => {
            ((u16::from(*a & 0x0f) << 12) | (u16::from(*b & 0x3f) << 6) | u16::from(*c & 0x3f), 3)
        }
        _ => (0xfffd, 1),
    }
}
