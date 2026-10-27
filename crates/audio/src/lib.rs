#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![warn(missing_docs)]
//! Audio file I/O and codecs for the Craft apps. Input and output are byte slices or a
//! [`craft_media::ByteSource`] (no filesystem access of its own); builds for
//! `wasm32-unknown-unknown`.
//!
//! - [`decode`], [`probe`], [`Decoder`] (streaming, seekable), [`tracks`], [`detect_container`]
//!   (feature `decode`): WAV and AIFF are read natively; AAC, MP1/MP2, AC-3/E-AC-3, Opus and PCM
//!   in MP4/MOV, Matroska/WebM, Ogg or raw streams go through craft-media's demuxers and
//!   [`TrackDecoder`]; everything else (MP3, FLAC, ALAC, Vorbis, CAF, ADPCM, A-law, µ-law) through
//!   `symphonia`, which is to be replaced by native decoders. Feature `decode-native` is the same
//!   without symphonia.
//! - [`TrackDecoder`] (feature `decode-native`): one track's packets from any demuxer to planar
//!   `f32` in WAV channel order.
//! - [`encode`], [`FileEncoder`]: WAV/BWF/RF64 and AIFF/AIFF-C (feature `pcm-encode`, which also
//!   gives [`wav_float_header`]) and FLAC (feature `flac-encode`), whole-buffer or incremental into
//!   any `Write + Seek`. Feature `lossless-encode` turns on both; feature `encode` turns on
//!   `lossless-encode`, `aac-encode` and `opus-encode`.
//! - [`peaks`] (feature `peaks`): min/max/RMS pyramid for waveform drawing.
//! - Codec modules for packet-level use, each with its own features: [`aac`] (`aac-decode`,
//!   `aac-encode`), [`mpa`] (`mpa-decode`), [`ac3`] (`ac3-decode`), [`opus`] (`opus-decode`, and
//!   [`opus::encoder`] with `opus-encode`).
//!
//! The shared types below build with no features.

#[cfg(any(feature = "aac-decode", feature = "aac-encode"))]
pub mod aac;
#[cfg(feature = "ac3-decode")]
pub mod ac3;
mod buffer;
mod layout;
#[cfg(feature = "mpa-decode")]
pub mod mpa;
#[cfg(any(feature = "opus-decode", feature = "opus-encode"))]
pub mod opus;
#[cfg(feature = "peaks")]
pub mod peaks;
#[cfg(feature = "decode-native")]
mod read;
#[cfg(any(feature = "decode-native", feature = "pcm-encode"))]
mod wav;
#[cfg(any(feature = "pcm-encode", feature = "flac-encode"))]
mod write;

pub use buffer::AudioBuffer;
pub use craft_media::{CodecId, CodecParams, Container, PcmFormat};
pub use layout::{ChannelLayout, Speaker};
#[cfg(feature = "decode-native")]
pub use read::{
    DEFAULT_MAX_SAMPLES, DecodeOptions, Decoder, LOOKAHEAD_PACKETS, TrackDecoder, TrackInfo, decode, decode_with, detect_container, probe,
    probe_with, tracks, tracks_of,
};
#[cfg(any(feature = "pcm-encode", feature = "flac-encode"))]
pub use write::{ENCODE_EXTENSIONS, FileEncoder, encode, encode_format_for, encode_with_layout};
#[cfg(feature = "pcm-encode")]
pub use write::{encode_wav_rf64, wav_float_header};

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

// Codec errors convert into `AudioError`, so code mixing the file API with the codec modules can
// use `?` throughout: a codec's "unsupported" stays `Unsupported`, anything else in the input is
// `Malformed`, and an encoder's refusal is `Encode`.

#[cfg(any(feature = "aac-decode", feature = "aac-encode"))]
impl From<aac::Error> for AudioError {
    fn from(e: aac::Error) -> Self {
        match e {
            aac::Error::Unsupported(_) => AudioError::Unsupported(e.to_string()),
            _ => AudioError::Malformed(e.to_string()),
        }
    }
}

#[cfg(feature = "mpa-decode")]
impl From<mpa::Error> for AudioError {
    fn from(e: mpa::Error) -> Self {
        match e {
            mpa::Error::Unsupported(_) => AudioError::Unsupported(e.to_string()),
            _ => AudioError::Malformed(e.to_string()),
        }
    }
}

#[cfg(feature = "ac3-decode")]
impl From<ac3::Error> for AudioError {
    fn from(e: ac3::Error) -> Self {
        match e {
            ac3::Error::Unsupported(_) => AudioError::Unsupported(e.to_string()),
            _ => AudioError::Malformed(e.to_string()),
        }
    }
}

#[cfg(feature = "opus-decode")]
impl From<opus::Error> for AudioError {
    fn from(e: opus::Error) -> Self {
        match e {
            opus::Error::Unsupported(_) => AudioError::Unsupported(e.to_string()),
            _ => AudioError::Malformed(e.to_string()),
        }
    }
}

#[cfg(feature = "opus-encode")]
impl From<opus::encoder::Error> for AudioError {
    fn from(e: opus::encoder::Error) -> Self {
        AudioError::Encode(e.to_string())
    }
}

/// The file formats [`encode`] writes. Each needs its feature: WAV and AIFF `pcm-encode`, FLAC
/// `flac-encode`; asking for one that is not built returns [`AudioError::Unsupported`].
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
    /// Integer PCM, 1-8 bits.
    Int8,
    /// Integer PCM, 9-16 bits.
    Int16,
    /// Integer PCM, 17-24 bits.
    Int24,
    /// Integer PCM, 25-32 bits.
    Int32,
    /// 32-bit IEEE float.
    Float32,
    /// 64-bit IEEE float.
    Float64,
    /// A lossy codec (MP3, AAC, Vorbis, ...): no fixed bit depth.
    Compressed,
}

/// Broadcast Wave (`bext`) metadata.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BwfInfo {
    /// `Description` (up to 256 bytes).
    pub description: String,
    /// `Originator` (up to 32 bytes).
    pub originator: String,
    /// `OriginatorReference` (up to 32 bytes).
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
    /// The stored sample representation.
    pub sample_format: SampleFormat,
    /// The decoded sample rate (for HE-AAC, the rate after spectral band replication).
    pub sample_rate: u32,
    /// Decoded channel count.
    pub channels: u16,
    /// Which speaker each decoded channel feeds. Decoded audio is in WAV channel order when the
    /// layout is ordered.
    pub layout: ChannelLayout,
    /// Length in frames (samples per channel), after container trimming.
    pub frames: u64,
    /// Broadcast Wave metadata from a WAV `bext` chunk.
    pub bwf: Option<BwfInfo>,
}

impl AudioInfo {
    /// Info for a stream with `layout`'s channels and no BWF metadata.
    #[cfg(feature = "decode-native")]
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
    /// 16-bit integer.
    Int16,
    /// 24-bit integer.
    Int24,
    /// 32-bit integer.
    Int32,
    /// Preserve finite samples, including values above full scale; map NaN to 0 and infinities to ±1.
    Float32,
}

/// Options for `encode`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EncodeOptions {
    /// Output file format.
    pub format: EncodeFormat,
    /// Sample depth to write.
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
