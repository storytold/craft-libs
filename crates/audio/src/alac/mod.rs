// Ported from Apple's ALAC reference implementation (ALACAudioTypes.h, ALACDecoder.cpp, ALACEncoder.cpp and ALACMagicCookieDescription.txt, macosforge/alac@c38887c,
// https://github.com/macosforge/alac). Copyright (c) 2011 Apple Inc.
// Licensed under the Apache License, Version 2.0 (LICENSE-APACHE at the repository root).
// Modified: translated to Rust and changed as the `alac` module docs describe.
// SPDX-License-Identifier: Apache-2.0

//! Apple Lossless (ALAC): [`Decoder`] (feature `alac-decode`) and [`Encoder`] (feature
//! `alac-encode`), packet by packet, with the stream configuration ([`SpecificConfig`]).
//!
//! Where the configuration goes: MP4's `alac` box (after its FullBox header) and Matroska's
//! `A_ALAC` CodecPrivate hold the 24-byte `ALACSpecificConfig` ([`SpecificConfig::to_bytes`]);
//! an MP4 of more than two channels carries the layout in a separate `chan` box, as Apple's
//! `afconvert` writes it. CAF's `kuki` chunk holds the magic cookie as Apple's encoder vends it,
//! the config followed by a `chan` atom for more than two channels ([`SpecificConfig::to_cookie`]).
//! The decoder takes either form.
//!
//! Samples are planar `i32`, right-aligned at the stream's bit depth (a 24-bit sample lies in
//! −2^23 ..= 2^23 − 1), channels in ALAC order (see [`channel_mask`]; [`wav_order`] maps them
//! to WAV order).
//!
//! Ported from Apple's reference implementation (<https://github.com/macosforge/alac>, commit
//! c38887c, `codec/`), Copyright © 2011 Apple Inc., Apache License 2.0. Changes from the
//! reference: hostile input returns errors instead of reading or writing out of bounds, integer
//! arithmetic wraps where C would overflow, the 4- and 8-tap predictor loops are folded into the
//! general one (same values), the sample-width variants of the matrix routines into one routine
//! over `i32`, and the encoder fills in the cookie's average bit rate.

// The bit I/O, coder, predictor and matrix modules serve both halves; with only one of
// `alac-decode` and `alac-encode` built, the other half's routines are unused.
#![cfg_attr(not(all(feature = "alac-decode", feature = "alac-encode")), allow(dead_code))]

mod ag;
mod bits;
#[cfg(feature = "alac-decode")]
mod decoder;
mod dp;
#[cfg(feature = "alac-encode")]
mod encoder;
mod matrix;

#[cfg(feature = "alac-decode")]
pub use decoder::Decoder;
#[cfg(feature = "decode-native")]
pub(crate) use decoder::header_frames;
#[cfg(feature = "alac-encode")]
pub use encoder::{Encoder, EncoderConfig};

/// The frames per packet Apple's encoder writes by default (`kALACDefaultFramesPerPacket`).
pub const DEFAULT_FRAME_LENGTH: u32 = 4096;

/// The most frames a packet may hold here: the decoder allocates a packet's samples up front, and
/// the demuxers allow packets of at most this many frames.
pub const MAX_FRAME_LENGTH: u32 = 1 << 20;

/// The most channels a stream has (`kALACMaxChannels`).
pub const MAX_CHANNELS: u8 = 8;

/// Size of the `ALACSpecificConfig` at the start of a cookie.
const CONFIG_LEN: usize = 24;

/// Size of the channel layout atom a cookie may carry after the config (`kChannelAtomSize` plus
/// `ALACAudioChannelLayout`).
const CHANNEL_LAYOUT_LEN: usize = 24;

/// The CoreAudio channel layout tag of each channel count, the only layouts ALAC uses
/// (`ALACChannelLayoutTags`): mono C; stereo L R; then C L R, C L R Cs, C L R Ls Rs,
/// C L R Ls Rs LFE, C L R Ls Rs Cs LFE and C Lc Rc L R Ls Rs LFE.
pub const CHANNEL_LAYOUT_TAGS: [u32; 8] =
    [(100 << 16) | 1, (101 << 16) | 2, (113 << 16) | 3, (116 << 16) | 4, (120 << 16) | 5, (124 << 16) | 6, (142 << 16) | 7, (127 << 16) | 8];

