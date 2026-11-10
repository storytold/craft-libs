//! Width of file addresses ("offsets") and lengths, as declared by the superblock.

use super::error::Hdf5Error;
use crate::container::bytes::ByteReader;

/// A file address, relative to the superblock's base address.
pub(crate) type Address = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sizes {
    offset: u8,
    length: u8,
}

impl Sizes {
    pub(crate) fn new(offset: u8, length: u8) -> Result<Self, Hdf5Error> {
        let valid = |n: u8| matches!(n, 2 | 4 | 8);
        if !valid(offset) || !valid(length) {
            return Err(Hdf5Error::Unsupported(format!("size of offsets {offset} / size of lengths {length}")));
        }
        Ok(Sizes { offset, length })
    }

    pub(crate) fn offset_size(self) -> usize {
        usize::from(self.offset)
    }

    pub(crate) fn length_size(self) -> usize {
        usize::from(self.length)
    }

    /// An address field; `None` for the "undefined address" (all bits set).
    pub(crate) fn address(self, r: &mut ByteReader) -> Result<Option<Address>, Hdf5Error> {
        let value = uint(r, self.offset)?;
        let undefined = u64::MAX >> (64 - 8 * u32::from(self.offset));
        Ok((value != undefined).then_some(value))
    }

    /// An address field that must be defined.
    pub(crate) fn required_address(self, r: &mut ByteReader, what: &str) -> Result<Address, Hdf5Error> {
        self.address(r)?.ok_or_else(|| Hdf5Error::Corrupt(format!("{what}: undefined address")))
    }

    pub(crate) fn length(self, r: &mut ByteReader) -> Result<u64, Hdf5Error> {
        uint(r, self.length)
    }
}

/// Little-endian unsigned integer of `n` (2, 4 or 8) bytes.
fn uint(r: &mut ByteReader, n: u8) -> Result<u64, Hdf5Error> {
    Ok(match n {
        2 => u64::from(r.u16_le()?),
        4 => u64::from(r.u32_le()?),
        _ => r.u64_le()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_defined_and_undefined_addresses() {
        let sizes = Sizes::new(4, 8).unwrap();
        let data = [0x10, 0, 0, 0, 0xff, 0xff, 0xff, 0xff];
        let mut r = ByteReader::new(&data);
        assert_eq!(sizes.address(&mut r).unwrap(), Some(0x10));
        assert_eq!(sizes.address(&mut r).unwrap(), None);
    }

    #[test]
    fn rejects_odd_widths() {
        assert!(matches!(Sizes::new(3, 8), Err(Hdf5Error::Unsupported(_))));
    }
}
