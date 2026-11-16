#![cfg(feature = "lenient")]
// From Craft Anything crates/formats/tests/zip_split_volumes.rs.
use craft_zip::lenient::{self as zip, ZipError};
#[path = "lenient_common/mod.rs"]
mod common;
use common::zip_stored;

fn assert_split(data: &[u8]) {
    assert!(matches!(zip::parse(data, 10), Err(ZipError::Unsupported(_))));
}

#[test]
fn classic_end_records_must_describe_one_complete_volume() {
    for field in [4, 6, 8] {
        let mut data = zip_stored(&[("a", b"x")]);
        let end = data.len() - 22;
        let value = if field == 8 { 0u16 } else { 1 };
        data[end + field..end + field + 2].copy_from_slice(&value.to_le_bytes());
        assert_split(&data);
    }
}

#[test]
fn central_entries_cannot_point_to_another_volume() {
    let mut data = zip_stored(&[("a", b"x")]);
    let central = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    data[central + 34..central + 36].copy_from_slice(&1u16.to_le_bytes());
    assert_split(&data);
}

fn zip64_entry_disk(disk: u32, with_sizes_and_offset: bool) -> Vec<u8> {
    let mut data = zip_stored(&[("a", b"x")]);
    let central = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    let end = data.len() - 22;
    data[central + 34..central + 36].copy_from_slice(&u16::MAX.to_le_bytes());
    let mut extra = vec![1, 0, 4, 0];
    if with_sizes_and_offset {
        data[central + 20..central + 28].fill(0xff);
        data[central + 42..central + 46].fill(0xff);
        extra[2..4].copy_from_slice(&28u16.to_le_bytes());
        // Uncompressed size, compressed size, and local header offset precede the disk.
        for value in [1u64, 1, 0] {
            extra.extend(value.to_le_bytes());
        }
    }
    extra.extend(disk.to_le_bytes());
    data[central + 30..central + 32].copy_from_slice(&(extra.len() as u16).to_le_bytes());
    let directory_size = (end - central + extra.len()) as u32;
    data.splice(end..end, extra);
    let end = data.len() - 22;
    data[end + 12..end + 16].copy_from_slice(&directory_size.to_le_bytes());
    data
}

#[test]
fn a_zip64_disk_sentinel_is_resolved_before_rejecting_the_entry() {
    let data = zip64_entry_disk(0, false);
    let archive = zip::parse(&data, 10).unwrap();
    assert_eq!(zip::read_entry(&data, &archive.entries[0], 10).unwrap(), b"x");
    assert_split(&zip64_entry_disk(1, false));
}

#[test]
fn a_zip64_disk_sentinel_follows_sizes_and_the_local_offset() {
    let data = zip64_entry_disk(0, true);
    let archive = zip::parse(&data, 10).unwrap();
    assert_eq!(archive.entries[0].uncompressed_size, 1);
    assert_eq!(archive.entries[0].compressed_size, 1);
    assert_eq!(archive.entries[0].local_header_offset, 0);
    assert_eq!(zip::read_entry(&data, &archive.entries[0], 10).unwrap(), b"x");
    assert_split(&zip64_entry_disk(1, true));
}

fn zip64_archive(record_disk: u32, central_disk: u32, locator_disk: u32, disks: u32, entries_on_disk: u64) -> Vec<u8> {
    let mut data = zip_stored(&[("a", b"x")]);
    let end = data.len() - 22;
    let central = u32::from_le_bytes(data[end + 16..end + 20].try_into().unwrap());
    let size = u32::from_le_bytes(data[end + 12..end + 16].try_into().unwrap());
    let mut record = b"PK\x06\x06".to_vec();
    record.extend(44u64.to_le_bytes());
    record.extend([45, 0, 45, 0]);
    record.extend(record_disk.to_le_bytes());
    record.extend(central_disk.to_le_bytes());
    record.extend(entries_on_disk.to_le_bytes());
    record.extend(1u64.to_le_bytes());
    record.extend(u64::from(size).to_le_bytes());
    record.extend(u64::from(central).to_le_bytes());
    record.extend(b"PK\x06\x07");
    record.extend(locator_disk.to_le_bytes());
    record.extend((end as u64).to_le_bytes());
    record.extend(disks.to_le_bytes());
    data[end + 8..end + 12].fill(0xff);
    data.splice(end..end, record);
    data
}

