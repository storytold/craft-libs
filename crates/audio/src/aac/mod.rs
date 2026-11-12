//! Clean-room AAC-LC encoder and decoder (ISO/IEC 14496-3, MPEG-4 Audio, Low Complexity profile).
//!
//! - [`Encoder`] (feature `aac-encode`): planar f32 → raw access units (+
//!   [`Encoder::audio_specific_config`] for MP4 `esds`, optional ADTS framing). Psychoacoustic
//!   model, window switching, M/S, TNS, bit reservoir, CBR/VBR, 1–8 channels.
//! - [`Decoder`] (feature `aac-decode`): `AudioSpecificConfig` + raw access units → planar f32.
//!   Supports M/S, intensity stereo, PNS, TNS, pulse data, both window shapes and every window
//!   sequence.
//! - [`latm_config`] / [`latm_split`] (feature `aac-decode`): the configuration and payload of
//!   LOAS / LATM frames (AAC in MPEG transport streams), for the decoder.
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
#[cfg(feature = "aac-decode")]
mod latm;
mod mdct;
mod tables;
mod tns;

#[cfg(feature = "aac-decode")]
pub(crate) use config::split_adts_upto;
pub use config::{AdtsHeader, AudioSpecificConfig, ElementType, ProgramConfig, config_layout, split_adts};
#[cfg(feature = "aac-decode")]
pub use decoder::Decoder;
#[cfg(feature = "aac-encode")]
pub use encoder::{BitrateMode, Encoder, EncoderConfig, EncoderStats};
#[cfg(feature = "aac-decode")]
pub use latm::{LatmConfig, latm_config, latm_split};
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

/// The channel configuration whose speakers a stream has: its own, or for configuration 0 the
/// configuration 3–6 that its program config element spells out. A program config element lists
/// front, side, back and LFE elements in that order, so one that opens with a centre and a front
/// pair and goes on with exactly the elements of a configuration has that configuration's speakers
/// (and [`wav_order`]). Any other list has no speaker order that can be told, and gives 0: ffmpeg's
/// 5.1(side), for one, lists the pair first and already decodes in WAV order.
#[cfg(feature = "aac-decode")]
pub fn speaker_config(asc: &AudioSpecificConfig) -> u8 {
    if asc.channel_config != 0 {
        return asc.channel_config;
    }
    let centre_then_pair = asc.pce.as_ref().is_some_and(|p| p.front.iter().map(|e| e.0).eq([false, true]));
    if !centre_then_pair {
        return 0;
    }
    let elements = asc.layout();
    let same = |config: &u8| config_layout(*config).is_some_and(|l| l.iter().map(|e| e.0).eq(elements.iter().map(|e| e.0)));
    (3..=6).find(same).unwrap_or(0)
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

/// Input samples [`Upsample2x`] delays its output by: 481, the delay of the SBR filterbanks (962
/// output samples), which HE-AAC encoders and the container's priming count on.
#[cfg(feature = "aac-decode")]
const UPSAMPLE_DELAY: usize = 481;

/// Input samples [`Upsample2x`] keeps per channel.
#[cfg(feature = "aac-decode")]
const UPSAMPLE_HISTORY: usize = UPSAMPLE_DELAY + UPSAMPLE_HALF;

/// 2× upsampler for HE-AAC. The decoder reconstructs the AAC-LC core at half the output rate and
/// not yet the SBR high band, so the core is interpolated to the output rate: timing and pitch are
/// right and the band above the core's Nyquist stays empty. Even outputs are the input samples,
/// odd ones a Blackman-windowed sinc half-band interpolation over 2 × `UPSAMPLE_HALF` inputs.
/// The output is delayed by `UPSAMPLE_DELAY` input samples, as an SBR decoder's is; the state
/// carries across packets, so a seek primed with the preceding packet gives the same samples as
/// continuous decoding. From FilmCraft's `filmcraft-codecs`.
#[cfg(feature = "aac-decode")]
#[derive(Clone)]
// Internal to HE-AAC output; public only so FilmCraft can re-export this module.
#[doc(hidden)]
pub struct Upsample2x {
    taps: [f32; 2 * UPSAMPLE_HALF],
    /// The last `UPSAMPLE_HISTORY` input samples per channel.
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
        Self { taps, hist: vec![vec![0.0; UPSAMPLE_HISTORY]; channels] }
    }

    /// Clear the history (e.g. after a seek).
    pub fn reset(&mut self) {
        for h in &mut self.hist {
            h.fill(0.0);
        }
    }

    /// Upsample planar channels by two: each output channel has twice the input samples.
    pub fn process(&mut self, input: Vec<Vec<f32>>) -> Vec<Vec<f32>> {
        // A changed channel count is a new layout: history of channels that were absent meanwhile
        // (or that held other speakers) would put a step into the output.
        if self.hist.len() != input.len() {
            self.hist = vec![vec![0.0; UPSAMPLE_HISTORY]; input.len()];
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
                *hist = buf.split_off(buf.len().saturating_sub(UPSAMPLE_HISTORY));
                out
            })
            .collect()
    }
}

