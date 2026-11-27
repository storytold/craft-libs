//! Bounds-checked reads of fixed-width integers in both byte orders: out of range is `None`.

#[derive(Debug, Clone, Copy)]
pub(crate) struct Reader<'a> {
    buf: &'a [u8],
}

macro_rules! int {
    ($le:ident, $be:ident, $ty:ty) => {
        pub(crate) fn $le(&self, at: usize) -> Option<$ty> {
            self.take(at).map(<$ty>::from_le_bytes)
        }

        pub(crate) fn $be(&self, at: usize) -> Option<$ty> {
            self.take(at).map(<$ty>::from_be_bytes)
        }
    };
}

impl<'a> Reader<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Self { buf }
    }

    pub(crate) fn u8(&self, at: usize) -> Option<u8> {
        self.buf.get(at).copied()
    }

    fn take<const N: usize>(&self, at: usize) -> Option<[u8; N]> {
        let end = at.checked_add(N)?;
        self.buf.get(at..end)?.try_into().ok()
    }

    int!(u16_le, u16_be, u16);
    int!(u32_le, u32_be, u32);
    int!(u64_le, u64_be, u64);
}
