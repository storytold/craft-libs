//! Bounded encoded ICC layout normalization, originating in LightCraft.

use std::collections::BTreeMap;

/// Zero the creation date, align tag payloads to four bytes, preserve shared payloads,
/// update the profile length and clear its stale profile ID. No colour tags are interpreted.
///
/// This is a layout operation, not profile validation. Malformed tables or layouts exceeding
/// 4096 tags or 64 MiB output are returned unchanged except for the creation date. The owned
/// input is reused on failure, so rejected input does not require an additional full copy.
/// Consumers needing rejection of invalid profiles must separately use `Profile::parse`.
pub fn normalize_for_embedding(mut bytes: Vec<u8>) -> Vec<u8> {
    if let Some(date) = bytes.get_mut(24..36) {
        date.fill(0);
    }
    align(&bytes).unwrap_or(bytes)
}

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_TAGS: usize = 4096;

fn be(bytes: &[u8], at: usize) -> Option<usize> {
    let s = bytes.get(at..at.checked_add(4)?)?;
    usize::try_from(u32::from_be_bytes([s[0], s[1], s[2], s[3]])).ok()
}

fn align(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    let count = be(bytes, 128)?;
    if count > MAX_TAGS {
        return None;
    }
    let end = 132usize.checked_add(count.checked_mul(12)?)?;
    bytes.get(..end)?;
    let mut placements = BTreeMap::new();
    let mut offsets = Vec::new();
    offsets.try_reserve_exact(count).ok()?;
    let mut size = end;
    for index in 0..count {
        let entry = 132 + index * 12;
        let off = be(bytes, entry + 4)?;
        let len = be(bytes, entry + 8)?;
        if off < end {
            return None;
        }
        bytes.get(off..off.checked_add(len)?)?;
        let new_off = if let Some(&placed) = placements.get(&(off, len)) {
            placed
        } else {
            let aligned = size.checked_add(3)? & !3;
            size = aligned.checked_add(len)?;
            if size > MAX_BYTES {
                return None;
            }
            placements.insert((off, len), aligned);
            aligned
        };
        offsets.push(new_off);
    }
    size = size.checked_add(3)? & !3;
    if size > MAX_BYTES {
        return None;
    }
    let mut out = Vec::new();
    out.try_reserve_exact(size).ok()?;
    out.resize(size, 0);
    out[..end].copy_from_slice(&bytes[..end]);
    for ((off, len), target) in placements {
        out[target..target + len].copy_from_slice(&bytes[off..off + len]);
    }
    for (index, off) in offsets.into_iter().enumerate() {
        let field = 132 + index * 12 + 4;
        out[field..field + 4].copy_from_slice(&u32::try_from(off).ok()?.to_be_bytes());
    }
    out[..4].copy_from_slice(&u32::try_from(size).ok()?.to_be_bytes());
    out[84..100].fill(0);
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "normalize_legacy_tests.rs"]
mod legacy;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn matches_frozen_writer_and_preserves_aliases() {
        for count in 0..20usize {
            let end = 132 + count * 12;
            let mut bytes = vec![17; end + 31];
            bytes[128..132].copy_from_slice(&(count as u32).to_be_bytes());
            for index in 0..count {
                let field = 132 + index * 12 + 4;
                let off = end + index % 3;
                bytes[field..field + 4].copy_from_slice(&(off as u32).to_be_bytes());
                bytes[field + 4..field + 8].copy_from_slice(&7u32.to_be_bytes());
            }
            let expected = legacy::align_tags(&legacy::fixed_date(bytes.clone()));
            let result = normalize_for_embedding(bytes);
            assert_eq!(result, expected);
            assert_eq!(normalize_for_embedding(result.clone()), result);
        }
    }

    #[test]
    fn real_profile_preserves_color_tags_and_legacy_bytes() {
        let encoded = crate::Builtin::Srgb.profile().clone().with_encoded_bytes();
        let input = encoded.to_bytes().to_vec();
        let normalized = normalize_for_embedding(input.clone());
        assert_eq!(normalized, legacy::align_tags(&legacy::fixed_date(input)));
        assert_eq!(&normalized[24..36], &[0; 12]);
        assert_eq!(&normalized[84..100], &[0; 16]);
        assert!(crate::Profile::parse(&normalized).is_ok());
        for index in 0..be(&normalized, 128).unwrap() {
            assert_eq!(be(&normalized, 136 + index * 12).unwrap() % 4, 0);
        }
    }

    #[test]
    fn hostile_tables_return_owned_input_without_expansion() {
        for len in 0..132 {
            let bytes = vec![1; len];
            assert_eq!(normalize_for_embedding(bytes.clone()), legacy::fixed_date(bytes));
        }
        for count in [4097u32, u32::MAX] {
            let mut bytes = vec![1; 144];
            bytes[128..132].copy_from_slice(&count.to_be_bytes());
            assert_eq!(normalize_for_embedding(bytes.clone()), legacy::fixed_date(bytes));
        }
        let mut bytes = vec![1; 144];
        bytes[128..132].copy_from_slice(&1u32.to_be_bytes());
        bytes[136..140].copy_from_slice(&u32::MAX.to_be_bytes());
        bytes[140..144].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(normalize_for_embedding(bytes.clone()), legacy::fixed_date(bytes));
    }
}
