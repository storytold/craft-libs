//! Streaming gzip output with optional stored segments.
//!
//! Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors.
//! Extracted from VectorCraft, licensed under MIT OR Apache-2.0.
//!
//! `flush` does not emit a deflate block; call [`Gz::finish`] to complete and flush the
//! stream. After an I/O error, discard the writer: the output may be incomplete.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use flate2::{Compress, Compression, Crc, FlushCompress, Status};
use std::io::{self, Write};

/// Bytes of compressed output gathered before they go to the writer.
const OUT_CHUNK: usize = 64 << 10;

/// A gzip stream, written as [`flate2::write::GzEncoder`] at the default level writes it (the
/// same header and the same data; the deflate blocks are the same too where flate2 deflates with
/// miniz_oxide, not with zlib-rs, which writes them by how the data is fed), except that data which
/// is compressed already ([`Gz::packed`]) is stored as it is: the default level spends a second
/// or more per photo on it for next to nothing, and makes it slower to unpack. Where the level
/// changes the deflate blocks so far end with a sync flush and a fresh compressor writes on,
/// which makes one valid deflate stream that every reader unpacks.
pub struct Gz<W: Write> {
    out: W,
    deflate: Compress,
    /// What `deflate` writes: stored blocks (packed data), else the default level.
    stored: bool,
    crc: Crc,
    buf: Vec<u8>,
    /// Bytes written (unpacked).
    total: u64,
}

impl<W: Write> Gz<W> {
    /// Write a gzip header and construct a writer.
    pub fn new(mut out: W) -> io::Result<Self> {
        // flate2's header for the default level: no name, no time, unknown OS.
        out.write_all(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255])?;
        Ok(Self {
            out,
            deflate: Compress::new(Compression::default(), false),
            stored: false,
            crc: Crc::new(),
            buf: Vec::with_capacity(OUT_CHUNK),
            total: 0,
        })
    }

    /// Run the compressor over `input` with `flush` until it has taken it all (and, flushing,
    /// written everything out).
    fn deflate(&mut self, mut input: &[u8], flush: FlushCompress) -> io::Result<()> {
        loop {
            if self.buf.len() == self.buf.capacity() {
                self.out.write_all(&self.buf)?;
                self.buf.clear();
            }
            let (before_in, before_out) = (self.deflate.total_in(), self.deflate.total_out());
            let status = self.deflate.compress_vec(input, &mut self.buf, flush).map_err(io::Error::other)?;
            let taken = usize::try_from(self.deflate.total_in() - before_in).unwrap_or(input.len()).min(input.len());
            input = input.get(taken..).unwrap_or_default();
            let made = self.deflate.total_out() - before_out;
            let full = self.buf.len() == self.buf.capacity();
            match flush {
                FlushCompress::None if input.is_empty() => return Ok(()),
                FlushCompress::Finish if status == Status::StreamEnd => return Ok(()),
                // A flush is done once it left room in the output.
                FlushCompress::Sync if input.is_empty() && !full => return Ok(()),
                _ => {}
            }
            if taken == 0 && made == 0 && !full {
                return Err(io::Error::other("the compressor made no progress"));
            }
        }
    }

    /// Return the number of uncompressed bytes supplied.
    pub fn written(&self) -> u64 {
        self.total
    }

    /// What is written next is compressed already (stored as it is), or not (compressed at the
    /// default level).
    pub fn packed(&mut self, packed: bool) -> io::Result<()> {
        if packed == self.stored {
            return Ok(());
        }
        self.deflate(&[], FlushCompress::Sync)?;
        self.deflate = Compress::new(if packed { Compression::none() } else { Compression::default() }, false);
        self.stored = packed;
        Ok(())
    }

    /// Complete the gzip trailer and flush the underlying writer.
    pub fn finish(mut self) -> io::Result<()> {
        self.deflate(&[], FlushCompress::Finish)?;
        self.out.write_all(&self.buf)?;
        self.out.write_all(&self.crc.sum().to_le_bytes())?;
        self.out.write_all(&self.crc.amount().to_le_bytes())?;
        self.out.flush()
    }
}

impl<W: Write> Write for Gz<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_all(buf)?;
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.total = self.total.checked_add(buf.len() as u64).ok_or_else(|| io::Error::other("uncompressed byte count overflow"))?;
        self.crc.update(buf);
        self.deflate(buf, FlushCompress::None)
    }

    fn flush(&mut self) -> io::Result<()> {
        // Flushing would end a deflate block: the stream is flushed when it is finished.
        Ok(())
    }
}
