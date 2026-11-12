//! Panasonic RW2, Leica RWL and the older Panasonic / Leica `.RAW` files: every raw encoding these cameras write.
//!
//! **Clean-room.** No raw-decoder source code (dcraw, LibRaw, rawspeed, rawloader/rawler, darktable, RawTherapee,
//! libopenraw, ExifTool's Perl code, …) was read or consulted. What we used:
//!
//! - Tag documentation: ExifTool's PanasonicRaw tag-name tables (IFD0 `0x0002`–`0x0007` sensor size and borders,
//!   `0x0009` CFAPattern with its four values, `0x000a` BitsPerSample, `0x000b` Compression, `0x000e`–`0x0010`
//!   LinearityLimit, `0x0017`/`0x0037` ISO, `0x001c`–`0x001e` black levels, `0x0024`–`0x0026` WB levels, `0x002d`
//!   RawFormat, `0x002e` JpgFromRaw, `0x002f`–`0x0032` crop, `0x0118` RawDataOffset) and the libopenraw format
//!   notes (prose: the `IIU` magic; Leica RWL files are RW2).
//! - Our own black-box analysis of 178 CC0 files from raw.pixls.us, about 120 bodies from the DMC-LX1 (2005) to
//!   the DC-S1RM2 (2025), every encoding below. The measurement that settled each rule is noted in brackets.
//!
//! Encodings, by raw format (tag `0x002d`; the oldest files have none and are told apart by compression `0x000b`):
//!
//! - **16-bit words** (compression 34828 and 34830: DMC-LX1, DMC-FZ50, Digilux 3): little-endian words at
//!   `StripOffsets`, the sample left-justified [every value is a multiple of 2^(16 − bits)].
//! - **Packed** (formats 2, 5 and 7): 16-byte blocks read as a little-endian 128-bit number holding ⌊128 / bits⌋
//!   samples from bit 0 up (12-bit: 10, 14-bit: 9). Format 5 stores every whole 0x4000-byte chunk rotated (its
//!   logical content is bytes `0x1ff8..0x4000` followed by `0..0x1ff8`); formats 2 (DMC-FZ8) and 7 (DC-S5 with
//!   later firmware) are not rotated [vertical same-colour differences 20–180× smaller with the right choice].
//! - **Format 4** (most bodies, 2008–2022): chunks rotated as format 5; each 16-byte block, read as a little-endian
//!   128-bit number from bit 127 down, codes 14 pixels of a row. Before pixels 2, 5, 8 and 11 comes a 2-bit scale
//!   `s` for the next three pixels, step `m` = 1, 2, 4 or 16 [per-bit statistics show the field boundaries]. The
//!   first pixel of each colour (parity of the pixel index `i`) is a seed: 8 bits `h`, then, if `h ≠ 0` or
//!   `i ≥ 12`, 4 bits `l`, value `16h + l`. `h = 0` (with `i < 12`) marks a mapped-out defective pixel, decoded as
//!   0, and the next pixel of that colour is read as the seed instead [these blocks decode smoothly only this way,
//!   10× closer to the rows above and below, and they sit at the same sensor sites in every shot of a body]. Every
//!   later pixel is an 8-bit code `j` against the colour's previous value `p`: `j = 0` keeps `p` [3–4× closer to
//!   the neighbouring rows than reading it as a difference]; otherwise `p + (j − 128)·m` when `s < 3` and
//!   `p ≥ 128·m`, else `j·m + (p mod m)` [scale-3 codes track the absolute level, ≈ level / 16; values stay on the
//!   camera's 4k + 3 lattice (GX80, 99.9–100 % per branch) only when the absolute reading keeps the low bits of `p`].
//! - **Format 6** (DC-S1, S1R, S1H, S5; DC-GH5M2): not rotated. 14-bit: two 14-bit seeds, then three groups of a
//!   2-bit scale and three 10-bit codes (11 pixels; the 4 lowest bits are unused); 12-bit: two 12-bit seeds and four
//!   groups of a 2-bit scale and three 8-bit codes (14 pixels). Read from bit 127 down and reconstructed as format 4
//!   with the bias 512 (14-bit) or 128 (12-bit) [code statistics per scale as in format 4; flat saturated blocks
//!   decode flat only when the absolute reading keeps the low bits]. Seeds are never deferred and zero codes don't
//!   occur in any sample; a zero code keeps the previous value as in format 4.
//! - **Format 8** (DC-GH6 and later: GH6, GH7, G9M2, S5M2, S5M2X, S9, S1RM2, S1M2): the image is split into vertical
//!   strips listed in `u16` arrays (a count, then the values; 32-bit values as low/high pairs): `0x0044` absolute
//!   offsets, `0x0045` left columns, `0x0046` lengths in bits, `0x0047` widths, `0x0048` heights. Each strip is an
//!   independent bit stream whose bits are taken from each byte starting at bit 0 [a constant run at a strip's end
//!   reads as repeats of one code only in this order; MSB-first reading gives code frequencies of exactly
//!   2^−length, the signature of random bits]. Tag `0x0040` lists (length, code) for 17 prefix codes, the difference
//!   categories 0–16 of ITU-T T.81 (category `c`, then `c` additional bits, a leading 0 meaning negative). Pixels
//!   are coded two rows at a time, cell by 2×2 cell, column by column (top-left, bottom-left, top-right,
//!   bottom-right) [in row order the two greens trade sites: an edge's even and odd rows of green then sit ~2 px
//!   apart with red and blue between them]; each is
//!   predicted from the same site of the previous cell, the first cell of a row pair from the first cell of the
//!   previous pair, 0 at the top [the strips decode to the camera JPEG's image without seams]. The streams end a
//!   few padding rows before their stated length. Tags `0x0039`–`0x003f` and `0x0041` carry the same
//!   values in every sample; files with other values are reported as unsupported rather than guessed at. The high-resolution
//!   mode of the DC-S1RM2 (16296 × 10856, `0x0043` = 2) tiles the image in two rows of four: `0x0045` then lists (x, y)
//!   `u16` pairs (y is 0 in single-row files, so the 32-bit reading used for them is the same numbers), the tiles are
//!   5632 and 5224 rows high and are decoded exactly like strips [the tiles' areas add up to the sensor, and `0x0043` equals
//!   the number of distinct tile tops in every sample]; the decoder takes the geometry from the lists, not from the body.
//!
//! Levels and geometry:
//!
//! - Black: per colour from `0x001c`–`0x001e`. Formats 4 and 6 store every sample 15 above it [flat dark regions
//!   (σ ≈ 1–3) of ~100 files sit 13–20 above the tag, e.g. 143.1 on a D-Lux 7 tagged 128], the other formats
//!   don't [131.4 on a format-5 G9 tagged 127.5; 128.7 on a format-8 S9 tagged 128]. Files that record no black
//!   (tags 0: bodies up to ~2010) use 15 [their darkest flat regions sit at 15–22].
//! - White: the saturation plateau of the active area's samples, else the full range of the bit depth. The linearity
//!   limits (`0x000e`–`0x0010`) are not used: extended low-ISO shots record limits far below their samples (TZ200D
//!   at ISO 100: 3277, samples up to 3529) while the camera's own JPEG renders them against the full range [the
//!   JPEG/raw brightness ratio matches normal shots of the same bodies only then], the Digilux 3 records 4095 but
//!   saturates at 15871 of 16383, and the GF1 records 16383 for 12-bit samples.
//! - Samples of 0 mark mapped-out defects (format 4 above; isolated zeros between normal neighbours in the 16-bit
//!   files) and are interpolated from the same colour two pixels away.
//! - The CFA pattern applies at sample (0, 0) of the data, not at the sensor borders [the two files with odd borders
//!   show the greens on the diagonal this predicts]; red and blue were checked against every file's JPEG.
//! - Active area: the sensor borders. Default crop: `0x002f`–`0x0032`, the in-camera aspect ratio (e.g. 4:3 or 1:1
//!   on a 3:2 sensor; the embedded JPEG always shows the whole sensor). Some bodies write zeros there: ignored.
//!
//! Lens distortion (tag `0x0119`, ExifTool name DistortionInfo; ExifTool's names for its words were the only outside
//! input, their meaning below is our own measurement). The tag holds 16 little-endian 16-bit words. The camera's
//! own correction, which its embedded JPEG shows, is applied as an `OpcodeList3` `WarpRectilinear` (the DNG form
//! LightCraft already applies under "Enable Profile Corrections"):
//!
//! - Word 7, low 4 bits: 1 = correction on [0 on the DMC-GH1, whose JPEG matches the uncorrected render].
//! - Word 12: the normalisation radius in pixels, the half diagonal of the active area [2870 for 4592 × 3448
//!   (2871.2), 3240 for 5184 × 3888, 3611 for 5776 × 4336, 3288 for the FZ1000 II's whole 5472 × 3648 sensor
//!   even in its 4:3 mode].
//! - Words 8, 4 and 11 (ExifTool: DistortionParam08, 04, 11), divided by 32768, are `k2`, `k4`, `k6` of the
//!   correction: a sensor point at radius ρ (in units of word 12, about the centre of the active area) appears in
//!   the corrected image at ρ·(1 + k2 ρ² + k4 ρ⁴ + k6 ρ⁶).
//! - Word 5 (ExifTool: DistortionScale), divided by 32768, plus 1, is the zoom `z` the camera crops by afterwards:
//!   output radius = corrected radius / z [z < 1 when the corrected corners would otherwise be empty].
//!
//! [These rules were chosen among alternative word orders, powers, directions and readings of the zoom by fitting
//! radial curves between our uncorrected renders and the embedded JPEGs of 38 DMC-GX85 photos (12–32 mm and 25 mm
//! lenses). With no free parameter the words then predict the curves of 35 held-out photos from 12 other bodies and
//! 9 other lenses (9.1–105 mm: two compacts, Panasonic, Leica, Olympus, Sigma and Samyang lenses) within 0.12 % of
//! the half diagonal, against up to 13 % uncorrected; corrected renders of all 79 photos line up with the camera
//! JPEG within 0.75 px at 1440 px at every corner measured.] Words 2, 3, 6, 9, 10
//! and 13 are not used (word 13 is always word 9 + 500; word 10 is fixed per body); words 0, 1, 14 and 15 change
//! from shot to shot. The correction is radial, so it is fitted by the DNG form's odd polynomial `u·(kr0 + kr1 u² +
//! kr2 u⁴ + kr3 u⁶)` over the output (within 0.4 px on a 20 MP sensor); implausible values (a radius far from the
//! active area's, a zoom beyond ±25 %, a correction that folds over) leave the photo uncorrected.
//!
//! Not supported ([`RawError::Unsupported`], the embedded preview is shown instead): compression 34826, raw formats
//! 1 and 3, and bit depths other than the ones above (no samples of any of them).

