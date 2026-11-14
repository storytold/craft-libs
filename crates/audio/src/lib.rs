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
//!   (feature `decode`): WAV and AIFF are read natively; AAC, ALAC, Opus, AC-3/E-AC-3,
//!   MP1/MP2/MP3 and PCM in MP4/MOV, Matroska/WebM, Ogg or raw streams go through craft-media's
//!   demuxers and [`TrackDecoder`]; everything else (FLAC, Vorbis, CAF, ADPCM, A-law, µ-law) through
//!   `symphonia`, which is to be replaced by native decoders. Feature `decode-native` is the
//!   same without symphonia.
//! - [`TrackDecoder`] (any decoder feature): one track's packets from any demuxer to planar
//!   `f32` in WAV channel order.
//! - [`encode`], [`FileEncoder`]: WAV/BWF/RF64 and AIFF/AIFF-C (feature `pcm-encode`, which also
//!   gives [`wav_float_header`]), FLAC (feature `flac-encode`), AAC-LC and ALAC in M4A
//!   (`aac-encode`, `alac-encode`) and MP3, AC-3 and E-AC-3 streams (`mp3-encode`, `ac3-encode`,
//!   `eac3-encode`), whole-buffer or incremental into any `Write + Seek`. Feature
//!   `lossless-encode` turns on WAV, AIFF and FLAC; feature `encode` turns on every format and encoder.
//! - [`wav_pcm`], [`aiff_pcm`] (feature `decode-native`) and [`pcm_wav_file`], [`pcm_wav_header`],
//!   [`pcm_aiff_file`], [`pcm_aiff_header`] (feature `pcm-encode`): the integer PCM of a WAV or AIFF
//!   file bit for bit, and minimal files around such PCM, for essence embedded in other containers.
//! - [`peaks`] (feature `peaks`): min/max/RMS pyramid for waveform drawing.
//! - Codec modules for packet-level use, each with its own features: [`aac`] (`aac-decode`,
//!   `aac-encode`), [`alac`] (`alac-decode`, `alac-encode`), [`opus`] (`opus-decode`, and
//!   [`opus::encoder`] with `opus-encode`), [`ac3`] (`ac3-decode`), [`mpa`] (`mpa-decode`), [`mp3`]
//!   (`mp3-decode`, `mp3-encode`), [`flac`] (`flac-encode`: the packet encoder).
//!
//! The shared types below build with no features.

#[cfg(any(feature = "aac-decode", feature = "aac-encode"))]
pub mod aac;
#[cfg(any(feature = "ac3-decode", feature = "ac3-encode", feature = "eac3-encode"))]
pub mod ac3;
#[cfg(any(feature = "alac-decode", feature = "alac-encode"))]
pub mod alac;
mod buffer;
mod layout;
#[cfg(any(feature = "mp3-encode", feature = "mp3-decode"))]
pub mod mp3;
#[cfg(feature = "mpa-decode")]
pub mod mpa;
#[cfg(any(feature = "opus-decode", feature = "opus-encode"))]
pub mod opus;
#[cfg(any(feature = "aac-decode", feature = "alac-decode", feature = "opus-decode", feature = "ac3-decode", feature = "mpa-decode"))]
mod pcm;
#[cfg(feature = "peaks")]
pub mod peaks;
#[cfg(feature = "decode-native")]
mod read;
#[cfg(any(feature = "aac-decode", feature = "alac-decode", feature = "opus-decode", feature = "ac3-decode", feature = "mpa-decode"))]
mod track;
#[cfg(any(feature = "decode-native", feature = "pcm-encode"))]
mod wav;
#[cfg(any(
    feature = "pcm-encode",
    feature = "flac-encode",
    feature = "aac-encode",
    feature = "alac-encode",
    feature = "mp3-encode",
    feature = "ac3-encode",
    feature = "eac3-encode"
))]
mod write;

/// The FLAC packet encoder (RFC 9639), for a caller that muxes FLAC itself (into MP4, Matroska or
/// its own `.flac` writer): planar integer samples in, frames out, and the STREAMINFO to patch in
/// at the end. [`FileEncoder`] and [`encode`] write whole `.flac` files from `f32` instead.
///
/// ```
/// use craft_audio::flac::{Encoder, EncoderConfig, STREAMINFO_OFFSET};
///
/// let left: Vec<i32> = (0..48_000).map(|i| ((i as f64 * 0.05).sin() * 20_000.0) as i32).collect();
/// let right = left.clone();
/// let mut enc = Encoder::new(EncoderConfig::new(48_000, 2, 16))?;
/// let mut file = enc.header(&[]);
/// file.extend(enc.encode(&[&left, &right])?);
/// file.extend(enc.finish());
/// let at = STREAMINFO_OFFSET as usize;
/// file[at..at + 34].copy_from_slice(&enc.streaminfo());
/// assert_eq!(&file[..4], b"fLaC");
/// assert!(file.len() < 48_000 * 2 * 2 / 2); // well under half of 16-bit PCM
/// # Ok::<(), craft_audio::flac::Error>(())
/// ```
#[cfg(feature = "flac-encode")]
pub mod flac {
    pub use crate::write::flac::encoder::{Encoder, EncoderConfig, Error, STREAMINFO_OFFSET};
}

