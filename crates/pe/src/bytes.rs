//! Bounds-checked little-endian field reads for the header parser: out of range is `None`.

#[derive(Debug, Clone, Copy)]
pub(crate) struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self(data)
    }

    /// `len` bytes at `off`.
    pub(crate) fn slice(&self, off: usize, len: usize) -> Option<&'a [u8]> {
        self.0.get(off..off.checked_add(len)?)
    }

    fn field<const N: usize>(&self, off: usize) -> Option<[u8; N]> {
        self.0.get(off..)?.first_chunk::<N>().copied()
    }

    pub(crate) fn u8(&self, off: usize) -> Option<u8> {
        self.field::<1>(off).map(|[b]| b)
    }

    pub(crate) fn u16_le(&self, off: usize) -> Option<u16> {
        Some(u16::from_le_bytes(self.field(off)?))
    }

    pub(crate) fn u32_le(&self, off: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.field(off)?))
    }

    pub(crate) fn u64_le(&self, off: usize) -> Option<u64> {
        Some(u64::from_le_bytes(self.field(off)?))
    }
}
