//! VC-5 (SMPTE ST 2073) wavelet decoding of GoPro GPR raws: a DNG whose raw image is one VC-5 coded tile
//! (`Compression` 9).
//!
//! Port of the VC-5 decoder in GoPro's GPR SDK (<https://github.com/gopro/gpr>, `source/lib/vc5_common` and
//! `source/lib/vc5_decoder`), Copyright 2018 GoPro Inc., licensed under the Apache License 2.0 or the MIT License, at
//! your option (see NOTICE). Translated to Rust and restructured by the LightCraft contributors (2026; moved here from
//! LightCraft's crates/raw): the stream is first split into its coded bands by their chunk sizes, the bands are then
//! decoded and the four channels reconstructed in parallel, and malformed input returns an error where the C code
//! asserts. The arithmetic (codebook, companding, dequantization, the 2/6 inverse wavelet with its edge filters, the
//! log curve and the Bayer repacking) follows the reference exactly. No other raw decoder's source was consulted.
//!
//! The stream, as GoPro's cameras write it (measured on the CC0 raw.pixls.us files of the HERO5 to HERO11 and the
//! FUSION):
//!
//! - 32-bit big-endian segments: a 16-bit tag (negative = optional) and a 16-bit value, starting with the marker
//!   `VC-5`. Header tags give the image size, four channels (`GS`, `RG`, `BG`, `GD`: green sum, red and blue minus
//!   green, green difference, each a quarter of the mosaic) and the 2×2 raw pattern.
//! - A codeblock tag (`0x6000` bits set) carries the size, in 32-bit words, of one coded subband of the current channel.
//!   Each channel has three wavelet levels and ten subbands: the lowpass band of the smallest level (fixed-width
//!   values) and nine highpass bands, run-length coded with codebook 17 and companded.
//! - Reconstruction inverts the levels from the smallest up, combines the four channels into the Bayer mosaic and
//!   expands the 12-bit log-coded values with the decoder's log curve to 14 bits (RGGB files) or 12 bits (the others),
//!   as the SDK does.

use crate::{RawError, Result};
use rayon::prelude::*;

/// The segment that starts every stream (`VC-5`).
const START_MARKER: u32 = 0x5643_2d35;

const TAG_CHANNEL_COUNT: u16 = 12;
const TAG_IMAGE_WIDTH: u16 = 20;
const TAG_IMAGE_HEIGHT: u16 = 21;
const TAG_LOWPASS_PRECISION: u16 = 35;
const TAG_SUBBAND_NUMBER: u16 = 48;
const TAG_QUANTIZATION: u16 = 53;
const TAG_CHANNEL_NUMBER: u16 = 62;
const TAG_IMAGE_FORMAT: u16 = 84;
const TAG_BITS_PER_COMPONENT: u16 = 101;
const TAG_MAX_BITS_PER_COMPONENT: u16 = 102;
const TAG_CHANNEL_WIDTH: u16 = 104;
const TAG_CHANNEL_HEIGHT: u16 = 105;
const TAG_PATTERN_WIDTH: u16 = 106;
const TAG_PATTERN_HEIGHT: u16 = 107;
const TAG_COMPONENTS_PER_SAMPLE: u16 = 108;
const TAG_PRESCALE_SHIFT: u16 = 109;
const TAG_LARGE_CODEBLOCK: u16 = 0x6000;
const TAG_SMALL_CHUNK: u16 = 0x4000;
const TAG_LARGE_CHUNK: u16 = 0x2000;
const TAG_INVERSE_PERMUTATION: u16 = 0x4001;
const TAG_INVERSE_TRANSFORM: u16 = 0x4002;
const TAG_INVERSE_TRANSFORM16: u16 = 0x4003;
const TAG_UNIQUE_IMAGE_IDENTIFIER: u16 = 0x4004;

/// `ImageFormat` of a Bayer raw image.
const IMAGE_FORMAT_RAW: u16 = 4;
const CHANNELS: usize = 4;
const WAVELETS: usize = 3;
const SUBBANDS: usize = 10;
/// Wavelet level of each subband, in stream order (smallest level first; bands LL, LH, HL, HH, then LH, HL, HH).
const SUBBAND_WAVELET: [usize; SUBBANDS] = [2, 2, 2, 2, 1, 1, 1, 0, 0, 0];
/// The channels' midpoint (12-bit) for the colour differences.
const MIDPOINT: i32 = 2048;

/// How the SDK lays the decoded colours out: the DNG's 2×2 `CFAPattern` picks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    /// `R G / G B`, 14-bit output.
    Rggb14,
    /// `G B / R G`, 12-bit output.
    Gbrg12,
}

impl Layout {
    /// The layout of a DNG `CFAPattern` (0 red, 1 green, 2 blue), `None` for patterns the SDK doesn't write.
    pub(crate) fn from_cfa(pattern: &[u8]) -> Option<Layout> {
        match pattern {
            [0, 1, 1, 2] => Some(Layout::Rggb14),
            [1, 2, 0, 1] => Some(Layout::Gbrg12),
            _ => None,
        }
    }

    fn bits(self) -> u32 {
        match self {
            Layout::Rggb14 => 14,
            Layout::Gbrg12 => 12,
        }
    }
}

fn corrupt(why: impl std::fmt::Display) -> RawError {
    RawError::Corrupt(format!("VC-5: {why}"))
}

fn unsupported(why: impl std::fmt::Display) -> RawError {
    RawError::Unsupported(format!("VC-5 {why}"))
}

/// Decode the VC-5 stream `src` of a `width` × `height` mosaic into its samples, row-major.
pub(crate) fn decode(src: &[u8], width: usize, height: usize, layout: Layout) -> Result<Vec<u16>> {
    let stream = parse(src)?;
    if (stream.width, stream.height) != (width, height) {
        return Err(corrupt(format!("{}×{} stream in a {width}×{height} image", stream.width, stream.height)));
    }
    let dims = Dims::new(width, height)?;
    // every coded band, decoded independently
    let bands: Vec<(usize, usize, Vec<i16>)> = stream
        .blocks
        .par_iter()
        .map(|b| {
            let (w, h) = SUBBAND_WAVELET.get(b.subband).and_then(|&l| dims.wavelet.get(l)).copied().ok_or_else(|| corrupt("subband number"))?;
            let bytes = src.get(b.start..b.end).ok_or_else(|| corrupt("codeblock outside the stream"))?;
            let band = if b.subband == 0 { lowpass_band(bytes, w * h, b.lowpass_precision)? } else { highpass_band(bytes, w * h)? };
            Ok((b.channel, b.subband, band))
        })
        .collect::<Result<_>>()?;
    let mut channels: Vec<Channel> = (0..CHANNELS).map(|_| Channel::default()).collect();
    for (b, (channel, subband, band)) in stream.blocks.iter().zip(bands) {
        let c = channels.get_mut(channel).ok_or_else(|| corrupt("channel number"))?;
        let slot = c.bands.get_mut(subband).ok_or_else(|| corrupt("subband number"))?;
        *slot = Some(band);
        if let Some(q) = c.quant.get_mut(subband) {
            *q = b.quant;
        }
    }
    let prescale = stream.prescale;
    let components: Vec<Vec<i16>> = channels.into_par_iter().map(|c| reconstruct(c, &dims, prescale)).collect::<Result<_>>()?;
    let [gs, rg, bg, gd] = components.as_slice() else { return Err(corrupt("channel count")) };
    Ok(pack([gs.as_slice(), rg.as_slice(), bg.as_slice(), gd.as_slice()], &dims, layout))
}

