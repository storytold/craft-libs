//! Borrowed little-endian AAF property-stream records, independent of CFB and object policy.

/// One indexed AAF property. The payload borrows the original stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Property<'a> {
    /// The property ID (pid).
    pub id: u16,
    /// The stored form of the value, as the index records it.
    pub form: u16,
    /// The value's bytes.
    pub payload: &'a [u8],
}

/// Malformed property-stream framing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyError {
    /// The stream ends before its 4-byte header.
    ShortHeader,
    /// The header's byte-order mark is not little-endian (0x4c).
    UnsupportedByteOrder,
    /// The stream ends inside the property index.
    TruncatedIndex,
    /// A property's length runs past the end of the stream.
    TruncatedValue,
}

/// Property-stream record iterator. It does not deduplicate IDs, interpret forms,
/// copy payloads or impose object budgets; those decisions belong to the caller.
#[derive(Clone, Debug)]
pub struct Properties<'a> {
    headers: std::slice::Iter<'a, [u8; 6]>,
    payload: &'a [u8],
}

impl<'a> Properties<'a> {
    /// Validate the header and complete index. Payload extents are checked during
    /// iteration; callers can validate all extents before copying by iterating a clone.
    pub fn new(stream: &'a [u8]) -> Result<Self, PropertyError> {
        let [order, _, lo, hi, tail @ ..] = stream else { return Err(PropertyError::ShortHeader) };
        if *order != 0x4c {
            return Err(PropertyError::UnsupportedByteOrder);
        }
        let count = usize::from(u16::from_le_bytes([*lo, *hi]));
        let (headers, payload) = tail.split_at_checked(count * 6).ok_or(PropertyError::TruncatedIndex)?;
        Ok(Self { headers: headers.as_chunks::<6>().0.iter(), payload })
    }
    pub(super) fn remaining_count(&self) -> usize {
        self.headers.len()
    }

    // The store checks duplicate IDs before validating the corresponding payload,
    // so a record that is both duplicate and truncated retains its old diagnostic.
    pub(super) fn next_id(&self) -> Option<u16> {
        let [lo, hi, ..] = self.headers.as_slice().first()?;
        Some(u16::from_le_bytes([*lo, *hi]))
    }
}

impl<'a> Iterator for Properties<'a> {
    type Item = Result<Property<'a>, PropertyError>;

    fn next(&mut self) -> Option<Self::Item> {
        let [id0, id1, form0, form1, len0, len1] = self.headers.next()?;
        let len = usize::from(u16::from_le_bytes([*len0, *len1]));
        let Some((payload, tail)) = self.payload.split_at_checked(len) else {
            self.headers = [].iter();
            return Some(Err(PropertyError::TruncatedValue));
        };
        self.payload = tail;
        Some(Ok(Property { id: u16::from_le_bytes([*id0, *id1]), form: u16::from_le_bytes([*form0, *form1]), payload }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_borrow_ordered_payloads_without_deduplicating() {
        let stream = [0x4c, 0x20, 2, 0, 0x55, 0, 0x82, 0, 2, 0, 0x55, 0, 0x22, 0, 1, 0, 9, 8, 7];
        let mut records = Properties::new(&stream).unwrap();
        assert_eq!(records.remaining_count(), 2);
        assert_eq!(records.next_id(), Some(0x55));
        let first = records.next().unwrap().unwrap();
        assert_eq!(records.next_id(), Some(0x55));
        let second = records.next().unwrap().unwrap();
        assert_eq!(first, Property { id: 0x55, form: 0x82, payload: &[9, 8] });
        assert_eq!(second, Property { id: 0x55, form: 0x22, payload: &[7] });
        assert!(std::ptr::eq(first.payload, &stream[16..18]));
        assert!(std::ptr::eq(second.payload, &stream[18..19]));
        assert!(records.next().is_none());
    }
}
