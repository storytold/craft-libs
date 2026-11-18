//! Bounded ZIP handling for dotLottie archives.
//!
//! The ZIP implementation delegates to `craft-zip`; this module applies dotLottie's relative
//! entry-name rule and caps archive expansion before returning entry bytes.

use craft_zip::{Archive, Error, Writer};
pub use craft_zip::{Compression, Limits};

/// Largest entry count accepted from a dotLottie archive.
pub const MAX_ENTRIES: u64 = 10_000;
/// Largest combined uncompressed payload returned from a dotLottie archive.
pub const MAX_TOTAL_BYTES: u64 = 1 << 30;

/// Whether an entry name is a relative path inside the archive: not absolute (`/…`, `\\…`), no
/// drive (`C:…`) and no `..` component.
fn is_relative_name(name: &str) -> bool {
    let drive = name.as_bytes().get(1) == Some(&b':');
    !name.starts_with(['/', '\\']) && !drive && !name.split(['/', '\\']).any(|c| c == "..")
}

/// Read files from a dotLottie ZIP archive within [`MAX_ENTRIES`] and [`MAX_TOTAL_BYTES`].
///
/// Both stored and deflated entries are supported. Directories and names that escape the archive
/// root are skipped. Duplicate names and overlapping payloads are rejected because a consumer
/// must not guess which content a manifest or animation name identifies.
pub fn read_entries(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, Error> {
    read_entries_with_limits(data, Limits { max_entries: MAX_ENTRIES, max_total_bytes: MAX_TOTAL_BYTES })
}

/// Read a dotLottie archive under caller-provided entry and expansion limits.
pub fn read_entries_with_limits(data: &[u8], limits: Limits) -> Result<Vec<(String, Vec<u8>)>, Error> {
    let mut archive = Archive::open(data, limits)?;
    if archive.has_duplicate_names() || archive.has_overlapping_files()? {
        return Err(Error::NotZip("duplicate names or overlapping entry data".into()));
    }
    let mut out = Vec::new();
    for index in 0..archive.len() {
        let (Some(name), Some(is_dir)) = (archive.entry_name(index)?, archive.entry_is_dir(index)?) else { continue };
        if is_dir || !is_relative_name(&name) {
            continue;
        }
        if let Some(bytes) = archive.read(&name, limits.max_total_bytes)? {
            out.push((name, bytes));
        }
    }
    Ok(out)
}

/// Read the stored local-file entries in a ZIP written by [`store`]. This compatibility reader
/// retains its original behavior: compressed entries are skipped, an incomplete trailing entry
/// stops the walk while already-read entries are returned, and unsafe names are skipped. New code
/// should use [`read_entries`] for central-directory validation and deflate support.
pub fn read_stored(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let u16_at = |p: usize| p.checked_add(2).and_then(|e| data.get(p..e)).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32_at = |p: usize| p.checked_add(4).and_then(|e| data.get(p..e)).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let mut out = Vec::new();
    let mut i = 0usize;
    while u32_at(i) == Some(0x0403_4b50) {
        let (Some(method), Some(size), Some(nlen), Some(xlen)) = (
            i.checked_add(8).and_then(u16_at),
            i.checked_add(18).and_then(u32_at),
            i.checked_add(26).and_then(u16_at),
            i.checked_add(28).and_then(u16_at),
        ) else {
            break;
        };
        let Some(name_start) = i.checked_add(30) else { break };
        let Some(name_end) = name_start.checked_add(nlen) else { break };
        let Some(body) = name_end.checked_add(xlen) else { break };
        let Some(body_end) = body.checked_add(size) else { break };
        let (Some(name), Some(bytes)) = (data.get(name_start..name_end), data.get(body..body_end)) else { break };
        let name = String::from_utf8_lossy(name).to_string();
        if method == 0 && is_relative_name(&name) {
            out.push((name, bytes.to_vec()));
        }
        i = body_end;
    }
    out
}

/// Write ZIP entries uncompressed, preserving input order and the legacy deterministic timestamp.
pub fn try_store(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, Error> {
    try_store_with_compression(entries, Compression::Stored)
}

/// Write entries using the selected shared ZIP compression mode.
pub fn try_store_with_compression(entries: &[(String, Vec<u8>)], compression: Compression) -> Result<Vec<u8>, Error> {
    let mut writer = Writer::legacy_utf8();
    for (name, bytes) in entries {
        writer.add(name, bytes, compression)?;
    }
    writer.finish()
}

/// Compatibility wrapper for the original infallible writer API. Use [`try_store`] when the
/// caller needs an actionable error for an invalid or unrepresentable archive.
pub fn store(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    try_store(entries).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn stored_writer_roundtrips_and_keeps_entry_order() {
        let entries = [("a.json".into(), b"{}".to_vec()), ("b/c.txt".into(), b"hello".to_vec())];
        let bytes = try_store(&entries).unwrap();
        assert_eq!(read_entries(&bytes).unwrap(), entries);
        assert_eq!(bytes, store(&entries));
    }

    #[test]
    fn deflated_entries_are_read_and_path_traversal_is_skipped() {
        let mut writer = Writer::new();
        writer.add("animations/main.json", br#"{"w":1,"h":1}"#, Compression::Deflated).unwrap();
        writer.add("../outside.json", b"{}", Compression::Deflated).unwrap();
        let bytes = writer.finish().unwrap();
        assert_eq!(read_entries(&bytes).unwrap(), [("animations/main.json".into(), br#"{"w":1,"h":1}"#.to_vec())]);
        assert!(read_entries_with_limits(&bytes, Limits { max_entries: 10, max_total_bytes: 4 }).is_err());
    }

    #[test]
    fn malformed_and_truncated_archives_return_errors_without_panicking() {
        let mut hostile = vec![0; 30];
        hostile[..4].copy_from_slice(&0x0403_4b50u32.to_le_bytes());
        hostile[8..10].copy_from_slice(&0u16.to_le_bytes());
        hostile[18..22].copy_from_slice(&u32::MAX.to_le_bytes());
        hostile[26..28].copy_from_slice(&u16::MAX.to_le_bytes());
        hostile[28..30].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(read_entries(&hostile).is_err());
        for end in 0..hostile.len() {
            let _ = read_entries(&hostile[..end]);
        }
    }

    #[test]
    fn legacy_stored_archive_bytes_and_partial_tail_are_compatible() {
        const HEX: &str = "504b03041400000800000000210043bfa6a3020000000200000006000000612e6a736f6e7b7d504b03041400000800000000210086a61036050000000500000007000000622f632e74787468656c6c6f504b010214001400000800000000210043bfa6a30200000002000000060000000000000000000000000000000000612e6a736f6e504b010214001400000800000000210086a610360500000005000000070000000000000000000000000026000000622f632e747874504b0506000000000200020069000000500000000000";
        let legacy: Vec<u8> = HEX.as_bytes().chunks(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect();
        let entries = [("a.json".into(), b"{}".to_vec()), ("b/c.txt".into(), b"hello".to_vec())];
        assert_eq!(read_stored(&legacy), entries);
        assert_eq!(read_entries(&legacy).unwrap(), entries);
        assert_eq!(try_store(&entries).unwrap(), legacy);
        let partial = &legacy[..50];
        assert_eq!(read_stored(partial), [("a.json".into(), b"{}".to_vec())]);
        assert!(read_entries(partial).is_err());
    }
}
