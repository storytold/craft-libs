#![cfg(feature = "lenient")]
// From Craft Anything crates/formats/tests/zip64_extra_boundaries.rs.

#[path = "lenient_common/mod.rs"]
mod common;
use craft_zip::lenient as zip;

fn archive(extra: &[u8], sentinel: usize) -> Vec<u8> {
    let mut data = common::zip_stored(&[("a", b"x")]);
    let central = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    let end = data.len() - 22;
    data[central + sentinel..central + sentinel + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    data[central + 30..central + 32].copy_from_slice(&(extra.len() as u16).to_le_bytes());
    data.splice(end..end, extra.iter().copied());
    let end = data.len() - 22;
    data[end + 12..end + 16].copy_from_slice(&((47 + extra.len()) as u32).to_le_bytes());
    data
}

#[test]
fn a_short_zip64_field_cannot_borrow_values_from_its_neighbor() {
    for field in [20, 24, 42] {
        let mut extra = vec![1, 0, 0, 0, 2, 0, 8, 0];
        extra.extend_from_slice(&1u64.to_le_bytes());
        assert!(zip::parse(&archive(&extra, field), 10).is_err(), "field {field}");
    }
}

#[test]
fn zip64_sentinels_require_complete_values() {
    for field in [20, 24, 42] {
        assert!(zip::parse(&archive(&[], field), 10).is_err());
        assert!(zip::parse(&archive(&[1, 0, 7, 0, 1, 0, 0, 0, 0, 0, 0], field), 10).is_err());
    }
}

#[test]
fn a_complete_zip64_field_preserves_the_value() {
    for field in [20, 24, 42] {
        let value = if field == 42 { 0u64 } else { 1u64 };
        let mut extra = vec![1, 0, 8, 0];
        extra.extend_from_slice(&value.to_le_bytes());
        let data = archive(&extra, field);
        let parsed = zip::parse(&data, 10).unwrap();
        assert_eq!(zip::read_entry(&data, &parsed.entries[0], 1).unwrap(), b"x");
    }
}

#[test]
fn a_legitimate_64_bit_value_equal_to_the_32_bit_sentinel_is_preserved() {
    let mut extra = vec![1, 0, 8, 0];
    extra.extend_from_slice(&u64::from(u32::MAX).to_le_bytes());
    let archive = zip::parse(&archive(&extra, 24), 10).unwrap();
    assert_eq!(archive.entries[0].uncompressed_size, u64::from(u32::MAX));
}

#[test]
fn trailing_bytes_too_short_for_a_field_header_are_padding() {
    for stray in 1..=3usize {
        let mut extra = vec![0xaa, 0xbb, 0, 0];
        extra.extend(std::iter::repeat_n(0u8, stray));
        let data = archive(&extra, 36);
        let parsed = zip::parse(&data, 10).unwrap();
        assert_eq!(zip::read_entry(&data, &parsed.entries[0], 1).unwrap(), b"x", "stray {stray}");
    }
}