use crate::huffman::{Bits, Huffman, diff};
use crate::shared::{active_rows, white_from_data_sourced};
use crate::{
    BlackLevel, Camera, Cfa, ColorData, Limits, Mode, Opcode, OpcodeLists, Orientation, RawData, RawError, RawFormat, RawImage, Rect, Result,
};
use craft_tiff::{ByteOrder, Ifd, Tiff, Value, tags as t};
use rayon::prelude::*;

const SENSOR_WIDTH: u16 = 0x0002;
const SENSOR_HEIGHT: u16 = 0x0003;
const BORDER_TOP: u16 = 0x0004;
const BORDER_LEFT: u16 = 0x0005;
const BORDER_BOTTOM: u16 = 0x0006;
const BORDER_RIGHT: u16 = 0x0007;
const CFA_PATTERN: u16 = 0x0009;
const BITS: u16 = 0x000a;
const COMPRESSION: u16 = 0x000b;
const LINEARITY_LIMIT: [u16; 3] = [0x000e, 0x000f, 0x0010];
const BLACK_RGB: [u16; 3] = [0x001c, 0x001d, 0x001e];
const WB_RGB: [u16; 3] = [0x0024, 0x0025, 0x0026];
const RAW_FORMAT: u16 = 0x002d;
/// The in-camera aspect-ratio crop: top, left, bottom, right in sensor coordinates.
const CROP: [u16; 4] = [0x002f, 0x0030, 0x0031, 0x0032];
const RAW_OFFSET: u16 = 0x0118;
const HUFFMAN_CODES: u16 = 0x0040;
const STRIP_OFFSETS: u16 = 0x0044;
const STRIP_LEFT: u16 = 0x0045;
const STRIP_BITS: u16 = 0x0046;
const STRIP_WIDTHS: u16 = 0x0047;
const STRIP_HEIGHTS: u16 = 0x0048;
/// Format-8 tags of unknown meaning and the values every sample carries (as `u16` arrays).
const FORMAT8_FIXED: [(u16, &[u16]); 8] = [
    (0x0039, &[6, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1]),
    (0x003a, &[6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
    (0x003b, &[65535]),
    (0x003c, &[0]),
    (0x003d, &[0]),
    (0x003e, &[0]),
    (0x003f, &[0]),
    (0x0041, &[17, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
];
/// Format-8 tile rows (ExifTool: no name): 1 for a single row of strips, 2 for the 16296 × 10856 high-resolution mode.
const TILE_ROWS: u16 = 0x0043;
/// The camera's lens distortion correction (ExifTool name: DistortionInfo); see the module docs.
const DISTORTION_INFO: u16 = 0x0119;
/// Fixed-point one of the distortion coefficients and zoom.
const DISTORTION_ONE: f64 = 32768.0;
/// Largest error of the fitted inverse (relative to the half diagonal) accepted: real lenses fit within 1.2e-4.
const DISTORTION_MAX_FIT_ERROR: f64 = 1e-3;
/// Most strips seen in a file is 8 (the 16296 × 10856 high-resolution mode: 4 columns by 2 rows of tiles).
const MAX_STRIPS: usize = 64;

const CHUNK: usize = 0x4000;
const ROTATE: usize = 0x1ff8;
/// Formats 4 and 6 store samples this far above the recorded black level.
const CODEC_BLACK_OFFSET: f32 = 15.0;
/// Black level of files that record none.
const DEFAULT_BLACK: f32 = 15.0;

/// How the samples are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    /// Little-endian 16-bit words, samples left-justified.
    Words,
    /// Continuous 16-byte blocks; `rotated`: whole 0x4000-byte chunks are stored rotated by 0x1ff8.
    Blocks { kind: Block, rotated: bool },
    /// Format 8: prefix-coded strips.
    Strips,
}

/// What one 16-byte block holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Block {
    /// ⌊128 / bits⌋ samples from bit 0 up.
    Packed {
        bits: u32,
    },
    Format4,
    Format6 {
        bits14: bool,
    },
}

impl Block {
    fn per(self) -> usize {
        match self {
            Block::Packed { bits } => (128 / bits.max(1)) as usize,
            Block::Format4 | Block::Format6 { bits14: false } => 14,
            Block::Format6 { bits14: true } => 11,
        }
    }

    fn decode(self, b: &[u8; 16], out: &mut [u16]) {
        match self {
            Block::Packed { bits } => packed(b, bits, out),
            Block::Format4 => format4(b, out),
            Block::Format6 { bits14 } => format6(b, bits14, out),
        }
    }
}

fn encoding(ifd: &Ifd, bits: u32) -> Result<Encoding> {
    let blocks = |kind, rotated| Ok(Encoding::Blocks { kind, rotated });
    match (ifd.u16(RAW_FORMAT), ifd.u16(COMPRESSION)) {
        (Some(2), _) if bits == 12 => blocks(Block::Packed { bits }, false),
        (Some(4), _) if bits == 12 => blocks(Block::Format4, true),
        (Some(5), _) if bits == 12 || bits == 14 => blocks(Block::Packed { bits }, true),
        (Some(6), _) if bits == 12 || bits == 14 => blocks(Block::Format6 { bits14: bits == 14 }, false),
        (Some(7), _) if bits == 14 => blocks(Block::Packed { bits }, false),
        (Some(8), _) if (12..=16).contains(&bits) => Ok(Encoding::Strips),
        (None, Some(34828 | 34830)) if (12..=16).contains(&bits) => Ok(Encoding::Words),
        (Some(f), _) => Err(RawError::Unsupported(format!("Panasonic raw format {f} with {bits}-bit samples"))),
        (None, c) => Err(RawError::Unsupported(format!("Panasonic raw without a raw format (compression {c:?})"))),
    }
}

/// Bit fields of a 16-byte block taken as a little-endian 128-bit number, read from bit 127 down.
struct Fields {
    word: u128,
    left: u32,
}

impl Fields {
    fn new(b: &[u8; 16]) -> Fields {
        Fields { word: u128::from_le_bytes(*b), left: 128 }
    }

    /// The next `n` (1..=16) bits.
    #[inline]
    fn take(&mut self, n: u32) -> i32 {
        self.left = self.left.saturating_sub(n);
        ((self.word >> self.left) & ((1u128 << n) - 1)) as i32
    }
}

/// Step and absolute flag of the 2-bit scales of formats 4 and 6.
const STEPS: [(i32, bool); 4] = [(1, false), (2, false), (4, false), (16, true)];

/// Formats 4 and 6: the next value of a colour from its previous value `p` and code `j` (see the module docs).
#[inline]
fn next(p: i32, j: i32, (m, absolute): (i32, bool), bias: i32) -> i32 {
    if j == 0 {
        p
    } else if !absolute && p >= bias * m {
        p + (j - bias) * m
    } else {
        j * m + (p & (m - 1))
    }
}

#[inline]
fn put(out: &mut [u16], i: usize, v: i32) {
    if let Some(o) = out.get_mut(i) {
        *o = v.clamp(0, u16::MAX as i32) as u16;
    }
}

fn packed(b: &[u8; 16], bits: u32, out: &mut [u16]) {
    let word = u128::from_le_bytes(*b);
    let mask = (1u128 << bits.min(16)) - 1;
    for (i, o) in out.iter_mut().enumerate().take((128 / bits.max(1)) as usize) {
        *o = ((word >> (i as u32 * bits)) & mask) as u16;
    }
}

fn format4(b: &[u8; 16], out: &mut [u16]) {
    let mut f = Fields::new(b);
    let (mut prev, mut seeded, mut step) = ([0i32; 2], [false; 2], STEPS[0]);
    for i in 0..14 {
        if i % 3 == 2 {
            step = STEPS[f.take(2) as usize & 3];
        }
        let c = i & 1;
        let v = if seeded[c] {
            prev[c] = next(prev[c], f.take(8), step, 128);
            prev[c]
        } else {
            let hi = f.take(8);
            if hi != 0 || i >= 12 {
                prev[c] = hi << 4 | f.take(4);
                seeded[c] = hi != 0;
                prev[c]
            } else {
                0 // a defect marker; this colour's seed comes with its next pixel
            }
        };
        put(out, i, v);
    }
}

fn format6(b: &[u8; 16], bits14: bool, out: &mut [u16]) {
    let (seed, code, groups, bias) = if bits14 { (14, 10, 3, 512) } else { (12, 8, 4, 128) };
    let mut f = Fields::new(b);
    let mut prev = [f.take(seed), f.take(seed)];
    put(out, 0, prev[0]);
    put(out, 1, prev[1]);
    for g in 0..groups {
        let step = STEPS[f.take(2) as usize & 3];
        for k in 0..3 {
            let i = 2 + 3 * g + k;
            prev[i & 1] = next(prev[i & 1], f.take(code), step, bias);
            put(out, i, prev[i & 1]);
        }
    }
}

/// Decode a continuous stream of 16-byte blocks into `out`, in parallel by 0x4000-byte chunks (whole chunks
/// rotated when `rotated`). Blocks missing from `src` give zeros.
fn decode_blocks(src: &[u8], kind: Block, rotated: bool, out: &mut [u16]) {
    let per = kind.per();
    out.par_chunks_mut(CHUNK / 16 * per).enumerate().for_each(|(c, o)| {
        let raw = src.get(c * CHUNK..).unwrap_or_default();
        let raw = raw.get(..CHUNK).unwrap_or(raw);
        let mut logical = Vec::new();
        let chunk = if rotated && raw.len() == CHUNK {
            logical.extend_from_slice(raw.get(ROTATE..).unwrap_or_default());
            logical.extend_from_slice(raw.get(..ROTATE).unwrap_or_default());
            &logical
        } else {
            raw
        };
        for (i, px) in o.chunks_mut(per).enumerate() {
            match chunk.get(i * 16..i * 16 + 16).and_then(|s| <&[u8; 16]>::try_from(s).ok()) {
                Some(b) => kind.decode(b, px),
                None => px.fill(0),
            }
        }
    });
}

fn decode_words(src: &[u8], shift: u32, out: &mut [u16]) {
    out.par_chunks_mut(1 << 16).enumerate().for_each(|(k, o)| {
        for (i, v) in o.iter_mut().enumerate() {
            let at = ((k << 16) + i) * 2;
            *v = src.get(at..at + 2).and_then(|b| <[u8; 2]>::try_from(b).ok()).map_or(0, |b| u16::from_le_bytes(b) >> shift);
        }
    });
}

/// A tag as a `u16` array: SHORT values, or the little-endian contents of an UNDEFINED / BYTE value.
fn u16_array(ifd: &Ifd, tag: u16, order: ByteOrder) -> Option<Vec<u16>> {
    match ifd.value(tag)? {
        Value::Undefined(b) | Value::Byte(b) => Some(b.as_chunks::<2>().0.iter().map(|c| order.u16(*c)).collect()),
        Value::Short(v) => Some(v.clone()),
        other => Some(other.to_u64_vec().into_iter().map(|v| v.min(u16::MAX as u64) as u16).collect()),
    }
}

/// The format-8 tile origins (tag `0x0045`): a count, then (x, y) pairs of `u16`.
fn origins(ifd: &Ifd, order: ByteOrder) -> Result<Vec<(usize, usize)>> {
    let a = u16_array(ifd, STRIP_LEFT, order).ok_or_else(|| RawError::Corrupt("RW2 format 8 without tag 0x0045".into()))?;
    let n = a.first().map_or(0, |&n| n as usize);
    let at = |i: usize| a.get(i).map(|&v| v as usize);
    let values: Option<Vec<(usize, usize)>> = (0..n).map(|i| Some((at(1 + 2 * i)?, at(2 + 2 * i)?))).collect();
    values.filter(|v| !v.is_empty() && v.len() <= MAX_STRIPS).ok_or_else(|| RawError::Corrupt("RW2 format 8: bad list in tag 0x0045".into()))
}

/// A format-8 `u16` list (count, then the values; `wide`: 32-bit values as low/high pairs).
fn list(ifd: &Ifd, tag: u16, order: ByteOrder, wide: bool) -> Result<Vec<u64>> {
    let a = u16_array(ifd, tag, order).ok_or_else(|| RawError::Corrupt(format!("RW2 format 8 without tag {tag:#06x}")))?;
    let n = a.first().map_or(0, |&n| n as usize);
    let at = |i: usize| a.get(i).map(|&v| v as u64);
    let values: Option<Vec<u64>> = (0..n).map(|i| if wide { Some(at(1 + 2 * i)? | at(2 + 2 * i)? << 16) } else { at(1 + i) }).collect();
    values.filter(|v| !v.is_empty() && v.len() <= MAX_STRIPS).ok_or_else(|| RawError::Corrupt(format!("RW2 format 8: bad list in tag {tag:#06x}")))
}

/// One format-8 strip or tile: its bit stream and placement.
struct Strip<'a> {
    data: &'a [u8],
    left: usize,
    top: usize,
    width: usize,
    height: usize,
    bits: u64,
}

/// The format-8 strips or tiles (together covering the whole image exactly once) and code table.
fn format8<'a>(ifd: &Ifd, order: ByteOrder, bytes: &'a [u8], w: usize, h: usize) -> Result<(Vec<Strip<'a>>, Huffman)> {
    for (tag, want) in FORMAT8_FIXED {
        if let Some(v) = u16_array(ifd, tag, order)
            && v != want
        {
            return Err(RawError::Unsupported(format!("Panasonic raw format 8 with tag {tag:#06x} = {v:?}")));
        }
    }
    let (offsets, origins, bits) = (list(ifd, STRIP_OFFSETS, order, true)?, origins(ifd, order)?, list(ifd, STRIP_BITS, order, true)?);
    let (widths, heights) = (list(ifd, STRIP_WIDTHS, order, false)?, list(ifd, STRIP_HEIGHTS, order, false)?);
    let n = offsets.len();
    if [origins.len(), bits.len(), widths.len(), heights.len()].iter().any(|&l| l != n) {
        return Err(RawError::Corrupt("RW2 format 8: strip lists of different lengths".into()));
    }
    let mut strips = Vec::with_capacity(n);
    let mut area = 0usize;
    for i in 0..n {
        let ((left, top), width, height) = (origins[i], widths[i] as usize, heights[i] as usize);
        let len = usize::try_from(bits[i].div_ceil(8)).unwrap_or(usize::MAX);
        let data = usize::try_from(offsets[i]).ok().and_then(|o| bytes.get(o..o.checked_add(len)?));
        let inside = width > 0 && height > 0 && left + width <= w && top + height <= h;
        let Some(data) = data.filter(|_| inside) else {
            return Err(RawError::Corrupt(format!("RW2 format 8: strip {i} outside the image or file")));
        };
        // every sample costs at least one bit (no code is shorter), so a strip stating fewer bits than samples is
        // corrupt: refusing it here keeps a few-byte file from sizing the full-sensor buffer
        if bits[i] < (width * height) as u64 {
            return Err(RawError::Corrupt(format!("RW2 format 8: strip {i} states {} bits for {} samples", bits[i], width * height)));
        }
        area += width * height;
        strips.push(Strip { data, left, top, width, height, bits: bits[i] });
    }
    // inside the image, no two overlapping and the areas adding up to the image: every sample has exactly one strip
    let overlap = strips.iter().enumerate().any(|(i, a)| {
        strips[i + 1..].iter().any(|b| a.left < b.left + b.width && b.left < a.left + a.width && a.top < b.top + b.height && b.top < a.top + a.height)
    });
    if overlap || area != w * h {
        return Err(RawError::Corrupt(format!("RW2 format 8: strips cover {area} of {} samples", w * h)));
    }
    // tag 0x0043 counts the rows of tiles; a file that says otherwise is not one we understand
    let mut tops: Vec<usize> = strips.iter().map(|s| s.top).collect();
    tops.sort_unstable();
    tops.dedup();
    if let Some(v) = u16_array(ifd, TILE_ROWS, order)
        && v != [tops.len() as u16]
    {
        return Err(RawError::Unsupported(format!("Panasonic raw format 8 with tag {TILE_ROWS:#06x} = {v:?} for {} tile rows", tops.len())));
    }
    let codes = u16_array(ifd, HUFFMAN_CODES, order).unwrap_or_default();
    let pairs: Vec<(u32, u32)> = match (codes.first(), codes.get(1..35)) {
        (Some(17), Some(c)) => c.as_chunks::<2>().0.iter().map(|&[l, c]| (l as u32, c as u32)).collect(),
        _ => return Err(RawError::Corrupt("RW2 format 8: bad code table".into())),
    };
    Ok((strips, code_table(&pairs)?))
}

