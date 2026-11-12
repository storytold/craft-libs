//! A ZIP archive being written.

use std::io::{Cursor, Write};

use crate::Error;

/// How an entry is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    /// Uncompressed: a `mimetype` entry that must be readable in place, or data that is
    /// compressed already.
    Stored,
    /// Deflate at level 6.
    Deflated,
    /// Deflate at the given level (0 to 9; higher is clamped).
    Level(u8),
}

/// A ZIP archive being written into memory. The output is deterministic: the same (1980)
/// timestamp on every entry. Entries are written in the order they are added.
pub struct Writer {
    zip: zip::ZipWriter<Cursor<Vec<u8>>>,
}

impl Default for Writer {
    fn default() -> Writer {
        Writer { zip: zip::ZipWriter::new(Cursor::new(Vec::new())) }
    }
}

impl Writer {
    pub fn new() -> Writer {
        Writer::default()
    }

    /// Adds an entry. A name the archive already has is an error.
    pub fn add(&mut self, name: &str, data: &[u8], compression: Compression) -> Result<(), Error> {
        let (method, level) = match compression {
            Compression::Stored => (zip::CompressionMethod::Stored, None),
            Compression::Deflated => (zip::CompressionMethod::Deflated, Some(6)),
            Compression::Level(l) => (zip::CompressionMethod::Deflated, Some(i64::from(l.min(9)))),
        };
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(method)
            .compression_level(level)
            .last_modified_time(zip::DateTime::default())
            // Deflate can grow incompressible data a little, so ZIP64 well before 4 GiB.
            .large_file(data.len() as u64 >= u64::from(u32::MAX) / 2);
        let fail = |e: &dyn std::fmt::Display| Error::Write(format!("{name}: {e}"));
        self.zip.start_file(name, options).map_err(|e| fail(&e))?;
        self.zip.write_all(data).map_err(|e| fail(&e))
    }

    /// The finished archive.
    pub fn finish(self) -> Result<Vec<u8>, Error> {
        self.zip.finish().map(Cursor::into_inner).map_err(|e| Error::Write(e.to_string()))
    }
}