/// The sizes of a mosaic's channels and of each wavelet level's bands.
struct Dims {
    width: usize,
    height: usize,
    /// One colour channel: a quarter of the mosaic.
    channel: (usize, usize),
    /// The bands of wavelet level 0 (largest), 1 and 2.
    wavelet: [(usize, usize); WAVELETS],
}

impl Dims {
    fn new(width: usize, height: usize) -> Result<Dims> {
        if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(unsupported(format!("image of odd size {width}×{height}")));
        }
        let channel = (width / 2, height / 2);
        let half = |(w, h): (usize, usize)| (w.div_ceil(2), h.div_ceil(2));
        let w0 = half(channel);
        let w1 = half(w0);
        let w2 = half(w1);
        // the edge filters read three rows and columns of every band
        if w2.0 < 3 || w2.1 < 3 {
            return Err(unsupported(format!("image too small ({width}×{height})")));
        }
        Ok(Dims { width, height, channel, wavelet: [w0, w1, w2] })
    }
}

/// One coded subband: where its codeblock lies in the stream and the parameters in effect for it.
struct Block {
    channel: usize,
    subband: usize,
    start: usize,
    end: usize,
    quant: i32,
    lowpass_precision: u32,
}

struct Stream {
    width: usize,
    height: usize,
    /// Prescale shift of each wavelet level's inverse transform.
    prescale: [u16; WAVELETS],
    blocks: Vec<Block>,
}

/// Walk the segments of a stream, as the reference decoder's `UpdateCodecState` does, and collect its codeblocks.
fn parse(src: &[u8]) -> Result<Stream> {
    let segment = |pos: usize| -> Result<(i16, u16)> {
        let b = src.get(pos..pos.saturating_add(4)).ok_or_else(|| corrupt("stream ends before every band was read"))?;
        Ok((i16::from_be_bytes([b[0], b[1]]), u16::from_be_bytes([b[2], b[3]])))
    };
    let (tag, value) = segment(0)?;
    if (tag as u16 as u32) << 16 | value as u32 != START_MARKER {
        return Err(corrupt("no start marker"));
    }
    let (mut width, mut height, mut channel_count) = (0usize, 0usize, 0usize);
    let (mut image_format, mut pattern, mut components_per_sample, mut max_bits) = (None, (0, 0), None, None);
    let (mut channel, mut subband, mut quant, mut lowpass_precision) = (0usize, 0usize, 0i32, 16u32);
    let mut prescale = [0u16; WAVELETS];
    let mut prescale_tag: Option<u16> = None;
    let mut header_finished = false;
    let mut blocks: Vec<Block> = Vec::new();
    let mut seen = [[false; SUBBANDS]; CHANNELS];
    let mut pos = 4usize;
    loop {
        let (raw_tag, value) = segment(pos)?;
        pos += 4;
        let optional = raw_tag < 0;
        let tag = raw_tag.unsigned_abs();
        let mut header = false;
        match tag {
            TAG_CHANNEL_COUNT => {
                channel_count = value as usize;
                header = true;
            }
            TAG_IMAGE_WIDTH => {
                width = value as usize;
                header = true;
            }
            TAG_IMAGE_HEIGHT => {
                height = value as usize;
                header = true;
            }
            TAG_IMAGE_FORMAT => {
                image_format = Some(value);
                header = true;
            }
            TAG_PATTERN_WIDTH => {
                pattern.0 = value;
                header = true;
            }
            TAG_PATTERN_HEIGHT => {
                pattern.1 = value;
                header = true;
            }
            TAG_COMPONENTS_PER_SAMPLE => {
                components_per_sample = Some(value);
                header = true;
            }
            TAG_MAX_BITS_PER_COMPONENT => {
                max_bits = Some(value);
                header = true;
            }
            TAG_SUBBAND_NUMBER => subband = value as usize,
            // a 16-bit tag value read as the reference's unsigned quantization
            TAG_QUANTIZATION => quant = i32::from(value),
            TAG_LOWPASS_PRECISION => {
                if !(8..=32).contains(&value) {
                    return Err(corrupt(format!("lowpass precision {value}")));
                }
                // the coefficients are 16-bit: wider values would wrap (as they do in the reference)
                if value > 16 {
                    return Err(unsupported(format!("lowpass precision of {value} bits")));
                }
                lowpass_precision = u32::from(value);
            }
            TAG_CHANNEL_NUMBER => channel = value as usize,
            TAG_BITS_PER_COMPONENT => {}
            TAG_PRESCALE_SHIFT => {
                // GoPro writes the same table before every channel. The SDK applies whichever table is current when a
                // level completes; one table for the whole stream keeps that true without tracking when it changed.
                if prescale_tag.is_some_and(|earlier| earlier != value) {
                    return Err(unsupported("prescale table that changes within the stream"));
                }
                prescale_tag = Some(value);
                for (i, p) in prescale.iter_mut().enumerate() {
                    *p = (value >> (14 - 2 * i)) & 3;
                    if *p != 0 && *p != 2 {
                        return Err(unsupported(format!("prescale value {}", *p)));
                    }
                }
            }
            TAG_CHANNEL_WIDTH | TAG_CHANNEL_HEIGHT => return Err(corrupt("channel size tag in an image-format stream")),
            _ => {
                let words = if tag & (TAG_SMALL_CHUNK | TAG_LARGE_CHUNK) == 0 {
                    0
                } else if tag & TAG_LARGE_CHUNK != 0 {
                    usize::from(value) | usize::from(tag & 0xff) << 16
                } else {
                    usize::from(value)
                };
                let end = || pos.checked_add(words * 4).filter(|&e| e <= src.len()).ok_or_else(|| corrupt("chunk past the end of the stream"));
                if tag & TAG_LARGE_CODEBLOCK == TAG_LARGE_CODEBLOCK {
                    let end = end()?;
                    if channel >= channel_count.min(CHANNELS) || subband >= SUBBANDS {
                        return Err(corrupt(format!("codeblock for channel {channel}, subband {subband}")));
                    }
                    // each band once: a repeated empty codeblock would make a short stream decode a band many times
                    let Some(s) = seen.get_mut(channel).and_then(|c| c.get_mut(subband)).filter(|s| !**s) else {
                        return Err(corrupt(format!("second codeblock for channel {channel}, subband {subband}")));
                    };
                    *s = true;
                    blocks.push(Block { channel, subband, start: pos, end, quant, lowpass_precision });
                    pos = end;
                    subband += 1;
                    if subband == SUBBANDS {
                        channel += 1;
                        subband = 0;
                    }
                } else if matches!(tag, TAG_INVERSE_PERMUTATION | TAG_INVERSE_TRANSFORM | TAG_INVERSE_TRANSFORM16) {
                    return Err(unsupported("inverse component transform"));
                } else if tag == TAG_UNIQUE_IMAGE_IDENTIFIER && optional {
                    pos = end()?;
                } else if tag & TAG_LARGE_CHUNK != 0 {
                    // section markers (and other large chunks) frame what follows; their contents are read in place
                } else if !optional {
                    return Err(corrupt(format!("unknown required tag {tag}")));
                } else {
                    pos = end()?;
                }
            }
        }
        // (an optional header tag is taken wherever it appears, as the reference does without the layers part)
        if header {
            if !optional && header_finished {
                return Err(corrupt(format!("header tag {tag} after the header")));
            }
        } else {
            header_finished = true;
        }
        if channel_count > 0 && seen.iter().take(channel_count).all(|c| c.iter().all(|&s| s)) {
            break;
        }
    }
    if channel_count != CHANNELS {
        return Err(unsupported(format!("stream with {channel_count} channels")));
    }
    if image_format != Some(IMAGE_FORMAT_RAW) || pattern != (2, 2) || components_per_sample.is_some_and(|c| c != 1) {
        return Err(unsupported(format!("image format {image_format:?} with a {}×{} pattern", pattern.0, pattern.1)));
    }
    if max_bits.is_some_and(|b| b != 12) {
        return Err(unsupported(format!("{}-bit components", max_bits.unwrap_or(0))));
    }
    if width.checked_mul(height).is_none_or(|n| n == 0 || n > crate::MAX_SAMPLES) {
        return Err(corrupt(format!("image size {width}×{height}")));
    }
    Ok(Stream { width, height, prescale, blocks })
}