/// The prefix-code table from (length, code) per difference category 0..=16. In 12- and 14-bit files the codes of the
/// categories the bit depth can't produce start with a shorter code (8 ones): read bit by bit, the shorter code wins
/// and the longer ones are unreachable.
fn code_table(pairs: &[(u32, u32)]) -> Result<Huffman> {
    let mut table = Vec::with_capacity(pairs.len());
    for (i, &(la, ca)) in pairs.iter().enumerate() {
        if la > 12 {
            return Err(RawError::Unsupported(format!("RW2 format 8: {la}-bit code")));
        }
        if la > 0 && ca >> la != 0 {
            return Err(RawError::Corrupt("RW2 format 8: code longer than its length".into()));
        }
        let shadowed = pairs.iter().any(|&(lb, cb)| lb > 0 && lb < la && ca >> (la - lb) == cb);
        let duplicate = pairs.iter().enumerate().any(|(j, &(lb, cb))| j != i && la > 0 && (lb, cb) == (la, ca));
        if duplicate {
            return Err(RawError::Corrupt("RW2 format 8: two categories with the same code".into()));
        }
        table.push(if la == 0 || shadowed { (0, 0) } else { ((ca << (12 - la)) as u16, la as u8) });
    }
    Huffman::new(&table)
}

/// Decode one strip into its column range of each output row.
fn decode_strip(s: &Strip, codes: &Huffman, max: i32, mut rows: Vec<&mut [u16]>) -> Result<()> {
    // the stream's bits are taken from each byte starting at bit 0: reversed, it is an ordinary MSB-first stream
    let reversed: Vec<u8> = s.data.iter().map(|b| b.reverse_bits()).collect();
    let mut bits = Bits::new(&reversed);
    let mut first = [0i32; 4];
    for pair in 0..s.height.div_ceil(2) {
        let mut cell = first;
        for cx in 0..s.width.div_ceil(2) {
            for (k, v) in cell.iter_mut().enumerate() {
                let d = diff(&mut bits, codes).ok_or_else(|| RawError::Corrupt(format!("RW2 format 8: invalid code in row {}", 2 * pair)))?;
                *v = v.saturating_add(d);
                // a value outside the sample depth is a corrupt stream, not a sample to clamp
                let sample = u16::try_from(*v).ok().filter(|_| *v <= max);
                let sample =
                    sample.ok_or_else(|| RawError::Corrupt(format!("RW2 format 8: sample {} outside the {max} range in row {}", *v, 2 * pair)))?;
                if let Some(px) = rows.get_mut(2 * pair + k % 2).and_then(|r| r.get_mut(2 * cx + k / 2)) {
                    *px = sample;
                }
            }
            if cx == 0 {
                first = cell;
            }
        }
        if bits.consumed_bits() as u64 > s.bits {
            return Err(RawError::Corrupt(format!("RW2 format 8: strip data ends at row {} of {}", 2 * pair, s.height)));
        }
    }
    Ok(())
}

