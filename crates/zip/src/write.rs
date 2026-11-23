//! A ZIP archive being written.

use std::io::{Cursor, Seek, Write};

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

/// A ZIP archive being written. The output is deterministic: the same (1980)
/// timestamp on every entry. Entries are written in the order they are added.
pub struct Writer<W: Write + Seek = Cursor<Vec<u8>>> {
    zip: zip::ZipWriter<W>,
    legacy_utf8: bool,
}

impl Default for Writer<Cursor<Vec<u8>>> {
    fn default() -> Writer<Cursor<Vec<u8>>> {
        Writer { zip: zip::ZipWriter::new(Cursor::new(Vec::new())), legacy_utf8: false }
    }
}

impl Writer<Cursor<Vec<u8>>> {
    pub fn new() -> Writer<Cursor<Vec<u8>>> {
        Writer::default()
    }

    /// Writes the UTF-8 flag and version fields used by the previous dotLottie writer.
    ///
    /// This keeps stored archives byte-for-byte stable for existing users while still using the
    /// shared writer for entry encoding and validation.
    pub fn legacy_utf8() -> Writer<Cursor<Vec<u8>>> {
        Writer { legacy_utf8: true, ..Writer::default() }
    }

    /// The finished archive.
    pub fn finish(self) -> Result<Vec<u8>, Error> {
        let mut bytes = self.zip.finish().map(Cursor::into_inner).map_err(|e| Error::Write(e.to_string()))?;
        if self.legacy_utf8 {
            patch_legacy_utf8_headers(&mut bytes)?;
        }
        Ok(bytes)
    }
}

impl<W: Write + Seek> Writer<W> {
    /// Starts writing an archive directly to a seekable destination without buffering the whole
    /// archive in memory.
    pub fn to_writer(destination: W) -> Self {
        Self { zip: zip::ZipWriter::new(destination), legacy_utf8: false }
    }

    /// Adds a directory entry with the deterministic 1980 timestamp.
    pub fn add_directory(&mut self, name: &str) -> Result<(), Error> {
        let options = zip::write::SimpleFileOptions::default().last_modified_time(zip::DateTime::default());
        self.zip.add_directory(name, options).map_err(|e| Error::Write(format!("{name}: {e}")))
    }

    /// Adds an entry from a byte slice. A name the archive already has is an error.
    pub fn add(&mut self, name: &str, data: &[u8], compression: Compression) -> Result<(), Error> {
        self.add_entry(name, data, compression, 1, data.len() as u64 >= u64::from(u32::MAX) / 2)
    }

    /// Adds a stored entry whose payload starts on an `alignment`-byte boundary.
    ///
    /// Entries use ordinary ZIP headers, matching the USDZ writer this replaces;
    /// oversized entries fail rather than opting into ZIP64 headers.
    /// USDZ uses 64. Only powers of two from 1 through 4096 are accepted, bounding
    /// per-entry padding. This does not impose a total archive-size budget; callers
    /// retain their format-specific size and name policies. Discard the writer after
    /// a write error, just as for [`Self::add`]. Invalid alignment is rejected before
    /// starting an entry.
    pub fn add_stored_aligned(&mut self, name: &str, data: &[u8], alignment: u16) -> Result<(), Error> {
        if !alignment.is_power_of_two() || alignment > 4096 {
            return Err(Error::Write("alignment must be a power of two between 1 and 4096".into()));
        }
        self.add_entry(name, data, Compression::Stored, alignment, false)
    }

    fn add_entry(&mut self, name: &str, data: &[u8], compression: Compression, alignment: u16, large_file: bool) -> Result<(), Error> {
        let (method, level) = match compression {
            Compression::Stored => (zip::CompressionMethod::Stored, None),
            Compression::Deflated => (zip::CompressionMethod::Deflated, Some(6)),
            Compression::Level(l) => (zip::CompressionMethod::Deflated, Some(i64::from(l.min(9)))),
        };
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(method)
            .compression_level(level)
            .last_modified_time(zip::DateTime::default())
            .large_file(large_file)
            .with_alignment(alignment);
        let fail = |e: &dyn std::fmt::Display| Error::Write(format!("{name}: {e}"));
        self.zip.start_file(name, options).map_err(|e| fail(&e))?;
        self.zip.write_all(data).map_err(|e| fail(&e))
    }

    /// Adds an entry by streaming it into the compressor, so large generated entries need not
    /// first be buffered in memory. A name the archive already has is an error.
    pub fn add_with(&mut self, name: &str, compression: Compression, write: impl FnOnce(&mut dyn Write) -> std::io::Result<()>) -> Result<(), Error> {
        let (method, level) = match compression {
            Compression::Stored => (zip::CompressionMethod::Stored, None),
            Compression::Deflated => (zip::CompressionMethod::Deflated, Some(6)),
            Compression::Level(l) => (zip::CompressionMethod::Deflated, Some(i64::from(l.min(9)))),
        };
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(method)
            .compression_level(level)
            .last_modified_time(zip::DateTime::default())
            .large_file(true);
        let fail = |e: &dyn std::fmt::Display| Error::Write(format!("{name}: {e}"));
        self.zip.start_file(name, options).map_err(|e| fail(&e))?;
        write(&mut self.zip).map_err(|e| fail(&e))
    }

    /// Finishes writing and returns the underlying destination.
    ///
    /// Legacy UTF-8 mode requires [`Writer::finish`] to apply its in-memory header patch.
    /// Calling this method in legacy mode consumes the writer and returns [`Error::Write`];
    /// use `finish` instead when constructing a writer with [`Writer::legacy_utf8`].
    pub fn finish_into(self) -> Result<W, Error> {
        // A generic destination need not support reads or header patching. Never silently
        // return an archive that omits the byte-compatibility contract of legacy mode.
        if self.legacy_utf8 {
            return Err(Error::Write("legacy UTF-8 output requires Writer::finish".into()));
        }
        self.zip.finish().map_err(|e| Error::Write(e.to_string()))
    }
}