/// MSB-first reader over big-endian bytes, reading zeros past the end ([`BitReader::overrun`] tells).
struct BitReader<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(src: &'a [u8]) -> Self {
        BitReader { src, pos: 0 }
    }

    /// The next `n` (1..=32) bits.
    #[inline]
    fn peek(&self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let byte = self.pos / 8;
        let word = match self.src.get(byte..byte + 8) {
            Some(b) => u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            None => (0..8).fold(0u64, |acc, i| acc << 8 | u64::from(self.src.get(byte + i).copied().unwrap_or(0))),
        };
        ((word << (self.pos % 8)) >> (64 - n)) as u32
    }

    #[inline]
    fn get(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.pos += n as usize;
        v
    }

    fn overrun(&self) -> bool {
        self.pos > self.src.len() * 8
    }
}

/// The lowpass band of the smallest wavelet: `n` values of `precision` bits each, kept as 16-bit coefficients.
fn lowpass_band(src: &[u8], n: usize, precision: u32) -> Result<Vec<i16>> {
    if n.checked_mul(precision as usize).is_none_or(|bits| bits > src.len() * 8) {
        return Err(corrupt("lowpass band longer than its codeblock"));
    }
    let mut r = BitReader::new(src);
    Ok((0..n).map(|_| r.get(precision) as i16).collect())
}

/// A highpass band: `n` run-length coded (companded, still quantized) coefficients and the band-end codeword.
fn highpass_band(src: &[u8], n: usize) -> Result<Vec<i16>> {
    let book = codebook();
    // every codeword takes at least one bit and covers at most `max_run` samples, and the band-end marker follows
    if n.div_ceil(book.max_run).saturating_add(1) > src.len().saturating_mul(8) {
        return Err(corrupt("band longer than its codeblock"));
    }
    let mut r = BitReader::new(src);
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let (count, magnitude) = book.read(&mut r)?;
        if count == 0 {
            return Err(corrupt("band ends early"));
        }
        let value = if magnitude != 0 && r.get(1) == 1 { -magnitude } else { magnitude };
        if r.overrun() {
            return Err(corrupt("band longer than its codeblock"));
        }
        if count > n - out.len() {
            return Err(corrupt("run past the end of its band"));
        }
        out.resize(out.len() + count, value);
    }
    let (count, magnitude) = book.read(&mut r)?;
    if count != 0 || magnitude != 1 {
        return Err(corrupt("no band-end marker"));
    }
    if r.overrun() {
        return Err(corrupt("band longer than its codeblock"));
    }
    Ok(out)
}

