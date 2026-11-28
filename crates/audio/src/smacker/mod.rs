//! Clean-room Smacker audio decoder, written from the public community description of the format
//! (the MultimediaWiki "Smacker" page) and checked against real files with ffmpeg as a black-box
//! reference.
//!
//! Smacker's own audio coding is DPCM with Huffman-coded differences: each chunk carries its
//! Huffman trees (one for each byte of a sample, for each channel), the first sample of each
//! channel, and then the differences. Samples are unsigned 8-bit or signed 16-bit, mono or
//! stereo. [`decode`] decodes one such chunk, up to a size the caller gives, and [`pcm`] reads a
//! chunk of an uncompressed track; the chunks come from `craft_media::smacker::split_frame`.
//!
//! ```no_run
//! use craft_audio::smacker::{decode, pcm};
//! use craft_media::smacker::{AudioCoding, open, split_frame};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let bytes = std::fs::read("intro.smk")?;
//! let file = open(&bytes)?;
//! // the first of the seven track slots
//! let track = &file.header.audio[0];
//! let mut samples: Vec<i16> = Vec::new();
//! for frame in &file.frames {
//!     let data = file.read(&bytes, frame)?;
//!     // a frame has a chunk of a track only when the track has data in it
//!     let Some(chunk) = split_frame(&data, frame)?.audio[0] else { continue };
//!     let decoded = match track.coding {
//!         AudioCoding::Dpcm => decode(chunk, 0)?,
//!         AudioCoding::Pcm => pcm(chunk, track.sixteen_bit, track.stereo),
//!         _ => break, // Bink audio
//!     };
//!     samples.extend(decoded.samples.to_i16());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! A tree whose flag bit is clear is read as the format description gives it: no tree follows, and
//! every difference is 0. No real file with such a chunk was available to check this against.
//!
//! Smacker tracks that carry Bink audio are not decoded.

#![warn(missing_docs)]

#[cfg(test)]
mod tests;

use craft_media::bitstream::{BitError, LsbBitReader};
use craft_media::smacker::{ByteTree, TreeError};

/// The most bytes of samples one chunk may decode to, whatever limit the caller passes (16 MiB).
/// A chunk can declare any size and code a sample in no bits at all, so its own size does not
/// bound the output.
pub const MAX_DECODED_SIZE: usize = 1 << 24;

/// Decoder errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The chunk ended inside a tree or a sample.
    #[error("truncated Smacker audio chunk")]
    Truncated,
    /// Structurally invalid data.
    #[error("invalid Smacker audio chunk: {0}")]
    Invalid(&'static str),
    /// The chunk declares more bytes of samples than the limit passed to [`decode`], or than
    /// [`MAX_DECODED_SIZE`].
    #[error("Smacker audio chunk declares {0} bytes of samples")]
    TooLarge(u32),
}

impl From<BitError> for Error {
    fn from(_: BitError) -> Self {
        Error::Truncated
    }
}

impl From<TreeError> for Error {
    fn from(e: TreeError) -> Self {
        match e {
            TreeError::Eof => Error::Truncated,
            TreeError::TooLarge => Error::Invalid("byte tree has too many nodes"),
        }
    }
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, Error>;

/// Decoded samples, channels interleaved (left first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Samples {
    /// Unsigned 8-bit samples (128 is silence).
    U8(Vec<u8>),
    /// Signed 16-bit samples.
    S16(Vec<i16>),
}

impl Samples {
    /// Sample values, counting every channel.
    pub fn len(&self) -> usize {
        match self {
            Samples::U8(s) => s.len(),
            Samples::S16(s) => s.len(),
        }
    }

    /// Whether there are no samples.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The samples as signed 16-bit: 8-bit samples are centred and scaled by 256.
    pub fn to_i16(&self) -> Vec<i16> {
        match self {
            Samples::U8(s) => s.iter().map(|&v| (i16::from(v) - 128) << 8).collect(),
            Samples::S16(s) => s.clone(),
        }
    }
}

/// The samples of one audio chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// 1 or 2. A DPCM chunk flagged as holding no data does not say, and reports 1.
    pub channels: u16,
    /// The samples, interleaved.
    pub samples: Samples,
}