pub use buffer::AudioBuffer;
/// The craft-media this crate is built with, for its containers: using it rather than a separate
/// dependency keeps the types the two crates share identical.
pub use craft_media;
pub use craft_media::{ByteSource, CodecId, CodecParams, Container, PcmFormat, ReadSeekSource};
pub use layout::{ChannelLayout, Speaker};
#[cfg(feature = "decode-native")]
pub use read::SharedSource;
#[cfg(feature = "decode-native")]
pub use read::{
    DECODE_EXTENSIONS, DEFAULT_MAX_SAMPLES, DecodeOptions, Decoder, TrackInfo, decode, decode_with, detect_container, probe, probe_with, tracks,
    tracks_of,
};
#[cfg(feature = "decode-native")]
pub use read::{RawPcm, aiff_pcm, wav_pcm};
#[cfg(any(feature = "aac-decode", feature = "alac-decode", feature = "opus-decode", feature = "ac3-decode", feature = "mpa-decode"))]
pub use track::{LOOKAHEAD_PACKETS, TrackDecoder};
#[cfg(feature = "pcm-encode")]
pub use write::wav_float_header;
#[cfg(any(
    feature = "pcm-encode",
    feature = "flac-encode",
    feature = "aac-encode",
    feature = "alac-encode",
    feature = "mp3-encode",
    feature = "ac3-encode",
    feature = "eac3-encode"
))]
pub use write::{ENCODE_EXTENSIONS, FileEncoder, encode, encode_format_for};
#[cfg(feature = "pcm-encode")]
pub use write::{pcm_aiff_file, pcm_aiff_header, pcm_wav_file, pcm_wav_header};

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