#[test]
fn zip64_end_records_and_locators_must_also_describe_one_volume() {
    for fields in [(1, 0, 0, 1, 1), (0, 1, 0, 1, 1), (0, 0, 1, 1, 1), (0, 0, 0, 2, 1), (0, 0, 0, 1, 0)] {
        assert_split(&zip64_archive(fields.0, fields.1, fields.2, fields.3, fields.4));
    }
    assert_eq!(zip::parse(&zip64_archive(0, 0, 0, 1, 1), 10).unwrap().entries.len(), 1);
}

#[test]
fn zip64_does_not_erase_explicit_classic_volume_markers() {
    for field in [4, 6] {
        let mut data = zip64_archive(0, 0, 0, 1, 1);
        let end = data.len() - 22;
        data[end + field..end + field + 2].copy_from_slice(&1u16.to_le_bytes());
        assert_split(&data);
        data[end + field..end + field + 2].fill(0xff);
        assert_eq!(zip::parse(&data, 10).unwrap().entries.len(), 1);
    }
}

#[test]
fn zip64_does_not_erase_explicit_classic_entry_counts() {
    for field in [8, 10] {
        for count in [0u16, 2] {
            let mut data = zip64_archive(0, 0, 0, 1, 1);
            let end = data.len() - 22;
            data[end + field..end + field + 2].copy_from_slice(&count.to_le_bytes());
            assert!(matches!(zip::parse(&data, 10), Err(ZipError::Unsupported(_))), "field {field}, count {count}");
        }
    }
}

#[test]
fn mixed_classic_and_zip64_entry_counts_can_describe_one_volume() {
    for field in [8, 10] {
        let mut data = zip64_archive(0, 0, 0, 1, 1);
        let end = data.len() - 22;
        data[end + field..end + field + 2].copy_from_slice(&1u16.to_le_bytes());
        let archive = zip::parse(&data, 10).unwrap();
        assert_eq!(archive.entries.len(), 1, "field {field}");
        assert_eq!(zip::read_entry(&data, &archive.entries[0], 10).unwrap(), b"x");
    }
}

#[test]
fn zip64_cannot_replace_explicit_classic_directory_bounds() {
    for field in [12, 16] {
        let mut data = zip64_archive(0, 0, 0, 1, 1);
        let end = data.len() - 22;
        let value = u32::from_le_bytes(data[end + field..end + field + 4].try_into().unwrap());
        data[end + field..end + field + 4].copy_from_slice(&(value + 1).to_le_bytes());
        assert!(matches!(zip::parse(&data, 10), Err(ZipError::Corrupt(_))), "field {field}");
    }
}

#[test]
fn each_zip64_field_can_resolve_its_own_classic_sentinel() {
    for (field, width) in [(4, 2), (6, 2), (8, 2), (10, 2), (12, 4), (16, 4)] {
        let mut data = zip64_archive(0, 0, 0, 1, 1);
        let end = data.len() - 22;
        data[end + 8..end + 10].copy_from_slice(&1u16.to_le_bytes());
        data[end + 10..end + 12].copy_from_slice(&1u16.to_le_bytes());
        data[end + field..end + field + width].fill(0xff);
        let archive = zip::parse(&data, 10).unwrap();
        assert!(archive.zip64, "field {field}");
        assert_eq!(archive.declared_entries, 1, "field {field}");
        assert_eq!(archive.entries.len(), 1, "field {field}");
        assert_eq!(archive.prefix_bytes, 0, "field {field}");
        assert_eq!(zip::read_entry(&data, &archive.entries[0], 10).unwrap(), b"x", "field {field}");
    }
}
