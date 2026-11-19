#![cfg(feature = "lenient")]
// From Craft Anything crates/formats/tests/zip_directory_bounds.rs.
use super::common;
use common::zip_stored;
use craft_zip::lenient::{self as zip, ZipError};

#[test]
fn a_short_declared_entry_is_rejected_before_its_signature_is_checked() {
    for remaining in 0..46 {
        let mut data = zip_stored(&[("a", b"x")]);
        let end = data.len() - 22;
        let size = u32::from_le_bytes(data[end + 12..end + 16].try_into().unwrap());
        data[end + 8..end + 12].copy_from_slice(&[2, 0, 2, 0]);
        data[end + 12..end + 16].copy_from_slice(&(size + remaining).to_le_bytes());
        data.splice(end..end, vec![0; remaining as usize]);
        assert!(matches!(zip::parse(&data, 10), Err(ZipError::Truncated("directory entry"))), "remaining {remaining} bytes");
    }
}

#[test]
fn directory_names_and_comments_cannot_borrow_end_record_bytes() {
    for field in [28, 32] {
        let mut data = zip_stored(&[("a", b"x")]);
        let central = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let length = if field == 28 { 2u16 } else { 1 };
        data[central + field..central + field + 2].copy_from_slice(&length.to_le_bytes());
        assert!(zip::parse(&data, 10).is_err(), "field {field}");
    }
}

#[test]
fn directory_extra_fields_cannot_borrow_end_record_bytes() {
    let mut data = zip_stored(&[("a", b"x")]);
    let central = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    data[central + 30..central + 32].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(zip::parse(&data, 10), Err(ZipError::Truncated("directory entry fields")));
}

#[test]
fn a_directory_cannot_extend_into_its_end_record() {
    let mut data = zip_stored(&[("a", b"x")]);
    let end = data.len() - 22;
    let size = u32::from_le_bytes(data[end + 12..end + 16].try_into().unwrap());
    data[end + 12..end + 16].copy_from_slice(&(size + 1).to_le_bytes());
    assert!(zip::parse(&data, 10).is_err());
}

#[test]
fn self_extracting_archives_keep_both_offset_conventions() {
    for adjusted in [false, true] {
        let mut data = zip_stored(&[("a", b"x")]);
        let central = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        let end = data.len() - 22;
        let stub = b"executable stub";
        if adjusted {
            data[central + 42..central + 46].copy_from_slice(&(stub.len() as u32).to_le_bytes());
            data[end + 16..end + 20].copy_from_slice(&((central + stub.len()) as u32).to_le_bytes());
        }
        let mut file = stub.to_vec();
        file.extend(data);
        let archive = zip::parse(&file, 10).unwrap();
        assert_eq!(zip::read_entry(&file, &archive.entries[0], 10).unwrap(), b"x");
    }
}

#[test]
fn a_directory_signature_after_the_entries_is_not_an_extra_file() {
    let mut data = zip_stored(&[("a", b"x")]);
    let end = data.len() - 22;
    let size = u32::from_le_bytes(data[end + 12..end + 16].try_into().unwrap());
    let signature = b"PK\x05\x05\x01\x00x";
    data[end + 12..end + 16].copy_from_slice(&(size + signature.len() as u32).to_le_bytes());
    data.splice(end..end, signature.iter().copied());
    assert_eq!(zip::parse(&data, 10).unwrap().entries.len(), 1);
}