/// Codebook 17 of the reference decoder (`table17.inc`): (code length, code, run length, magnitude), sorted by code
/// length. A run length of 0 is the band-end marker.
#[rustfmt::skip]
const TABLE17: [(u8, u32, u16, i16); 264] = [
    (1, 0x0, 1, 0), (2, 0x2, 1, 1), (3, 0x7, 1, 2), (5, 0x19, 1, 3),
    (6, 0x30, 1, 4), (6, 0x36, 1, 5), (7, 0x6F, 1, 8), (7, 0x63, 1, 6),
    (7, 0x69, 12, 0), (7, 0x6B, 1, 7), (8, 0xD1, 20, 0), (8, 0xD4, 1, 9),
    (8, 0xDC, 1, 10), (9, 0x189, 1, 11), (9, 0x18A, 32, 0), (9, 0x1A0, 1, 12),
    (9, 0x1AB, 1, 13), (10, 0x377, 1, 18), (10, 0x310, 1, 14), (10, 0x316, 1, 15),
    (10, 0x343, 60, 0), (10, 0x354, 1, 16), (10, 0x375, 1, 17), (11, 0x623, 1, 19),
    (11, 0x684, 1, 20), (11, 0x685, 100, 0), (11, 0x6AB, 1, 21), (11, 0x6EC, 1, 22),
    (12, 0xDDB, 1, 29), (12, 0xC5C, 1, 24), (12, 0xC5E, 1, 25), (12, 0xC44, 1, 23),
    (12, 0xD55, 1, 26), (12, 0xDD1, 1, 27), (12, 0xDD3, 1, 28), (13, 0x1BB5, 1, 35),
    (13, 0x188B, 1, 30), (13, 0x18BB, 1, 31), (13, 0x18BF, 180, 0), (13, 0x1AA8, 1, 32),
    (13, 0x1BA0, 1, 33), (13, 0x1BA5, 320, 0), (13, 0x1BA4, 1, 34), (14, 0x3115, 1, 36),
    (14, 0x3175, 1, 37), (14, 0x317D, 1, 38), (14, 0x3553, 1, 39), (14, 0x3768, 1, 40),
    (15, 0x6E87, 1, 46), (15, 0x6ED3, 1, 47), (15, 0x62E8, 1, 42), (15, 0x62F8, 1, 43),
    (15, 0x6228, 1, 41), (15, 0x6AA4, 1, 44), (15, 0x6E85, 1, 45), (16, 0xC453, 1, 48),
    (16, 0xC5D3, 1, 49), (16, 0xC5F3, 1, 50), (16, 0xDDA4, 1, 53), (16, 0xDD08, 1, 51),
    (16, 0xDD0C, 1, 52), (17, 0x1BB4B, 1, 61), (17, 0x1BB4A, 1, 60), (17, 0x18BA5, 1, 55),
    (17, 0x18BE5, 1, 56), (17, 0x1AA95, 1, 57), (17, 0x1AA97, 1, 58), (17, 0x188A4, 1, 54),
    (17, 0x1BA13, 1, 59), (18, 0x31748, 1, 62), (18, 0x317C8, 1, 63), (18, 0x35528, 1, 64),
    (18, 0x3552C, 1, 65), (18, 0x37424, 1, 66), (18, 0x37434, 1, 67), (18, 0x37436, 1, 68),
    (19, 0x62294, 1, 69), (19, 0x62E92, 1, 70), (19, 0x62F92, 1, 71), (19, 0x6AA52, 1, 72),
    (19, 0x6AA5A, 1, 73), (19, 0x6E86A, 1, 75), (19, 0x6E86E, 1, 76), (19, 0x6E84A, 1, 74),
    (20, 0xC452A, 1, 77), (20, 0xC5D27, 1, 78), (20, 0xC5F26, 1, 79), (20, 0xD54A6, 1, 80),
    (20, 0xD54B6, 1, 81), (20, 0xDD096, 1, 82), (20, 0xDD0D6, 1, 83), (20, 0xDD0DE, 1, 84),
    (21, 0x188A56, 1, 85), (21, 0x18BA4D, 1, 86), (21, 0x18BE4E, 1, 87), (21, 0x18BE4F, 1, 88),
    (21, 0x1AA96E, 1, 89), (21, 0x1BA12E, 1, 90), (21, 0x1BA12F, 1, 91), (21, 0x1BA1AF, 1, 92),
    (21, 0x1BA1BF, 1, 93), (22, 0x37435D, 1, 99), (22, 0x37437D, 1, 100), (22, 0x317498, 1, 94),
    (22, 0x35529C, 1, 95), (22, 0x35529D, 1, 96), (22, 0x3552DE, 1, 97), (22, 0x3552DF, 1, 98),
    (23, 0x62E933, 1, 102), (23, 0x62295D, 1, 101), (23, 0x6AA53D, 1, 103), (23, 0x6AA53F, 1, 105),
    (23, 0x6AA53E, 1, 104), (23, 0x6E86B9, 1, 106), (23, 0x6E86F8, 1, 107), (24, 0xD54A79, 1, 111),
    (24, 0xC5D265, 1, 109), (24, 0xC452B8, 1, 108), (24, 0xDD0D71, 1, 113), (24, 0xD54A78, 1, 110),
    (24, 0xDD0D70, 1, 112), (24, 0xDD0DF2, 1, 114), (24, 0xDD0DF3, 1, 115), (25, 0x188A5F6, 1, 225),
    (25, 0x188A5F5, 1, 189), (25, 0x188A5F4, 1, 188), (25, 0x188A5F3, 1, 203), (25, 0x188A5F2, 1, 202),
    (25, 0x188A5F1, 1, 197), (25, 0x188A5F0, 1, 207), (25, 0x188A5EF, 1, 169), (25, 0x188A5EE, 1, 223),
    (25, 0x188A5ED, 1, 159), (25, 0x188A5AA, 1, 235), (25, 0x188A5E3, 1, 152), (25, 0x188A5DF, 1, 192),
    (25, 0x188A589, 1, 179), (25, 0x188A5DD, 1, 201), (25, 0x188A578, 1, 172), (25, 0x188A5E0, 1, 149),
    (25, 0x188A588, 1, 178), (25, 0x188A5D6, 1, 120), (25, 0x188A5DB, 1, 219), (25, 0x188A5E1, 1, 150),
    (25, 0x188A587, 1, 127), (25, 0x188A59A, 1, 211), (25, 0x188A5C4, 1, 125), (25, 0x188A5EC, 1, 158),
    (25, 0x188A586, 1, 247), (25, 0x188A573, 1, 238), (25, 0x188A59C, 1, 163), (25, 0x188A5C8, 1, 228),
    (25, 0x188A5FB, 1, 183), (25, 0x188A5A1, 1, 217), (25, 0x188A5EB, 1, 168), (25, 0x188A5A8, 1, 122),
    (25, 0x188A584, 1, 128), (25, 0x188A5D2, 1, 249), (25, 0x188A599, 1, 187), (25, 0x188A598, 1, 186),
    (25, 0x188A583, 1, 136), (25, 0x18BA4C9, 1, 181), (25, 0x188A5D0, 1, 255), (25, 0x188A594, 1, 230),
    (25, 0x188A582, 1, 135), (25, 0x188A5CB, 1, 233), (25, 0x188A5D8, 1, 222), (25, 0x188A5E7, 1, 145),
    (25, 0x188A581, 1, 134), (25, 0x188A5EA, 1, 167), (25, 0x188A5A9, 1, 248), (25, 0x188A5A6, 1, 209),
    (25, 0x188A580, 1, 243), (25, 0x188A5A0, 1, 216), (25, 0x188A59D, 1, 164), (25, 0x188A5C3, 1, 140),
    (25, 0x188A57F, 1, 157), (25, 0x188A5C0, 1, 239), (25, 0x188A5DE, 1, 191), (25, 0x188A5D4, 1, 251),
    (25, 0x188A57E, 1, 156), (25, 0x188A5C2, 1, 139), (25, 0x188A592, 1, 242), (25, 0x188A5CD, 1, 133),
    (25, 0x188A57D, 1, 162), (25, 0x188A5A3, 1, 213), (25, 0x188A5E8, 1, 165), (25, 0x188A5A2, 1, 212),
    (25, 0x188A57C, 1, 227), (25, 0x188A58E, 1, 198), (25, 0x188A5B3, 1, 236), (25, 0x188A5B2, 1, 234),
    (25, 0x188A5B1, 1, 117), (25, 0x188A5B0, 1, 215), (25, 0x188A5AF, 1, 124), (25, 0x188A5AE, 1, 123),
    (25, 0x188A5AD, 1, 254), (25, 0x188A5AC, 1, 253), (25, 0x188A5AB, 1, 148), (25, 0x188A5DA, 1, 218),
    (25, 0x188A5E4, 1, 146), (25, 0x188A5E5, 1, 147), (25, 0x188A5D9, 1, 224), (25, 0x188A5B5, 1, 143),
    (25, 0x188A5BC, 1, 184), (25, 0x188A5BD, 1, 185), (25, 0x188A5E9, 1, 166), (25, 0x188A5CC, 1, 132),
    (25, 0x188A585, 1, 129), (25, 0x188A5D3, 1, 250), (25, 0x188A5E2, 1, 151), (25, 0x188A595, 1, 119),
    (25, 0x188A596, 1, 193), (25, 0x188A5B8, 1, 176), (25, 0x188A590, 1, 245), (25, 0x188A5C9, 1, 229),
    (25, 0x188A5A4, 1, 206), (25, 0x188A5E6, 1, 144), (25, 0x188A5A5, 1, 208), (25, 0x188A5CE, 1, 137),
    (25, 0x188A5BF, 1, 241), (25, 0x188A572, 1, 237), (25, 0x188A59B, 1, 190), (25, 0x188A5BE, 1, 240),
    (25, 0x188A5C7, 1, 131), (25, 0x188A5CA, 1, 232), (25, 0x188A5D5, 1, 252), (25, 0x188A57B, 1, 171),
    (25, 0x188A58D, 1, 205), (25, 0x188A58C, 1, 204), (25, 0x188A58B, 1, 118), (25, 0x188A58A, 1, 214),
    (25, 0x18BA4C8, 1, 180), (25, 0x188A5C5, 1, 126), (25, 0x188A5FA, 1, 182), (25, 0x188A5BB, 1, 175),
    (25, 0x188A5C1, 1, 141), (25, 0x188A5CF, 1, 138), (25, 0x188A5B9, 1, 177), (25, 0x188A5B6, 1, 153),
    (25, 0x188A597, 1, 194), (25, 0x188A5FE, 1, 160), (25, 0x188A5D7, 1, 121), (25, 0x188A5BA, 1, 174),
    (25, 0x188A591, 1, 246), (25, 0x188A5C6, 1, 130), (25, 0x188A5DC, 1, 200), (25, 0x188A57A, 1, 170),
    (25, 0x188A59F, 1, 221), (25, 0x188A5F9, 1, 196), (25, 0x188A5B4, 1, 142), (25, 0x188A5A7, 1, 210),
    (25, 0x188A58F, 1, 199), (25, 0x188A5FD, 1, 155), (25, 0x188A5B7, 1, 154), (25, 0x188A593, 1, 244),
    (25, 0x188A59E, 1, 220), (25, 0x188A5F8, 1, 195), (25, 0x188A5FF, 1, 161), (25, 0x188A5FC, 1, 231),
    (25, 0x188A579, 1, 173), (25, 0x188A5F7, 1, 226), (26, 0x3114BA2, 1, 116), (26, 0x3114BA3, 0, 1),
];

