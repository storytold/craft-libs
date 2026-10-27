#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Audio file I/O and codecs for the Craft apps. Input and output are byte slices (no filesystem
//! access); builds for `wasm32-unknown-unknown`.
//!
//! - [`detect_format`], [`probe`], [`decode`] (feature `decode`): WAV and AIFF are read natively;
//!   AAC, MP1/MP2, AC-3/E-AC-3, Opus and PCM in MP4/MOV, Matroska/WebM, Ogg or raw streams go
//!   through craft-media's demuxers and this crate's decoders; everything else (MP3, FLAC, ALAC,
//!   Vorbis, CAF, ADPCM, A-law, µ-law) through `symphonia`, which is to be replaced by native
//!   decoders.
//! - [`encode`], [`FileEncoder`], [`wav_float_header`] (feature `encode`): WAV/BWF/RF64,
//!   AIFF/AIFF-C and FLAC, whole-buffer or incremental into any `Write + Seek`.
//! - [`peaks`] (feature `peaks`): min/max/RMS pyramid for waveform drawing.
//! - Codec modules for packet-level use: [`aac`] (AAC-LC decoder and encoder), [`mpa`] (MP1/MP2
//!   decoder), [`ac3`] (AC-3/E-AC-3 decoder), [`opus`] (decoder and [`opus::encoder`]).
//!
//! The shared types below build with no features.

#[cfg(any(feature = "decode", feature = "encode"))]
pub mod aac;
#[cfg(feature = "decode")]
pub mod ac3;
mod buffer;
mod layout;
#[cfg(feature = "decode")]
pub mod mpa;
#[cfg(any(feature = "decode", feature = "encode"))]
pub mod opus;
#[cfg(feature = "peaks")]
pub mod peaks;
#[cfg(feature = "decode")]
mod read;
#[cfg(any(feature = "decode", feature = "encode"))]
mod wav;
#[cfg(feature = "encode")]
mod write;

pub use buffer::AudioBuffer;
pub use craft_media::{CodecId, Container};
pub use layout::{ChannelLayout, Speaker};
#[cfg(feature = "decode")]
pub use read::{decode, detect_container, probe};
#[cfg(feature = "encode")]
pub use write::{ENCODE_EXTENSIONS, FileEncoder, encode, encode_format_for, encode_wav_rf64, encode_with_layout, wav_float_header};

/// Errors from probing, decoding and encoding.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// The format, codec or option is valid but not supported.
    #[error("unsupported audio: {0}")]
    Unsupported(String),
    /// The input is corrupt or not what it claims to be.
    #[error("malformed audio: {0}")]
    Malformed(String),
    /// The input (or requested output) exceeds a size limit.
    #[error("audio too large: {0}")]
    TooLarge(String),
    /// Encoding failed (e.g. an empty buffer).
    #[error("audio encode failed: {0}")]
    Encode(String),
    /// Writing the output failed.
    #[error("audio write failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, AudioError>;

/// The file formats [`encode`] writes.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EncodeFormat {
    /// WAV / BWF, RF64 above 4 GiB.
    Wav,
    /// AIFF (integer depths) or AIFF-C `fl32` (float).
    Aiff,
    /// FLAC (integer depths).
    Flac,
}

impl EncodeFormat {
    /// The container this format writes.
    pub fn container(self) -> Container {
        match self {
            EncodeFormat::Wav => Container::Wav,
            EncodeFormat::Aiff => Container::Aiff,
            EncodeFormat::Flac => Container::Flac,
        }
    }
}

/// Stored sample representation.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SampleFormat {
    Int8,
    Int16,
    Int24,
    Int32,
    Float32,
    Float64,
    /// A lossy codec (MP3, AAC, Vorbis, ...): no fixed bit depth.
    Compressed,
}

/// Broadcast Wave (`bext`) metadata.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BwfInfo {
    pub description: String,
    pub originator: String,
    pub originator_reference: String,
    /// `yyyy-mm-dd`.
    pub origination_date: String,
    /// `hh:mm:ss`.
    pub origination_time: String,
    /// Sample count since midnight of the first sample (the timestamp Pro Tools spots to).
    pub time_reference: u64,
}

/// What `probe` learns about a file without decoding all of it.
///
/// `#[non_exhaustive]`: fields may be added, so it is only built by this crate.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AudioInfo {
    /// The file format around the audio.
    pub container: Container,
    /// The codec of the audio track ([`CodecId::Pcm`] for uncompressed audio).
    pub codec: CodecId,
    pub sample_format: SampleFormat,
    /// The decoded sample rate (for HE-AAC, the rate after spectral band replication).
    pub sample_rate: u32,
    pub channels: u16,
    /// Which speaker each decoded channel feeds. Decoded audio is in WAV channel order when the
    /// layout is ordered.
    pub layout: ChannelLayout,
    /// Length in frames (samples per channel), after container trimming.
    pub frames: u64,
    pub bwf: Option<BwfInfo>,
}

impl AudioInfo {
    /// Info for a stream with `layout`'s channels and no BWF metadata.
    #[cfg(feature = "decode")]
    pub(crate) fn new(
        container: Container,
        codec: CodecId,
        sample_format: SampleFormat,
        sample_rate: u32,
        layout: ChannelLayout,
        frames: u64,
    ) -> AudioInfo {
        let channels = u16::try_from(layout.channels()).unwrap_or(u16::MAX);
        AudioInfo { container, codec, sample_format, sample_rate, channels, layout, frames, bwf: None }
    }

    /// Duration in seconds.
    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 { 0.0 } else { self.frames as f64 / f64::from(self.sample_rate) }
    }
}

/// Bit depth for `encode`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BitDepth {
    Int16,
    Int24,
    Int32,
    /// Preserve finite samples, including values above full scale; map NaN to 0 and infinities to ±1.
    Float32,
}

/// Options for `encode`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EncodeOptions {
    pub format: EncodeFormat,
    pub bit_depth: BitDepth,
    /// Add TPDF dither when quantizing to an integer depth.
    pub dither: bool,
    /// Write a BWF `bext` chunk (WAV only; ignored for other formats).
    pub bwf: Option<BwfInfo>,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self { format: EncodeFormat::Wav, bit_depth: BitDepth::Int24, dither: false, bwf: None }
    }
}
