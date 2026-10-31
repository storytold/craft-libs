#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![warn(missing_docs)]
// The docs link items of every feature; built with fewer features, those links have no target.
// CI and docs.rs build the docs with all features, where every link must resolve.
#![cfg_attr(not(all(feature = "decode", feature = "encode", feature = "peaks")), allow(rustdoc::broken_intra_doc_links))]
//! Audio file I/O and codecs for the Craft apps. Input and output are byte slices or a
//! [`craft_media::ByteSource`] (no filesystem access of its own); builds for
//! `wasm32-unknown-unknown`.
//!
//! - [`decode`], [`probe`], [`Decoder`] (streaming, seekable), [`tracks`], [`detect_container`]
//!   (feature `decode`): WAV and AIFF are read natively; AAC, AC-3/E-AC-3, Opus and PCM in
//!   MP4/MOV, Matroska/WebM, Ogg or raw streams go through craft-media's demuxers and
//!   [`TrackDecoder`]; everything else (MP1/MP2/MP3, FLAC, ALAC, Vorbis, CAF, ADPCM, A-law,
//!   µ-law) through `symphonia`, which is to be replaced by native decoders. Feature
//!   `decode-native` is the same without symphonia.
//! - [`TrackDecoder`] (any decoder feature): one track's packets from any demuxer to planar
//!   `f32` in WAV channel order.
//! - [`encode`], [`FileEncoder`]: WAV/BWF/RF64 and AIFF/AIFF-C (feature `pcm-encode`, which also
//!   gives [`wav_float_header`]) and FLAC (feature `flac-encode`), whole-buffer or incremental into
//!   any `Write + Seek`. Feature `lossless-encode` turns on both; feature `encode` turns on
//!   `lossless-encode`, `aac-encode` and `opus-encode`.
//! - [`peaks`] (feature `peaks`): min/max/RMS pyramid for waveform drawing.
//! - Codec modules for packet-level use, each with its own features: [`aac`] (`aac-decode`,
//!   `aac-encode`), [`ac3`] (`ac3-decode`), [`opus`] (`opus-decode`, and
//!   [`opus::encoder`] with `opus-encode`).
//!
//! The shared types below build with no features.

#[cfg(any(feature = "aac-decode", feature = "aac-encode"))]
pub mod aac;
#[cfg(feature = "ac3-decode")]
pub mod ac3;
mod buffer;
mod layout;
#[cfg(any(feature = "opus-decode", feature = "opus-encode"))]
pub mod opus;
#[cfg(any(feature = "aac-decode", feature = "opus-decode", feature = "ac3-decode"))]
mod pcm;
#[cfg(feature = "peaks")]
pub mod peaks;
#[cfg(feature = "decode-native")]
mod read;
#[cfg(any(feature = "aac-decode", feature = "opus-decode", feature = "ac3-decode"))]
mod track;
#[cfg(any(feature = "decode-native", feature = "pcm-encode"))]
mod wav;
#[cfg(any(feature = "pcm-encode", feature = "flac-encode"))]
mod write;

pub use buffer::AudioBuffer;
/// The craft-media this crate is built with, for its containers: using it rather than a separate
/// dependency keeps the types the two crates share identical.
pub use craft_media;
pub use craft_media::{ByteSource, CodecId, CodecParams, Container, PcmFormat, ReadSeekSource};
pub use layout::{ChannelLayout, Speaker};
#[cfg(feature = "decode-native")]
pub use read::SharedSource;
#[cfg(feature = "decode-native")]
pub use read::{DEFAULT_MAX_SAMPLES, DecodeOptions, Decoder, TrackInfo, decode, decode_with, detect_container, probe, probe_with, tracks, tracks_of};
#[cfg(any(feature = "aac-decode", feature = "opus-decode", feature = "ac3-decode"))]
pub use track::{LOOKAHEAD_PACKETS, TrackDecoder};
#[cfg(feature = "pcm-encode")]
pub use write::wav_float_header;
#[cfg(any(feature = "pcm-encode", feature = "flac-encode"))]
pub use write::{ENCODE_EXTENSIONS, FileEncoder, encode, encode_format_for};

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
    /// Reading the input or writing the output failed (a source that stopped serving reads, a
    /// full disk). Bytes missing because the input is truncated are [`AudioError::Malformed`].
    #[error("audio I/O failed: {0}")]
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
            aac::Error::InvalidSettings(_) => AudioError::Encode(e.to_string()),
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
///
/// `#[non_exhaustive]`: build it with `BwfInfo::default()` and the `with_*` methods.
#[non_exhaustive]
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