/// Bits the first lookup reads.
const LUT_BITS: u32 = 12;
/// Longest codeword of codebook 17.
const MAX_CODE_BITS: u32 = 26;

struct Codebook {
    /// For each `LUT_BITS`-bit prefix: index + 1 of the codeword it starts with (0: a longer codeword).
    lut: Vec<u16>,
    /// The first entry longer than `LUT_BITS`.
    long_start: usize,
    /// The longest run one codeword codes.
    max_run: usize,
}

impl Codebook {
    fn build() -> Codebook {
        let mut lut = vec![0u16; 1 << LUT_BITS];
        let mut long_start = TABLE17.len();
        for (i, &(len, code, _, _)) in TABLE17.iter().enumerate() {
            let len = u32::from(len);
            if len > LUT_BITS {
                long_start = long_start.min(i);
                continue;
            }
            let first = (code << (LUT_BITS - len)) as usize;
            for slot in lut.iter_mut().skip(first).take(1 << (LUT_BITS - len)) {
                *slot = i as u16 + 1;
            }
        }
        let max_run = TABLE17.iter().map(|e| usize::from(e.2)).max().unwrap_or(1).max(1);
        Codebook { lut, long_start, max_run }
    }

    /// The next codeword's (run length, magnitude).
    #[inline]
    fn read(&self, r: &mut BitReader) -> Result<(usize, i16)> {
        let i = self.lut.get(r.peek(LUT_BITS) as usize).copied().unwrap_or(0);
        let entry = if i > 0 {
            TABLE17.get(usize::from(i) - 1)
        } else {
            let bits = r.peek(MAX_CODE_BITS);
            TABLE17.iter().skip(self.long_start).find(|&&(len, code, _, _)| bits >> (MAX_CODE_BITS - u32::from(len)) == code)
        };
        let &(len, _, count, magnitude) = entry.ok_or_else(|| corrupt("invalid codeword"))?;
        r.pos += usize::from(len);
        Ok((usize::from(count), magnitude))
    }
}

fn codebook() -> &'static Codebook {
    static BOOK: std::sync::OnceLock<Codebook> = std::sync::OnceLock::new();
    BOOK.get_or_init(Codebook::build)
}

/// One channel's coded bands (by subband number) and their quantization.
#[derive(Default)]
struct Channel {
    bands: [Option<Vec<i16>>; SUBBANDS],
    quant: [i32; SUBBANDS],
}

/// Invert a channel's three wavelet levels, smallest first, into its component values (stored as the reference's
/// unsigned 16-bit component array).
fn reconstruct(mut c: Channel, dims: &Dims, prescale: [u16; WAVELETS]) -> Result<Vec<i16>> {
    let mut take = |subband: usize| c.bands.get_mut(subband).and_then(Option::take).ok_or_else(|| corrupt("missing band"));
    let mut lowpass = take(0)?;
    for level in (0..WAVELETS).rev() {
        let first = match level {
            2 => 1,
            1 => 4,
            _ => 7,
        };
        let (lh, hl, hh) = (take(first)?, take(first + 1)?, take(first + 2)?);
        let quant =
            [c.quant.get(first).copied().unwrap_or(0), c.quant.get(first + 1).copied().unwrap_or(0), c.quant.get(first + 2).copied().unwrap_or(0)];
        let input = dims.wavelet[level];
        let output = if level == 0 { dims.channel } else { dims.wavelet[level - 1] };
        lowpass = invert(&lowpass, [&lh, &hl, &hh], input, output, quant, prescale[level]);
    }
    Ok(lowpass)
}

