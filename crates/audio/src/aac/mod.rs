//! Clean-room AAC-LC encoder and decoder (ISO/IEC 14496-3, MPEG-4 Audio, Low Complexity profile).
//!
//! - [`Encoder`] (feature `aac-encode`): planar f32 → raw access units (+
//!   [`Encoder::audio_specific_config`] for MP4 `esds`, optional ADTS framing). Psychoacoustic
//!   model, window switching, M/S, TNS, bit reservoir, CBR/VBR, 1–8 channels.
//! - [`Decoder`] (feature `aac-decode`): `AudioSpecificConfig` + raw access units → planar f32.
//!   Supports M/S, intensity stereo, PNS, TNS, pulse data, both window shapes and every window
//!   sequence.
//!
//! HE-AAC (SBR/PS), Main/SSR/LTP profiles, coupling channels and 960-sample frames are not
//! supported; for HE-AAC only the AAC-LC core is decoded (at the core rate; [`Decoder::sbr`] says
//! when).
//!
//! From FilmCraft's `filmcraft-aac` (storytold/filmcraft@8fda2f2). Public items keep their names,
//! so FilmCraft can re-export this module.
//!
//! ```
//! # #[cfg(all(feature = "aac-encode", feature = "aac-decode"))] {
//! use craft_audio::aac::{Decoder, Encoder, EncoderConfig};
//!
//! let mut enc = Encoder::new(EncoderConfig::cbr(48_000, 2, 128_000)).unwrap();
//! let left: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
//! let right = left.clone();
//! let mut aus = enc.encode(&[&left, &right]); // 0..n access units
//! aus.extend(enc.flush());
//! let asc = enc.audio_specific_config(); // for the MP4 `esds`
//! assert_eq!(enc.priming_samples(), 1024); // MP4 edit list media_time
//!
//! let mut dec = Decoder::new(&asc).unwrap();
//! for au in &aus {
//!     let pcm = dec.decode(au).unwrap(); // planar, 1024 samples per channel
//!     assert_eq!(pcm.len(), 2);
//! }
//! # }
//! ```

// Index loops follow the specification's pseudo-code (FilmCraft allows this lint workspace-wide).
#![allow(clippy::needless_range_loop)]
// Tables, Huffman coding, MDCT, window and TNS code shared by the decoder and the encoder; each
// side uses only part of it.
#![cfg_attr(not(all(feature = "aac-decode", feature = "aac-encode")), allow(dead_code))]

mod config;
#[cfg(feature = "aac-decode")]
mod decoder;
#[cfg(feature = "aac-encode")]
mod encoder;
mod huffman;
mod huffman_tables;
mod ics;
mod mdct;
mod tables;
mod tns;

pub use config::{AdtsHeader, AudioSpecificConfig, ElementType, ProgramConfig, config_layout, split_adts};
#[cfg(feature = "aac-decode")]
pub use decoder::Decoder;
#[cfg(feature = "aac-encode")]
pub use encoder::{BitrateMode, Encoder, EncoderConfig, EncoderStats};
pub use mdct::WindowShape;
pub use tables::{SAMPLE_RATES, sample_rate_index};

/// Samples per AAC-LC frame (per channel).
pub const FRAME_LEN: usize = 1024;