impl BwfInfo {
    /// Sets `description`.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Sets `originator`.
    pub fn with_originator(mut self, originator: impl Into<String>) -> Self {
        self.originator = originator.into();
        self
    }

    /// Sets `originator_reference`.
    pub fn with_originator_reference(mut self, reference: impl Into<String>) -> Self {
        self.originator_reference = reference.into();
        self
    }

    /// `date` as `yyyy-mm-dd`, `time` as `hh:mm:ss`.
    pub fn with_origination(mut self, date: impl Into<String>, time: impl Into<String>) -> Self {
        (self.origination_date, self.origination_time) = (date.into(), time.into());
        self
    }

    /// Sets `time_reference`.
    pub fn with_time_reference(mut self, time_reference: u64) -> Self {
        self.time_reference = time_reference;
        self
    }
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
    /// Packets that were damaged or failed to decode and were replaced by concealment (silence of
    /// their length), counted as audio is decoded: by [`decode`] for the whole file, and so far by
    /// a [`Decoder`]. 0 from [`probe`] when it read only headers.
    pub damaged: u64,
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
        let channels = layout.channels();
        AudioInfo { container, codec, sample_format, sample_rate, channels, layout, frames, bwf: None, damaged: 0 }
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

/// Options for `encode` and `FileEncoder`.
///
/// `#[non_exhaustive]`: build it with [`EncodeOptions::new`] and the `with_*` methods.
///
/// ```
/// use craft_audio::{BitDepth, EncodeFormat, EncodeOptions};
///
/// let opts = EncodeOptions::new(EncodeFormat::Flac).with_bit_depth(BitDepth::Int16).with_dither(true);
/// assert_eq!(opts.format, EncodeFormat::Flac);
/// ```
#[non_exhaustive]
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
    /// The speaker layout to record, for layouts a channel count alone cannot identify (5.1 with
    /// side vs back surrounds, 7.0 vs 6.1, Ambisonics as [`ChannelLayout::unordered`]). `None` is
    /// the usual layout for the channel count. WAV stores it as `dwChannelMask`; AIFF and FLAC
    /// have fixed channel orders and ignore it. It must have as many channels as the audio.
    pub layout: Option<ChannelLayout>,
    /// Write WAV in the RF64 layout even below 4 GiB (WAV only; ignored for other formats).
    pub force_rf64: bool,
}

impl EncodeOptions {
    /// Options for `format`: 24-bit, no dither, no BWF chunk, the usual layout.
    pub fn new(format: EncodeFormat) -> Self {
        Self { format, bit_depth: BitDepth::Int24, dither: false, bwf: None, layout: None, force_rf64: false }
    }

    /// Sets `format`.
    pub fn with_format(mut self, format: EncodeFormat) -> Self {
        self.format = format;
        self
    }

    /// Sets `bit_depth`.
    pub fn with_bit_depth(mut self, bit_depth: BitDepth) -> Self {
        self.bit_depth = bit_depth;
        self
    }

    /// Sets `dither`.
    pub fn with_dither(mut self, dither: bool) -> Self {
        self.dither = dither;
        self
    }

    /// Sets `bwf`.
    pub fn with_bwf(mut self, bwf: BwfInfo) -> Self {
        self.bwf = Some(bwf);
        self
    }

    /// Sets `layout`.
    pub fn with_layout(mut self, layout: ChannelLayout) -> Self {
        self.layout = Some(layout);
        self
    }

    /// Sets `force_rf64`.
    pub fn with_force_rf64(mut self, force_rf64: bool) -> Self {
        self.force_rf64 = force_rf64;
        self
    }
}

impl Default for EncodeOptions {
    /// 24-bit WAV.
    fn default() -> Self {
        Self::new(EncodeFormat::Wav)
    }
}

// An encoder refusing its settings is an encode error, not damaged input.
#[cfg(all(test, feature = "aac-encode"))]
mod tests {
    use super::*;

    #[test]
    fn aac_encoder_settings_are_encode_errors() {
        let e = match aac::Encoder::new(aac::EncoderConfig::cbr(44_000, 2, 128_000)) {
            Err(e) => AudioError::from(e),
            Ok(_) => panic!("44 kHz is not an AAC rate"),
        };
        assert!(matches!(e, AudioError::Encode(_)), "{e}");
    }
}