/// Element tags (`ID_SCE` … `ID_END`).
const ID_SCE: u32 = 0;
const ID_CPE: u32 = 1;
const ID_CCE: u32 = 2;
const ID_LFE: u32 = 3;
const ID_DSE: u32 = 4;
const ID_PCE: u32 = 5;
const ID_FIL: u32 = 6;
const ID_END: u32 = 7;

/// The elements a frame of `channels` channels holds, in order (`sChannelMaps`): single channels
/// for C, the LFE and the 6.1 back centre, pairs for the rest.
pub(crate) fn element_map(channels: usize) -> &'static [u32] {
    match channels {
        1 => &[ID_SCE],
        2 => &[ID_CPE],
        3 => &[ID_SCE, ID_CPE],
        4 => &[ID_SCE, ID_CPE, ID_SCE],
        5 => &[ID_SCE, ID_CPE, ID_CPE],
        6 => &[ID_SCE, ID_CPE, ID_CPE, ID_SCE],
        7 => &[ID_SCE, ID_CPE, ID_CPE, ID_SCE, ID_SCE],
        8 => &[ID_SCE, ID_CPE, ID_CPE, ID_CPE, ID_SCE],
        _ => &[],
    }
}

/// The WAVEFORMATEXTENSIBLE speaker mask of the layout for `channels` (the tags in
/// [`CHANNEL_LAYOUT_TAGS`], surrounds as back speakers): 0 for a count ALAC does not have.
pub fn channel_mask(channels: u8) -> u32 {
    match channels {
        1 => 0x4,
        2 => 0x3,
        3 => 0x7,
        4 => 0x107,
        5 => 0x37,
        6 => 0x3F,
        7 => 0x13F,
        8 => 0xFF,
        _ => 0,
    }
}

/// Channel order for WAV/SMPTE output: WAV channel → decoded (ALAC-order) channel, for 3 to 8
/// channels; `None` for mono and stereo, which need none, and other counts. The inverse of
/// [`alac_order`].
pub fn wav_order(channels: u8) -> Option<&'static [usize]> {
    Some(match channels {
        3 => &[1, 2, 0],
        4 => &[1, 2, 0, 3],
        5 => &[1, 2, 0, 3, 4],
        6 => &[1, 2, 0, 5, 3, 4],
        7 => &[1, 2, 0, 6, 3, 4, 5],
        8 => &[3, 4, 0, 7, 5, 6, 1, 2],
        _ => return None,
    })
}

/// Channel order for encoding WAV/SMPTE-order audio: ALAC channel → WAV channel (the
/// encoder's channel `j` is WAV channel `alac_order(n)[j]`), for 3 to 8 channels; `None` for
/// mono, stereo and other counts. The inverse of [`wav_order`].
pub fn alac_order(channels: u8) -> Option<&'static [usize]> {
    Some(match channels {
        3 => &[2, 0, 1],
        4 => &[2, 0, 1, 3],
        5 => &[2, 0, 1, 3, 4],
        6 => &[2, 0, 1, 4, 5, 3],
        7 => &[2, 0, 1, 4, 5, 6, 3],
        8 => &[2, 6, 7, 0, 1, 4, 5, 3],
        _ => return None,
    })
}