/// Errors from configuration parsing, encoding and decoding.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The configuration is malformed or outside the supported range.
    #[error("invalid configuration: {0}")]
    InvalidConfig(&'static str),
    /// [`EncoderConfig`] asks for something the encoder cannot do (a rate that is not an AAC
    /// rate, too many channels, a bitrate out of range).
    #[error("invalid encoder settings: {0}")]
    InvalidSettings(&'static str),
    /// A valid stream uses a feature this crate does not implement.
    #[error("unsupported feature: {0}")]
    Unsupported(&'static str),
    /// Malformed bitstream data.
    #[error("invalid bitstream: {0}")]
    Bitstream(&'static str),
    /// The bit reader ran past the end of the data.
    #[error("unexpected end of access unit")]
    Eof,
}

impl From<craft_media::bitstream::BitError> for Error {
    fn from(_: craft_media::bitstream::BitError) -> Self {
        Error::Eof
    }
}

/// Result with this module's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Channel order for AAC surround (ISO/IEC 14496-3 Table 1.19, the decoder's element order: C, L R,
/// surrounds, LFE): output channel → decoded channel, giving the WAV/SMPTE order (L R C LFE Ls Rs)
/// the rest of craft-audio uses. Only the unambiguous channel configurations 3–6, 11 (6.1: … LFE,
/// back centre, sides) and 12 (7.1: … LFE, backs, sides); 1, 2, 7 and PCE layouts keep the decoded
/// order. From FilmCraft's `filmcraft-codecs`.
pub fn wav_order(channel_config: u8) -> Option<&'static [usize]> {
    Some(match channel_config {
        3 => &[1, 2, 0],
        4 => &[1, 2, 0, 3],
        5 => &[1, 2, 0, 3, 4],
        6 => &[1, 2, 0, 5, 3, 4],
        11 => &[1, 2, 0, 6, 5, 3, 4],
        12 => &[1, 2, 0, 7, 5, 6, 3, 4],
        _ => return None,
    })
}

/// The order the [`Encoder`] takes its input in, from WAV order: for each encoder input channel
/// (AAC element order, see [`EncoderConfig::channels`]), the WAV-order channel to feed it. The
/// inverse of [`wav_order`] for 3 to 7 channels; `None` for mono and stereo (already in order) and
/// for 8 channels, whose AAC layout (configuration 7) has no WAV equivalent.
///
/// ```
/// let wav: Vec<Vec<f32>> = vec![vec![0.0; 1024]; 6]; // L R C LFE Ls Rs
/// let order = craft_audio::aac::aac_order(6).unwrap();
/// let input: Vec<&[f32]> = order.iter().map(|&c| wav[c].as_slice()).collect(); // C L R Ls Rs LFE
/// assert_eq!(order, &[2, 0, 1, 4, 5, 3]);
/// ```
pub fn aac_order(channels: usize) -> Option<&'static [usize]> {
    Some(match channels {
        3 => &[2, 0, 1],
        4 => &[2, 0, 1, 3],
        5 => &[2, 0, 1, 3, 4],
        6 => &[2, 0, 1, 4, 5, 3],
        7 => &[2, 0, 1, 5, 6, 4, 3],
        _ => return None,
    })
}

/// The output rate of an AAC stream: twice the core rate for HE-AAC (ISO/IEC 14496-3 §1.6.5).
/// Explicit signalling names the rate in the `AudioSpecificConfig`; with implicit signalling (an
/// AAC-LC config at 24 kHz or less) SBR is only visible as fill-element data in the access units,
/// so the first ones are decoded to look for it. `None` if `asc` is not one the decoder reads.
/// From FilmCraft's `filmcraft-codecs`.
#[cfg(feature = "aac-decode")]
pub fn output_rate<'a>(asc: &[u8], first_units: impl IntoIterator<Item = &'a [u8]>) -> Option<u32> {
    let mut dec = Decoder::new(asc).ok()?;
    let core = dec.sample_rate();
    if let Some(ext) = dec.config().extension_sample_rate {
        return Some(ext);
    }
    if core <= 24_000 {
        for au in first_units {
            let _ = dec.decode(au);
            if dec.sbr() {
                return Some(core * 2);
            }
        }
    }
    Some(core)
}

/// Taps on each side of the interpolation point in [`Upsample2x`].
#[cfg(feature = "aac-decode")]
const UPSAMPLE_HALF: usize = 12;

