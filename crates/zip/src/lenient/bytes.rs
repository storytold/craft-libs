//! Checked little-endian reads over untrusted bytes. Every read returns `None` when out of range.

#[derive(Debug, Clone, Copy)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Reader { data }
    }

    pub(crate) fn data(&self) -> &'a [u8] {
        self.data
    }

    pub(crate) fn slice(&self, off: usize, len: usize) -> Option<&'a [u8]> {
        let end = off.checked_add(len)?;
        self.data.get(off..end)
    }

    pub(crate) fn u8(&self, off: usize) -> Option<u8> {
        self.data.get(off).copied()
    }

    fn arr<const N: usize>(&self, off: usize) -> Option<[u8; N]> {
        self.slice(off, N)?.try_into().ok()
    }

    pub(crate) fn u16_le(&self, off: usize) -> Option<u16> {
        self.arr(off).map(u16::from_le_bytes)
    }

    pub(crate) fn u32_le(&self, off: usize) -> Option<u32> {
        self.arr(off).map(u32::from_le_bytes)
    }

    pub(crate) fn u64_le(&self, off: usize) -> Option<u64> {
        self.arr(off).map(u64::from_le_bytes)
    }
}
