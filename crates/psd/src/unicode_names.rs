//! Counted UTF-16 name records, as used by PSD Unicode Alpha Names (resource 1045).
//!
//! Adapted from PhotoCraft's channel-name reader. Framing is shared; decoding malformed
//! UTF-16, removing terminators and assigning names to document channels remain caller policy.

/// Iterate complete records as borrowed big-endian UTF-16 byte slices.
///
/// A record starts with a big-endian `u32` code-unit count. Empty records are retained.
/// An incomplete count, truncated record or arithmetic overflow ends the list, preserving
/// preceding complete records. No allocation is driven by the declared count, and bytes
/// after a malformed record are not interpreted as new records.
pub fn records(mut data: &[u8]) -> impl Iterator<Item = &[u8]> {
    std::iter::from_fn(move || {
        let count = data.get(..4)?;
        let count = u32::from_be_bytes([count[0], count[1], count[2], count[3]]);
        let input = data;
        // On any malformed length the iterator stays exhausted on subsequent calls.
        data = &[];
        let len = usize::try_from(count).ok()?.checked_mul(2)?;
        let end = 4usize.checked_add(len)?;
        let record = input.get(4..end)?;
        data = input.get(end..)?;
        Some(record)
    })
}

/// Encode counted NUL-terminated big-endian UTF-16 name records (resource 1045).
pub fn write<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<u8> {
    let mut out = Vec::new();
    for name in names {
        let count = name.encode_utf16().count().saturating_add(1);
        let count_u32 = u32::try_from(count).unwrap_or(u32::MAX);
        out.extend_from_slice(&count_u32.to_be_bytes());
        for unit in name.encode_utf16().take(count_u32 as usize - 1).chain(std::iter::once(0)) {
            out.extend_from_slice(&unit.to_be_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrows_empty_and_uninterpreted_code_units() {
        let data = [0, 0, 0, 0, 0, 0, 0, 3, 0xd8, 0, 0, 65, 0, 0];
        let found: Vec<_> = records(&data).collect();
        assert_eq!(found, [&data[4..4], &data[8..14]]);
        assert_eq!(found[1].as_ptr(), data[8..].as_ptr());
    }

    #[test]
    fn partial_tail_and_hostile_count_stop_without_losing_prefix() {
        let first = [0, 0, 0, 1, 0, 65];
        for tail in [&[0][..], &[0, 0, 0], &[0, 0, 0, 2, 0, 66], &[255, 255, 255, 255, 0, 66]] {
            let mut bytes = first.to_vec();
            bytes.extend(tail);
            let mut it = records(&bytes);
            assert_eq!(it.next(), Some(&first[4..]));
            assert_eq!(it.next(), None);
            assert_eq!(it.next(), None);
        }
        for end in 0..first.len() {
            assert_eq!(records(&first[..end]).count(), 0);
        }
    }

    #[test]
    fn write_roundtrips_with_nul_terminator() {
        let bytes = write(["Alpha 1", "café"]);
        let decoded: Vec<_> = records(&bytes).collect();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0], [0, b'A', 0, b'l', 0, b'p', 0, b'h', 0, b'a', 0, b' ', 0, b'1', 0, 0]);
        assert_eq!(decoded[1], [0, b'c', 0, b'a', 0, b'f', 0, 0xe9, 0, 0]);
    }
}
