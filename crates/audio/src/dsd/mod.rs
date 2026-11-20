//! Direct Stream Digital (DSD) audio: Sony DSF and Philips DSDIFF (`.dff`) readers and
//! DSD-to-PCM decimation.
//!
//! DSD stores 1-bit Pulse Density Modulation (PDM) sampled at high rates (e.g. 2,822,400 Hz
//! for DSD64, 5,644,800 Hz for DSD128, etc.).
//!
//! This module provides:
//! - File parsers for Sony DSF (`.dsf`) and Philips DSDIFF (`.dff`) containers.
//! - Raw 1-bit audio extraction with preservation of native bit ordering (LSB-first for DSF,
//!   MSB-first for DSDIFF).
//! - Clean-room 96-tap symmetric FIR decimation filter converting 1-bit PDM to multi-bit PCM
//!   (decimation by 8 to 352,800 Hz for DSD64, or downsampled to 176.4, 88.2, or 44.1 kHz).
//! - Reference encoders for generating synthetic DSF and DSDIFF files.

pub mod dsdiff;
pub mod dsf;
pub mod filter;

use crate::AudioBuffer;

/// Maximum number of audio channels supported (1 to 8).
pub const MAX_CHANNELS: usize = 8;

/// Minimum allowable DSD sample rate in Hz.
pub const MIN_SAMPLE_RATE: u32 = 44_100;

/// Maximum allowable DSD sample rate in Hz (e.g. DSD1024 at 45.1584 MHz).
pub const MAX_SAMPLE_RATE: u32 = 45_158_400;

/// Maximum safe block size for DSF channel blocks (16 MiB).
pub const MAX_BLOCK_SIZE: u32 = 16 * 1024 * 1024;

/// Maximum audio-buffer allocation budget (256 MiB).
///
/// Raw readers limit their channel bytes; PCM conversion separately limits the
/// simultaneously live PCM output and scratch buffers. Caller-owned raw input,
/// metadata, and the small filter lookup table are not included.
pub const MAX_DECODE_BYTES: usize = 256 * 1024 * 1024;

/// Maximum encoded container size, including headers and block padding (256 MiB).
pub const MAX_ENCODE_BYTES: usize = 256 * 1024 * 1024;

fn encoded_size(payload: usize, header: usize) -> Result<usize, Error> {
    payload.checked_add(header).filter(|&n| n <= MAX_ENCODE_BYTES).ok_or(Error::MemoryLimitExceeded)
}

// Account for retained channel output plus the current filter's two live buffers.
fn pcm_budget(lengths: impl Iterator<Item = usize>, factor: u32) -> Result<(), Error> {
    let mut retained = 0usize;
    for len in lengths {
        let (working, output) = filter::allocation_sizes(len, factor)?;
        if retained.checked_add(working).is_none_or(|n| n > MAX_DECODE_BYTES) {
            return Err(Error::MemoryLimitExceeded);
        }
        retained = retained.checked_add(output).ok_or(Error::MemoryLimitExceeded)?;
    }
    Ok(())
}

/// Supported DSD container formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// Sony DSF (Direct Stream Digital File) format.
    Dsf,
    /// Philips DSDIFF (Direct Stream Digital Interchange File Format, `.dff`).
    Dsdiff,
}

/// Bit ordering of 1-bit DSD audio samples within bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BitOrder {
    /// Least-significant bit first (LSB-first, as used in DSF).
    /// Bit 0 is the earliest temporal sample; bit 7 is the latest.
    LsbFirst,
    /// Most-significant bit first (MSB-first, as used in DSDIFF).
    /// Bit 7 is the earliest temporal sample; bit 0 is the latest.
    MsbFirst,
}

/// Stream properties and metadata parsed from a DSD container header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    /// Container format (DSF or DSDIFF).
    pub format: Format,
    /// DSD sampling frequency in Hz (e.g. 2,822,400 for DSD64).
    pub sample_rate: u32,
    /// Number of audio channels (1 to 8).
    pub channels: u16,
    /// Total number of 1-bit audio samples per channel.
    pub sample_count: u64,
    /// Channel identification labels (e.g. "SLFT", "SRGT", "Center").
    pub channel_names: Vec<String>,
    /// Optional track title if present in container metadata.
    pub title: Option<String>,
    /// Optional artist name if present in container metadata.
    pub artist: Option<String>,
}