/// The reference's `UncompandedValue`: undo the cubic companding of a coefficient.
#[inline]
fn uncompand(value: i32) -> i32 {
    let m = value.unsigned_abs() as f64;
    let magnitude = value.unsigned_abs() as i32 + (m * m * m * 768.0 / (255.0 * 255.0 * 255.0)) as i32;
    if value < 0 { -magnitude } else { magnitude }
}

#[inline]
fn clamp_pixel(v: i32) -> i16 {
    v.clamp(i16::MIN.into(), i16::MAX.into()) as i16
}

/// A highpass band, uncompanded and dequantized.
fn dequantize(band: &[i16], quant: i32) -> Vec<i16> {
    band.iter().map(|&v| clamp_pixel(uncompand(v.into()).saturating_mul(quant))).collect()
}

/// One inverse 2/6 wavelet step (the reference's `InvertSpatialQuant16s` / `InvertSpatialQuantDescale16s`): the
/// `input` sized lowpass band and the three highpass bands `[LH, HL, HH]` give an `output` sized band. A `prescale` of
/// 2 scales the result back up by one bit (and keeps it signed); otherwise it is clamped to 14 bits.
fn invert(lowpass: &[i16], highpass: [&[i16]; 3], input: (usize, usize), output: (usize, usize), quant: [i32; 3], prescale: u16) -> Vec<i16> {
    let (w, h) = input;
    let (ow, oh) = output;
    let [lh, hl, hh] = [0, 1, 2].map(|i| dequantize(highpass[i], quant[i]));
    let at = |band: &[i16], row: usize, col: usize| -> i32 { band.get(row * w + col).copied().map_or(0, i32::from) };
    let mut out = vec![0i16; ow * oh];
    out.par_chunks_mut(2 * ow).enumerate().for_each(|(row, rows)| {
        let mut even_low = vec![0i16; w];
        let mut odd_low = vec![0i16; w];
        let mut even_high = vec![0i16; w];
        let mut odd_high = vec![0i16; w];
        for col in 0..w {
            let vertical = |band: &[i16], highpass: i32| -> (i16, i16) {
                let v = |r: usize| at(band, r, col);
                let (even, odd) = if row == 0 {
                    (((11 * v(0) - 4 * v(1) + v(2) + 4) >> 3) + highpass, ((5 * v(0) + 4 * v(1) - v(2) + 4) >> 3) - highpass)
                } else if row + 1 == h {
                    let r = row;
                    (((5 * v(r) + 4 * v(r - 1) - v(r - 2) + 4) >> 3) + highpass, ((11 * v(r) - 4 * v(r - 1) + v(r - 2) + 4) >> 3) - highpass)
                } else {
                    let r = row;
                    (((v(r - 1) - v(r + 1) + 4) >> 3) + v(r) + highpass, ((v(r + 1) - v(r - 1) + 4) >> 3) + v(r) - highpass)
                };
                (clamp_pixel(even >> 1), clamp_pixel(odd >> 1))
            };
            (even_low[col], odd_low[col]) = vertical(lowpass, at(&hl, row, col));
            (even_high[col], odd_high[col]) = vertical(&lh, at(&hh, row, col));
        }
        let (even_out, odd_out) = rows.split_at_mut(ow.min(rows.len()));
        horizontal(&even_low, &even_high, even_out, prescale);
        if !odd_out.is_empty() {
            horizontal(&odd_low, &odd_high, odd_out, prescale);
        }
    });
    out
}

/// The horizontal half of [`invert`]: one row of lowpass and highpass values into an output row.
fn horizontal(low: &[i16], high: &[i16], out: &mut [i16], prescale: u16) {
    let w = low.len();
    let l = |c: usize| low.get(c).copied().map_or(0, i32::from);
    let hp = |c: usize| high.get(c).copied().map_or(0, i32::from);
    let finish = |v: i32| -> i16 {
        match prescale {
            p if p > 1 => clamp_pixel(v << u32::from(p == 2)),
            _ => (v >> 1).clamp(0, (1 << 14) - 1) as i16,
        }
    };
    let mut put = |i: usize, v: i32| {
        if let Some(o) = out.get_mut(i) {
            *o = finish(v);
        }
    };
    for col in 0..w {
        let (even, odd) = if col == 0 {
            (((11 * l(0) - 4 * l(1) + l(2) + 4) >> 3) + hp(0), ((5 * l(0) + 4 * l(1) - l(2) + 4) >> 3) - hp(0))
        } else if col + 1 == w {
            (((5 * l(col) + 4 * l(col - 1) - l(col - 2) + 4) >> 3) + hp(col), ((11 * l(col) - 4 * l(col - 1) + l(col - 2) + 4) >> 3) - hp(col))
        } else {
            (((l(col - 1) - l(col + 1) + 4) >> 3) + l(col) + hp(col), ((l(col + 1) - l(col - 1) + 4) >> 3) + l(col) - hp(col))
        };
        put(2 * col, even);
        put(2 * col + 1, odd);
    }
}

/// The decoder's log curve (`SetupDecoderLogCurve`): 12-bit log-coded values to 16-bit linear ones.
fn log_curve() -> &'static [u16; 4096] {
    static CURVE: std::sync::OnceLock<[u16; 4096]> = std::sync::OnceLock::new();
    CURVE.get_or_init(|| {
        let mut curve = [0u16; 4096];
        for (i, v) in curve.iter_mut().enumerate() {
            let output = (65535.0 * (113f64.powf(i as f64 / 4095.0) - 1.0) / 112.0) as f32;
            *v = (output as i32).clamp(0, 65535) as u16;
        }
        curve
    })
}

