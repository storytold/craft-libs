//! Bound the reads a ZIP indexer may perform before accessing entry payloads.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A seekable reader whose indexing phase is restricted to a tail window and read budget.
///
/// Keep the returned [`Indexing`] handle until the ZIP central directory has been parsed, then
/// call [`Indexing::finish`] to allow normal entry reads. Repeated reads during indexing count
/// towards the allowance, including reads after seeks. This bounds metadata scanning without
/// buffering the whole file or restricting the size of its payload entries.
pub struct MetadataReader<R> {
    inner: R,
    indexing: Arc<AtomicBool>,
    metadata_start: u64,
    remaining: u64,
    position: u64,
}

/// Controls the indexing restriction for the associated reader.
#[derive(Clone)]
pub struct Indexing(Arc<AtomicBool>);

impl Indexing {
    /// Permit reads outside the metadata window once indexing is finished.
    pub fn finish(&self) {
        self.0.store(false, Ordering::Release);
    }
}

impl<R: Read + Seek> MetadataReader<R> {
    /// Limit indexing to the last `metadata_bytes` bytes and at most twice that many bytes read.
    pub fn new(mut inner: R, metadata_bytes: u64) -> io::Result<(Self, Indexing)> {
        let end = inner.seek(SeekFrom::End(0))?;
        let indexing = Arc::new(AtomicBool::new(true));
        let handle = Indexing(indexing.clone());
        Ok((
            Self { inner, indexing, metadata_start: end.saturating_sub(metadata_bytes), remaining: metadata_bytes.saturating_mul(2), position: end },
            handle,
        ))
    }
}

impl<R: Read> Read for MetadataReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let indexing = self.indexing.load(Ordering::Acquire);
        if indexing && (self.position < self.metadata_start || self.remaining == 0) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "ZIP metadata exceeds the indexing limit"));
        }
        let length = if indexing { bytes.len().min(usize::try_from(self.remaining).unwrap_or(usize::MAX)) } else { bytes.len() };
        let read = self.inner.read(bytes.get_mut(..length).unwrap_or_default())?;
        self.position = self.position.saturating_add(read as u64);
        if indexing {
            self.remaining -= read as u64;
        }
        Ok(read)
    }
}

impl<R: Seek> Seek for MetadataReader<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.position = self.inner.seek(from)?;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn indexing_cannot_read_payloads_outside_the_metadata_window() {
        let (mut reader, indexing) = MetadataReader::new(Cursor::new(vec![7; 100]), 8).unwrap();
        reader.seek(SeekFrom::Start(91)).unwrap();
        assert!(reader.read(&mut [0; 1]).is_err());
        reader.seek(SeekFrom::Start(92)).unwrap();
        reader.read_exact(&mut [0; 8]).unwrap();
        indexing.finish();
        reader.seek(SeekFrom::Start(0)).unwrap();
        reader.read_exact(&mut [0; 100]).unwrap();
    }

    #[test]
    fn zip_indexing_is_tail_bounded_then_payload_reads_stream() {
        let mut writer = crate::Writer::new();
        writer.add("large.bin", &vec![42; 131_072], crate::Compression::Stored).unwrap();
        let bytes = writer.finish().unwrap();
        let (reader, indexing) = MetadataReader::new(Cursor::new(bytes), 16_384).unwrap();
        let config = zip::read::Config { archive_offset: zip::read::ArchiveOffset::Known(0) };
        let mut archive = zip::ZipArchive::with_config(config, reader).unwrap();
        assert!(archive.by_name("large.bin").is_err(), "indexing must not read the payload header");
        indexing.finish();
        let mut entry = archive.by_name("large.bin").unwrap();
        let mut payload = Vec::new();
        entry.read_to_end(&mut payload).unwrap();
        assert_eq!(payload, vec![42; 131_072]);
    }

    #[test]
    fn repeated_metadata_scans_share_one_read_allowance() {
        let (mut reader, _indexing) = MetadataReader::new(Cursor::new(vec![7; 8]), 8).unwrap();
        for _ in 0..2 {
            reader.seek(SeekFrom::Start(0)).unwrap();
            reader.read_exact(&mut [0; 8]).unwrap();
        }
        reader.seek(SeekFrom::Start(0)).unwrap();
        assert!(reader.read(&mut [0; 1]).is_err());
    }
}
