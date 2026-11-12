//! Clean-room MXF demuxer, written from the SMPTE specifications (editions in the crate README):
//! ST 377-1 (file format: KLV, partitions, primer pack, header metadata, index tables, random
//! index pack), ST 378 (OP1a), ST 390 (OP-Atom), ST 379-1/2 (generic container), ST 381-1 (MPEG
//! video mapping), ST 381-3 (AVC mapping), ST 382 (AES3 / Broadcast Wave audio mapping), ST 331
//! (AES3 element data, D-10 sound), ST 2019-4 (VC-3 mapping) and RDD 44 (ProRes mapping).
//!
//! [`open`] walks the file's KLV packets (keys and lengths only; essence payloads are never read
//! while opening), parses the partition packs, the primer pack and header metadata of the most
//! complete partition, and every index table segment, then resolves the material package to the
//! file (source) package tracks that carry essence in this file. The result is a list of
//! [`EssenceTrack`]s with per-edit-unit [`Sample`] tables for pictures (file offset, size,
//! presentation position from the index temporal offsets, random-access flag) and PCM chunk
//! tables for sound, plus the start timecode of the material package.
//!
//! [`MxfWriter`] ([`mod@write`]) writes OP1a and OP-Atom files (VC-3, ProRes, AVC byte stream, PCM)
//! with index tables and start timecode, streaming to any `Write + Seek` sink.
//!
//! One dependency (`chrono`, for the writer's calendar dates), no `unsafe`, builds for
//! `wasm32-unknown-unknown`.
//!
//! From FilmCraft's `filmcraft-mxf` (storytold/filmcraft@4bec22b). Public items keep their names.
//! `ByteSource` is the crate's one source trait.

// FilmCraft allows these lints workspace-wide: index loops follow the specifications' tables.
#![allow(clippy::needless_range_loop, clippy::type_complexity)]

mod essence;
mod file;
mod index;
mod klv;
mod meta;
pub mod write;

pub use crate::source::{ByteSource, ReadSeekSource};
pub use essence::{Codec, PictureInfo, SoundFormat, SoundInfo, aes3_element_samples, decode_pcm};
pub use file::{Chunk, EssenceTrack, MxfFile, OperationalPattern, Partition, PartitionKind, Sample, Timecode, TrackKind, Wrapping, open};
pub use index::{IndexEntry, IndexSegment};
pub use klv::{Rational, Ul};
pub use write::{
    CodedKind, ColorSpace, FrameInfo, MxfWriter, OpAtomPcm, PackageIds, Pattern, PictureCoding, PictureDesc, SoundDesc, StartTimecode, Timestamp,
    Umid, WriterConfig, encode_pcm, write_opatom_pcm, write_opatom_pcm_as,
};

use std::fmt;
use std::io;

/// Up to `len` bytes at `offset` (fewer at the end of the source).
pub(crate) fn read_upto(src: &(impl ByteSource + ?Sized), offset: u64, len: usize) -> io::Result<Vec<u8>> {
    let n = src.len().saturating_sub(offset).min(len as u64) as usize;
    let mut v = vec![0u8; n];
    src.read_at(offset, &mut v)?;
    Ok(v)
}

/// Errors produced by the demuxer.
#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    /// Underlying I/O failure.
    Io(io::Error),
    /// Not an MXF file (no header partition pack).
    NotMxf(String),
    /// Structurally invalid data that could not be recovered from.
    Invalid(String),
    /// Valid but unsupported (e.g. essence stored in another file).
    Unsupported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::NotMxf(s) => write!(f, "not an MXF file: {s}"),
            Error::Invalid(s) => write!(f, "invalid MXF: {s}"),
            Error::Unsupported(s) => write!(f, "unsupported MXF: {s}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, Error>;

/// Whether `head` (the first bytes of a file) looks like MXF: a header partition pack key within
/// the run-in (at most 64 KiB, ST 377-1 §6.5).
pub fn sniff(head: &[u8]) -> bool {
    klv::find_header_partition(head).is_some()
}

#[cfg(test)]
mod hostile_tests;
#[cfg(test)]
mod tests;