/// Combine the four channels into the Bayer mosaic (the reference's `PackComponentsToRAW`).
fn pack(c: [&[i16]; CHANNELS], dims: &Dims, layout: Layout) -> Vec<u16> {
    let curve = log_curve();
    let shift = 16 - layout.bits();
    let (cw, _) = dims.channel;
    let lin = |v: i32| curve.get(v.clamp(0, 4095) as usize).copied().unwrap_or(0) >> shift;
    let mut out = vec![0u16; dims.width * dims.height];
    out.par_chunks_mut(2 * dims.width).enumerate().for_each(|(row, rows)| {
        let (top, bottom) = rows.split_at_mut(dims.width.min(rows.len()));
        for col in 0..cw {
            // the component arrays hold unsigned 16-bit values
            let get = |k: usize| c[k].get(row * cw + col).map_or(0, |&v| i32::from(v as u16));
            let gs = get(0);
            let (rg, bg, gd) = (get(1) - MIDPOINT, get(2) - MIDPOINT, get(3) - MIDPOINT);
            let (r, b) = (lin((rg << 1) + gs), lin((bg << 1) + gs));
            let (g1, g2) = (lin(gs + gd), lin(gs - gd));
            let (first, second) = match layout {
                Layout::Rggb14 => ([r, g1], [g2, b]),
                Layout::Gbrg12 => ([g1, b], [r, g2]),
            };
            if let Some(t) = top.get_mut(2 * col..2 * col + 2) {
                t.copy_from_slice(&first);
            }
            if let Some(t) = bottom.get_mut(2 * col..2 * col + 2) {
                t.copy_from_slice(&second);
            }
        }
    });
    out
}