/// `bits`: the sample depth; a decoded value above it (or below 0) makes the strip corrupt.
fn decode_strips(strips: &[Strip], codes: &Huffman, w: usize, bits: u32, out: &mut [u16]) -> Result<()> {
    let max = (1i32 << bits.clamp(1, 16)) - 1;
    let mut parts: Vec<Vec<&mut [u16]>> = strips.iter().map(|s| Vec::with_capacity(s.height)).collect();
    for (y, row) in out.chunks_mut(w).enumerate() {
        // the strips crossing this row, left to right, each take their columns off its front
        let mut here: Vec<usize> = (0..strips.len()).filter(|&i| strips.get(i).is_some_and(|s| s.top <= y && y < s.top + s.height)).collect();
        here.sort_by_key(|&i| strips.get(i).map_or(0, |s| s.left));
        let (mut rest, mut x) = (row, 0usize);
        for i in here {
            let Some(s) = strips.get(i) else { continue };
            let skip = s.left.saturating_sub(x).min(rest.len());
            let (_, tail) = rest.split_at_mut(skip);
            let (part, tail) = tail.split_at_mut(s.width.min(tail.len()));
            if let Some(rows) = parts.get_mut(i) {
                rows.push(part);
            }
            rest = tail;
            x = s.left + s.width;
        }
    }
    strips.par_iter().zip(parts).try_for_each(|(s, rows)| decode_strip(s, codes, max, rows))
}

/// Replace samples of 0 inside `area` (mapped-out defects) by the mean of the non-zero samples of the same colour two
/// pixels away.
fn fill_defects(data: &mut [u16], w: usize, area: Rect) {
    for y in area.y..area.y + area.height {
        let Some(row) = data.get(y * w + area.x..y * w + area.x + area.width) else { break };
        if !row.contains(&0) {
            continue;
        }
        for x in area.x..area.x + area.width {
            let i = y * w + x;
            if data.get(i) != Some(&0) {
                continue;
            }
            let near = [(x.checked_sub(2), Some(y)), (Some(x + 2).filter(|&x| x < w), Some(y)), (Some(x), y.checked_sub(2)), (Some(x), Some(y + 2))];
            let (mut sum, mut n) = (0u32, 0u32);
            for (nx, ny) in near {
                if let (Some(nx), Some(ny)) = (nx, ny)
                    && let Some(&v) = data.get(ny * w + nx).filter(|&&v| v != 0)
                {
                    sum += v as u32;
                    n += 1;
                }
            }
            if n > 0
                && let Some(v) = data.get_mut(i)
            {
                *v = (sum / n) as u16;
            }
        }
    }
}

fn cfa(code: u16) -> Cfa {
    Cfa::bayer_static(match code {
        2 => "GRBG",
        3 => "GBRG",
        4 => "BGGR",
        _ => "RGGB",
    })
}

pub(crate) fn decode(bytes: &[u8], mode: Mode, limits: &Limits) -> Result<RawImage> {
    let tiff = Tiff::parse(bytes)?;
    let ifd0 = tiff.ifds.first().ok_or_else(|| RawError::Corrupt("RW2 without IFD0".into()))?;
    let get = |tag: u16| ifd0.u64(tag).and_then(|v| usize::try_from(v).ok());
    let (w, h) = (get(SENSOR_WIDTH).unwrap_or(0), get(SENSOR_HEIGHT).unwrap_or(0));
    if w == 0 || h == 0 {
        return Err(RawError::Corrupt("RW2 without sensor size".into()));
    }
    let n = w.checked_mul(h).filter(|n| *n <= limits.samples()).ok_or(RawError::Limit("image too large"))?;
    let bits = ifd0.u32(BITS).unwrap_or(12);
    let encoding = encoding(ifd0, bits)?;
    let src = || {
        let off = get(RAW_OFFSET).or_else(|| get(t::STRIP_OFFSETS)).ok_or_else(|| RawError::Corrupt("RW2 without raw data offset".into()))?;
        bytes.get(off..).ok_or_else(|| RawError::Corrupt("RW2 raw data outside file".into()))
    };
    let truncated = |have: usize, need: usize| RawError::Corrupt(format!("RW2 raw data truncated ({have} of {need} bytes)"));
    let mut data = Vec::new();
    match encoding {
        Encoding::Words => {
            let src = src()?;
            let need = n * 2;
            if src.len() < need {
                return Err(truncated(src.len(), need));
            }
            if mode == Mode::Full {
                data = vec![0u16; n];
                decode_words(src, 16u32.saturating_sub(bits), &mut data);
            }
        }
        Encoding::Blocks { kind, rotated } => {
            let src = src()?;
            let need = n.div_ceil(kind.per()) * 16;
            if src.len() < need {
                return Err(truncated(src.len(), need));
            }
            if mode == Mode::Full {
                data = vec![0u16; n];
                decode_blocks(src, kind, rotated, &mut data);
            }
        }
        Encoding::Strips => {
            let (strips, codes) = format8(ifd0, tiff.order, bytes, w, h)?;
            if mode == Mode::Full {
                data = vec![0u16; n];
                decode_strips(&strips, &codes, w, bits, &mut data)?;
            }
        }
    }

    let active = match (get(BORDER_TOP), get(BORDER_LEFT), get(BORDER_BOTTOM), get(BORDER_RIGHT)) {
        (Some(tp), Some(l), Some(b), Some(r)) if b > tp && r > l => Rect::new(l, tp, r - l, b - tp).clipped(w, h),
        _ => Rect::new(0, 0, w, h),
    };
    let active = if active.width == 0 || active.height == 0 { Rect::new(0, 0, w, h) } else { active };
    let crop = match CROP.map(get) {
        [Some(tp), Some(l), Some(b), Some(r)]
            if b > tp && r > l && l >= active.x && tp >= active.y && r <= active.x + active.width && b <= active.y + active.height =>
        {
            Rect::new(l - active.x, tp - active.y, r - l, b - tp)
        }
        _ => Rect::new(0, 0, active.width, active.height),
    };
    if !data.is_empty() {
        fill_defects(&mut data, w, active);
    }

    let cfa = cfa(ifd0.u16(CFA_PATTERN).unwrap_or(1));
    let read = BLACK_RGB.map(|tag| ifd0.f64(tag).filter(|v| v.is_finite() && *v >= 0.0));
    let tags = read.map(|v| v.unwrap_or(0.0) as f32);
    let offset = match encoding {
        Encoding::Blocks { kind: Block::Format4 | Block::Format6 { .. }, .. } => CODEC_BLACK_OFFSET,
        _ => 0.0,
    };
    let (rgb, black_source) = if tags.iter().all(|&v| v == 0.0) {
        ([DEFAULT_BLACK; 3], crate::LevelSource::Fallback)
    } else {
        // a missing tag reads 0: the file stated only part of the level
        let source = if read.iter().all(Option::is_some) { crate::LevelSource::Metadata } else { crate::LevelSource::Fallback };
        (tags.map(|v| v + offset), source)
    };
    // per CFA position, anchored at the active area
    let values = cfa.shifted(active.x, active.y).pattern.iter().map(|&c| rgb[c as usize]).collect();
    let black = BlackLevel { repeat_rows: 2, repeat_cols: 2, values, ..Default::default() };
    let full = ((1u32 << bits.clamp(1, 16)) - 1) as f32;
    let (white, white_source) = if data.is_empty() {
        // headers only (white isn't part of `RawInfo`): the recorded limit as a placeholder
        let limit = LINEARITY_LIMIT.iter().filter_map(|&tag| ifd0.f64(tag)).map(|v| v as f32).filter(|v| *v > 0.0 && *v <= full).fold(full, f32::min);
        (limit, crate::LevelSource::Unknown)
    } else {
        white_from_data_sourced(&active_rows(&data, w, active), bits)
    };
    let (white, white_source) = if white > black.mean() { (white, white_source) } else { (full, crate::LevelSource::Fallback) };
    let wb = match WB_RGB.map(|tag| ifd0.f64(tag)) {
        [Some(r), Some(g), Some(b)] if r > 0.0 && g > 0.0 && b > 0.0 => Some([(r / g) as f32, 1.0, (b / g) as f32]),
        _ => None,
    };
    let img = RawImage {
        format: RawFormat::Rw2,
        width: w,
        height: h,
        cpp: 1,
        data: RawData::U16(data),
        cfa: Some(cfa),
        bits,
        black,
        white: vec![white],
        active_area: active,
        crop,
        orientation: Orientation::from_exif(ifd0.u16(t::ORIENTATION).unwrap_or(1)),
        color: ColorData::default(),
        wb_multipliers: wb,
        linearized: false,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists { list3: distortion(ifd0, active).into_iter().collect(), ..Default::default() },
        camera: tiff.ifds.first().map(Camera::from_ifd0).unwrap_or_default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: crate::LevelSources { black: black_source, white: white_source },
    };
    img.validate_for(mode)?;
    Ok(img)
}