fn get_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn get_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) -> Result<(), Error> {
    let dst = bytes
        .get_mut(at..at.checked_add(4).ok_or_else(|| Error::Write("ZIP header offset overflow".into()))?)
        .ok_or_else(|| Error::Write("short ZIP header".into()))?;
    dst.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn put_u16(bytes: &mut [u8], at: usize, value: u16) -> Result<(), Error> {
    let dst = bytes
        .get_mut(at..at.checked_add(2).ok_or_else(|| Error::Write("ZIP header offset overflow".into()))?)
        .ok_or_else(|| Error::Write("short ZIP header".into()))?;
    dst.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// Patches the two header versions and UTF-8 bit to match the former dotLottie writer.
fn patch_legacy_utf8_headers(bytes: &mut [u8]) -> Result<(), Error> {
    const EOCD: u32 = 0x0605_4b50;
    const CENTRAL: u32 = 0x0201_4b50;
    const LOCAL: u32 = 0x0403_4b50;
    let eocd = bytes
        .windows(4)
        .rposition(|window| u32::from_le_bytes([window[0], window[1], window[2], window[3]]) == EOCD)
        .ok_or_else(|| Error::Write("missing ZIP end record".into()))?;
    let central_start = usize::try_from(get_u32(bytes, eocd + 16).ok_or_else(|| Error::Write("short ZIP end record".into()))?)
        .map_err(|_| Error::Write("ZIP central directory offset overflow".into()))?;
    let central_size = usize::try_from(get_u32(bytes, eocd + 12).ok_or_else(|| Error::Write("short ZIP end record".into()))?)
        .map_err(|_| Error::Write("ZIP central directory size overflow".into()))?;
    let central_end =
        central_start.checked_add(central_size).filter(|end| *end <= eocd).ok_or_else(|| Error::Write("invalid ZIP central directory".into()))?;
    let mut at = central_start;
    while at < central_end {
        if get_u32(bytes, at) != Some(CENTRAL) {
            return Err(Error::Write("invalid ZIP central directory entry".into()));
        }
        let name = usize::from(get_u16(bytes, at + 28).ok_or_else(|| Error::Write("short ZIP entry".into()))?);
        let extra = usize::from(get_u16(bytes, at + 30).ok_or_else(|| Error::Write("short ZIP entry".into()))?);
        let comment = usize::from(get_u16(bytes, at + 32).ok_or_else(|| Error::Write("short ZIP entry".into()))?);
        let length = 46usize
            .checked_add(name)
            .and_then(|n| n.checked_add(extra))
            .and_then(|n| n.checked_add(comment))
            .ok_or_else(|| Error::Write("ZIP entry size overflow".into()))?;
        let next = at.checked_add(length).filter(|next| *next <= central_end).ok_or_else(|| Error::Write("short ZIP entry".into()))?;
        let local = usize::try_from(get_u32(bytes, at + 42).ok_or_else(|| Error::Write("short ZIP entry".into()))?)
            .map_err(|_| Error::Write("ZIP local-header offset overflow".into()))?;
        if get_u32(bytes, local) != Some(LOCAL) {
            return Err(Error::Write("invalid ZIP local header".into()));
        }
        // Version 45 is required by ZIP64; retain it. Ordinary entries use 2.0 to match dotLottie.
        if get_u16(bytes, at + 6).is_some_and(|version| version < 45) {
            put_u16(bytes, at + 6, 20)?;
            put_u16(bytes, local + 4, 20)?;
        }
        put_u16(bytes, at + 4, 20)?;
        put_u32(bytes, at + 38, 0)?;
        let central_flags = get_u16(bytes, at + 8).ok_or_else(|| Error::Write("short ZIP entry".into()))? | 0x0800;
        let local_flags = get_u16(bytes, local + 6).ok_or_else(|| Error::Write("short ZIP local header".into()))? | 0x0800;
        put_u16(bytes, at + 8, central_flags)?;
        put_u16(bytes, local + 6, local_flags)?;
        at = next;
    }
    if at != central_end {
        return Err(Error::Write("invalid ZIP central directory size".into()));
    }
    Ok(())
}

#[cfg(test)]
mod finish_tests {
    use super::*;

    #[test]
    fn legacy_ascii_output_requires_the_header_patching_finish_path() {
        let mut legacy = Writer::legacy_utf8();
        legacy.add("a.txt", b"ordinary text", Compression::Stored).unwrap();
        assert!(matches!(legacy.finish_into(), Err(Error::Write(message))
            if message == "legacy UTF-8 output requires Writer::finish"));

        let mut legacy = Writer::legacy_utf8();
        legacy.add("a.txt", b"ordinary text", Compression::Stored).unwrap();
        let bytes = legacy.finish().unwrap();
        assert_eq!(get_u16(&bytes, 6).unwrap() & 0x0800, 0x0800);

        let mut ordinary = Writer::to_writer(Cursor::new(Vec::new()));
        ordinary.add("a.txt", b"ordinary text", Compression::Stored).unwrap();
        let bytes = ordinary.finish_into().unwrap().into_inner();
        let mut archive = crate::Archive::open(&bytes, crate::Limits { max_entries: 1, max_total_bytes: 32 }).unwrap();
        assert_eq!(archive.read("a.txt", 32).unwrap().unwrap(), b"ordinary text");
    }
}
