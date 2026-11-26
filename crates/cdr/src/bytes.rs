//! Little-endian reads that fail instead of panicking.

/// A cursor over one record's bytes. Every read returns `None` past the end.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rd<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Rd<'a> {
    pub(crate) fn new(b: &'a [u8]) -> Self {
        Rd { b, pos: 0 }
    }

    pub(crate) fn at(b: &'a [u8], pos: usize) -> Self {
        Rd { b, pos }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn left(&self) -> usize {
        self.b.len().saturating_sub(self.pos)
    }

    pub(crate) fn skip(&mut self, n: usize) -> Option<()> {
        let end = self.pos.checked_add(n)?;
        (end <= self.b.len()).then(|| self.pos = end)
    }

    pub(crate) fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.b.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.bytes(N)?.try_into().ok()
    }

    pub(crate) fn u8(&mut self) -> Option<u8> {
        Some(self.array::<1>()?[0])
    }

    pub(crate) fn u16(&mut self) -> Option<u16> {
        self.array().map(u16::from_le_bytes)
    }

    pub(crate) fn i16(&mut self) -> Option<i16> {
        self.array().map(i16::from_le_bytes)
    }

    pub(crate) fn u32(&mut self) -> Option<u32> {
        self.array().map(u32::from_le_bytes)
    }

    pub(crate) fn i32(&mut self) -> Option<i32> {
        self.array().map(i32::from_le_bytes)
    }

    /// A finite f64; NaN and the infinities read as `None`.
    pub(crate) fn f64(&mut self) -> Option<f64> {
        self.array().map(f64::from_le_bytes).filter(|v| v.is_finite())
    }

    pub(crate) fn fourcc(&mut self) -> Option<[u8; 4]> {
        self.array()
    }
}

/// The u32 at `pos` of `b`.
pub(crate) fn u32_at(b: &[u8], pos: usize) -> Option<u32> {
    Rd::at(b, pos).u32()
}