/// A VC-5 stream of a `width` × `height` mosaic as GoPro's cameras lay it out, for tests: each channel's lowpass
/// band holds `lowpass[channel]` everywhere and every highpass coefficient is `highpass` (0, or a magnitude codebook
/// 17 codes in a run of one). With `highpass` 0 the four components come out as `lowpass / 4` (`lowpass` a multiple
/// of 4, at most 32764).
#[cfg(any(test, feature = "testdata"))]
pub fn synthetic_stream(width: usize, height: usize, lowpass: [u16; CHANNELS], highpass: i16) -> Vec<u8> {
    struct Writer {
        out: Vec<u8>,
        acc: u64,
        n: u32,
    }
    impl Writer {
        fn put(&mut self, bits: u32, n: u32) {
            self.acc = self.acc << n | u64::from(bits);
            self.n += n;
            while self.n >= 8 {
                self.out.push((self.acc >> (self.n - 8)) as u8);
                self.n -= 8;
            }
        }
        /// Pad to a 32-bit word.
        fn align(&mut self) {
            if self.n > 0 {
                self.put(0, 8 - self.n);
            }
            while !self.out.len().is_multiple_of(4) {
                self.out.push(0);
            }
        }
    }
    let code = |count: u16, magnitude: i16| TABLE17.iter().find(|e| e.2 == count && e.3 == magnitude).map_or((1, 0), |e| (e.1, u32::from(e.0)));
    let mut s = Writer { out: Vec::new(), acc: 0, n: 0 };
    let seg = |s: &mut Writer, tag: i16, value: u16| s.put(u32::from(tag as u16) << 16 | u32::from(value), 32);
    seg(&mut s, 0x5643, 0x2d35);
    for (tag, value) in [(12, 4), (20, width as u16), (21, height as u16), (84, 4), (106, 2), (107, 2), (108, 1), (102, 12)] {
        seg(&mut s, tag, value);
    }
    let dims = (width / 2, height / 2);
    let half = |(w, h): (usize, usize)| (w.div_ceil(2), h.div_ceil(2));
    let w0 = half(dims);
    let w1 = half(w0);
    let wavelets = [w0, w1, half(w1)];
    for (channel, &low) in lowpass.iter().enumerate() {
        seg(&mut s, TAG_PRESCALE_SHIFT as i16, 0x2800);
        for subband in 0..SUBBANDS {
            let (w, h) = wavelets[SUBBAND_WAVELET[subband]];
            let mut band = Writer { out: Vec::new(), acc: 0, n: 0 };
            if subband == 0 {
                for _ in 0..w * h {
                    band.put(u32::from(low), 16);
                }
            } else {
                let (bits, len) = code(1, highpass.saturating_abs());
                for _ in 0..w * h {
                    band.put(bits, len);
                    if highpass != 0 {
                        band.put(u32::from(highpass < 0), 1);
                    }
                }
                let (end, len) = code(0, 1);
                band.put(end, len);
            }
            band.align();
            let words = band.out.len() / 4;
            for (tag, value) in [(62, channel as u16), (48, subband as u16), (35, 16), (53, 4)] {
                seg(&mut s, tag, value);
            }
            seg(&mut s, -((0x6000 | (words >> 16) & 0xff) as i16), words as u16);
            s.out.extend_from_slice(&band.out);
        }
    }
    s.out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_flat(width: usize, height: usize, lowpass: [u16; 4], layout: Layout) -> Vec<u16> {
        decode(&synthetic_stream(width, height, lowpass, 0), width, height, layout).unwrap()
    }

    /// Flat channels invert to flat components (`lowpass / 4`), whatever the band sizes (odd ones included).
    #[test]
    fn flat_bands_give_a_flat_mosaic() {
        for (w, h) in [(64, 48), (100, 76), (34, 34)] {
            let out = decode_flat(w, h, [4000, 8192, 8192, 8192], Layout::Rggb14);
            assert_eq!(out.len(), w * h);
            let want = log_curve()[1000] >> 2;
            assert!(out.iter().all(|&v| v == want), "{w}×{h}: {:?}", &out[..8]);
        }
    }

    /// The channels are green sum, red and blue differences (doubled) and green difference around 2048; the GBRG
    /// layout puts G1 B over R G2 and keeps 12 bits.
    #[test]
    fn channels_combine_into_the_layout() {
        let lowpass = [4 * 1000, 4 * (2048 + 100), 4 * (2048 - 50), 4 * (2048 + 10)];
        let curve = log_curve();
        let lin = |v: usize, bits: u32| curve[v] >> (16 - bits);
        let gbrg = decode_flat(64, 48, lowpass, Layout::Gbrg12);
        assert_eq!(&gbrg[..2], &[lin(1010, 12), lin(900, 12)]);
        assert_eq!(&gbrg[64..66], &[lin(1200, 12), lin(990, 12)]);
        let rggb = decode_flat(64, 48, lowpass, Layout::Rggb14);
        assert_eq!(&rggb[..2], &[lin(1200, 14), lin(1010, 14)]);
        assert_eq!(&rggb[64..66], &[lin(990, 14), lin(900, 14)]);
    }

    #[test]
    fn log_curve_spans_sixteen_bits() {
        let curve = log_curve();
        assert_eq!((curve[0], curve[4095]), (0, 65535));
        assert!(curve.windows(2).all(|p| p[0] <= p[1]));
    }

    #[test]
    fn companding_matches_the_reference_cubic() {
        // magnitude + magnitude³ · 768 / 255³, truncated
        assert_eq!([0, 1, 40, 128, 255, -255].map(uncompand), [0, 1, 42, 225, 1023, -1023]);
    }

    /// Codebook 17 is a prefix code and the decoder finds every codeword, short or long.
    #[test]
    fn every_codeword_decodes_to_its_entry() {
        for (i, a) in TABLE17.iter().enumerate() {
            for b in TABLE17.iter().skip(i + 1) {
                assert!(a.0 > b.0 || b.1 >> (b.0 - a.0) != a.1, "{a:?} is a prefix of {b:?}");
            }
            let bytes = ((a.1 as u64) << (64 - a.0)).to_be_bytes();
            let mut r = BitReader::new(&bytes);
            assert_eq!(codebook().read(&mut r).unwrap(), (usize::from(a.2), a.3), "{a:?}");
            assert_eq!(r.pos, usize::from(a.0));
        }
    }

    /// `TABLE17` is GoPro's codebook entry for entry, in order: `tests/data/gpr-table17.inc` is a verbatim copy of
    /// the SDK's `source/lib/vc5_common/table17.inc` (Apache-2.0 or MIT, see NOTICE), pinned by its SHA-256 at
    /// <https://raw.githubusercontent.com/gopro/gpr/446c736a38fb14f51343605c0780d347dc602f89/source/lib/vc5_common/table17.inc>.
    #[test]
    fn codebook_matches_the_sdk_table() {
        use sha2::{Digest, Sha256};
        // hashed with LF line ends, whatever the checkout's
        let src = include_str!("../tests/data/gpr-table17.inc").replace("\r\n", "\n");
        assert_eq!(format!("{:x}", Sha256::digest(src.as_bytes())), "7b3190bda6039fba5c74e9342e88f32dcd0b26b9fef37df1c85477834f7c38db");
        // the entries are the lines `{length, 0xcode, run, magnitude},`
        let entry = |line: &str| -> Option<(u8, u32, u16, i16)> {
            let inner = line.trim().strip_prefix('{')?.split_once('}')?.0;
            let [len, code, run, magnitude] = inner.split(',').map(str::trim).collect::<Vec<_>>()[..] else { return None };
            Some((len.parse().ok()?, u32::from_str_radix(code.strip_prefix("0x")?, 16).ok()?, run.parse().ok()?, magnitude.parse().ok()?))
        };
        let sdk: Vec<_> = src.lines().filter_map(entry).collect();
        assert_eq!(sdk.len(), 264, "entries parsed from the SDK table");
        assert_eq!(sdk, TABLE17);
    }

    /// Non-zero highpass coefficients (signed, companded, quantized) decode; the result is in range.
    #[test]
    fn highpass_detail_decodes() {
        for highpass in [5, -5, 200] {
            let s = synthetic_stream(64, 48, [4000, 8192, 8192, 8192], highpass);
            let out = decode(&s, 64, 48, Layout::Rggb14).unwrap();
            assert!(out.iter().all(|&v| v < 1 << 14));
            assert!(out.iter().any(|&v| v != out[0]), "{highpass}: detail lost");
        }
    }

    /// Only the band-end codeword ends a band, and a band can't claim more samples than its codeblock can code.
    #[test]
    fn highpass_bands_end_on_the_marker_and_fit_their_codeblock() {
        // `0` (a zero), then `10` (a run of one 1), not the band-end marker
        assert!(matches!(highpass_band(&[0b0100_0000, 0, 0, 0], 1), Err(RawError::Corrupt(why)) if why.contains("band-end")));
        let (end, len) = TABLE17.iter().find(|e| e.2 == 0).map(|e| (u64::from(e.1), e.0)).unwrap();
        let bytes = (end << (63 - len)).to_be_bytes();
        assert_eq!(highpass_band(&bytes, 1).unwrap(), [0]);
        // four bytes code at most 31 codewords of at most 320 samples before the marker
        assert!(matches!(highpass_band(&[0; 4], 31 * 320 + 1), Err(RawError::Corrupt(why)) if why.contains("longer")));
    }

    #[test]
    fn malformed_streams_are_errors() {
        let good = synthetic_stream(64, 48, [4000, 8192, 8192, 8192], 7);
        assert!(matches!(decode(&good, 66, 48, Layout::Rggb14), Err(RawError::Corrupt(_))), "size mismatch");
        assert!(matches!(decode(&good[4..], 64, 48, Layout::Rggb14), Err(RawError::Corrupt(_))), "no marker");
        let mut three = good.clone();
        three[4..8].copy_from_slice(&[0, 12, 0, 3]);
        assert!(decode(&three, 64, 48, Layout::Rggb14).is_err(), "three channels");
        // every truncation and many single-byte corruptions end in Ok or Err, never a panic
        for len in (0..good.len()).step_by(7) {
            assert!(decode(&good[..len], 64, 48, Layout::Rggb14).is_err(), "truncated at {len}");
        }
        for i in (0..good.len()).step_by(3) {
            let mut bad = good.clone();
            bad[i] ^= 0xa5;
            let _ = decode(&bad, 64, 48, Layout::Gbrg12);
        }
        assert!(matches!(decode(&synthetic_stream(32, 32, [0; 4], 0), 32, 32, Layout::Rggb14), Err(RawError::Unsupported(_))), "too small");
        // lowpass precision: 8 to 16 bits decode; wider ones don't fit the 16-bit coefficients, others are corrupt
        let precision = good.windows(4).position(|w| w == [0, 35, 0, 16]).unwrap();
        for (bits, unsupported) in [(17u8, true), (32, true), (7, false), (33, false)] {
            let mut bad = good.clone();
            bad[precision + 3] = bits;
            let result = decode(&bad, 64, 48, Layout::Rggb14);
            assert!(matches!(result, Err(RawError::Unsupported(_))) == unsupported && result.is_err(), "precision {bits}: {result:?}");
        }
        // the same prescale table before every channel decodes; a table that changes is refused
        let tables: Vec<usize> = good.windows(4).enumerate().filter(|(_, w)| *w == [0, 109, 0x28, 0]).map(|(i, _)| i).collect();
        assert_eq!(tables.len(), 4);
        let mut changed = good.clone();
        changed[tables[2] + 2] = 0;
        assert!(matches!(decode(&changed, 64, 48, Layout::Rggb14), Err(RawError::Unsupported(why)) if why.contains("prescale table")));
        // a repeated codeblock (here the first one, at the end of the header) is refused
        let first = good.windows(4).position(|w| w[0] == 0xa0 && w[1] == 0).unwrap();
        let words = u16::from_be_bytes([good[first + 2], good[first + 3]]) as usize;
        let mut twice = good[..first + 4 + 4 * words].to_vec();
        twice.extend_from_slice(&good[first - 16..]);
        assert!(matches!(decode(&twice, 64, 48, Layout::Rggb14), Err(RawError::Corrupt(why)) if why.contains("second codeblock")));
    }
}