/// The camera's distortion correction (tag `0x0119`, see the module docs) as the equivalent DNG `WarpRectilinear`
/// opcode for `OpcodeList3` (relative to the active area), so it reaches the same "profile corrections" as a DNG's
/// embedded warp. `None` when the camera recorded the correction as off, when it is the identity, or when the
/// values are implausible (the photo is then shown uncorrected rather than wrongly warped).
fn distortion(ifd: &Ifd, active: Rect) -> Option<Opcode> {
    let b = ifd.bytes(DISTORTION_INFO)?;
    let word = |i: usize| -> Option<[u8; 2]> { b.get(2 * i..2 * i + 2)?.try_into().ok() };
    let signed = |i: usize| word(i).map(|w| i16::from_le_bytes(w) as f64);
    if u16::from_le_bytes(word(7)?) & 0x000f != 1 {
        return None;
    }
    let (k2, k4, k6) = (signed(8)? / DISTORTION_ONE, signed(4)? / DISTORTION_ONE, signed(11)? / DISTORTION_ONE);
    let zoom = 1.0 + signed(5)? / DISTORTION_ONE;
    let radius = u16::from_le_bytes(word(12)?) as f64;
    if k2 == 0.0 && k4 == 0.0 && k6 == 0.0 && zoom == 1.0 {
        return None;
    }
    // DNG's normalisation: the distance from the centre to the farthest corner, in pixel-index units
    let half_diag = (active.width as f64 - 1.0).hypot(active.height as f64 - 1.0) / 2.0;
    if !(half_diag >= 1.0 && (0.5 * half_diag..=2.0 * half_diag).contains(&radius) && (0.75..=1.25).contains(&zoom)) {
        return None;
    }
    // the camera's correction: a source point at radius ρ (in units of `radius`) moves to ρ·g(ρ)
    let corrected = |r: f64| {
        let r2 = r * r;
        r * (1.0 + r2 * (k2 + r2 * (k4 + r2 * k6)))
    };
    // work in DNG-normalised units (corner distance = 1), where the output corners sit at radius `zoom`
    let scale = radius / half_diag;
    let k = crate::shared::warp_from_forward_map(&|s| corrected(s / scale) * scale, zoom, DISTORTION_MAX_FIT_ERROR)?;
    Some(Opcode::WarpRectilinear { planes: vec![[k[0], k[1], k[2], k[3], 0.0, 0.0]], center: [0.5, 0.5] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_tiff::{IfdBuilder, TiffWriter};

    /// Writes bit fields into a 16-byte block from bit 127 down (the inverse of [`Fields`]).
    struct FieldWriter {
        word: u128,
        left: u32,
    }

    impl FieldWriter {
        fn new() -> Self {
            FieldWriter { word: 0, left: 128 }
        }
        fn put(&mut self, n: u32, v: u32) -> &mut Self {
            assert!(v < 1 << n, "{v} doesn't fit {n} bits");
            self.left -= n;
            self.word |= (v as u128) << self.left;
            self
        }
        fn block(&self) -> [u8; 16] {
            self.word.to_le_bytes()
        }
    }

    /// A seed of format 4: 8 bits `h`, 4 bits `l`.
    fn seed(f: &mut FieldWriter, v: u32) {
        f.put(8, v >> 4).put(4, v & 15);
    }

    fn decode4(b: [u8; 16]) -> [u16; 14] {
        let mut out = [0u16; 14];
        format4(&b, &mut out);
        out
    }

    #[test]
    fn format4_known_blocks() {
        // every branch: differences at each scale, a zero code, absolute scale-3 codes keeping the low bits
        let mut f = FieldWriter::new();
        seed(&mut f, 1000);
        seed(&mut f, 2000);
        f.put(2, 0).put(8, 130).put(8, 120).put(8, 0); // 1002, 1992, 1002 (kept)
        f.put(2, 1).put(8, 129).put(8, 100).put(8, 128); // 1994, 946, 1994
        f.put(2, 2).put(8, 200).put(8, 64).put(8, 131); // 1234, 1738, 1246
        f.put(2, 3).put(8, 50).put(8, 255).put(8, 0); // 50·16 + 1738 % 16 = 810, 4080 + 14 = 4094, 810 (kept)
        assert_eq!(f.left, 0);
        assert_eq!(decode4(f.block()), [1000, 2000, 1002, 1992, 1002, 1994, 946, 1994, 1234, 1738, 1246, 810, 4094, 810]);

        // a deferred seed (defect marker, 8 bits only) and absolute codes because the previous value is too small
        let mut f = FieldWriter::new();
        f.put(8, 0); // pixel 0: defect; the next even pixel brings the seed
        seed(&mut f, 325);
        f.put(2, 1);
        seed(&mut f, 481); // pixel 2: the even seed
        f.put(8, 140).put(8, 10); // 325 + 24 = 349, 481 − 236 = 245
        f.put(2, 2).put(8, 100).put(8, 129).put(8, 129); // < 512: 400 + 349 % 4 = 401, 516 + 245 % 4 = 517, 517
        f.put(2, 0).put(8, 127).put(8, 0).put(8, 255); // 516, 517 (kept), 643
        f.put(2, 0).put(8, 1).put(8, 128).put(8, 128); // 390, 643, 390
        assert_eq!(f.left, 0);
        assert_eq!(decode4(f.block()), [0, 325, 481, 349, 245, 401, 517, 517, 516, 517, 643, 390, 643, 390]);

        // a colour without a seed until pixel 12, which then always reads its 4 low bits
        let mut f = FieldWriter::new();
        f.put(8, 0);
        seed(&mut f, 1600);
        for g in 0..4 {
            f.put(2, 0);
            for k in 0..3 {
                let i = 2 + 3 * g + k;
                if i % 2 == 1 {
                    f.put(8, 128);
                } else if i < 12 {
                    f.put(8, 0);
                } else {
                    f.put(8, 0).put(4, 7);
                }
            }
        }
        assert_eq!(f.left, 0);
        assert_eq!(decode4(f.block()), [0, 1600, 0, 1600, 0, 1600, 0, 1600, 0, 1600, 0, 1600, 7, 1600]);
    }

    #[test]
    fn format6_known_blocks() {
        let mut f = FieldWriter::new();
        f.put(14, 5000).put(14, 6000);
        f.put(2, 0).put(10, 520).put(10, 500).put(10, 512); // 5008, 5988, 5008
        f.put(2, 3).put(10, 1023).put(10, 300).put(10, 0); // 16368 + 5988 % 16, 4800 + 5008 % 16, kept
        f.put(2, 1).put(10, 600).put(10, 400).put(10, 10); // 4976, 16148, 3972
        let mut out = [0u16; 11];
        format6(&f.block(), true, &mut out);
        assert_eq!(out, [5000, 6000, 5008, 5988, 5008, 16372, 4800, 16372, 4976, 16148, 3972]);

        let mut f = FieldWriter::new();
        f.put(12, 300).put(12, 4000);
        f.put(2, 2).put(8, 140).put(8, 100).put(8, 130); // 300 < 512: 560 + 0; 3888; 568
        f.put(2, 0).put(8, 129).put(8, 127).put(8, 128); // 3889, 567, 3889
        f.put(2, 3).put(8, 10).put(8, 255).put(8, 20); // 160 + 7, 4080 + 1, 320 + 7
        f.put(2, 1).put(8, 128).put(8, 130).put(8, 0); // 4081, 331, 4081 (kept)
        assert_eq!(f.left, 0);
        let mut out = [0u16; 14];
        format6(&f.block(), false, &mut out);
        assert_eq!(out, [300, 4000, 560, 3888, 568, 3889, 567, 3889, 167, 4081, 327, 4081, 331, 4081]);
    }

    /// A camera-like format 4/6 encoder: per group the smallest scale that codes the targets exactly, else scale 3.
    /// Returns the block and what a decoder must reconstruct.
    fn encode_block(target: &[u16], fmt6_bits: Option<u32>) -> ([u8; 16], Vec<u16>) {
        let (seed_bits, code_bits, bias) = match fmt6_bits {
            Some(14) => (14, 10, 512),
            Some(_) => (12, 8, 128),
            None => (12, 8, 128),
        };
        let n = target.len();
        let mut f = FieldWriter::new();
        let mut prev = [target[0] as i32, target[1] as i32];
        let mut out = vec![target[0], target[1]];
        for &s in &target[..2] {
            if fmt6_bits.is_some() {
                f.put(seed_bits, s as u32);
            } else {
                seed(&mut f, s as u32);
            }
        }
        for g in 0..(n - 2) / 3 {
            let idx: Vec<usize> = (0..3).map(|k| 2 + 3 * g + k).collect();
            let fits = |s: usize| -> Option<Vec<i32>> {
                let (m, absolute) = STEPS[s];
                let mut p = prev;
                let mut codes = vec![];
                for &i in &idx {
                    let (v, c) = (target[i] as i32, i & 1);
                    let j = if !absolute && p[c] >= bias * m {
                        let d = v - p[c];
                        (d % m == 0 && (d / m).abs() < bias).then_some(d / m + bias)?
                    } else {
                        // nearest step, as a camera would
                        let j = (v - (p[c] & (m - 1)) + m / 2) / m;
                        (absolute || (v - (p[c] & (m - 1))) % m == 0).then_some(j.clamp(1, (1 << code_bits) - 1))?
                    };
                    p[c] = next(p[c], j, (m, absolute), bias);
                    codes.push(j);
                }
                Some(codes)
            };
            let s = (0..4).find(|&s| fits(s).is_some()).unwrap_or(3);
            let codes = fits(s).unwrap();
            f.put(2, s as u32);
            for (k, &i) in idx.iter().enumerate() {
                f.put(code_bits, codes[k] as u32);
                prev[i & 1] = next(prev[i & 1], codes[k], STEPS[s], bias);
                out.push(prev[i & 1] as u16);
            }
        }
        (f.block(), out)
    }

    /// A smooth synthetic Bayer-like image with some sharp edges, values `lo..hi`.
    fn image(w: usize, h: usize, lo: u32, hi: u32) -> Vec<u16> {
        (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let smooth = lo + (hi - lo) * ((x * 7 + y * 3) % 997) as u32 / 997 / 2;
                let edge = if (x / 37 + y / 23) % 3 == 0 { (hi - lo) / 3 } else { 0 };
                (smooth + edge + (x % 2 * 40) as u32).min(hi) as u16
            })
            .collect()
    }

    /// Rotate every whole 0x4000-byte chunk the way formats 4 and 5 store them.
    fn rotate(mut stream: Vec<u8>) -> Vec<u8> {
        stream.resize(stream.len().div_ceil(CHUNK) * CHUNK, 0);
        stream.chunks(CHUNK).flat_map(|c| c[CHUNK - ROTATE..].iter().chain(&c[..CHUNK - ROTATE]).copied().collect::<Vec<_>>()).collect()
    }

    /// Pack samples LSB-first into 16-byte blocks (formats 2, 5 and 7).
    fn pack(px: &[u16], bits: u32) -> Vec<u8> {
        let per = (128 / bits) as usize;
        px.chunks(per).flat_map(|blk| blk.iter().enumerate().fold(0u128, |w, (i, &v)| w | (v as u128) << (i as u32 * bits)).to_le_bytes()).collect()
    }

    /// A minimal RW2: IFD0 with `tags` plus sensor size, borders (2 rows, 4 columns at the top/left) and Make, the
    /// payload at the end. `at_payload` adds tags that hold the payload's offset.
    fn file(w: u16, h: u16, tags: &[(u16, Value)], payload: &[u8], at_payload: &dyn Fn(u32) -> Vec<(u16, Value)>) -> Vec<u8> {
        let build = |off: u32| {
            let mut ifd = IfdBuilder::new();
            for (tag, v) in [(SENSOR_WIDTH, w), (SENSOR_HEIGHT, h), (BORDER_TOP, 2), (BORDER_LEFT, 4), (BORDER_BOTTOM, h), (BORDER_RIGHT, w)] {
                ifd.set(tag, Value::Short(vec![v]));
            }
            ifd.set(t::MAKE, Value::Ascii("Panasonic".into()));
            for (tag, v) in tags.iter().cloned().chain(at_payload(off)) {
                ifd.set(tag, v);
            }
            let mut b = TiffWriter::new(ByteOrder::Little, false).write(&[ifd]).unwrap();
            b[2] = 0x55;
            b
        };
        let off = build(0).len().div_ceil(16) * 16;
        let mut b = build(off as u32);
        b.resize(off, 0);
        b.extend_from_slice(payload);
        b
    }

    fn short(tag: u16, v: u16) -> (u16, Value) {
        (tag, Value::Short(vec![v]))
    }

    /// Tags of a format-`format`, `bits`-bit file with black levels `black` and the payload at `RawDataOffset`.
    fn rw2(w: u16, h: u16, bits: u16, format: u16, black: u16, payload: &[u8]) -> Vec<u8> {
        let mut tags = vec![short(CFA_PATTERN, 1), short(BITS, bits), short(RAW_FORMAT, format)];
        tags.extend(BLACK_RGB.iter().enumerate().map(|(i, &tag)| short(tag, black + i as u16)));
        tags.extend(WB_RGB.iter().zip([512, 256, 384]).map(|(&tag, v)| short(tag, v)));
        file(w, h, &tags, payload, &|off| vec![(RAW_OFFSET, Value::Long(vec![off]))])
    }

    fn samples(r: &RawImage) -> &[u16] {
        match &r.data {
            RawData::U16(d) => d,
            RawData::F32(_) => panic!("float samples"),
        }
    }

    #[test]
    fn format4_and_6_round_trip_through_files() {
        // format 4: 14 pixels per block, rotated chunks, blocks continuous across rows (here 70 = 5 blocks per row)
        let (w, h) = (70usize, 300usize);
        let target = image(w, h, 200, 4000);
        let (mut stream, mut want) = (vec![], vec![]);
        for px in target.chunks(14) {
            let (b, out) = encode_block(px, None);
            stream.extend_from_slice(&b);
            want.extend(out);
        }
        assert!(want.iter().zip(&target).all(|(a, b)| a.abs_diff(*b) <= 8), "the encoder is too lossy for the test image");
        let bytes = rw2(w as u16, h as u16, 12, 4, 128, &rotate(stream));
        let r = crate::decode(&bytes).unwrap();
        assert_eq!(samples(&r), want);
        assert_eq!(crate::probe_info(&bytes).unwrap(), r.info());
        assert_eq!(r.black.values, vec![143.0; 4].into_iter().zip([0., 1., 1., 2.]).map(|(b, o)| b + o).collect::<Vec<f32>>());

        // format 6: not rotated; 11 (14-bit) or 14 (12-bit) pixels per block
        for (bits, per, top) in [(14u16, 11usize, 16000u32), (12, 14, 4000)] {
            let w = per * 6;
            let target = image(w, 40, 600, top);
            let (mut stream, mut want) = (vec![], vec![]);
            for px in target.chunks(per) {
                let (b, out) = encode_block(px, Some(bits as u32));
                stream.extend_from_slice(&b);
                want.extend(out);
            }
            let bytes = rw2(w as u16, 40, bits, 6, 512, &stream);
            let r = crate::decode(&bytes).unwrap_or_else(|e| panic!("{bits}: {e}"));
            assert_eq!(samples(&r), want, "{bits}-bit");
            assert_eq!(r.black.at(0, 0, 0, 1), 527.0, "format 6 stores samples 15 above the black level");
        }
    }

    /// A tag-0x0119 block: 16 little-endian words, the given ones set.
    fn distortion_block(words: &[(usize, i16)]) -> Value {
        let mut w = [0i16; 16];
        for &(i, v) in words {
            w[i] = v;
        }
        Value::Undefined(w.iter().flat_map(|v| v.to_le_bytes()).collect())
    }

    fn distorted(info: Option<Value>) -> Vec<Opcode> {
        let (w, h) = (90usize, 60usize);
        let px: Vec<u16> = (0..w * h).map(|i| (i % 4000) as u16 + 1).collect();
        let mut tags = vec![short(CFA_PATTERN, 1), short(BITS, 12), short(RAW_FORMAT, 5)];
        tags.extend(info.map(|v| (DISTORTION_INFO, v)));
        let bytes = file(w as u16, h as u16, &tags, &rotate(pack(&px, 12)), &|off| vec![(RAW_OFFSET, Value::Long(vec![off]))]);
        let r = crate::decode(&bytes).unwrap();
        assert_eq!(crate::probe_info(&bytes).unwrap().opcodes, r.opcodes, "headers-only probe reads the same correction");
        r.opcodes.list3
    }

    /// Known answer: correction k2 = 4096/32768 = 1/8 (word 8), zoom 1 − 1024/32768 (word 5), radius 51 px (word 12)
    /// on an 86 × 58 active area. A sensor point at ρ appears at ρ + ρ³/8, so the output corner (corrected radius
    /// t = half diagonal · zoom / 51) shows the root of ρ³ + 8ρ − 8t = 0, here by Cardano's formula.
    #[test]
    fn distortion_correction_becomes_a_rectilinear_warp() {
        let on = |more: &[(usize, i16)]| {
            let mut w = vec![(7, 0x0001), (8, 4096), (5, -1024), (12, 51)];
            w.extend_from_slice(more);
            distortion_block(&w)
        };
        let ops = distorted(Some(on(&[])));
        let [Opcode::WarpRectilinear { planes, center }] = ops.as_slice() else { panic!("{ops:?}") };
        assert_eq!((planes.len(), *center), (1, [0.5, 0.5]));
        let k = planes[0];
        assert_eq!((k[4], k[5]), (0.0, 0.0), "radial only");
        let half_diag = 85f64.hypot(57.0) / 2.0;
        for u in [0.1, 0.25, 0.5, 0.75, 1.0] {
            let t = u * half_diag * (1.0 - 1024.0 / 32768.0) / 51.0;
            let (p, q) = (8.0, -8.0 * t);
            let d = (q * q / 4.0 + p * p * p / 27.0).sqrt();
            let rho = (-q / 2.0 + d).cbrt() + (-q / 2.0 - d).cbrt();
            let want = rho * 51.0 / half_diag;
            let got = u * (k[0] + k[1] * u * u + k[2] * u.powi(4) + k[3] * u.powi(6));
            // the 4-term odd polynomial approximates the exact inverse (5e-5 of the half diagonal ≈ 0.2 px at 20 MP)
            assert!((got - want).abs() < 5e-5, "u {u}: source radius {got}, want {want}");
        }

        // no correction: no block, recorded as off, or the identity
        assert!(distorted(None).is_empty());
        assert!(distorted(Some(distortion_block(&[(7, 0x0000), (8, 4096), (12, 51)]))).is_empty(), "off");
        assert!(distorted(Some(distortion_block(&[(7, 0x7f00u16 as i16), (8, 4096), (12, 51)]))).is_empty(), "off, other bits set");
        assert!(distorted(Some(distortion_block(&[(7, 0x0001), (12, 51)]))).is_empty(), "identity");
        // hostile values are ignored, never a panic or a wild warp
        for (what, block) in [
            ("truncated", Value::Undefined(vec![1, 0, 2, 0, 3])),
            ("radius 0", on(&[(12, 0)])),
            ("radius far beyond the sensor", on(&[(12, -1)])),
            ("zoom 0", on(&[(5, -32768)])),
            ("folds over", on(&[(8, -32768)])),
            ("folds over at high order", on(&[(11, -32768), (4, -32768)])),
            ("runs away", on(&[(8, 32767), (4, 32767), (11, 32767)])),
        ] {
            let ops = distorted(Some(block));
            assert!(ops.iter().all(|op| !matches!(op, Opcode::WarpRectilinear { .. })) || what == "runs away", "{what}: {ops:?}");
            if let [Opcode::WarpRectilinear { planes, .. }] = ops.as_slice() {
                assert!(planes[0].iter().all(|c| c.is_finite()), "{what}");
            }
        }
    }

    #[test]
    fn packed_and_word_formats() {
        let (w, h) = (90usize, 200usize); // > one chunk of data
        for (format, bits, rotated) in [(5u16, 12u32, true), (5, 14, true), (7, 14, false), (2, 12, false)] {
            let px: Vec<u16> = (0..w * h).map(|i| (((i * 2654435761usize) >> 7) as u16 & ((1 << bits) - 1)).max(1)).collect();
            let stream = pack(&px, bits);
            let bytes = rw2(w as u16, h as u16, bits as u16, format, 100, &if rotated { rotate(stream) } else { stream });
            assert_eq!(crate::probe(&bytes), Some(RawFormat::Rw2));
            let r = crate::decode(&bytes).unwrap_or_else(|e| panic!("format {format}, {bits}-bit: {e}"));
            assert_eq!(crate::probe_info(&bytes).unwrap(), r.info());
            assert_eq!(samples(&r), px, "format {format}, {bits}-bit");
            assert_eq!(r.active_area, Rect::new(4, 2, w - 4, h - 2));
            assert_eq!(r.cfa.as_ref().unwrap().name(), "RGGB");
            // packed formats store samples at the recorded black level, per colour, anchored at the active area
            assert_eq!([r.black.at(0, 0, 0, 1), r.black.at(1, 0, 0, 1), r.black.at(1, 1, 0, 1)], [100.0, 101.0, 102.0]);
            assert_eq!(r.wb_multipliers, Some([2.0, 1.0, 1.5]));
        }
        // the oldest files: 16-bit words with the sample left-justified, no black level recorded
        for (compression, bits) in [(34828u16, 12u32), (34830, 14)] {
            let px: Vec<u16> = (0..w * h).map(|i| 100 + (i * 7 % 3000) as u16).collect();
            let payload: Vec<u8> = px.iter().flat_map(|&v| (v << (16 - bits)).to_le_bytes()).collect();
            let tags = [short(BITS, bits as u16), short(COMPRESSION, compression), short(CFA_PATTERN, 4)];
            let bytes = file(w as u16, h as u16, &tags, &payload, &|off| vec![(t::STRIP_OFFSETS, Value::Long(vec![off]))]);
            let r = crate::decode(&bytes).unwrap_or_else(|e| panic!("{compression}: {e}"));
            assert_eq!(samples(&r), px);
            assert_eq!(r.black.mean(), DEFAULT_BLACK);
            assert_eq!(r.white_at(0), ((1 << bits) - 1) as f32, "no saturation plateau: the full range");
        }
    }

    #[test]
    fn levels_crop_defects_and_cfa() {
        let (w, h) = (90usize, 60usize);
        let mut px: Vec<u16> = (0..w * h).map(|i| 300 + (i % 7) as u16).collect();
        // a saturation plateau sets the white level; zeros inside the active area are defects
        px[10 * w..20 * w].fill(3900);
        px[30 * w + 40] = 0;
        px[1] = 0; // in the border: left alone
        let mut tags = vec![short(CFA_PATTERN, 2), short(BITS, 12), short(RAW_FORMAT, 5)];
        tags.extend([0x002f, 0x0030, 0x0031, 0x0032].iter().zip([12u16, 10, 52, 80]).map(|(&tag, v)| short(tag, v)));
        tags.extend(LINEARITY_LIMIT.map(|tag| short(tag, 2000))); // recorded limits are not used
        let bytes = file(w as u16, h as u16, &tags, &rotate(pack(&px, 12)), &|off| vec![(RAW_OFFSET, Value::Long(vec![off]))]);
        let r = crate::decode(&bytes).unwrap();
        let d = samples(&r);
        assert_eq!(d[30 * w + 40], ((px[30 * w + 38] as u32 + px[30 * w + 42] as u32 + px[28 * w + 40] as u32 + px[32 * w + 40] as u32) / 4) as u16);
        assert_eq!(d[1], 0);
        assert!((3800.0..=3900.0).contains(&r.white_at(0)), "{}", r.white_at(0));
        assert_eq!(r.black.mean(), DEFAULT_BLACK, "no black recorded");
        assert_eq!(r.crop, Rect::new(6, 10, 70, 40), "crop tags are sensor coordinates; the crop is relative to the active area");
        assert_eq!(r.info().developed_size(), (70, 40));
        // the CFA tag describes sample (0, 0) of the data, not the active area's origin
        assert_eq!(r.cfa.as_ref().unwrap().name(), "GRBG");
        let mut tags = vec![short(CFA_PATTERN, 2), short(BITS, 12), short(RAW_FORMAT, 5)];
        tags.extend([0x002f, 0x0030, 0x0031, 0x0032].map(|tag| short(tag, 0))); // some bodies write zeros
        let bytes = file(w as u16, h as u16, &tags, &rotate(pack(&px, 12)), &|off| vec![(RAW_OFFSET, Value::Long(vec![off]))]);
        assert_eq!(crate::decode(&bytes).unwrap().crop, Rect::new(0, 0, w - 4, h - 2));
    }

    /// The black is `Metadata` only when all three `BLACK_RGB` tags were read: a missing one is a 0 the file never
    /// stated, so the level is a fallback (its values stay as they were).
    #[test]
    fn black_source_needs_every_black_tag() {
        let (w, h) = (90usize, 60usize);
        let px: Vec<u16> = (0..w * h).map(|i| 300 + (i % 7) as u16).collect();
        let decode = |black: &[(u16, u16)]| {
            let mut tags = vec![short(CFA_PATTERN, 2), short(BITS, 12), short(RAW_FORMAT, 5)];
            tags.extend(black.iter().map(|&(tag, v)| short(tag, v)));
            crate::decode(&file(w as u16, h as u16, &tags, &rotate(pack(&px, 12)), &|off| vec![(RAW_OFFSET, Value::Long(vec![off]))])).unwrap()
        };
        let [r, g, b] = BLACK_RGB;
        assert_eq!(decode(&[(r, 100), (g, 110), (b, 120)]).level_sources.black, crate::LevelSource::Metadata);
        let partial = decode(&[(r, 100), (g, 110)]);
        assert_eq!(partial.level_sources.black, crate::LevelSource::Fallback, "the blue black was never stated");
        let mut values = partial.black.values.clone();
        values.sort_by(f32::total_cmp);
        assert_eq!(values, vec![0.0, 100.0, 110.0, 110.0], "the missing channel still reads 0");
        assert_eq!(decode(&[]).level_sources.black, crate::LevelSource::Fallback);
    }

    /// A plateau at or below the black level can't be the white: the full range is used, a `Fallback`. A
    /// headers-only decode reads no samples, so both sources are `Unknown`.
    #[test]
    fn white_source_of_a_plateau_below_the_black_and_of_headers_only() {
        let (w, h) = (90usize, 60usize);
        let mut px: Vec<u16> = (0..w * h).map(|i| 300 + (i % 7) as u16).collect();
        px[10 * w..20 * w].fill(2500);
        let make = |black: u16| {
            let mut tags = vec![short(CFA_PATTERN, 2), short(BITS, 12), short(RAW_FORMAT, 5)];
            tags.extend(BLACK_RGB.map(|tag| short(tag, black)));
            file(w as u16, h as u16, &tags, &rotate(pack(&px, 12)), &|off| vec![(RAW_OFFSET, Value::Long(vec![off]))])
        };
        let r = crate::decode(&make(100)).unwrap();
        assert!((2400.0..=2500.0).contains(&r.white_at(0)), "{}", r.white_at(0));
        assert_eq!(r.level_sources, crate::LevelSources { black: crate::LevelSource::Metadata, white: crate::LevelSource::Inferred });
        let r = crate::decode(&make(3000)).unwrap();
        assert_eq!(r.white_at(0), 4095.0);
        assert_eq!(r.level_sources, crate::LevelSources { black: crate::LevelSource::Metadata, white: crate::LevelSource::Fallback });
        assert_eq!(
            crate::decode_in(&make(100), crate::Mode::Header, &crate::Limits::default()).unwrap().level_sources,
            crate::LevelSources::default()
        );
    }

    /// Format 8 code tables as two cameras write them (16-bit DC-GH6; 14-bit DC-S5M2, whose codes for categories 15
    /// and 16 start with the 8-bit code of category 14 and are unreachable).
    const TABLE16: [(u32, u32); 17] = [
        (10, 1022),
        (11, 2046),
        (8, 254),
        (9, 510),
        (7, 126),
        (4, 14),
        (4, 12),
        (3, 4),
        (3, 2),
        (2, 0),
        (3, 3),
        (3, 5),
        (4, 13),
        (5, 30),
        (6, 62),
        (12, 4094),
        (12, 4095),
    ];
    type Tags = Vec<(u16, Value)>;

    const TABLE14: [(u32, u32); 17] = [
        (6, 62),
        (7, 126),
        (6, 61),
        (5, 28),
        (4, 12),
        (3, 4),
        (3, 2),
        (2, 0),
        (3, 3),
        (3, 5),
        (4, 13),
        (5, 29),
        (6, 60),
        (8, 254),
        (8, 255),
        (12, 4094),
        (12, 4095),
    ];

    /// Encode a strip (the inverse of [`decode_strip`]): returns the stored bytes and the number of bits.
    fn encode_strip(px: &[u16], w: usize, h: usize, table: &[(u32, u32); 17], padding_rows: usize) -> (Vec<u8>, u64) {
        let mut bits: Vec<bool> = vec![];
        let mut put = |n: u32, v: u32| bits.extend((0..n).rev().map(|i| v >> i & 1 == 1));
        let mut first = [0i32; 4];
        let at = |x: usize, y: usize| if y < h { px[y * w + x] as i32 } else { 1000 };
        for pair in 0..(h / 2 + padding_rows) {
            let mut cell = first;
            for cx in 0..w / 2 {
                for (k, p) in cell.iter_mut().enumerate() {
                    let v = at(2 * cx + k / 2, 2 * pair + k % 2);
                    let d = v - *p;
                    let c = 32 - d.unsigned_abs().leading_zeros();
                    let (len, code) = table[c as usize];
                    put(len, code);
                    put(c, if d >= 0 { d as u32 } else { (d + (1 << c) - 1) as u32 });
                    *p = v;
                }
                if cx == 0 {
                    first = cell;
                }
            }
        }
        let n = bits.len() as u64;
        // stored with each byte's bits taken from bit 0 up
        (bits.chunks(8).map(|b| b.iter().enumerate().fold(0u8, |a, (i, &x)| a | (x as u8) << i)).collect(), n)
    }

    /// A format-8 file with the given strip widths; `tweak` edits the tags before writing.
    fn format8_file(w: usize, h: usize, px: &[u16], widths: &[usize], table: &[(u32, u32); 17], tweak: &dyn Fn(&mut Tags)) -> Vec<u8> {
        format8_tiled_file(w, px, widths, &[h], table, tweak)
    }

    /// A format-8 file tiled in rows of the given heights, each cut into the given widths.
    fn format8_tiled_file(w: usize, px: &[u16], widths: &[usize], rows: &[usize], table: &[(u32, u32); 17], tweak: &dyn Fn(&mut Tags)) -> Vec<u8> {
        let h: usize = rows.iter().sum();
        let mut payload = vec![];
        let mut strips = vec![]; // (offset in payload, bits, left, width, top, height)
        let mut top = 0;
        for &th in rows {
            let mut left = 0;
            for &sw in widths {
                let sub: Vec<u16> = (top..top + th).flat_map(|y| px[y * w + left..y * w + left + sw].to_vec()).collect();
                let (data, bits) = encode_strip(&sub, sw, th, table, 2);
                strips.push((payload.len(), bits, left, sw, top, th));
                payload.extend(data);
                payload.resize(payload.len().div_ceil(16) * 16, 0);
                left += sw;
            }
            top += th;
        }
        let u16s = |v: Vec<u16>| undefined(&v);
        let wide = |v: Vec<u64>| u16s(std::iter::once(v.len() as u16).chain(v.iter().flat_map(|&x| [x as u16, (x >> 16) as u16])).collect());
        let narrow = |v: Vec<u64>| u16s(std::iter::once(v.len() as u16).chain(v.iter().map(|&x| x as u16)).collect());
        let mut tags = vec![short(CFA_PATTERN, 1), short(BITS, 14), short(RAW_FORMAT, 8)];
        tags.extend(BLACK_RGB.map(|tag| short(tag, 512)));
        tags.push((HUFFMAN_CODES, u16s(std::iter::once(17).chain(table.iter().flat_map(|&(l, c)| [l as u16, c as u16])).collect())));
        for (tag, v) in FORMAT8_FIXED {
            tags.push(if v.len() == 1 { short(tag, v[0]) } else { (tag, u16s(v.to_vec())) });
        }
        tags.push(short(TILE_ROWS, rows.len() as u16));
        tags.push((STRIP_LEFT, u16s(std::iter::once(strips.len() as u16).chain(strips.iter().flat_map(|s| [s.2 as u16, s.4 as u16])).collect())));
        tags.push((STRIP_BITS, wide(strips.iter().map(|s| s.1).collect())));
        tags.push((STRIP_WIDTHS, narrow(strips.iter().map(|s| s.3 as u64).collect())));
        tags.push((STRIP_HEIGHTS, narrow(strips.iter().map(|s| s.5 as u64).collect())));
        tweak(&mut tags);
        let offsets: Vec<usize> = strips.iter().map(|s| s.0).collect();
        file(w as u16, h as u16, &tags, &payload, &|off| {
            vec![(RAW_OFFSET, Value::Long(vec![off])), (STRIP_OFFSETS, wide(offsets.iter().map(|&o| off as u64 + o as u64).collect()))]
        })
    }

    #[test]
    fn format8_strips_round_trip() {
        let (w, h) = (64usize, 30usize);
        let px = image(w, h, 520, 16000);
        for (table, widths) in [(&TABLE14, vec![64]), (&TABLE14, vec![30, 34]), (&TABLE16, vec![20, 22, 22])] {
            let bytes = format8_file(w, h, &px, &widths, table, &|_| {});
            let r = crate::decode(&bytes).unwrap_or_else(|e| panic!("{widths:?}: {e}"));
            assert_eq!(samples(&r), px, "{widths:?}");
            assert_eq!(crate::probe_info(&bytes).unwrap(), r.info());
            assert_eq!(r.black.mean(), 512.0, "format 8 stores samples at the recorded black level");
        }
    }

    /// A format-8 sample above the file's bit depth is a corrupt stream: refused, not clamped or kept. The top of
    /// the range itself decodes.
    #[test]
    fn format8_samples_above_the_bit_depth_are_refused() {
        let (w, h) = (64usize, 30usize);
        for depth in [12u16, 14, 16] {
            let top = ((1u32 << depth) - 1) as u16;
            let mut px = image(w, h, 100, 4000);
            px[5 * w + 9] = top;
            let file = |px: &[u16]| format8_file(w, h, px, &[30, 34], &TABLE16, &|t| replace(t, BITS, Value::Short(vec![depth])));
            let r = crate::decode(&file(&px)).unwrap_or_else(|e| panic!("{depth}-bit: {e}"));
            assert_eq!(samples(&r)[5 * w + 9], top, "{depth}-bit");
            if depth < 16 {
                px[5 * w + 9] = top + 1;
                let got = crate::decode(&file(&px));
                assert!(matches!(&got, Err(RawError::Corrupt(m)) if m.contains("outside")), "{depth}-bit: {got:?}");
            }
        }
    }

    #[test]
    fn format8_tiles_in_two_rows_round_trip() {
        // the DC-S1RM2 high-resolution mode: four columns by two rows of tiles, tag 0x0043 = 2
        let (w, h) = (96usize, 40usize);
        let px = image(w, h, 520, 16000);
        for (widths, rows) in [(vec![24, 24, 24, 24], vec![24, 16]), (vec![40, 56], vec![20, 20]), (vec![96], vec![8, 32])] {
            let bytes = format8_tiled_file(w, &px, &widths, &rows, &TABLE14, &|_| {});
            let r = crate::decode(&bytes).unwrap_or_else(|e| panic!("{widths:?} x {rows:?}: {e}"));
            assert_eq!(samples(&r), px, "{widths:?} x {rows:?}");
            assert_eq!(crate::probe_info(&bytes).unwrap(), r.info());
        }
        // a tile row count that disagrees with the tiles, or tiles that leave a gap, are refused
        let case = |tag: u16, v: Value| crate::decode(&format8_tiled_file(w, &px, &[48, 48], &[20, 20], &TABLE14, &|t| replace(t, tag, v.clone())));
        assert!(case(TILE_ROWS, Value::Short(vec![1])).is_err_and(|e| matches!(e, RawError::Unsupported(_))));
        assert!(case(STRIP_HEIGHTS, undefined(&[4, 20, 20, 20, 12])).is_err_and(|e| matches!(e, RawError::Corrupt(_))));
    }

    #[test]
    fn format8_cell_is_coded_column_by_column() {
        // One 2×2 cell coded by hand (not with `encode_strip`, which shares the decoder's site order): the values
        // 1, 2, 3, 4 in stream order. Row order would put the two greens of an RGGB cell on each other's sites.
        let mut bits: Vec<bool> = vec![];
        let mut put = |n: u32, v: u32| bits.extend((0..n).rev().map(|i| v >> i & 1 == 1));
        for v in [1u32, 2, 3, 4] {
            let c = 32 - v.leading_zeros();
            let (len, code) = TABLE14[c as usize];
            put(len, code);
            put(c, v);
        }
        let n = bits.len() as u64;
        let mut data: Vec<u8> = bits.chunks(8).map(|b| b.iter().enumerate().fold(0u8, |a, (i, &x)| a | (x as u8) << i)).collect();
        data.extend([0; 8]);
        let (mut top, mut bottom) = ([0u16; 2], [0u16; 2]);
        decode_strip(
            &Strip { data: &data, left: 0, top: 0, width: 2, height: 2, bits: n },
            &code_table(&TABLE14).unwrap(),
            16383,
            vec![&mut top, &mut bottom],
        )
        .unwrap();
        assert_eq!((top, bottom), ([1, 3], [2, 4]), "top-left, bottom-left, top-right, bottom-right");
    }

    fn replace(tags: &mut Vec<(u16, Value)>, tag: u16, v: Value) {
        tags.retain(|(t, _)| *t != tag);
        tags.push((tag, v));
    }

    fn undefined(v: &[u16]) -> Value {
        Value::Undefined(v.iter().flat_map(|x| x.to_le_bytes()).collect())
    }

    #[test]
    fn format8_rejects_what_it_cannot_decode() {
        let (w, h) = (64usize, 30usize);
        let px = image(w, h, 520, 16000);
        let case = |tag: u16, v: Value| crate::decode(&format8_file(w, h, &px, &[32, 32], &TABLE14, &|t| replace(t, tag, v.clone())));
        // an auxiliary tag with a value no sample has
        assert!(matches!(case(0x003b, Value::Short(vec![4095])), Err(RawError::Unsupported(_))));
        // strips that don't tile the width, or extend past the file
        assert!(matches!(case(STRIP_WIDTHS, undefined(&[2, 32, 30])), Err(RawError::Corrupt(_))));
        assert!(matches!(case(STRIP_BITS, undefined(&[2, 0, 256, 0, 256])), Err(RawError::Corrupt(_))));
        // a strip whose stated length ends before its data does
        assert!(matches!(case(STRIP_BITS, undefined(&[2, 64, 0, 64, 0])), Err(RawError::Corrupt(_))));
        // strips stating fewer bits than they have samples (one bit each at least): refused while reading the layout,
        // so before the sensor-sized buffer is made (probing, which never decodes, refuses it too)
        let short_bits = format8_file(w, h, &px, &[32, 32], &TABLE14, &|t| replace(t, STRIP_BITS, undefined(&[2, 959, 0, 959, 0])));
        assert!(matches!(crate::probe_info(&short_bits), Err(RawError::Corrupt(_))));
        assert!(matches!(crate::decode(&short_bits), Err(RawError::Corrupt(_))));
        // only the second strip short (the first keeps its real length), by one bit or entirely: still refused, naming it
        let second_strip_bits = |stated: u32| {
            format8_file(w, h, &px, &[32, 32], &TABLE14, &|t| {
                let Some((_, Value::Undefined(b))) = t.iter_mut().find(|(tag, _)| *tag == STRIP_BITS) else { panic!("strip bits tag") };
                b[6..10].copy_from_slice(&[stated as u8, (stated >> 8) as u8, (stated >> 16) as u8, (stated >> 24) as u8]);
            })
        };
        for stated in [959, 0] {
            let file = second_strip_bits(stated);
            for got in [crate::probe_info(&file).map(|_| ()), crate::decode(&file).map(|_| ())] {
                assert!(matches!(&got, Err(RawError::Corrupt(m)) if m.contains("strip 1 states")), "{stated} bits: {got:?}");
            }
        }
        // code tables: two categories with the same code, a code longer than its length, missing entries
        let table: Vec<u16> = std::iter::once(17).chain(TABLE14.iter().flat_map(|&(l, c)| [l as u16, c as u16])).collect();
        let mut dup = table.clone();
        dup[1..3].copy_from_slice(&[2, 0]);
        assert!(matches!(case(HUFFMAN_CODES, undefined(&dup)), Err(RawError::Corrupt(_))));
        let mut long = table.clone();
        long[1..3].copy_from_slice(&[3, 9]);
        assert!(matches!(case(HUFFMAN_CODES, undefined(&long)), Err(RawError::Corrupt(_))));
        assert!(matches!(case(HUFFMAN_CODES, undefined(&table[..20])), Err(RawError::Corrupt(_))));
        // the shadowed 12-bit codes of the 14-bit table are fine
        assert!(case(HUFFMAN_CODES, undefined(&table)).is_ok());
    }

    #[test]
    fn unsupported_and_truncated_files() {
        let payload = [0u8; 64];
        for (format, bits) in [(1u16, 12u16), (3, 12), (4, 14), (7, 12), (9, 12)] {
            assert!(matches!(crate::decode(&rw2(28, 4, bits, format, 128, &payload)), Err(RawError::Unsupported(_))), "{format}/{bits}");
        }
        let tags = [short(BITS, 12), short(COMPRESSION, 34826)];
        let bytes = file(28, 4, &tags, &payload, &|off| vec![(t::STRIP_OFFSETS, Value::Long(vec![off]))]);
        assert!(matches!(crate::decode(&bytes), Err(RawError::Unsupported(_))));
        for format in [4u16, 5, 6] {
            let bytes = rw2(280, 20, 12, format, 128, &payload);
            assert!(matches!(crate::decode(&bytes), Err(RawError::Corrupt(_))), "{format}");
            assert!(matches!(crate::probe_info(&bytes), Err(RawError::Corrupt(_))), "{format}");
        }
    }
}
