//! One entry read straight from a ZIP file without loading the file: only its end records, its
//! central directory and that entry's bytes are read. For archives too big to hold in memory just
//! to look at one small entry (a JAR's manifest).

use crate::lenient::bytes::Reader;
use crate::lenient::{self as zip, ZipError};
use std::io::{Read, Seek, SeekFrom};

/// Most central directory read (some hundreds of thousands of entries); a bigger one is refused.
const MAX_DIRECTORY: u64 = 16 << 20;
/// The ZIP64 end record as `zip::parse` reads it, without any extensible data after it.
const ZIP64_RECORD: u64 = 56;

/// The entry named `name` (not a folder), decompressed to at most `max_bytes` and checked against
/// its CRC-32. `None`: the archive has no such entry.
pub fn read_named_entry(file: &mut (impl Read + Seek), name: &str, max_bytes: u64) -> Result<Option<Vec<u8>>, ZipError> {
    let len = file.seek(SeekFrom::End(0)).map_err(io)?;
    let tail_start = len.saturating_sub(zip::EOCD_SEARCH as u64);
    let tail = read_at(file, tail_start, len - tail_start)?;
    let eocd = zip::find_eocd(&tail).ok_or(ZipError::NotZip)?;
    let mut end = tail.get(eocd..).ok_or(ZipError::Truncated("end record"))?.to_vec();
    let r = Reader::new(&end);
    let field = |at| r.u16_le(at).ok_or(ZipError::Truncated("end record"));
    let (size, classic_offset) = (r.u32_le(12).ok_or(ZipError::Truncated("end record"))?, r.u32_le(16).ok_or(ZipError::Truncated("end record"))?);
    // Any of these saturated makes `zip::parse` read the ZIP64 records, as below.
    let saturated = [field(4)?, field(6)?, field(8)?, field(10)?].contains(&0xffff) || size == 0xffff_ffff || classic_offset == 0xffff_ffff;
    let end_at = tail_start + eocd as u64;
    // A ZIP64 archive puts its own end record and a locator between the directory and the classic
    // end record, whether or not the classic fields are saturated.
    let locator_at = end_at.checked_sub(20);
    let locator = match locator_at {
        Some(at) => read_at(file, at, 20)?,
        None => Vec::new(),
    };
    // Its stated offset doesn't count a prepended stub; it then sits right before the locator. A
    // locator signature with no record at either place is other data (an entry's comment).
    let record_at = match (locator_at, locator.starts_with(b"PK\x06\x07")) {
        (Some(at), true) => {
            let declared = Reader::new(&locator).u64_le(8).ok_or(ZipError::Truncated("zip64 locator"))?;
            [declared, at.saturating_sub(ZIP64_RECORD)].into_iter().find(|&at| read_at(file, at, 4).is_ok_and(|sig| sig == b"PK\x06\x06"))
        }
        _ => None,
    };
    let zip64 = match record_at {
        Some(record_at) => {
            let record = read_at(file, record_at, ZIP64_RECORD)?;
            let rr = Reader::new(&record);
            let size = rr.u64_le(40).ok_or(ZipError::Truncated("zip64 record"))?;
            let offset = rr.u64_le(48).ok_or(ZipError::Truncated("zip64 record"))?;
            Some((record_at, size, offset, record))
        }
        None => None,
    };
    // Unsaturated classic fields must agree with a real ZIP64 record, as `zip::parse` requires;
    // one that disagrees is other data shaped like it.
    let zip64 = zip64.filter(|&(_, z_size, z_offset, _)| saturated || (z_size == u64::from(size) && z_offset == u64::from(classic_offset)));
    let (directory_end, size, offset, zip64) = match zip64 {
        Some((record_at, size, offset, record)) => (record_at, size, offset, saturated.then_some((record, locator))),
        None => (end_at, u64::from(size), u64::from(classic_offset), None),
    };
    if size > MAX_DIRECTORY {
        return Err(ZipError::TooLarge(MAX_DIRECTORY));
    }
    // The directory ends where the end records start; anything before its stated offset is a
    // prepended stub, as `zip::parse` allows.
    let start = directory_end.checked_sub(size).ok_or(ZipError::Corrupt("directory starts before the file".into()))?;
    let prefix = start.checked_sub(offset).ok_or(ZipError::Corrupt("directory sits before its stated offset".into()))?;
    // The directory and its end records alone, their offsets rewritten for that buffer. The
    // ZIP64 records go along only when the classic fields need them, as `zip::parse` reads them.
    let mut directory = read_at(file, start, size)?;
    if let Some((mut record, mut locator)) = zip64 {
        put(&mut record, 4, &(ZIP64_RECORD - 12).to_le_bytes())?;
        put(&mut record, 48, &0u64.to_le_bytes())?;
        put(&mut locator, 8, &size.to_le_bytes())?;
        directory.extend_from_slice(&record);
        directory.extend_from_slice(&locator);
    }
    // A saturated classic offset stays so: `zip::parse` then takes the rewritten ZIP64 one.
    if classic_offset != 0xffff_ffff {
        put(&mut end, 16, &[0; 4])?;
    }
    directory.extend_from_slice(&end);
    // `MAX_DIRECTORY` already bounds how many entries there can be.
    let archive = zip::parse(&directory, usize::MAX)?;
    let Some(mut entry) = archive.entries.into_iter().find(|e| e.name == name && !e.is_dir) else { return Ok(None) };
    if entry.compressed_size > max_bytes.saturating_add(1 << 16) {
        return Err(ZipError::TooLarge(max_bytes));
    }
    let at = entry.local_header_offset.checked_add(prefix).ok_or(ZipError::Corrupt("local header offset".into()))?;
    let header = read_at(file, at, 30)?;
    let fields = Reader::new(&header);
    let name_len = u64::from(fields.u16_le(26).ok_or(ZipError::Truncated("local header"))?);
    let extra_len = u64::from(fields.u16_le(28).ok_or(ZipError::Truncated("local header"))?);
    let local_len = (30 + name_len + extra_len).checked_add(entry.compressed_size).ok_or(ZipError::Corrupt("entry size".into()))?;
    let local = read_at(file, at, local_len)?;
    entry.local_header_offset = 0;
    zip::read_entry(&local, &entry, max_bytes).map(Some)
}