/// Raw 1-bit DSD audio extracted from a file without decimation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawAudio {
    /// Stream metadata from the file header.
    pub metadata: Metadata,
    /// Bit ordering of the bytes in `channel_bytes`.
    pub bit_order: BitOrder,
    /// Planar 1-bit audio bytes per channel.
    pub channel_bytes: Vec<Vec<u8>>,
}

impl RawAudio {
    /// Converts all channel byte streams to the requested bit ordering in place.
    pub fn convert_bit_order(&mut self, target: BitOrder) {
        if self.bit_order == target {
            return;
        }
        for ch in &mut self.channel_bytes {
            for b in ch.iter_mut() {
                *b = b.reverse_bits();
            }
        }
        self.bit_order = target;
    }

    /// Decimates the raw 1-bit DSD audio to multi-bit PCM `f32` samples.
    ///
    /// The default decimation factor is 8 (e.g. 352,800 Hz for DSD64).
    pub fn to_pcm(&self) -> Result<DecodedAudio, Error> {
        self.to_pcm_with_factor(8)
    }

    /// Decimates the raw 1-bit DSD audio to multi-bit PCM `f32` samples with the given factor (8, 16, 32, or 64).
    pub fn to_pcm_with_factor(&self, factor: u32) -> Result<DecodedAudio, Error> {
        // Validate before iterating: public RawAudio may contain no channels.
        if !matches!(factor, 8 | 16 | 32 | 64) {
            return Err(Error::Unsupported("decimation factor must be 8, 16, 32, or 64"));
        }
        if self.channel_bytes.len() > MAX_CHANNELS {
            return Err(Error::Unsupported("channel count must be between 1 and 8"));
        }
        pcm_budget(self.channel_bytes.iter().map(Vec::len), factor)?;
        let mut channels = Vec::with_capacity(self.channel_bytes.len());
        for ch_bytes in &self.channel_bytes {
            let pcm = filter::decimate_channel(ch_bytes, self.bit_order, factor)?;
            channels.push(pcm);
        }
        let sample_rate = self.metadata.sample_rate / factor;
        Ok(DecodedAudio { metadata: self.metadata.clone(), sample_rate, channels })
    }
}

/// Decoded multi-bit PCM audio produced by DSD-to-PCM decimation.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedAudio {
    /// Stream metadata from the DSD file header.
    pub metadata: Metadata,
    /// PCM sample rate in Hz (e.g. 352,800 Hz for DSD64 decimated by 8).
    pub sample_rate: u32,
    /// Planar normalized PCM samples in `[-1.0, 1.0]`, one `Vec<f32>` per channel.
    pub channels: Vec<Vec<f32>>,
}

impl DecodedAudio {
    /// Converts the decoded PCM audio into an [`AudioBuffer`].
    pub fn to_audio_buffer(&self) -> AudioBuffer {
        AudioBuffer { sample_rate: self.sample_rate, channels: self.channels.clone() }
    }
}

