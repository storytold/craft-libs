//! Tests for the ZIP preflight and the bounded entry read.
//!
//! Most cases build their bytes here rather than carrying archive fixtures, so the hostile shapes
//! (lying counts, offsets past the end, ZIP64 variants, overflow attempts) are visible in the test
//! and can be adjusted a byte at a time.

use crate::Budget;
use crate::zip::{self, Directory, Error, Limits, MAX_PREALLOCATION, ReadError};

/// Limits wide enough that only the case under test can trip them.
fn roomy() -> Limits {
    Limits { max_entries: 100_000, max_central_directory_bytes: 32 * 1024 * 1024, max_name_bytes: 4_096 }
}

const EOCD_SIGNATURE: &[u8; 4] = b"PK\x05\x06";
const ZIP64_EOCD_SIGNATURE: &[u8; 4] = b"PK\x06\x06";
const ZIP64_LOCATOR_SIGNATURE: &[u8; 4] = b"PK\x06\x07";
const CENTRAL_FILE_SIGNATURE: &[u8; 4] = b"PK\x01\x02";

/// One central-directory file header carrying `name`, with no extra field or comment.
fn central_entry(name: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(CENTRAL_FILE_SIGNATURE);
    out.extend_from_slice(&20u16.to_le_bytes()); // version made by
    out.extend_from_slice(&20u16.to_le_bytes()); // version needed
    out.extend_from_slice(&0u16.to_le_bytes()); // flags
    out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
    out.extend_from_slice(&0u16.to_le_bytes()); // time
    out.extend_from_slice(&0u16.to_le_bytes()); // date
    out.extend_from_slice(&0u32.to_le_bytes()); // crc32
    out.extend_from_slice(&0u32.to_le_bytes()); // compressed size
    out.extend_from_slice(&0u32.to_le_bytes()); // uncompressed size
    out.extend_from_slice(&u16::try_from(name.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // extra length
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    out.extend_from_slice(&0u16.to_le_bytes()); // disk
    out.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
    out.extend_from_slice(&0u32.to_le_bytes()); // external attributes
    out.extend_from_slice(&0u32.to_le_bytes()); // local header offset
    out.extend_from_slice(name);
    assert_eq!(out.len(), 46 + name.len());
    out
}

/// A minimal, well-formed archive holding `names`, with `comment` after its end record. The
/// returned offset is where that end record starts.
fn archive(names: &[&[u8]], comment: &[u8]) -> (Vec<u8>, usize) {
    assert!(comment.len() <= usize::from(u16::MAX));
    let mut out = Vec::new();
    for name in names {
        out.extend_from_slice(&central_entry(name));
    }
    let directory_size = out.len();
    out.extend_from_slice(EOCD_SIGNATURE);
    out.extend_from_slice(&0u16.to_le_bytes()); // disk
    out.extend_from_slice(&0u16.to_le_bytes()); // directory disk
    out.extend_from_slice(&u16::try_from(names.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&u16::try_from(names.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&u32::try_from(directory_size).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // directory offset: the directory starts at 0
    out.extend_from_slice(&u16::try_from(comment.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(comment);
    let eocd_at = out.len() - comment.len() - 22;
    (out, eocd_at)
}

/// An end-record-shaped run declaring `comment_len` bytes after itself. It is a candidate end
/// record only when it also ends exactly at the end of the file.
fn forged_end_record(comment_len: u16) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(EOCD_SIGNATURE);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&comment_len.to_le_bytes());
    assert_eq!(out.len(), 22);
    out
}

/// A single-entry archive whose central directory is preceded by `pad` bytes of file data, so the
/// directory does not start at zero.
fn archive_with_padding(pad: usize) -> Vec<u8> {
    let mut out = vec![0u8; pad];
    let directory_at = out.len();
    out.extend_from_slice(&central_entry(b"word/document.xml"));
    let directory_size = out.len() - directory_at;
    out.extend_from_slice(EOCD_SIGNATURE);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&u32::try_from(directory_size).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&u32::try_from(directory_at).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// A ZIP64 archive holding `names`: the entries, then the ZIP64 end record, its locator and the
/// classic end record with sentinel fields.
fn zip64_archive(names: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for name in names {
        out.extend_from_slice(&central_entry(name));
    }
    let directory_size = out.len();
    let zip64_at = out.len();
    out.extend_from_slice(ZIP64_EOCD_SIGNATURE);
    out.extend_from_slice(&44u64.to_le_bytes()); // the 44 fixed bytes that follow
    out.extend_from_slice(&45u16.to_le_bytes());
    out.extend_from_slice(&45u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // this disk
    out.extend_from_slice(&0u32.to_le_bytes()); // directory disk
    out.extend_from_slice(&u64::from(names.len() as u32).to_le_bytes());
    out.extend_from_slice(&u64::from(names.len() as u32).to_le_bytes());
    out.extend_from_slice(&u64::try_from(directory_size).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes()); // directory offset
    let locator_at = out.len();
    out.extend_from_slice(ZIP64_LOCATOR_SIGNATURE);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&u64::try_from(zip64_at).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    assert_eq!(out.len() - locator_at, 20);
    out.extend_from_slice(EOCD_SIGNATURE);
    out.extend_from_slice(&u16::MAX.to_le_bytes());
    out.extend_from_slice(&u16::MAX.to_le_bytes());
    out.extend_from_slice(&u16::MAX.to_le_bytes());
    out.extend_from_slice(&u16::MAX.to_le_bytes());
    out.extend_from_slice(&u32::MAX.to_le_bytes());
    out.extend_from_slice(&u32::MAX.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// Overwrite the bytes at `at` in `bytes`; every offset here comes from the fixture, not a file.
fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    let end = at + value.len();
    bytes.get_mut(at..end).unwrap().copy_from_slice(value);
}

/// Overwrite the little-endian `u16` at `at`.
fn put_u16(bytes: &mut [u8], at: usize, value: u16) {
    put(bytes, at, &value.to_le_bytes());
}

/// Overwrite the little-endian `u32` at `at`.
fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    put(bytes, at, &value.to_le_bytes());
}

/// An entry error's message. `ReadError` carries a `std::io::Error`, which is not comparable, so
/// tests compare what the caller would see.
fn message(error: Option<ReadError>) -> Option<String> {
    error.map(|error| error.to_string())
}

#[test]
fn a_plain_archive_is_counted_and_its_offsets_are_honoured() {
    let (bytes, _) = archive(&[b"a.txt", b"dir/b.xml"], b"");
    assert_eq!(zip::preflight(&bytes, roomy()).unwrap(), Directory { entries: 2, central_directory_bytes: 106 });

    // A directory that does not start at zero, because local file data precedes it.
    let padded = archive_with_padding(1_000);
    assert_eq!(zip::preflight(&padded, roomy()).unwrap(), Directory { entries: 1, central_directory_bytes: 46 + 17 });
}

#[test]
fn an_archive_comment_ending_exactly_at_eof_is_accepted() {
    let (bytes, _) = archive(&[b"a.txt"], b"a comment with PK\x05\x06 inside");
    assert_eq!(zip::preflight(&bytes, roomy()).unwrap(), Directory { entries: 1, central_directory_bytes: 51 });
}

#[test]
fn a_file_too_short_for_an_end_record_is_refused() {
    for length in 0..22 {
        assert_eq!(zip::preflight(&vec![0u8; length], roomy()), Err(Error::MissingEndRecord));
    }
    // A signature whose comment length runs past the end of the file is not an end record.
    let mut bytes = vec![0u8; 22];
    put(&mut bytes, 0, EOCD_SIGNATURE);
    put_u16(&mut bytes, 20, 7);
    assert_eq!(zip::preflight(&bytes, roomy()), Err(Error::MissingEndRecord));
}

#[test]
fn a_second_end_record_inside_the_comment_is_ambiguous() {
    // The real end record's comment contains another end record that also ends exactly at EOF.
    let trailing = b"trailing";
    let mut comment = forged_end_record(u16::try_from(trailing.len()).unwrap());
    comment.extend_from_slice(trailing);
    let (bytes, _) = archive(&[b"a.txt"], &comment);
    assert_eq!(bytes.len() - comment.len() + 22 + trailing.len(), bytes.len(), "the forged record must reach EOF");
    assert_eq!(zip::preflight(&bytes, roomy()), Err(Error::AmbiguousEndRecords));
}

#[test]
fn a_truncated_archive_is_refused() {
    let (bytes, _) = archive(&[b"a.txt", b"b.txt"], b"");
    for keep in [0, 1, 10, 21, 22, bytes.len() - 23, bytes.len() - 1] {
        let truncated = bytes.get(..keep).unwrap_or_default().to_vec();
        assert!(zip::preflight(&truncated, roomy()).is_err(), "a truncation to {keep} bytes was accepted");
    }
    // A directory size that no longer reaches the end records.
    let (mut short, eocd_at) = archive(&[b"a.txt"], b"");
    put_u32(&mut short, eocd_at + 12, 1);
    assert_eq!(zip::preflight(&short, roomy()), Err(Error::Invalid("central-directory offset and size do not reach the end records")));
}

#[test]
fn a_directory_offset_past_the_end_of_the_file_is_refused() {
    let (mut bytes, eocd_at) = archive(&[b"a.txt"], b"");
    put_u32(&mut bytes, eocd_at + 16, 1_000_000);
    assert_eq!(zip::preflight(&bytes, roomy()), Err(Error::Invalid("central-directory offset and size do not reach the end records")));

    // An offset that does not match the declared size.
    let (mut shifted, eocd_at) = archive(&[b"a.txt"], b"");
    put_u32(&mut shifted, eocd_at + 16, 1);
    assert_eq!(zip::preflight(&shifted, roomy()), Err(Error::Invalid("central-directory offset and size do not reach the end records")));

    // Offset and size whose sum is far past the end of the file. The limits are lifted so this
    // exercises the offset arithmetic rather than the size limit catching it first. The classic
    // maximum on either field means "look at the ZIP64 records" in the format, so the lying values
    // stop one short of it and the ZIP64 case below uses it.
    let unlimited = Limits { max_entries: u64::MAX, max_central_directory_bytes: u64::MAX, ..roomy() };
    let (mut far, eocd_at) = archive(&[b"a.txt"], b"");
    put_u32(&mut far, eocd_at + 12, u32::MAX - 1);
    put_u32(&mut far, eocd_at + 16, u32::MAX - 1);
    assert_eq!(zip::preflight(&far, unlimited), Err(Error::Invalid("central-directory offset and size do not reach the end records")));

    // The same on the ZIP64 path, where the sum cannot even be represented.
    let mut zip64 = zip64_archive(&[b"a.txt"]);
    let zip64_at = zip64.len() - 22 - 20 - 56;
    put(&mut zip64, zip64_at + 48, &u64::MAX.to_le_bytes());
    put(&mut zip64, zip64_at + 40, &u64::MAX.to_le_bytes());
    assert_eq!(zip::preflight(&zip64, unlimited), Err(Error::Invalid("central-directory offset overflow")));
}

#[test]
fn the_entry_count_limit_is_enforced() {
    let (bytes, _) = archive(&[b"a.txt", b"b.txt", b"c.txt"], b"");
    assert_eq!(zip::preflight(&bytes, Limits { max_entries: 3, ..roomy() }).unwrap().entries, 3);
    assert_eq!(zip::preflight(&bytes, Limits { max_entries: 2, ..roomy() }), Err(Error::TooManyEntries { actual: 3, limit: 2 }));
    assert_eq!(zip::preflight(&bytes, Limits { max_entries: 0, ..roomy() }), Err(Error::TooManyEntries { actual: 3, limit: 0 }));
}

#[test]
fn the_directory_size_limit_is_enforced() {
    let (bytes, _) = archive(&[b"a.txt"], b"");
    let size = 51u64;
    assert_eq!(zip::preflight(&bytes, Limits { max_central_directory_bytes: size, ..roomy() }).unwrap().central_directory_bytes, size);
    assert_eq!(
        zip::preflight(&bytes, Limits { max_central_directory_bytes: size - 1, ..roomy() }),
        Err(Error::CentralDirectoryTooLarge { actual: size, limit: size - 1 })
    );
}

#[test]
fn the_entry_name_limit_is_enforced() {
    let long = vec![b'n'; 200];
    let (bytes, _) = archive(&[&long], b"");
    assert_eq!(zip::preflight(&bytes, Limits { max_name_bytes: 200, ..roomy() }).unwrap().entries, 1);
    assert_eq!(zip::preflight(&bytes, Limits { max_name_bytes: 199, ..roomy() }), Err(Error::NameTooLong { actual: 200, limit: 199 }));

    // An empty name is malformed whatever the limit says.
    let (empty, _) = archive(&[b""], b"");
    assert_eq!(zip::preflight(&empty, roomy()), Err(Error::NameTooLong { actual: 0, limit: 4_096 }));

    // A name length that would run past the declared directory.
    let (mut lying, _) = archive(&[b"a.txt"], b"");
    put_u16(&mut lying, 28, u16::MAX);
    assert!(zip::preflight(&lying, roomy()).is_err());
}

#[test]
fn end_record_counts_that_disagree_with_the_directory_are_caught() {
    // Far more entries than the directory holds: the signatures run out inside the directory.
    let (mut too_many, eocd_at) = archive(&[b"a.txt"], b"");
    put_u16(&mut too_many, eocd_at + 8, 1_000);
    put_u16(&mut too_many, eocd_at + 10, 1_000);
    assert!(matches!(zip::preflight(&too_many, roomy()), Err(Error::Invalid(_))));

    // Fewer entries than the directory holds: the entries no longer fill the declared size.
    let (mut too_few, eocd_at) = archive(&[b"a.txt", b"b.txt"], b"");
    put_u16(&mut too_few, eocd_at + 8, 1);
    put_u16(&mut too_few, eocd_at + 10, 1);
    assert_eq!(zip::preflight(&too_few, roomy()), Err(Error::Invalid("the central-directory entry count and size disagree")));

    // A huge declared count is refused by the walk once the limit is lifted, and by the limit
    // before the walk when the limit is the tighter bound.
    let (mut huge, eocd_at) = archive(&[b"a.txt"], b"");
    put_u16(&mut huge, eocd_at + 8, u16::MAX - 1);
    put_u16(&mut huge, eocd_at + 10, u16::MAX - 1);
    assert!(matches!(zip::preflight(&huge, roomy()), Err(Error::Invalid(_))));
    assert_eq!(zip::preflight(&huge, Limits { max_entries: 10, ..roomy() }), Err(Error::TooManyEntries { actual: 65_534, limit: 10 }));
}

#[test]
fn entries_or_end_records_that_name_another_disk_are_refused() {
    let (mut entry, _) = archive(&[b"a.txt"], b"");
    put_u16(&mut entry, 34, 1);
    assert_eq!(zip::preflight(&entry, roomy()), Err(Error::Invalid("a central-directory entry refers to another disk")));

    let (mut directory_disk, eocd_at) = archive(&[b"a.txt"], b"");
    put_u16(&mut directory_disk, eocd_at + 6, 1);
    assert_eq!(zip::preflight(&directory_disk, roomy()), Err(Error::Invalid("multi-disk ZIP archive")));

    let (mut disk, eocd_at) = archive(&[b"a.txt"], b"");
    put_u16(&mut disk, eocd_at + 4, 1);
    assert_eq!(zip::preflight(&disk, roomy()), Err(Error::Invalid("multi-disk ZIP archive")));

    let (mut disk_entries, eocd_at) = archive(&[b"a.txt"], b"");
    put_u16(&mut disk_entries, eocd_at + 8, 2);
    assert_eq!(zip::preflight(&disk_entries, roomy()), Err(Error::Invalid("multi-disk ZIP archive")));
}

#[test]
fn one_trailing_digital_signature_is_allowed_and_other_trailing_data_is_not() {
    // A one-entry archive whose directory is preceded by 1,000 bytes, so offsets are not zero and
    // the directory size can be adjusted without moving the end record.
    let mut bytes = vec![0u8; 1_000];
    bytes.extend_from_slice(&central_entry(b"a.txt"));
    let directory_size = bytes.len() - 1_000;
    bytes.extend_from_slice(EOCD_SIGNATURE);
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(directory_size).unwrap_or(0).to_le_bytes());
    bytes.extend_from_slice(&1_000u32.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    let eocd_at = bytes.len() - 22;
    assert_eq!(zip::preflight(&bytes, roomy()).unwrap(), Directory { entries: 1, central_directory_bytes: 51 });

    // A central-directory digital signature record appended to the directory is legal.
    let mut signed = bytes.get(..eocd_at).unwrap_or_default().to_vec();
    signed.extend_from_slice(b"PK\x05\x05");
    signed.extend_from_slice(&3u16.to_le_bytes());
    signed.extend_from_slice(b"xyz");
    signed.extend_from_slice(bytes.get(eocd_at..).unwrap_or_default());
    let signed_eocd_at = signed.len() - 22;
    put_u32(&mut signed, signed_eocd_at + 12, directory_size as u32 + 6 + 3);
    assert_eq!(zip::preflight(&signed, roomy()).unwrap(), Directory { entries: 1, central_directory_bytes: 60 });

    // Any other trailing data makes the entry count and the declared size disagree.
    let mut junk = bytes.get(..eocd_at).unwrap_or_default().to_vec();
    junk.extend_from_slice(b"junk");
    junk.extend_from_slice(bytes.get(eocd_at..).unwrap_or_default());
    let junk_eocd_at = junk.len() - 22;
    put_u32(&mut junk, junk_eocd_at + 12, directory_size as u32 + 4);
    assert_eq!(zip::preflight(&junk, roomy()), Err(Error::Invalid("the central-directory entry count and size disagree")));
}

#[test]
fn zip64_end_records_are_walked_and_their_totals_are_limited() {
    let bytes = zip64_archive(&[b"a.txt", b"b.txt"]);
    assert_eq!(zip::preflight(&bytes, roomy()).unwrap(), Directory { entries: 2, central_directory_bytes: 2 * 46 + 4 + 6 });

    assert_eq!(zip::preflight(&bytes, Limits { max_entries: 1, ..roomy() }), Err(Error::TooManyEntries { actual: 2, limit: 1 }));
}

#[test]
fn zip64_sentinels_without_a_zip64_end_record_are_refused() {
    let (mut bytes, eocd_at) = archive(&[b"a.txt"], b"");
    put_u16(&mut bytes, eocd_at + 10, u16::MAX);
    assert_eq!(zip::preflight(&bytes, roomy()), Err(Error::Invalid("ZIP64 sentinel without a ZIP64 end record")));
}

#[test]
fn zip64_records_are_checked_against_their_locator_and_each_other() {
    let good = zip64_archive(&[b"a.txt", b"b.txt"]);
    let locator_at = good.len() - 22 - 20;
    let zip64_at = locator_at - 56;

    // A locator pointing where there is no ZIP64 end record.
    let mut missing = good.clone();
    put(&mut missing, locator_at + 8, &7u64.to_le_bytes());
    assert_eq!(zip::preflight(&missing, roomy()), Err(Error::Invalid("the ZIP64 end record is missing at its declared offset")));

    // A ZIP64 record whose length does not put its end at the locator.
    let mut bad_size = good.clone();
    put(&mut bad_size, zip64_at + 4, &45u64.to_le_bytes());
    assert_eq!(zip::preflight(&bad_size, roomy()), Err(Error::Invalid("the ZIP64 end record does not end at its locator")));

    // A ZIP64 record too short to hold the fields it must.
    let mut too_short = good.clone();
    put(&mut too_short, zip64_at + 4, &43u64.to_le_bytes());
    assert_eq!(zip::preflight(&too_short, roomy()), Err(Error::Invalid("the ZIP64 end record is too short")));

    // A locator that describes more than one disk.
    let mut multi_disk = good.clone();
    put(&mut multi_disk, locator_at + 16, &2u32.to_le_bytes());
    assert_eq!(zip::preflight(&multi_disk, roomy()), Err(Error::Invalid("multi-disk ZIP64 archive")));

    // Classic fields that contradict the ZIP64 record.
    let mut lying = good.clone();
    let eocd_at = lying.len() - 22;
    put_u16(&mut lying, eocd_at + 10, 1);
    assert_eq!(zip::preflight(&lying, roomy()), Err(Error::Invalid("the classic and ZIP64 end records disagree")));

    // A ZIP64 offset that cannot be a platform offset.
    let mut huge = good.clone();
    put(&mut huge, locator_at + 8, &u64::MAX.to_le_bytes());
    assert!(matches!(zip::preflight(&huge, roomy()), Err(Error::Invalid(_))));
}

#[test]
fn corrupting_any_single_byte_of_an_archive_never_panics() {
    let (bytes, _) = archive(&[b"a.txt", b"b.xml"], b"comment");
    for at in 0..bytes.len() {
        for value in [0x00u8, 0x01, 0x41, 0xff] {
            let mut corrupted = bytes.clone();
            if let Some(byte) = corrupted.get_mut(at) {
                *byte = value;
            }
            let _ = zip::preflight(&corrupted, roomy());
        }
    }
}

#[test]
fn an_entry_exactly_at_its_declared_size_is_read_and_charged() {
    let mut total = Budget::new("document", 1_000);
    let data = vec![7u8; 100];
    let out = zip::read_entry(std::io::Cursor::new(data.as_slice()), 100, 1_000, &mut total).unwrap();
    assert_eq!(out.len(), 100);
    assert_eq!(total.used(), 100);
    assert_eq!(total.remaining(), 900);
}

#[test]
fn a_declared_size_over_the_per_entry_limit_is_refused_before_reading() {
    let mut total = Budget::new("document", 1_000_000);
    let data = vec![0u8; 10];
    assert_eq!(
        message(zip::read_entry(std::io::Cursor::new(data.as_slice()), 5_000, 4_096, &mut total).err()),
        Some("the entry declares 5000 bytes; the limit is 4096".to_string())
    );
    assert_eq!(total.used(), 0, "a refused entry must not be charged");
}

#[test]
fn a_lying_declared_size_is_caught_and_an_overstated_one_only_bounds_the_read() {
    let mut total = Budget::new("document", 1_000_000);
    let data = vec![7u8; 100];

    // A hundred bytes arrive from an entry that declared ten.
    assert_eq!(
        message(zip::read_entry(std::io::Cursor::new(data.as_slice()), 10, 1_000, &mut total).err()),
        Some("the entry produced more than the 10 bytes it is allowed".to_string())
    );
    assert_eq!(total.used(), 0, "an overrun must not be charged");

    // A hundred bytes arrive from an entry that declared a thousand: bounded, and only the real
    // bytes are charged.
    let out = zip::read_entry(std::io::Cursor::new(data.as_slice()), 1_000, 4_096, &mut total).unwrap();
    assert_eq!(out.len(), 100);
    assert_eq!(total.used(), 100);
}

#[test]
fn a_decompression_bomb_is_rejected_at_the_smallest_bound() {
    // The entry declares a megabyte and inflates past it: the per-entry bound stops the read.
    let mut total = Budget::new("document", 64 * 1024 * 1024);
    let bomb = Bomb { remaining: 128 * 1024 * 1024, chunk: 64 * 1024 };
    let error = zip::read_entry(bomb, 1_000_000, 4 * 1024 * 1024, &mut total).err();
    assert_eq!(message(error), Some("the entry produced more than the 1000000 bytes it is allowed".to_string()));
    assert_eq!(total.used(), 0, "a bomb must not be charged anything");

    // The same bomb under a per-entry limit smaller than the declaration: still refused.
    let mut total = Budget::new("document", 64 * 1024 * 1024);
    let bomb = Bomb { remaining: 128 * 1024 * 1024, chunk: 64 * 1024 };
    let error = zip::read_entry(bomb, 100 * 1024 * 1024, 4_096, &mut total).err();
    assert_eq!(message(error), Some("the entry declares 104857600 bytes; the limit is 4096".to_string()));
}

#[test]
fn a_bomb_is_rejected_at_the_total_budget_when_that_is_the_tighter_bound() {
    let mut total = Budget::new("document", 8 * 1024);
    let bomb = Bomb { remaining: 16 * 1024 * 1024, chunk: 1_024 };
    let error = zip::read_entry(bomb, 1024 * 1024, 1024 * 1024, &mut total).err();
    assert!(matches!(error, Some(ReadError::Budget(_))), "{error:?}");
    assert_eq!(total.used(), 0);
}
#[test]
fn the_total_budget_bounds_a_series_of_entries() {
    let mut total = Budget::new("document", 250);
    let first = vec![1u8; 100];
    let second = vec![2u8; 100];
    let third = vec![3u8; 100];
    assert_eq!(zip::read_entry(std::io::Cursor::new(first.as_slice()), 100, 1_000, &mut total).unwrap().len(), 100);
    assert_eq!(zip::read_entry(std::io::Cursor::new(second.as_slice()), 100, 1_000, &mut total).unwrap().len(), 100);
    assert_eq!(total.remaining(), 50);
    // Only 50 bytes fit and the third entry wants 100.
    assert!(matches!(zip::read_entry(std::io::Cursor::new(third.as_slice()), 100, 1_000, &mut total).err(), Some(ReadError::Budget(_))));
    assert_eq!(total.used(), 200);
}

#[test]
fn a_declared_size_does_not_preallocate_more_than_the_fixed_cap() {
    let mut total = Budget::new("document", u64::from(u32::MAX));
    let data = vec![0u8; 8];
    let out = zip::read_entry(std::io::Cursor::new(data.as_slice()), u64::from(u32::MAX), u64::from(u32::MAX), &mut total).unwrap();
    assert_eq!(out.len(), 8);
    // The buffer must not have reserved the declared size. Capacity may exceed the length, so this
    // checks the reservation really was capped rather than a rounded-up length.
    assert!(out.capacity() < 1024 * 1024, "a lying declaration reserved {} bytes", out.capacity());
    assert!(u64::try_from(out.capacity()).unwrap_or(u64::MAX) <= MAX_PREALLOCATION.saturating_add(64));
}

#[test]
fn an_exhausted_budget_admits_nothing_and_a_zero_length_entry_still_reads() {
    let mut total = Budget::new("document", 0);
    assert!(matches!(zip::read_entry(std::io::Cursor::new([1u8, 2, 3].as_slice()), 3, 100, &mut total).err(), Some(ReadError::Budget(_))));
    let mut empty = Budget::new("document", 0);
    assert_eq!(zip::read_entry(std::io::Cursor::new([].as_slice()), 0, 100, &mut empty).ok(), Some(Vec::new()));
}

#[test]
fn a_reader_that_fails_is_reported_and_nothing_is_charged() {
    let mut total = Budget::new("document", 1_000);
    let error = zip::read_entry(Failing, 100, 1_000, &mut total).err();
    assert!(matches!(error, Some(ReadError::Io(_))), "{error:?}");
    assert_eq!(total.used(), 0);
}

#[test]
fn read_errors_describe_themselves() {
    assert_eq!(ReadError::DeclaredTooLarge { declared: 9, limit: 1 }.to_string(), "the entry declares 9 bytes; the limit is 1");
    assert_eq!(ReadError::Overrun { limit: 4 }.to_string(), "the entry produced more than the 4 bytes it is allowed");
    assert_eq!(
        ReadError::Budget(crate::Exceeded { what: "document", limit: 8, requested: 9 }).to_string(),
        "the document limit is 8; another 9 was requested"
    );
    assert!(std::error::Error::source(&ReadError::Budget(crate::Exceeded { what: "x", limit: 0, requested: 1 })).is_some());
    assert!(std::error::Error::source(&ReadError::Io(std::io::Error::other("closed"))).is_some());
    assert!(std::error::Error::source(&ReadError::Overrun { limit: 1 }).is_none());
}

#[test]
fn preflight_errors_describe_themselves() {
    let cases = [
        (Error::MissingEndRecord, "the ZIP end record is missing"),
        (Error::AmbiguousEndRecords, "the ZIP has more than one plausible end record"),
        (Error::Invalid("why"), "invalid ZIP structure: why"),
        (Error::TooManyEntries { actual: 5, limit: 2 }, "the ZIP declares 5 entries; the limit is 2"),
        (Error::CentralDirectoryTooLarge { actual: 9, limit: 8 }, "the ZIP central directory is 9 bytes; the limit is 8"),
        (Error::NameTooLong { actual: 3, limit: 2 }, "a ZIP entry name is 3 bytes; the limit is 2"),
    ];
    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

/// A reader that produces far more bytes than anyone asked for, standing in for a zip bomb.
struct Bomb {
    remaining: usize,
    chunk: usize,
}

impl std::io::Read for Bomb {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let want = buf.len().min(self.chunk).min(self.remaining);
        for byte in buf.get_mut(..want).unwrap_or_default() {
            *byte = 0x41;
        }
        self.remaining = self.remaining.saturating_sub(want);
        Ok(want)
    }
}

/// A reader that always fails.
struct Failing;

impl std::io::Read for Failing {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("the entry is corrupt"))
    }
}