/// Overwrite `bytes` at `at` in a record read from the file.
fn put(record: &mut [u8], at: usize, bytes: &[u8]) -> Result<(), ZipError> {
    record.get_mut(at..at + bytes.len()).ok_or(ZipError::Truncated("end record"))?.copy_from_slice(bytes);
    Ok(())
}

/// Exactly `len` bytes from `at`; a file that ends sooner is truncated.
fn read_at(file: &mut (impl Read + Seek), at: u64, len: u64) -> Result<Vec<u8>, ZipError> {
    file.seek(SeekFrom::Start(at)).map_err(io)?;
    let mut out = Vec::new();
    file.take(len).read_to_end(&mut out).map_err(io)?;
    if (out.len() as u64) < len {
        return Err(ZipError::Truncated("file"));
    }
    Ok(out)
}

fn io(e: std::io::Error) -> ZipError {
    ZipError::Corrupt(format!("couldn't read it: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const MANIFEST: &[u8] = b"Manifest-Version: 1.0\r\nMain-Class: app.Main\r\n\r\n";

    fn read(bytes: &[u8], name: &str) -> Result<Option<Vec<u8>>, ZipError> {
        read_named_entry(&mut Cursor::new(bytes.to_vec()), name, 1 << 20)
    }

    /// The same archive with a ZIP64 end record and locator before its classic end record, whose
    /// fields at these offsets are saturated (entry counts and disk numbers 2 bytes, others 4).
    fn as_zip64(archive: &[u8], saturate: &[usize]) -> Vec<u8> {
        let end = archive.len() - 22;
        let r = Reader::new(archive);
        let (entries, size, offset) =
            (u64::from(r.u16_le(end + 10).unwrap()), u64::from(r.u32_le(end + 12).unwrap()), u64::from(r.u32_le(end + 16).unwrap()));
        let mut out = archive[..end].to_vec();
        let record_at = out.len() as u64;
        out.extend_from_slice(b"PK\x06\x06");
        out.extend_from_slice(&44u64.to_le_bytes());
        out.extend_from_slice(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        for v in [entries, entries, size, offset] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(b"PK\x06\x07\0\0\0\0");
        out.extend_from_slice(&record_at.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        let mut classic = archive[end..].to_vec();
        for &at in saturate {
            let width = if at < 12 { 2 } else { 4 };
            classic[at..at + width].fill(0xff);
        }
        out.extend_from_slice(&classic);
        out
    }

    #[test]
    fn an_entry_is_read_from_the_directory_without_the_whole_file() {
        let filler = vec![7u8; 200_000];
        let archive = crate::lenient::testkit::zip_stored(&[("app/Big.class", &filler), ("META-INF/MANIFEST.MF", MANIFEST), ("META-INF/", b"")]);
        assert_eq!(read(&archive, "META-INF/MANIFEST.MF").unwrap().as_deref(), Some(MANIFEST));
        assert_eq!(read(&archive, "META-INF/MISSING").unwrap(), None);
        assert_eq!(read(&archive, "META-INF/").unwrap(), None, "a folder isn't an entry to read");

        let mut stubbed = b"#!/bin/sh\nexec java -jar \"$0\" \"$@\"\n".to_vec();
        stubbed.extend_from_slice(&archive);
        assert_eq!(read(&stubbed, "META-INF/MANIFEST.MF").unwrap().as_deref(), Some(MANIFEST), "a launcher stub before the archive");

        assert_eq!(read(b"not a zip", "META-INF/MANIFEST.MF"), Err(ZipError::NotZip));
        assert!(read(&archive[..archive.len() - 30], "META-INF/MANIFEST.MF").is_err(), "a cut-off file");
        assert_eq!(read_named_entry(&mut Cursor::new(archive), "app/Big.class", 1000), Err(ZipError::TooLarge(1000)));
    }

    #[test]
    fn a_directory_bigger_than_the_searched_tail_is_read() {
        let names: Vec<String> = (0..3000).map(|i| format!("com/example/generated/Class{i:05}.class")).collect();
        let mut files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
        files.push(("META-INF/MANIFEST.MF", MANIFEST));
        let archive = crate::lenient::testkit::zip_stored(&files);
        assert_eq!(read(&archive, "META-INF/MANIFEST.MF").unwrap().as_deref(), Some(MANIFEST));
    }

    /// A one-entry archive whose entry is deflated, as `jar` writes a manifest.
    fn zip_deflated(name: &str, body: &[u8]) -> Vec<u8> {
        let packed = miniz_oxide::deflate::compress_to_vec(body, 6);
        let fixed = |out: &mut Vec<u8>| {
            out.extend_from_slice(&[20, 0, 0, 0, 8, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&zip::crc32(body).to_le_bytes());
            out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
        };
        let mut out = b"PK\x03\x04".to_vec();
        fixed(&mut out);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&packed);
        let directory_at = out.len() as u32;
        out.extend_from_slice(b"PK\x01\x02\x14\0");
        fixed(&mut out);
        out.extend_from_slice(&[0; 14]);
        out.extend_from_slice(name.as_bytes());
        let directory_len = out.len() as u32 - directory_at;
        out.extend_from_slice(b"PK\x05\x06\0\0\0\0\x01\0\x01\0");
        out.extend_from_slice(&directory_len.to_le_bytes());
        out.extend_from_slice(&directory_at.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn a_deflated_entry_and_a_comment_that_looks_like_an_end_record() {
        let deflated = zip_deflated("META-INF/MANIFEST.MF", MANIFEST);
        assert_eq!(read(&deflated, "META-INF/MANIFEST.MF").unwrap().as_deref(), Some(MANIFEST));

        let mut commented = crate::lenient::testkit::zip_stored(&[("META-INF/MANIFEST.MF", MANIFEST)]);
        let comment = b"built by PK\x05\x06 tools";
        let at = commented.len() - 2;
        commented[at..].copy_from_slice(&(comment.len() as u16).to_le_bytes());
        commented.extend_from_slice(comment);
        assert_eq!(read(&commented, "META-INF/MANIFEST.MF").unwrap().as_deref(), Some(MANIFEST));
    }

    /// The last entry's comment ends like a ZIP64 record and locator, saying the directory is huge;
    /// the classic fields disagree with it and win, as they do for `zip::parse`.
    #[test]
    fn a_comment_shaped_like_zip64_records_is_not_read_as_them() {
        let mut archive = crate::lenient::testkit::zip_stored(&[("META-INF/MANIFEST.MF", MANIFEST)]);
        let end = archive.len() - 22;
        let last = archive.windows(4).rposition(|w| w == b"PK\x01\x02").unwrap();
        let mut fake = b"PK\x06\x06".to_vec();
        fake.extend_from_slice(&44u64.to_le_bytes());
        fake.extend_from_slice(&[0; 28]);
        fake.extend_from_slice(&(MAX_DIRECTORY + 1).to_le_bytes());
        fake.extend_from_slice(&0u64.to_le_bytes());
        fake.extend_from_slice(b"PK\x06\x07\0\0\0\0");
        fake.extend_from_slice(&0u64.to_le_bytes());
        fake.extend_from_slice(&1u32.to_le_bytes());
        archive[last + 32..last + 34].copy_from_slice(&(fake.len() as u16).to_le_bytes());
        let size = Reader::new(&archive).u32_le(end + 12).unwrap() + fake.len() as u32;
        archive[end + 12..end + 16].copy_from_slice(&size.to_le_bytes());
        archive.splice(end..end, fake);
        assert!(zip::parse(&archive, 10).is_ok(), "precondition: a valid archive");
        assert_eq!(read(&archive, "META-INF/MANIFEST.MF").unwrap().as_deref(), Some(MANIFEST));
    }

    /// ZIP64 end records are read with any classic fields saturated or none (`zip -fz` saturates
    /// only the offset), after a stub too.
    #[test]
    fn zip64_archives_are_read_in_parts() {
        let archive = crate::lenient::testkit::zip_stored(&[("app/Main.class", b"x"), ("META-INF/MANIFEST.MF", MANIFEST)]);
        for saturate in [&[][..], &[8, 10, 12, 16], &[16], &[12], &[10], &[4]] {
            let zip64 = as_zip64(&archive, saturate);
            assert_eq!(read(&zip64, "META-INF/MANIFEST.MF").unwrap().as_deref(), Some(MANIFEST), "saturated {saturate:?}");
            let mut stubbed = b"#!/bin/sh\n".to_vec();
            stubbed.extend_from_slice(&zip64);
            assert_eq!(read(&stubbed, "META-INF/MANIFEST.MF").unwrap().as_deref(), Some(MANIFEST), "saturated {saturate:?}, after a stub");
        }
    }
}
