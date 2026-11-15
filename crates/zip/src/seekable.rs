//! Checks for an already indexed, seekable ZIP archive.

use std::io::{Read, Seek};

/// Why an indexed seekable archive exceeded its caller's limits.
#[derive(Debug, thiserror::Error)]
pub enum InspectError {
    #[error("the archive has {actual} entries, more than {limit}")]
    TooManyEntries { actual: usize, limit: usize },
    #[error("declared entry sizes exceed {limit} bytes")]
    DeclaredBytesExceeded { limit: u64 },
    #[error("ZIP entries overlap")]
    OverlappingEntries,
    #[error("ZIP index: {0}")]
    Zip(#[from] zip::result::ZipError),
}

/// Check every declared payload, including entries the caller may ignore, and reject overlapping
/// entry headers and compressed ranges. This is a post-index check; it does not replace a pre-index count guard or
/// bound bytes actually decompressed by a caller.
pub fn inspect_seekable<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, max_entries: usize, max_declared_bytes: u64) -> Result<(), InspectError> {
    if archive.len() > max_entries {
        return Err(InspectError::TooManyEntries { actual: archive.len(), limit: max_entries });
    }
    let mut remaining = max_declared_bytes;
    for index in 0..archive.len() {
        remaining =
            remaining.checked_sub(archive.by_index_raw(index)?.size()).ok_or(InspectError::DeclaredBytesExceeded { limit: max_declared_bytes })?;
    }
    if archive.has_overlapping_entries()? {
        return Err(InspectError::OverlappingEntries);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[test]
    fn counts_all_declared_entries_and_detects_aliases() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        writer.start_file("first", options).unwrap();
        writer.write_all(b"payload").unwrap();
        writer.shallow_copy_file("first", "alias").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert!(matches!(inspect_seekable(&mut archive, 1, 14), Err(InspectError::TooManyEntries { .. })));
        assert!(matches!(inspect_seekable(&mut archive, 2, 13), Err(InspectError::DeclaredBytesExceeded { .. })));
        assert!(matches!(inspect_seekable(&mut archive, 2, 14), Err(InspectError::OverlappingEntries)));
    }

    #[test]
    fn empty_aliases_still_overlap_at_the_header() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("empty", zip::write::SimpleFileOptions::default()).unwrap();
        writer.shallow_copy_file("empty", "alias").unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(writer.finish().unwrap().into_inner())).unwrap();
        assert!(matches!(inspect_seekable(&mut archive, 2, 0), Err(InspectError::OverlappingEntries)));
    }

    #[test]
    fn permits_exact_limits_and_empty_archive() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("a", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(b"123").unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(writer.finish().unwrap().into_inner())).unwrap();
        assert!(inspect_seekable(&mut archive, 1, 3).is_ok());
        let empty = zip::ZipWriter::new(Cursor::new(Vec::new())).finish().unwrap().into_inner();
        assert!(inspect_seekable(&mut zip::ZipArchive::new(Cursor::new(empty)).unwrap(), 0, 0).is_ok());
    }
}