/// Errors from cookie parsing, decoding and encoding.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The magic cookie is malformed or outside the supported range.
    #[error("invalid configuration: {0}")]
    InvalidConfig(&'static str),
    /// A valid stream uses something this module does not implement.
    #[error("unsupported feature: {0}")]
    Unsupported(&'static str),
    /// Malformed packet.
    #[error("invalid bitstream: {0}")]
    Bitstream(&'static str),
    /// The packet ended early.
    #[error("unexpected end of packet")]
    Eof,
    /// Samples or settings the encoder cannot take.
    #[error("invalid encoder input: {0}")]
    InvalidInput(&'static str),
}

/// Result with this module's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// The `ALACSpecificConfig` of a magic cookie, with the channel layout tag when the cookie has one.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpecificConfig {
    /// Frames per packet (the last packet may hold fewer).
    pub frame_length: u32,
    /// 0 for every stream the reference writes or reads.
    pub compatible_version: u8,
    /// Bits per source sample (16, 20, 24 or 32 from Apple's encoder).
    pub bit_depth: u8,
    /// Rice coder tuning: history multiplier (40).
    pub pb: u8,
    /// Rice coder tuning: initial history (10).
    pub mb: u8,
    /// Rice coder tuning: largest Rice parameter (14).
    pub kb: u8,
    /// Channel count.
    pub channels: u8,
    /// Unused by the coder (255).
    pub max_run: u16,
    /// Largest packet in bytes, 0 when unknown.
    pub max_frame_bytes: u32,
    /// Average bit rate, 0 when unknown.
    pub avg_bit_rate: u32,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// The CoreAudio channel layout tag of the cookie's `chan` atom, when it has one.
    pub channel_layout_tag: Option<u32>,
}

impl SpecificConfig {
    /// A configuration with the reference encoder's tuning and no statistics.
    pub fn new(sample_rate: u32, channels: u8, bit_depth: u8, frame_length: u32) -> Self {
        SpecificConfig {
            frame_length,
            compatible_version: 0,
            bit_depth,
            pb: ag::PB0 as u8,
            mb: ag::MB0 as u8,
            kb: ag::KB0 as u8,
            channels,
            max_run: ag::MAX_RUN_DEFAULT as u16,
            max_frame_bytes: 0,
            avg_bit_rate: 0,
            sample_rate,
            channel_layout_tag: None,
        }
    }

    /// Parse a magic cookie. As in the reference decoder, `frma` and `alac` atom headers in front
    /// of the config (cookies from older QuickTime files) are skipped. The fields are not checked
    /// here; [`Decoder::new`] checks what it needs.
    pub fn parse(cookie: &[u8]) -> Result<Self> {
        let mut c = cookie;
        for atom in [b"frma", b"alac"] {
            if c.get(4..8) == Some(atom.as_slice()) {
                c = c.get(12..).ok_or(Error::InvalidConfig("magic cookie shorter than its atom header"))?;
            }
        }
        let b = c.get(..CONFIG_LEN).ok_or(Error::InvalidConfig("magic cookie shorter than 24 bytes"))?;
        let u32_at = |i: usize| b.get(i..i + 4).and_then(|s| s.try_into().ok()).map_or(0, u32::from_be_bytes);
        let byte = |i: usize| b.get(i).copied().unwrap_or(0);
        // `chan` atom: size, 'chan', version and flags, layout tag, bitmap, description count.
        let rest = c.get(CONFIG_LEN..).unwrap_or_default();
        // Apple's reference encoder copies the tag in host byte order (little-endian on x86 and
        // ARM) although the cookie is big-endian: a byte-swapped standard tag is read as that tag.
        let channel_layout_tag = (rest.get(4..8) == Some(b"chan".as_slice()))
            .then(|| rest.get(12..16).and_then(|s| s.try_into().ok()).map(u32::from_be_bytes))
            .flatten()
            .map(|t| if !CHANNEL_LAYOUT_TAGS.contains(&t) && CHANNEL_LAYOUT_TAGS.contains(&t.swap_bytes()) { t.swap_bytes() } else { t });
        Ok(SpecificConfig {
            frame_length: u32_at(0),
            compatible_version: byte(4),
            bit_depth: byte(5),
            pb: byte(6),
            mb: byte(7),
            kb: byte(8),
            channels: byte(9),
            max_run: u16::from_be_bytes([byte(10), byte(11)]),
            max_frame_bytes: u32_at(12),
            avg_bit_rate: u32_at(16),
            sample_rate: u32_at(20),
            channel_layout_tag,
        })
    }

    /// The 24-byte `ALACSpecificConfig`, as MP4's `alac` box and Matroska's CodecPrivate hold it.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CONFIG_LEN + CHANNEL_LAYOUT_LEN);
        out.extend_from_slice(&self.frame_length.to_be_bytes());
        out.extend_from_slice(&[self.compatible_version, self.bit_depth, self.pb, self.mb, self.kb, self.channels]);
        out.extend_from_slice(&self.max_run.to_be_bytes());
        out.extend_from_slice(&self.max_frame_bytes.to_be_bytes());
        out.extend_from_slice(&self.avg_bit_rate.to_be_bytes());
        out.extend_from_slice(&self.sample_rate.to_be_bytes());
        out
    }

    /// The magic cookie as the reference encoder vends it (`GetMagicCookie`) and CAF stores it:
    /// [`Self::to_bytes`], then for more than two channels a `chan` atom with
    /// [`Self::channel_layout_tag`] (the standard tag for the count when `None`). The tag is
    /// written big-endian, as Apple's cookie description specifies (the reference encoder writes
    /// it in host byte order).
    pub fn to_cookie(&self) -> Vec<u8> {
        let mut out = self.to_bytes();
        let standard = CHANNEL_LAYOUT_TAGS.get(usize::from(self.channels).wrapping_sub(1)).copied();
        if let Some(tag) = self.channel_layout_tag.or(standard).filter(|_| self.channels > 2) {
            out.extend_from_slice(&(CHANNEL_LAYOUT_LEN as u32).to_be_bytes());
            out.extend_from_slice(b"chan");
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&tag.to_be_bytes());
            out.extend_from_slice(&[0; 8]);
        }
        out
    }

    /// Whether the channels are the standard layout for their count: the cookie names that
    /// layout, or none (the reference decoder ignores the tag and every ALAC encoder writes the
    /// standard order).
    pub fn has_standard_layout(&self) -> bool {
        let standard = CHANNEL_LAYOUT_TAGS.get(usize::from(self.channels).wrapping_sub(1)).copied();
        standard.is_some() && self.channel_layout_tag.is_none_or(|t| Some(t) == standard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_roundtrip() {
        let mut c = SpecificConfig::new(48_000, 6, 24, 4096);
        c.max_frame_bytes = 12_345;
        c.avg_bit_rate = 2_000_000;
        let bytes = c.to_cookie();
        assert_eq!(bytes.len(), 48);
        let mut parsed = SpecificConfig::parse(&bytes).unwrap();
        assert_eq!(parsed.channel_layout_tag, Some((124 << 16) | 6));
        assert!(parsed.has_standard_layout());
        parsed.channel_layout_tag = None;
        assert_eq!(parsed, c);
        assert_eq!(c.to_bytes(), bytes[..24]);
        let stereo = SpecificConfig::new(44_100, 2, 16, 4096).to_cookie();
        assert_eq!(stereo.len(), 24);
        assert_eq!(&stereo[..12], &[0, 0, 0x10, 0, 0, 16, 40, 10, 14, 2, 0, 255]);
    }

    #[test]
    fn a_little_endian_layout_tag_is_read() {
        // as Apple's reference encoder writes the tag of a 7.1 stream
        let mut cookie = SpecificConfig::new(44_100, 8, 16, 4096).to_cookie();
        cookie[36..40].copy_from_slice(&((127u32 << 16) | 8).to_le_bytes());
        let c = SpecificConfig::parse(&cookie).unwrap();
        assert_eq!(c.channel_layout_tag, Some((127 << 16) | 8));
        assert!(c.has_standard_layout());
    }

    #[test]
    fn old_quicktime_atoms_are_skipped() {
        let config = SpecificConfig::new(44_100, 2, 16, 4096).to_cookie();
        let mut wrapped = vec![0, 0, 0, 12, b'f', b'r', b'm', b'a', b'a', b'l', b'a', b'c', 0, 0, 0, 36, b'a', b'l', b'a', b'c', 0, 0, 0, 0];
        wrapped.extend_from_slice(&config);
        assert_eq!(SpecificConfig::parse(&wrapped).unwrap(), SpecificConfig::parse(&config).unwrap());
        assert!(SpecificConfig::parse(&wrapped[..30]).is_err());
        assert!(SpecificConfig::parse(&[0; 5]).is_err());
    }

    #[test]
    fn the_orders_are_permutations() {
        for n in 3..=8u8 {
            let order = wav_order(n).unwrap();
            let inverse = alac_order(n).unwrap();
            assert!((0..usize::from(n)).all(|i| inverse[order[i]] == i && order[inverse[i]] == i), "{n} channels");
            let mut sorted = order.to_vec();
            sorted.sort_unstable();
            assert_eq!(sorted, (0..usize::from(n)).collect::<Vec<_>>());
            assert_eq!(channel_mask(n).count_ones(), u32::from(n));
            let elements: usize = element_map(usize::from(n)).iter().map(|&e| if e == ID_CPE { 2 } else { 1 }).sum();
            assert_eq!(elements, usize::from(n));
        }
    }
}
