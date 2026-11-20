// Checked reader subset retained from Craft Anything; MIT OR Apache-2.0.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }
    pub fn u8(&self, off: usize) -> Option<u8> {
        self.data.get(off).copied()
    }
    pub fn slice(&self, off: usize, len: usize) -> Option<&'a [u8]> {
        self.data.get(off..off.checked_add(len)?)
    }
    /// Unsigned LEB128 at `off`: (value, bytes consumed). At most 10 bytes.
    pub fn uleb128(&self, off: usize) -> Option<(u64, usize)> {
        let mut result = 0u64;
        for i in 0..10usize {
            let b = self.u8(off.checked_add(i)?)?;
            let payload = u64::from(b & 0x7f);
            let shift = 7 * i as u32;
            if shift > 0 && payload.checked_shl(shift)? >> shift != payload {
                return None;
            }
            result |= payload << shift;
            if b & 0x80 == 0 {
                return Some((result, i + 1));
            }
        }
        None
    }
}