/// Errors returned when parsing or decoding DSD audio.
#[non_exhaustive]
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// Input data ended prematurely while parsing a header or chunk.
    #[error("unexpected end of file: {0}")]
    UnexpectedEof(&'static str),
    /// Invalid magic bytes or malformed chunk header.
    #[error("invalid DSD header or chunk: {0}")]
    InvalidHeader(&'static str),
    /// An unsupported feature, compression, or channel configuration.
    #[error("unsupported DSD feature: {0}")]
    Unsupported(&'static str),
    /// Audio data length or structure does not match the header specification.
    #[error("invalid audio data: {0}")]
    InvalidData(&'static str),
    /// Header or data requested exceeds maximum safe allocation budget.
    #[error("memory budget exceeded")]
    MemoryLimitExceeded,
}

/// Inspects the magic bytes of `bytes` and detects whether it is a DSF or DSDIFF file.
pub fn probe_format(bytes: &[u8]) -> Result<Format, Error> {
    if bytes.len() < 4 {
        return Err(Error::UnexpectedEof("input is too short to determine DSD format"));
    }
    if &bytes[0..4] == b"DSD " {
        Ok(Format::Dsf)
    } else if bytes.len() >= 16 && &bytes[0..4] == b"FRM8" && &bytes[12..16] == b"DSD " {
        Ok(Format::Dsdiff)
    } else {
        Err(Error::InvalidHeader("unknown or unsupported DSD container format"))
    }
}

/// Parses the metadata from a DSF or DSDIFF file without decoding audio samples.
pub fn read_metadata(bytes: &[u8]) -> Result<Metadata, Error> {
    match probe_format(bytes)? {
        Format::Dsf => {
            let (meta, _, _) = dsf::parse_dsf_header(bytes)?;
            Ok(meta)
        }
        Format::Dsdiff => {
            let (meta, _, _) = dsdiff::parse_dsdiff_header(bytes)?;
            Ok(meta)
        }
    }
}

/// Reads raw 1-bit planar audio streams from a DSF or DSDIFF file without decimation.
pub fn read_raw(bytes: &[u8]) -> Result<RawAudio, Error> {
    match probe_format(bytes)? {
        Format::Dsf => dsf::parse_dsf(bytes),
        Format::Dsdiff => dsdiff::parse_dsdiff(bytes),
    }
}

/// Decodes a DSF or DSDIFF file to multi-bit planar `f32` PCM audio using decimation by 8.
///
/// For DSD64 (2,822,400 Hz), the output sample rate is 352,800 Hz.
pub fn decode(bytes: &[u8]) -> Result<DecodedAudio, Error> {
    decode_with_factor(bytes, 8)
}

/// Decodes a DSF or DSDIFF file to multi-bit planar `f32` PCM audio with the specified decimation factor.
///
/// Supported decimation factors are:
/// - 8: 1/8th of DSD rate (e.g. 352,800 Hz for DSD64).
/// - 16: 1/16th of DSD rate (e.g. 176,400 Hz for DSD64).
/// - 32: 1/32th of DSD rate (e.g. 88,200 Hz for DSD64).
/// - 64: 1/64th of DSD rate (e.g. 44,100 Hz for DSD64).
pub fn decode_with_factor(bytes: &[u8], factor: u32) -> Result<DecodedAudio, Error> {
    let raw = read_raw(bytes)?;
    raw.to_pcm_with_factor(factor)
}

/// Encodes planar 1-bit audio channels into a Sony DSF file.
pub fn encode_dsf(sample_rate: u32, channels: &[&[u8]], sample_count: u64, block_size: u32) -> Result<Vec<u8>, Error> {
    dsf::write_dsf(sample_rate, channels, sample_count, block_size)
}

/// Encodes planar 1-bit audio channels into a Philips DSDIFF file.
pub fn encode_dsdiff(sample_rate: u32, channels: &[&[u8]], sample_count: u64) -> Result<Vec<u8>, Error> {
    dsdiff::write_dsdiff(sample_rate, channels, sample_count)
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn pcm_budget_counts_all_channels_and_live_scratch() {
        let quarter = MAX_DECODE_BYTES / 4;
        assert!(pcm_budget([quarter / 2, quarter / 2].into_iter(), 8).is_ok());
        assert_eq!(pcm_budget([quarter / 2, quarter / 2 + 1].into_iter(), 8), Err(Error::MemoryLimitExceeded));
        assert!(pcm_budget([quarter / 2].into_iter(), 16).is_ok());
        assert_eq!(pcm_budget([quarter].into_iter(), 16), Err(Error::MemoryLimitExceeded));
        assert_eq!(pcm_budget([usize::MAX].into_iter(), 8), Err(Error::MemoryLimitExceeded));
    }

    #[test]
    fn encoded_budget_includes_headers_without_large_allocations() {
        assert_eq!(encoded_size(MAX_ENCODE_BYTES - 92, 92), Ok(MAX_ENCODE_BYTES));
        assert_eq!(encoded_size(MAX_ENCODE_BYTES - 91, 92), Err(Error::MemoryLimitExceeded));
        assert_eq!(encoded_size(usize::MAX, 92), Err(Error::MemoryLimitExceeded));
    }
}