/// 2× upsampler for HE-AAC. The decoder reconstructs the AAC-LC core at half the output rate and
/// not yet the SBR high band, so the core is interpolated to the output rate: timing and pitch are
/// right and the band above the core's Nyquist stays empty. Even outputs are the input samples,
/// odd ones a Blackman-windowed sinc half-band interpolation over 2 × `UPSAMPLE_HALF` inputs.
/// The look-ahead delays the output by `UPSAMPLE_HALF` input samples (0.5 ms at 24 kHz); the
/// state carries across packets, so a seek primed with the preceding packet gives the same
/// samples as continuous decoding. From FilmCraft's `filmcraft-codecs`.
#[cfg(feature = "aac-decode")]
#[derive(Clone)]
// Internal to HE-AAC output; public only so FilmCraft can re-export this module.
#[doc(hidden)]
pub struct Upsample2x {
    taps: [f32; 2 * UPSAMPLE_HALF],
    /// The last `2 × UPSAMPLE_HALF` input samples per channel.
    hist: Vec<Vec<f32>>,
}

#[cfg(feature = "aac-decode")]
impl Upsample2x {
    /// An upsampler for `channels` channels with empty history.
    pub fn new(channels: usize) -> Self {
        let m = UPSAMPLE_HALF as f64;
        let mut taps = [0f32; 2 * UPSAMPLE_HALF];
        let mut sum = 0.0;
        for (k, t) in taps.iter_mut().enumerate() {
            // distance from the interpolation point (halfway between two inputs)
            let x = k as f64 - m + 0.5;
            let sinc = (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x);
            let w = 0.42 + 0.5 * (std::f64::consts::PI * x / m).cos() + 0.08 * (2.0 * std::f64::consts::PI * x / m).cos();
            *t = (sinc * w) as f32;
            sum += sinc * w;
        }
        for t in &mut taps {
            *t = (*t as f64 / sum) as f32;
        }
        Self { taps, hist: vec![vec![0.0; 2 * UPSAMPLE_HALF]; channels] }
    }

    /// Clear the history (e.g. after a seek).
    pub fn reset(&mut self) {
        for h in &mut self.hist {
            h.fill(0.0);
        }
    }

    /// Upsample planar channels by two: each output channel has twice the input samples.
    pub fn process(&mut self, input: Vec<Vec<f32>>) -> Vec<Vec<f32>> {
        if self.hist.len() < input.len() {
            self.hist.resize(input.len(), vec![0.0; 2 * UPSAMPLE_HALF]);
        }
        let m = UPSAMPLE_HALF;
        input
            .into_iter()
            .zip(&mut self.hist)
            .map(|(x, hist)| {
                let mut buf = std::mem::take(hist);
                buf.extend_from_slice(&x);
                let mut out = Vec::with_capacity(x.len() * 2);
                // output pair i is centred on buf[i + m]: the sample itself, then the point halfway
                // to the next one from buf[i + 1 ..= i + 2m]
                for w in buf.windows(2 * m + 1).take(x.len()) {
                    out.push(w.get(m).copied().unwrap_or(0.0));
                    out.push(w.get(1..).unwrap_or_default().iter().zip(&self.taps).map(|(a, b)| a * b).sum());
                }
                *hist = buf.split_off(buf.len().saturating_sub(2 * m));
                out
            })
            .collect()
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;

    /// `aac_order` undoes `wav_order` for each channel count the encoder maps to a configuration
    /// (3–6 channels: configurations 3–6; 7: configuration 11).
    #[test]
    fn aac_order_inverts_wav_order() {
        for (channels, config) in [(3, 3), (4, 4), (5, 5), (6, 6), (7, 11)] {
            let (to_wav, to_aac) = (wav_order(config).unwrap(), aac_order(channels).unwrap());
            for (wav_channel, &aac_channel) in to_wav.iter().enumerate() {
                assert_eq!(to_aac[aac_channel], wav_channel, "{channels} channels");
            }
        }
        assert_eq!(aac_order(2), None);
        assert_eq!(aac_order(8), None);
    }
}