/// Decodes one DPCM audio chunk (a track whose coding is `AudioCoding::Dpcm`): the `u32` size of
/// the decoded samples in bytes, then the bitstream.
///
/// The chunk says itself whether it is stereo and 16-bit. A chunk flagged as holding no data
/// decodes to no samples.
///
/// `max_decoded_size` is the most bytes of samples the caller accepts from one chunk; a larger
/// chunk is an error ([`Error::TooLarge`]). 0 sets no limit of the caller's.
/// [`MAX_DECODED_SIZE`] applies in every case. The file header's audio size field
/// (`AudioTrack::size_hint`) is not such a limit: chunks of real files decode to more.
///
/// A chunk declares its own size and can code a sample in no bits, so a chunk of a few bytes
/// can decode to the full limit, and a file can hold one such chunk for each frame and track. A
/// caller that reads files it does not trust and keeps what it decodes bounds the total itself,
/// or passes a limit that suits the track, such as a few seconds of samples.
pub fn decode(chunk: &[u8], max_decoded_size: u32) -> Result<Chunk> {
    let (size, data) = chunk.split_first_chunk::<4>().ok_or(Error::Truncated)?;
    let size = u32::from_le_bytes(*size);
    let mut r = LsbBitReader::new(data);
    if !r.read_bit()? {
        return Ok(Chunk { channels: 1, samples: Samples::U8(Vec::new()) });
    }
    let stereo = r.read_bit()?;
    let sixteen_bit = r.read_bit()?;
    let channels = if stereo { 2usize } else { 1 };
    if (max_decoded_size != 0 && size > max_decoded_size) || size as usize > MAX_DECODED_SIZE {
        return Err(Error::TooLarge(size));
    }
    let bytes_per_sample = if sixteen_bit { 2 } else { 1 };
    let frame_bytes = channels * bytes_per_sample;
    // Validate bytes before division so a partial 16-bit sample cannot be discarded.
    if (size as usize) < frame_bytes || !(size as usize).is_multiple_of(frame_bytes) {
        return Err(Error::Invalid("sample count"));
    }

    let count = size as usize / bytes_per_sample;

    // a tree for each byte of a sample, for each channel: left low, left high, right low, right high
    let mut trees: [Option<ByteTree>; 4] = [None, None, None, None];
    let tree_count = channels * if sixteen_bit { 2 } else { 1 };
    for t in trees.iter_mut().take(tree_count) {
        *t = Some(ByteTree::read_optional(&mut r)?);
    }
    let symbol = |i: usize, r: &mut LsbBitReader<'_>| -> Result<u8> {
        let tree = trees.get(i).and_then(Option::as_ref).ok_or(Error::Invalid("missing tree"))?;
        Ok(tree.decode(r)?)
    };

    // room for the samples the data can hold at a bit each; samples of no bits grow the buffer
    let capacity = capacity_for(count, r.bits_left(), channels);
    let samples = if sixteen_bit {
        // the first samples: the last channel first, each high byte first
        let mut pred = [0i16; 2];
        for p in pred.iter_mut().take(channels).rev() {
            let high = r.read_bits(8)?;
            let low = r.read_bits(8)?;
            *p = (high << 8 | low) as u16 as i16;
        }
        let mut out = Vec::with_capacity(capacity);
        out.extend_from_slice(&pred[..channels]);
        for i in channels..count {
            let ch = i % channels;
            let low = symbol(2 * ch, &mut r)?;
            let high = symbol(2 * ch + 1, &mut r)?;
            pred[ch] = pred[ch].wrapping_add(i16::from_le_bytes([low, high]));
            out.push(pred[ch]);
        }
        Samples::S16(out)
    } else {
        let mut pred = [0u8; 2];
        for p in pred.iter_mut().take(channels).rev() {
            *p = r.read_bits(8)? as u8;
        }
        let mut out = Vec::with_capacity(capacity);
        out.extend_from_slice(&pred[..channels]);
        for i in channels..count {
            let ch = i % channels;
            pred[ch] = pred[ch].wrapping_add(symbol(ch, &mut r)?);
            out.push(pred[ch]);
        }
        Samples::U8(out)
    };
    Ok(Chunk { channels: channels as u16, samples })
}

/// Reads one chunk of an uncompressed track (`AudioCoding::Pcm`): unsigned 8-bit or signed
/// 16-bit little-endian samples, interleaved. Trailing bytes that do not make up a sample of
/// every channel are dropped.
pub fn pcm(chunk: &[u8], sixteen_bit: bool, stereo: bool) -> Chunk {
    let channels = if stereo { 2 } else { 1 };
    let frame = channels * if sixteen_bit { 2 } else { 1 };
    let whole = chunk.get(..chunk.len() - chunk.len() % frame).unwrap_or(chunk);
    let samples = if sixteen_bit {
        Samples::S16(whole.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect())
    } else {
        Samples::U8(whole.to_vec())
    };
    Chunk { channels: channels as u16, samples }
}

/// The buffer to reserve for `count` samples when `bits` bits of data are left: room for the
/// first sample of each channel and a sample for each bit.
fn capacity_for(count: usize, bits: usize, channels: usize) -> usize {
    count.min(bits.saturating_add(channels))
}