#[cfg(feature = "opus-decode")]
impl From<opus::Error> for AudioError {
    fn from(e: opus::Error) -> Self {
        match e {
            opus::Error::Unsupported(_) => AudioError::Unsupported(e.to_string()),
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

#[cfg(any(feature = "ac3-encode", feature = "eac3-encode"))]
impl From<ac3::encoder::Error> for AudioError {
    fn from(e: ac3::encoder::Error) -> Self {
        AudioError::Encode(e.to_string())
    }
}

#[cfg(feature = "mp3-encode")]
impl From<mp3::Error> for AudioError {
    fn from(e: mp3::Error) -> Self {
        AudioError::Encode(e.to_string())
    }
}

#[cfg(feature = "flac-encode")]
impl From<flac::Error> for AudioError {
    fn from(e: flac::Error) -> Self {
        AudioError::Encode(format!("FLAC: {e}"))
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

#[cfg(any(feature = "alac-decode", feature = "alac-encode"))]
impl From<alac::Error> for AudioError {
    fn from(e: alac::Error) -> Self {
        match e {
            alac::Error::Unsupported(_) => AudioError::Unsupported(e.to_string()),
            alac::Error::InvalidInput(_) => AudioError::Encode(e.to_string()),
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
/// `flac-encode`, AAC `aac-encode`, ALAC `alac-encode`, MP3 `mp3-encode`, AC-3 `ac3-encode` and
/// E-AC-3 `eac3-encode`; asking for one that is not built returns [`AudioError::Unsupported`].
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
    /// AAC-LC in an M4A (MP4) file, 1–8 channels at one of the 13 AAC sample rates. The edit list
    /// skips the encoder's priming and ends at the last frame, so the file decodes to the length
    /// that was written.
    Aac,
    /// Apple Lossless in an M4A (MP4) file (integer depths), 1–8 channels.
    Alac,
    /// An MP3 stream: mono or stereo at 32, 44.1 or 48 kHz, or 16, 22.05 or 24 kHz.
    Mp3,
    /// An AC-3 stream: up to 5.1 at 32, 44.1 or 48 kHz.
    Ac3,
    /// An E-AC-3 stream: up to 7.1 at 32, 44.1 or 48 kHz.
    Eac3,
}

impl EncodeFormat {
    /// Every format, in the order menus list them. Whether [`encode`] writes one depends on its
    /// feature ([`EncodeFormat::is_built`]).
    pub const ALL: &[EncodeFormat] = &[
        EncodeFormat::Wav,
        EncodeFormat::Aiff,
        EncodeFormat::Flac,
        EncodeFormat::Alac,
        EncodeFormat::Aac,
        EncodeFormat::Mp3,
        EncodeFormat::Ac3,
        EncodeFormat::Eac3,
    ];

    /// The extension a new file of this format gets, lower case and without the dot: `"wav"`,
    /// `"aif"`, `"flac"`, `"m4a"` (AAC and ALAC), `"mp3"`, `"ac3"`, `"eac3"`.
    /// [`encode_format_for`] maps it back, `"m4a"` to AAC.
    pub fn extension(self) -> &'static str {
        match self {
            EncodeFormat::Wav => "wav",
            EncodeFormat::Aiff => "aif",
            EncodeFormat::Flac => "flac",
            EncodeFormat::Aac | EncodeFormat::Alac => "m4a",
            EncodeFormat::Mp3 => "mp3",
            EncodeFormat::Ac3 => "ac3",
            EncodeFormat::Eac3 => "eac3",
        }
    }

    /// The media type a file of this format is served or downloaded as.
    pub fn mime_type(self) -> &'static str {
        match self {
            EncodeFormat::Wav => "audio/wav",
            EncodeFormat::Aiff => "audio/aiff",
            EncodeFormat::Flac => "audio/flac",
            EncodeFormat::Aac | EncodeFormat::Alac => "audio/mp4",
            EncodeFormat::Mp3 => "audio/mpeg",
            EncodeFormat::Ac3 => "audio/ac3",
            EncodeFormat::Eac3 => "audio/eac3",
        }
    }

    /// The container this format writes.
    pub fn container(self) -> Container {
        match self {
            EncodeFormat::Wav => Container::Wav,
            EncodeFormat::Aiff => Container::Aiff,
            EncodeFormat::Flac => Container::Flac,
            EncodeFormat::Aac | EncodeFormat::Alac => Container::Mp4,
            EncodeFormat::Mp3 => Container::MpegAudio,
            EncodeFormat::Ac3 | EncodeFormat::Eac3 => Container::Ac3,
        }
    }

    /// The codec of the audio this format writes.
    pub fn codec(self) -> CodecId {
        match self {
            EncodeFormat::Wav | EncodeFormat::Aiff => CodecId::Pcm,
            EncodeFormat::Flac => CodecId::Flac,
            EncodeFormat::Aac => CodecId::Aac,
            EncodeFormat::Alac => CodecId::Alac,
            EncodeFormat::Mp3 => CodecId::Mp3,
            EncodeFormat::Ac3 => CodecId::Ac3,
            EncodeFormat::Eac3 => CodecId::Eac3,
        }
    }

    /// The feature that builds this format's writer.
    pub fn feature(self) -> &'static str {
        match self {
            EncodeFormat::Wav | EncodeFormat::Aiff => "pcm-encode",
            EncodeFormat::Flac => "flac-encode",
            EncodeFormat::Aac => "aac-encode",
            EncodeFormat::Alac => "alac-encode",
            EncodeFormat::Mp3 => "mp3-encode",
            EncodeFormat::Ac3 => "ac3-encode",
            EncodeFormat::Eac3 => "eac3-encode",
        }
    }

    /// Whether [`encode`] and [`FileEncoder`] write this format in this build (its
    /// [`feature`](Self::feature) is on): what to filter [`EncodeFormat::ALL`] by for a menu.
    pub fn is_built(self) -> bool {
        match self {
            EncodeFormat::Wav | EncodeFormat::Aiff => cfg!(feature = "pcm-encode"),
            EncodeFormat::Flac => cfg!(feature = "flac-encode"),
            EncodeFormat::Aac => cfg!(feature = "aac-encode"),
            EncodeFormat::Alac => cfg!(feature = "alac-encode"),
            EncodeFormat::Mp3 => cfg!(feature = "mp3-encode"),
            EncodeFormat::Ac3 => cfg!(feature = "ac3-encode"),
            EncodeFormat::Eac3 => cfg!(feature = "eac3-encode"),
        }
    }

    /// The sample rates the format takes, ascending, or `None` when it has no fixed list (WAV,
    /// AIFF, FLAC and ALAC take any rate their headers can hold).
    pub fn sample_rates(self) -> Option<&'static [u32]> {
        match self {
            EncodeFormat::Aac => Some(&[7350, 8000, 11_025, 12_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 64_000, 88_200, 96_000]),
            EncodeFormat::Mp3 => Some(&[16_000, 22_050, 24_000, 32_000, 44_100, 48_000]),
            EncodeFormat::Ac3 | EncodeFormat::Eac3 => Some(&[32_000, 44_100, 48_000]),
            EncodeFormat::Wav | EncodeFormat::Aiff | EncodeFormat::Flac | EncodeFormat::Alac => None,
        }
    }

    /// The most channels a file of this format holds. A WAV's frame size is a 16-bit field, so it
    /// holds 65535 / bytes per sample: 32767 at 16 bits (the count given here), 21845 at 24 and
    /// 16383 at 32; [`EncodeOptions::check`] answers for the depth chosen.
    pub fn max_channels(self) -> u16 {
        match self {
            EncodeFormat::Wav | EncodeFormat::Aiff => 32_767,
            EncodeFormat::Flac | EncodeFormat::Aac | EncodeFormat::Alac | EncodeFormat::Eac3 => 8,
            EncodeFormat::Ac3 => 6,
            EncodeFormat::Mp3 => 2,
        }
    }

    /// The speaker layouts of a format that has a fixed set of them, as WAV masks, for a channel
    /// count: one each for AAC, ALAC and MP3, one per coding mode for AC-3 and E-AC-3. Empty for a
    /// count the format has not, and for WAV, AIFF and FLAC, which have no such set.
    pub(crate) fn layout_masks(self, channels: u16) -> &'static [u32] {
        let per_count: &[&[u32]] = match self {
            // Channel configurations 1–6, the 6.1 program config element, and configuration 7
            // (7.1 with a front pair left and right of centre).
            EncodeFormat::Aac => &[&[0x4], &[0x3], &[0x7], &[0x107], &[0x607], &[0x60F], &[0x70F], &[0xFF]],
            // Apple's layout tags for 1–8 channels, surrounds as back speakers.
            EncodeFormat::Alac => &[&[0x4], &[0x3], &[0x7], &[0x107], &[0x37], &[0x3F], &[0x13F], &[0xFF]],
            EncodeFormat::Mp3 => &[&[0x4], &[0x3]],
            // The coding modes 1/0 to 3/2 with and without an LFE, by channel count.
            EncodeFormat::Ac3 => &[&[0x4], &[0x3, 0xC], &[0x7, 0x103, 0xB], &[0x107, 0x603, 0xF, 0x10B], &[0x607, 0x10F, 0x60B], &[0x60F]],
            // The same, and 7.1 with backs and sides.
            EncodeFormat::Eac3 => {
                &[&[0x4], &[0x3, 0xC], &[0x7, 0x103, 0xB], &[0x107, 0x603, 0xF, 0x10B], &[0x607, 0x10F, 0x60B], &[0x60F], &[], &[0x63F]]
            }
            EncodeFormat::Wav | EncodeFormat::Aiff | EncodeFormat::Flac => &[],
        };
        usize::from(channels).checked_sub(1).and_then(|i| per_count.get(i)).copied().unwrap_or_default()
    }

    /// The speaker layouts a format with a fixed set of them has for `channels` channels: one for
    /// AAC, ALAC and MP3, one per coding mode for AC-3 and E-AC-3; empty when the format cannot
    /// hold that many channels. `None` for WAV, AIFF and FLAC, which take any layout. Audio in
    /// another layout has to be rearranged into one of these before it is written.
    ///
    /// ```
    /// use craft_audio::{ChannelLayout, EncodeFormat};
    ///
    /// assert_eq!(EncodeFormat::Aac.fixed_layouts(6), Some(vec![ChannelLayout::SURROUND_5_1_SIDE]));
    /// assert_eq!(EncodeFormat::Mp3.fixed_layouts(6), Some(vec![]));
    /// assert_eq!(EncodeFormat::Wav.fixed_layouts(6), None);
    /// ```
    pub fn fixed_layouts(self, channels: u16) -> Option<Vec<ChannelLayout>> {
        match self {
            EncodeFormat::Wav | EncodeFormat::Aiff | EncodeFormat::Flac => None,
            _ => Some(self.layout_masks(channels).iter().map(|&mask| ChannelLayout::from_mask(mask)).collect()),
        }
    }

    /// Whether the format holds audio with `layout`: for WAV, AIFF and FLAC any layout of up to
    /// [`max_channels`](Self::max_channels) channels, for the others one of
    /// [`fixed_layouts`](Self::fixed_layouts), where side and back surrounds stand for each other
    /// in a layout with one surround pair.
    pub fn supports_layout(self, layout: &ChannelLayout) -> bool {
        match self {
            EncodeFormat::Wav | EncodeFormat::Aiff | EncodeFormat::Flac => (1..=self.max_channels()).contains(&layout.channels()),
            _ => self.layout_masks(layout.channels()).iter().any(|&mask| layout::channel_map(mask, None, layout).is_some()),
        }
    }

    /// Whether the format takes [`RateControl::Quality`]: AAC. The other lossy formats are
    /// written at a constant bit rate only.
    pub fn supports_quality(self) -> bool {
        self == EncodeFormat::Aac
    }

    /// Whether the format loses audio: AAC, MP3, AC-3 and E-AC-3. These take
    /// [`EncodeOptions::rate_control`] and ignore the bit depth and dither.
    pub fn is_lossy(self) -> bool {
        matches!(self, EncodeFormat::Aac | EncodeFormat::Mp3 | EncodeFormat::Ac3 | EncodeFormat::Eac3)
    }
}

/// How a lossy format spends bits ([`EncodeOptions::rate_control`]).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RateControl {
    /// A constant bit rate in bits per second, for all channels together. A format with a fixed
    /// set of rates (MP3, AC-3) uses the highest one not above it; a rate outside what the
    /// stream can carry is moved to the nearest one inside. AAC is written at 6 kbit/s or a
    /// quarter of the sample rate per channel at least: from about half of that down its encoder
    /// codes silence.
    Bitrate(u32),
    /// A constant quality from 0.0 (smallest) to 1.0 (best), at whatever bit rate each frame
    /// needs. AAC only; values outside the range are clamped, and NaN is an error.
    Quality(f32),
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
    /// their length), and runs of FLAC frames lost to a failed checksum, counted as audio is decoded: by [`decode`] for the whole file, and so far by
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
    /// Sample depth to write (ignored by the lossy formats).
    pub bit_depth: BitDepth,
    /// Add TPDF dither when quantizing to an integer depth (ignored by the lossy formats).
    pub dither: bool,
    /// Write a BWF `bext` chunk (WAV only; ignored for other formats).
    pub bwf: Option<BwfInfo>,
    /// The speaker layout to record, for layouts a channel count alone cannot identify (5.1 with
    /// side vs back surrounds, 7.0 vs 6.1, Ambisonics as [`ChannelLayout::unordered`]). `None` is
    /// the usual layout for the channel count. WAV stores it as `dwChannelMask`, FLAC as a
    /// `WAVEFORMATEXTENSIBLE_CHANNEL_MASK` comment when decoders would assume another layout for
    /// the count (they read 5 and 6 channels with side surrounds and 7 as 6.1 with sides, so the
    /// usual layouts for those counts get one; see the README's limits); AIFF has a fixed channel
    /// order and ignores it. It must have as many channels as the audio.
    ///
    /// AAC, ALAC, MP3, AC-3 and E-AC-3 store channels in their own order, so here the layout says
    /// which speaker each channel of the audio (in WAV order) is, and the channels are reordered
    /// to match. Each of these formats has a fixed set of layouts (AAC and ALAC one per channel
    /// count, AC-3 one per coding mode); a layout outside it is [`AudioError::Unsupported`]. Side
    /// and back surrounds are taken as the same pair where the format has only one.
    pub layout: Option<ChannelLayout>,
    /// Write WAV in the RF64 layout even below 4 GiB (WAV only; ignored for other formats).
    pub force_rf64: bool,
    /// The bit rate or quality of a lossy format (ignored by the others). `None` is a constant
    /// 64 kbit/s per channel for AAC, 96 kbit/s per channel for MP3, and 96 kbit/s per
    /// full-bandwidth channel (the LFE not counted) for AC-3 and E-AC-3, moved to the nearest rate
    /// the stream can carry.
    pub rate_control: Option<RateControl>,
}

impl EncodeOptions {
    /// Options for `format`: 24-bit, no dither, no BWF chunk, the usual layout, the format's
    /// default bit rate.
    pub fn new(format: EncodeFormat) -> Self {
        Self { format, bit_depth: BitDepth::Int24, dither: false, bwf: None, layout: None, force_rf64: false, rate_control: None }
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

    /// Sets `rate_control` to a constant bit rate in bits per second ([`RateControl::Bitrate`]).
    pub fn with_bitrate(mut self, bits_per_second: u32) -> Self {
        self.rate_control = Some(RateControl::Bitrate(bits_per_second));
        self
    }

    /// Sets `rate_control` to a constant quality from 0.0 to 1.0 ([`RateControl::Quality`]).
    pub fn with_quality(mut self, quality: f32) -> Self {
        self.rate_control = Some(RateControl::Quality(quality));
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
