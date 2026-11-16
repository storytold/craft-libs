//! Bounds-checked fixed-width reads in either byte order: out of range is `None`.

#[derive(Debug, Clone, Copy)]
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// `len` bytes starting at `at`.
    pub(crate) fn slice(&self, at: usize, len: usize) -> Option<&'a [u8]> {
        let end = at.checked_add(len)?;
        self.bytes.get(at..end)
    }

    fn word<const N: usize>(&self, at: usize) -> Option<[u8; N]> {
        let (word, _) = self.bytes.get(at..)?.split_first_chunk::<N>()?;
        Some(*word)
    }

    pub(crate) fn u32_le(&self, at: usize) -> Option<u32> {
        self.word(at).map(u32::from_le_bytes)
    }

    pub(crate) fn u64_le(&self, at: usize) -> Option<u64> {
        self.word(at).map(u64::from_le_bytes)
    }

    pub(crate) fn u32_be(&self, at: usize) -> Option<u32> {
        self.word(at).map(u32::from_be_bytes)
    }

    pub(crate) fn u64_be(&self, at: usize) -> Option<u64> {
        self.word(at).map(u64::from_be_bytes)
    }
}