#[cfg(all(test, feature = "aac-decode"))]
mod tests {
    use super::*;

    /// The output lags the input by the SBR filterbanks' delay, whatever the block size.
    #[test]
    fn the_upsampler_delays_by_the_sbr_filterbanks() {
        let mut impulse = vec![0.0; 2000];
        impulse[100] = 1.0;
        let mut up = Upsample2x::new(1);
        let mut out = Vec::new();
        for block in impulse.chunks(300) {
            out.extend(up.process(vec![block.to_vec()]).remove(0));
        }
        assert_eq!(out.len(), 4000);
        assert_eq!(UPSAMPLE_DELAY, 481);
        assert_eq!(out[(100 + 481) * 2], 1.0);
        assert!(out[..(100 + 481 - UPSAMPLE_HALF) * 2].iter().all(|&s| s == 0.0));
        // a change of channel count starts each channel's history afresh
        let mut fresh = Upsample2x::new(2);
        let block = vec![vec![0.5; 1024], vec![0.25; 1024]];
        let mut up = Upsample2x::new(2);
        up.process(block.clone());
        up.process(vec![vec![0.5; 1024]]);
        assert_eq!(up.process(block.clone()), fresh.process(block));
        // a reset forgets the history
        up.reset();
        assert!(up.process(vec![vec![0.0; 600]]).remove(0).iter().all(|&s| s == 0.0));
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;

    /// A program config element in the shape of a channel configuration has that configuration's
    /// speakers, with the centre first. ffmpeg's 5.1(side) lists its pair first and already decodes
    /// in WAV order, so it stays as it is.
    #[cfg(feature = "aac-decode")]
    #[test]
    fn a_program_config_element_takes_the_configuration_it_matches() {
        let with = |pce| AudioSpecificConfig { pce: Some(pce), ..AudioSpecificConfig::lc(3, 0) };
        let centre_first = ProgramConfig { front: vec![(false, 0), (true, 0)], back: vec![(true, 1)], lfe: vec![0], ..Default::default() };
        assert_eq!(speaker_config(&with(centre_first)), 6);
        let three = ProgramConfig { front: vec![(false, 0), (true, 0)], ..Default::default() };
        assert_eq!(speaker_config(&with(three)), 3);
        let ffmpeg = ProgramConfig { front: vec![(true, 0), (false, 0)], side: vec![(false, 1)], back: vec![(true, 1)], ..Default::default() };
        assert_eq!(speaker_config(&with(ffmpeg)), 0);
        // the same element types with no front pair: the side pair is not left and right
        let no_front_pair =
            ProgramConfig { front: vec![(false, 0)], side: vec![(true, 0)], back: vec![(true, 1)], lfe: vec![0], ..Default::default() };
        assert_eq!(speaker_config(&with(no_front_pair)), 0);
        // 7.1 in a program config element is not one of the unambiguous configurations
        let wide =
            ProgramConfig { front: vec![(false, 0), (true, 0)], side: vec![(true, 1)], back: vec![(true, 2)], lfe: vec![0], ..Default::default() };
        assert_eq!(speaker_config(&with(wide)), 0);
        assert_eq!(speaker_config(&AudioSpecificConfig::lc(3, 6)), 6);
    }

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
