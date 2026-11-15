//! DirectDraw Surface (DDS) textures, after Microsoft's public DDS documentation ("DDS",
//! "DDS_HEADER", "DDS_PIXELFORMAT", "DDS_HEADER_DXT10").
//!
//! Reading: the top mip level of a 2D texture, either uncompressed (8 to 32 bits per pixel
//! described by channel masks, RGB or luminance) or block-compressed BC1/BC2/BC3
//! (DXT1/DXT3/DXT5), with or without the DX10 header. Cubemaps, texture arrays and volume
//! textures are refused rather than flattened to one face or slice.
//!
//! Writing: a 2D texture, uncompressed (BGRA or BGRX, luminance with or without alpha) or
//! BC1/BC2/BC3, optionally with its full mip chain. The legacy header describes all of these,
//! so no DX10 header is written; formats only DXGI can name (BC4–BC7, sRGB) aren't written.
//!
//! Pixels are 8 bits per channel, interleaved, top row first, in one of the [`Layout`]s. The
//! crate has no dependencies; the `parallel` feature encodes and decodes rows of blocks (and mip
//! levels) on several threads with rayon.
//!
//! ```
//! use craft_dds::{DdsCompression, DdsOptions, Layout};
//!
//! let pixels = [10, 20, 30, 255].repeat(6 * 5);
//! let options = DdsOptions { compression: DdsCompression::Bc1, mipmaps: true };
//! let file = craft_dds::encode(6, 5, Layout::Rgba, &pixels, options)?;
//! let texture = craft_dds::Texture::parse(&file)?;
//! assert_eq!((texture.width(), texture.height(), texture.layout()), (6, 5, Layout::Rgba));
//! assert_eq!(texture.decode()?.len(), 6 * 5 * 4);
//! # Ok::<(), craft_dds::DdsError>(())
//! ```
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod bc;

use std::borrow::Cow;
use std::fmt;

use bc::{Block, Mask};

/// The channels of a pixel, 8 bits each, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Gray,
    /// Gray, then straight (not premultiplied) alpha.
    GrayA,
    Rgb,
    /// Red, green, blue, then straight alpha.
    Rgba,
}

impl Layout {
    /// Samples per pixel.
    pub fn channels(self) -> usize {
        match self {
            Layout::Gray => 1,
            Layout::GrayA => 2,
            Layout::Rgb => 3,
            Layout::Rgba => 4,
        }
    }
}

/// Why a texture could not be read or written. More kinds may be added, so a `match` on it needs
/// a wildcard arm.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DdsError {
    /// The file is damaged or not a DDS file.
    Malformed(String),
    /// A valid texture in a form this crate doesn't read (a cubemap, an array, a compression it
    /// doesn't decode).
    Unsupported(String),
    /// The pixels can't be written with the options given.
    Encode(String),
    /// An output buffer of this many bytes could not be allocated.
    Alloc(usize),
}

impl fmt::Display for DdsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DdsError::Malformed(reason) => write!(f, "malformed DDS: {reason}"),
            DdsError::Unsupported(reason) => write!(f, "unsupported DDS: {reason}"),
            DdsError::Encode(reason) => write!(f, "DDS encode failed: {reason}"),
            DdsError::Alloc(len) => write!(f, "cannot allocate {len} bytes for the texture"),
        }
    }
}

impl std::error::Error for DdsError {}

/// How a DDS texture stores its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DdsCompression {
    /// Uncompressed, 8 bits per channel: BGRA (BGRX without alpha), or luminance with or
    /// without alpha for grayscale. Lossless.
    #[default]
    None,
    /// BC1 (DXT1): 4 bits per pixel. Alpha is on or off: texels under half opacity become
    /// transparent black.
    Bc1,
    /// BC2 (DXT3): 8 bits per pixel, with 4 bits of explicit alpha per texel.
    Bc2,
    /// BC3 (DXT5): 8 bits per pixel, with smooth interpolated alpha.
    Bc3,
}

impl DdsCompression {
    pub const ALL: [DdsCompression; 4] = [DdsCompression::None, DdsCompression::Bc1, DdsCompression::Bc2, DdsCompression::Bc3];

    /// The name scripts and the CLI use: `none`, `bc1`, `bc2` or `bc3`.
    pub fn name(self) -> &'static str {
        match self {
            DdsCompression::None => "none",
            DdsCompression::Bc1 => "bc1",
            DdsCompression::Bc2 => "bc2",
            DdsCompression::Bc3 => "bc3",
        }
    }

    /// [`Self::name`] back to a compression, also taking the DXT names (`dxt1`, `dxt3`,
    /// `dxt5`), in any case. Anything else is `None`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "none" => Some(DdsCompression::None),
            "bc1" | "dxt1" => Some(DdsCompression::Bc1),
            "bc2" | "dxt3" => Some(DdsCompression::Bc2),
            "bc3" | "dxt5" => Some(DdsCompression::Bc3),
            _ => None,
        }
    }

    /// A block compression (lossy), not uncompressed pixels.
    pub fn is_block(self) -> bool {
        self != DdsCompression::None
    }
}

/// What a DDS file is written with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DdsOptions {
    pub compression: DdsCompression,
    /// Write the full mip chain down to 1×1, each level a box filter of the one above.
    pub mipmaps: bool,
}

/// `DDS ` plus the 124-byte DDS_HEADER.
const HEADER: usize = 128;
/// DDS_HEADER_DXT10, present when the FourCC is `DX10`.
const DX10_HEADER: usize = 20;

const DDSD_DEPTH: u32 = 0x80_0000;
const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x2_0000;
const DDSCAPS2_CUBEMAP: u32 = 0x200;
const DDSCAPS2_CUBEMAP_FACES: u32 = 0xfc00;
const DDSCAPS2_VOLUME: u32 = 0x20_0000;
const DIMENSION_TEXTURE2D: u32 = 3;
const MISC_TEXTURECUBE: u32 = 0x4;

fn bad(reason: impl fmt::Display) -> DdsError {
    DdsError::Malformed(reason.to_string())
}
fn unsupported(reason: impl Into<String>) -> DdsError {
    DdsError::Unsupported(reason.into())
}
fn le32(b: &[u8], at: usize) -> Result<u32, DdsError> {
    let bytes = b.get(at..at + 4).and_then(|s| <[u8; 4]>::try_from(s).ok()).ok_or_else(|| bad("truncated header"))?;
    Ok(u32::from_le_bytes(bytes))
}

/// Whether the header declares more than the one image a reader takes: further mip levels,
/// cubemap faces, volume slices or array textures, even without their presence flags.
pub fn has_additional_images(bytes: &[u8]) -> Result<bool, DdsError> {
    let depth = le32(bytes, 24)?;
    let mips = le32(bytes, 28)?;
    let faces = (le32(bytes, 112)? & DDSCAPS2_CUBEMAP_FACES).count_ones();
    let dx10 = le32(bytes, 80)? & DDPF_FOURCC != 0 && bytes.get(84..88) == Some(b"DX10");
    let (cube, array) = if dx10 { (le32(bytes, 136)? & MISC_TEXTURECUBE != 0, le32(bytes, 140)?) } else { (false, 0) };
    Ok(depth > 1 || mips > 1 || faces > 1 || cube || array > 1)
}

/// A FourCC as text when it is printable, else as the number legacy writers store there.
fn fourcc_name(fourcc: u32) -> String {
    let bytes = fourcc.to_le_bytes();
    if bytes.iter().all(|c| c.is_ascii_graphic() || *c == b' ') { bytes.iter().map(|&c| char::from(c)).collect() } else { fourcc.to_string() }
}

/// One channel of an uncompressed pixel: where it sits and its largest value.
#[derive(Clone, Copy)]
struct Field {
    shift: u32,
    max: u32,
}

impl Field {
    /// `None` for an empty mask; an error for one with gaps or wider than the pixel.
    fn new(mask: u32, bits: u32) -> Result<Option<Field>, DdsError> {
        if mask == 0 {
            return Ok(None);
        }
        let shift = mask.trailing_zeros();
        let max = mask >> shift;
        if max & max.wrapping_add(1) != 0 || (bits < 32 && mask >> bits != 0) {
            return Err(bad(format!("channel mask {mask:#x} is not a contiguous field of a {bits}-bit pixel")));
        }
        Ok(Some(Field { shift, max }))
    }

    fn get(self, pixel: u32) -> u8 {
        let v = u64::from((pixel >> self.shift) & self.max);
        let max = u64::from(self.max);
        ((v * 255 + max / 2) / max) as u8
    }
}

#[derive(Clone, Copy)]
enum Encoding {
    /// Uncompressed: bytes per pixel, then red (or luminance), green, blue and alpha.
    Masks {
        bytes: usize,
        gray: bool,
        fields: [Option<Field>; 4],
    },
    Bc1,
    Bc2,
    Bc3,
}

impl Encoding {
    fn masks(bits: u32, masks: [u32; 4], gray: bool) -> Result<Encoding, DdsError> {
        if !matches!(bits, 8 | 16 | 24 | 32) {
            return Err(bad(format!("{bits} bits per pixel is not 8, 16, 24 or 32")));
        }
        let mut fields = [None; 4];
        for (field, mask) in fields.iter_mut().zip(masks) {
            *field = Field::new(mask, bits)?;
        }
        let colour = if gray { fields[0].is_some() } else { fields[..3].iter().any(Option::is_some) };
        if !colour {
            return Err(bad("the pixel format has no colour channel masks"));
        }
        Ok(Encoding::Masks { bytes: bits as usize / 8, gray, fields })
    }

    fn layout(&self) -> Layout {
        match self {
            Encoding::Masks { gray: true, fields, .. } if fields[3].is_some() => Layout::GrayA,
            Encoding::Masks { gray: true, .. } => Layout::Gray,
            Encoding::Masks { fields, .. } if fields[3].is_none() => Layout::Rgb,
            _ => Layout::Rgba,
        }
    }

    /// Stored row stride and byte size of the top mip level (block rows for BC).
    fn level_layout(&self, width: u32, height: u32, pitch: Option<u32>) -> Result<(usize, usize), DdsError> {
        let overflow = || bad("pixel data size overflow");
        let (w, h) = (usize::try_from(width).map_err(|_| overflow())?, usize::try_from(height).map_err(|_| overflow())?);
        let (row_bytes, rows) = match self {
            Encoding::Masks { bytes, .. } => {
                let packed = w.checked_mul(*bytes).ok_or_else(overflow)?;
                let row_bytes = match pitch {
                    Some(pitch) => usize::try_from(pitch).map_err(|_| overflow())?,
                    None => packed,
                };
                if row_bytes < packed {
                    return Err(bad(format!("row pitch {row_bytes} is smaller than the {packed}-byte pixel row")));
                }
                (row_bytes, h)
            }
            Encoding::Bc1 | Encoding::Bc2 | Encoding::Bc3 => {
                let block = if matches!(self, Encoding::Bc1) { 8 } else { 16 };
                (w.div_ceil(4).checked_mul(block).ok_or_else(overflow)?, h.div_ceil(4))
            }
        };
        let size = row_bytes.checked_mul(rows).ok_or_else(overflow)?;
        Ok((row_bytes, size))
    }
}

/// The legacy pixel format: a FourCC, or channel masks.
fn legacy_encoding(b: &[u8]) -> Result<Encoding, DdsError> {
    let (flags, fourcc, bits) = (le32(b, 80)?, le32(b, 84)?, le32(b, 88)?);
    let alpha = if flags & DDPF_ALPHAPIXELS != 0 { le32(b, 104)? } else { 0 };
    if flags & DDPF_FOURCC != 0 {
        return match &fourcc.to_le_bytes() {
            b"DXT1" => Ok(Encoding::Bc1),
            b"DXT3" => Ok(Encoding::Bc2),
            b"DXT5" => Ok(Encoding::Bc3),
            b"DXT2" | b"DXT4" => Err(unsupported("premultiplied-alpha DXT2/DXT4 textures are not supported")),
            _ => Err(unsupported(format!("compression `{}` is not supported (DXT1, DXT3 and DXT5 are)", fourcc_name(fourcc)))),
        };
    }
    if flags & DDPF_RGB != 0 {
        return Encoding::masks(bits, [le32(b, 92)?, le32(b, 96)?, le32(b, 100)?, alpha], false);
    }
    if flags & DDPF_LUMINANCE != 0 {
        return Encoding::masks(bits, [le32(b, 92)?, 0, 0, alpha], true);
    }
    Err(unsupported(format!("pixel format flags {flags:#x} are not RGB, luminance or a supported compression")))
}

/// The DX10 header's DXGI format, refusing anything but a single 2D texture.
fn dx10_encoding(b: &[u8]) -> Result<Encoding, DdsError> {
    let (dxgi, dimension, misc, array) = (le32(b, HEADER)?, le32(b, HEADER + 4)?, le32(b, HEADER + 8)?, le32(b, HEADER + 12)?);
    if misc & MISC_TEXTURECUBE != 0 {
        return Err(unsupported("cubemaps are not supported"));
    }
    if dimension != DIMENSION_TEXTURE2D {
        return Err(unsupported(format!("only 2D textures are supported (resource dimension {dimension})")));
    }
    if array > 1 {
        return Err(unsupported(format!("texture arrays are not supported ({array} textures)")));
    }
    const RGBA: [u32; 4] = [0xff, 0xff00, 0xff_0000, 0xff00_0000];
    const BGRA: [u32; 4] = [0xff_0000, 0xff00, 0xff, 0xff00_0000];
    const BGRX: [u32; 4] = [0xff_0000, 0xff00, 0xff, 0];
    match dxgi {
        27..=29 => Encoding::masks(32, RGBA, false),
        87 | 90 | 91 => Encoding::masks(32, BGRA, false),
        88 | 92 | 93 => Encoding::masks(32, BGRX, false),
        70..=72 => Ok(Encoding::Bc1),
        73..=75 => Ok(Encoding::Bc2),
        76..=78 => Ok(Encoding::Bc3),
        _ => Err(unsupported(format!("DXGI format {dxgi} is not supported (BC1, BC2, BC3 and 8-bit RGBA or BGRA are)"))),
    }
}

/// A DDS file whose header has been read: its size and pixel layout are known before
/// [`Texture::decode`] allocates anything, so a caller can check them against its limits.
pub struct Texture<'a> {
    bytes: &'a [u8],
    flags: u32,
    width: u32,
    height: u32,
    encoding: Encoding,
    /// Where the top level's pixel data starts.
    start: usize,
}

impl<'a> Texture<'a> {
    /// Reads the header, refusing what [`Texture::decode`] can't read.
    pub fn parse(b: &'a [u8]) -> Result<Texture<'a>, DdsError> {
        if !b.starts_with(b"DDS ") {
            return Err(bad("missing the `DDS ` signature"));
        }
        if le32(b, 4)? != 124 {
            return Err(bad("header size is not 124"));
        }
        let (flags, height, width, depth, caps2) = (le32(b, 8)?, le32(b, 12)?, le32(b, 16)?, le32(b, 24)?, le32(b, 112)?);
        if caps2 & DDSCAPS2_CUBEMAP != 0 {
            return Err(unsupported("cubemaps are not supported"));
        }
        if caps2 & DDSCAPS2_VOLUME != 0 || (flags & DDSD_DEPTH != 0 && depth > 1) {
            return Err(unsupported("volume textures are not supported"));
        }
        let dx10 = le32(b, 80)? & DDPF_FOURCC != 0 && &le32(b, 84)?.to_le_bytes() == b"DX10";
        let (encoding, start) = if dx10 { (dx10_encoding(b)?, HEADER + DX10_HEADER) } else { (legacy_encoding(b)?, HEADER) };
        Ok(Texture { bytes: b, flags, width, height, encoding, start })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The layout [`Texture::decode`] returns.
    pub fn layout(&self) -> Layout {
        self.encoding.layout()
    }

    /// The top mip level's pixels in [`Texture::layout`].
    pub fn decode(&self) -> Result<Vec<u8>, DdsError> {
        let (b, flags, encoding) = (self.bytes, self.flags, self.encoding);
        let (width, height) = (self.width, self.height);
        // The same header field is a total level size for BC, but a row stride when DDSD_PITCH is set.
        let pitch = if flags & DDSD_PITCH != 0 { Some(le32(b, 20)?) } else { None };
        let (row_pitch, size) = encoding.level_layout(width, height, pitch)?;
        let data = b.get(self.start..).and_then(|d| d.get(..size)).ok_or_else(|| bad(format!("truncated pixel data: {size} bytes needed")))?;
        let (w, h) = (width as usize, height as usize);
        let channels = encoding.layout().channels();
        let row_size = w.checked_mul(channels).ok_or_else(|| bad("pixel size overflow"))?;
        let total = row_size.checked_mul(h).ok_or_else(|| bad("pixel size overflow"))?;
        let mut pixels = vec![0u8; total];
        match encoding {
            Encoding::Masks { bytes, gray, fields } => decode_masks(Masks { bytes, gray, fields }, data, row_pitch, &mut pixels, row_size),
            Encoding::Bc1 | Encoding::Bc2 | Encoding::Bc3 => decode_blocks(encoding, data, (w, channels), &mut pixels),
        }
        Ok(pixels)
    }
}

/// An uncompressed pixel format: bytes per pixel, luminance or RGB, and the red (or
/// luminance), green, blue and alpha fields.
struct Masks {
    bytes: usize,
    gray: bool,
    fields: [Option<Field>; 4],
}

/// Uncompressed rows `row_pitch` apart into `pixels` (rows of `row_size` bytes), one row of
/// pixels per task.
fn decode_masks(m: Masks, data: &[u8], row_pitch: usize, pixels: &mut [u8], row_size: usize) {
    let Masks { bytes, gray, fields } = m;
    let channels = if gray { 1 + usize::from(fields[3].is_some()) } else { 3 + usize::from(fields[3].is_some()) };
    // The field each output channel comes from.
    let picks = if gray { [0, 3, 0, 0] } else { [0, 1, 2, 3] };
    // An 8-bit field on a byte boundary scales to itself, so when every channel is one (or
    // absent, and 0) the pixels are a byte shuffle: the byte each channel copies.
    let mut at: [Option<usize>; 4] = [None; 4];
    let mut whole_bytes = true;
    for (slot, &pick) in at.iter_mut().zip(&picks).take(channels) {
        match fields.get(pick).copied().flatten() {
            None => {}
            Some(f) if f.max == 255 && f.shift % 8 == 0 => *slot = Some(f.shift as usize / 8),
            Some(_) => whole_bytes = false,
        }
    }
    // Four bytes to four channels, each present: RGBA, BGRA and the like.
    let four = (whole_bytes && bytes == 4 && channels == 4 && at.iter().all(Option::is_some)).then(|| at.map(|i| i.unwrap_or(0) & 3));
    rows(pixels, row_size, |y, dst_row| {
        // Stop at the output width, leaving any stored row padding unread.
        let src_row = data.get(y * row_pitch..).unwrap_or_default();
        if let Some(order) = four {
            let (src, dst) = (src_row.as_chunks::<4>().0, dst_row.as_chunks_mut::<4>().0);
            if order == [0, 1, 2, 3] {
                for (d, s) in dst.iter_mut().zip(src) {
                    *d = *s;
                }
            } else {
                for (d, s) in dst.iter_mut().zip(src) {
                    *d = order.map(|i| s[i]);
                }
            }
        } else if whole_bytes {
            for (src, dst) in src_row.chunks_exact(bytes).zip(dst_row.chunks_exact_mut(channels)) {
                for (v, i) in dst.iter_mut().zip(at) {
                    *v = i.and_then(|i| src.get(i)).copied().unwrap_or(0);
                }
            }
        } else {
            for (src, dst) in src_row.chunks_exact(bytes).zip(dst_row.chunks_exact_mut(channels)) {
                let pixel = src.iter().rev().fold(0u32, |v, &byte| v << 8 | u32::from(byte));
                let channel = |i: usize| fields[i].map_or(0, |f| f.get(pixel));
                let values = if gray { [channel(0), channel(3), 0, 0] } else { [channel(0), channel(1), channel(2), channel(3)] };
                dst.copy_from_slice(&values[..channels]);
            }
        }
    });
}

/// Block-compressed data into `pixels` of `width` × `channels` bytes a row, one row of blocks
/// per task. Blocks wholly inside the image copy whole rows of texels; only those on the right
/// edge check each texel.
fn decode_blocks(encoding: Encoding, data: &[u8], (width, channels): (usize, usize), pixels: &mut [u8]) {
    let block_size = if matches!(encoding, Encoding::Bc1) { 8 } else { 16 };
    let blocks_x = width.div_ceil(4);
    let row_size = width * channels;
    rows(pixels, row_size.saturating_mul(4), |by, band| {
        let src = data.get(by * blocks_x * block_size..).unwrap_or_default();
        for (bx, block) in src.chunks_exact(block_size).take(blocks_x).enumerate() {
            let texels = match encoding {
                Encoding::Bc1 => bc1(block, true),
                Encoding::Bc2 => with_alpha(bc1(block.get(8..).unwrap_or_default(), false), bc2_alpha(block)),
                _ => with_alpha(bc1(block.get(8..).unwrap_or_default(), false), bc3_alpha(block)),
            };
            let x0 = bx * 4;
            // The band holds the block's rows that lie inside the image (fewer than four in the
            // last band of a height that isn't a multiple of four).
            for (line, row) in band.chunks_exact_mut(row_size).zip(texels.as_chunks::<4>().0) {
                if x0 + 4 <= width && channels == 4 {
                    if let Some(dst) = line.get_mut(x0 * 4..x0 * 4 + 16) {
                        dst.copy_from_slice(row.as_flattened());
                    }
                } else {
                    let dst = line.get_mut(x0 * channels..).unwrap_or_default();
                    for (d, texel) in dst.chunks_exact_mut(channels).zip(row) {
                        d.copy_from_slice(texel.get(..channels).unwrap_or_default());
                    }
                }
            }
        }
    });
}

/// A BC1 colour block. BC2 and BC3 colour blocks always have four colours; only BC1's
/// `c0 <= c1` order selects three colours and transparent black.
fn bc1(block: &[u8], one_bit_alpha: bool) -> [[u8; 4]; 16] {
    let byte = |i: usize| block.get(i).copied().unwrap_or(0);
    let (c0, c1) = (u16::from_le_bytes([byte(0), byte(1)]), u16::from_le_bytes([byte(2), byte(3)]));
    let palette = bc::colour_palette(c0, c1, c0 > c1 || !one_bit_alpha);
    let indices = u32::from_le_bytes([byte(4), byte(5), byte(6), byte(7)]);
    std::array::from_fn(|t| palette[(indices >> (2 * t)) as usize & 3])
}

/// BC2: four explicit bits of alpha per texel.
fn bc2_alpha(block: &[u8]) -> [u8; 16] {
    let bits = u64::from_le_bytes(std::array::from_fn(|i| block.get(i).copied().unwrap_or(0)));
    std::array::from_fn(|t| ((bits >> (4 * t)) & 0xf) as u8 * 17)
}

/// BC3: two alpha endpoints and a 3-bit index per texel into eight values, or into six plus
/// 0 and 255 when `a0 <= a1`.
fn bc3_alpha(block: &[u8]) -> [u8; 16] {
    let byte = |i: usize| block.get(i).copied().unwrap_or(0);
    let palette = bc::alpha_palette(byte(0), byte(1));
    let bits = u64::from_le_bytes([byte(2), byte(3), byte(4), byte(5), byte(6), byte(7), 0, 0]);
    std::array::from_fn(|t| palette[((bits >> (3 * t)) & 7) as usize])
}

fn with_alpha(mut texels: [[u8; 4]; 16], alpha: [u8; 16]) -> [[u8; 4]; 16] {
    for (texel, a) in texels.iter_mut().zip(alpha) {
        texel[3] = a;
    }
    texels
}

const DDSD_CAPS: u32 = 0x1;
const DDSD_HEIGHT: u32 = 0x2;
const DDSD_WIDTH: u32 = 0x4;
const DDSD_PITCH: u32 = 0x8;
const DDSD_PIXELFORMAT: u32 = 0x1000;
const DDSD_MIPMAPCOUNT: u32 = 0x2_0000;
const DDSD_LINEARSIZE: u32 = 0x8_0000;
const DDSCAPS_COMPLEX: u32 = 0x8;
const DDSCAPS_TEXTURE: u32 = 0x1000;
const DDSCAPS_MIPMAP: u32 = 0x40_0000;

fn encode_err(reason: impl fmt::Display) -> DdsError {
    DdsError::Encode(reason.to_string())
}

/// A zeroed buffer of `len` bytes, or [`DdsError::Alloc`] when it can't be reserved.
fn alloc_zeroed(len: usize) -> Result<Vec<u8>, DdsError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| DdsError::Alloc(len))?;
    v.resize(len, 0);
    Ok(v)
}

/// How a written texture stores its pixels.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Store {
    /// 8-bit luminance.
    Luminance,
    /// 8-bit luminance, then 8-bit alpha.
    LuminanceAlpha,
    /// Blue, green, red and an unused byte (written as 255, so a reader that takes it for
    /// alpha still sees an opaque texture).
    Bgrx,
    Bgra,
    Bc1,
    Bc2,
    Bc3,
}

impl Store {
    /// The storage for pixels of `layout`. Block compressions take RGBA only.
    fn new(compression: DdsCompression, layout: Layout) -> Result<Store, DdsError> {
        Ok(match (compression, layout) {
            (DdsCompression::None, Layout::Gray) => Store::Luminance,
            (DdsCompression::None, Layout::GrayA) => Store::LuminanceAlpha,
            (DdsCompression::None, Layout::Rgb) => Store::Bgrx,
            (DdsCompression::None, Layout::Rgba) => Store::Bgra,
            (DdsCompression::Bc1, Layout::Rgba) => Store::Bc1,
            (DdsCompression::Bc2, Layout::Rgba) => Store::Bc2,
            (DdsCompression::Bc3, Layout::Rgba) => Store::Bc3,
            (_, layout) => return Err(encode_err(format!("{layout:?} pixels can't be written as `{}`", compression.name()))),
        })
    }

    /// Bytes per pixel, or per 4×4 block for a block compression.
    fn unit(self) -> usize {
        match self {
            Store::Luminance => 1,
            Store::LuminanceAlpha => 2,
            Store::Bgrx | Store::Bgra => 4,
            Store::Bc1 => 8,
            Store::Bc2 | Store::Bc3 => 16,
        }
    }

    fn is_block(self) -> bool {
        matches!(self, Store::Bc1 | Store::Bc2 | Store::Bc3)
    }

    /// Bytes of one row of pixels (or of 4×4 blocks) of a level `width` wide.
    fn row_bytes(self, width: usize) -> Option<usize> {
        if self.is_block() { width.div_ceil(4).checked_mul(self.unit()) } else { width.checked_mul(self.unit()) }
    }

    fn level_bytes(self, width: usize, height: usize) -> Option<usize> {
        let rows = if self.is_block() { height.div_ceil(4) } else { height };
        self.row_bytes(width)?.checked_mul(rows)
    }

    /// DDS_PIXELFORMAT: flags, FourCC, bits per pixel and the red, green, blue and alpha masks.
    fn pixel_format(self) -> (u32, [u8; 4], u32, [u32; 4]) {
        match self {
            Store::Luminance => (DDPF_LUMINANCE, [0; 4], 8, [0xff, 0, 0, 0]),
            Store::LuminanceAlpha => (DDPF_LUMINANCE | DDPF_ALPHAPIXELS, [0; 4], 16, [0xff, 0, 0, 0xff00]),
            Store::Bgrx => (DDPF_RGB, [0; 4], 32, [0xff_0000, 0xff00, 0xff, 0]),
            Store::Bgra => (DDPF_RGB | DDPF_ALPHAPIXELS, [0; 4], 32, [0xff_0000, 0xff00, 0xff, 0xff00_0000]),
            Store::Bc1 => (DDPF_FOURCC, *b"DXT1", 0, [0; 4]),
            Store::Bc2 => (DDPF_FOURCC, *b"DXT3", 0, [0; 4]),
            Store::Bc3 => (DDPF_FOURCC, *b"DXT5", 0, [0; 4]),
        }
    }
}

/// Levels in a full mip chain: the larger side halves down to 1.
fn mip_levels(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

fn half((w, h): (usize, usize)) -> (usize, usize) {
    ((w / 2).max(1), (h / 2).max(1))
}

/// A DDS file of `width` × `height` `pixels` in `layout`, interleaved and top row first.
/// Block compressions take [`Layout::Rgba`] pixels; uncompressed textures take every layout.
pub fn encode(width: u32, height: u32, layout: Layout, pixels: &[u8], options: DdsOptions) -> Result<Vec<u8>, DdsError> {
    let store = Store::new(options.compression, layout)?;
    let expected = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(layout.channels()));
    if expected != Some(pixels.len()) {
        return Err(encode_err(format!("{} bytes of pixels for a {width}x{height} {layout:?} texture", pixels.len())));
    }
    let levels = if options.mipmaps { mip_levels(width, height) } else { 1 };
    let top = (width as usize, height as usize);
    // Every level's size, checked before anything is allocated.
    let mut sizes = Vec::new();
    let mut size = top;
    for _ in 0..levels {
        sizes.push(store.level_bytes(size.0, size.1).ok_or_else(|| encode_err("texture size overflow"))?);
        size = half(size);
    }
    let total = sizes.iter().try_fold(HEADER, |total, &s| total.checked_add(s)).ok_or_else(|| encode_err("texture size overflow"))?;
    let header = header(width, height, store, levels, sizes.first().copied().unwrap_or(0))?;
    let mut out = alloc_zeroed(total)?;
    out.get_mut(..HEADER).ok_or_else(|| encode_err("output buffer too small"))?.copy_from_slice(&header);

    let channels = layout.channels();
    let mut level = Cow::Borrowed(pixels);
    let (mut size, mut at) = (top, HEADER);
    for (i, &bytes) in sizes.iter().enumerate() {
        let dst = out.get_mut(at..at + bytes).ok_or_else(|| encode_err("output buffer too small"))?;
        write_level(store, &level, size, dst)?;
        at += bytes;
        if i + 1 < sizes.len() {
            let next = half(size);
            level = Cow::Owned(downsample(&level, size, next, channels)?);
            size = next;
        }
    }
    Ok(out)
}

/// The `DDS ` signature and DDS_HEADER. `top` is the top level's size in bytes.
fn header(width: u32, height: u32, store: Store, levels: u32, top: usize) -> Result<[u8; HEADER], DdsError> {
    let mut b = [0u8; HEADER];
    let mut put = |at: usize, v: u32| {
        if let Some(d) = b.get_mut(at..at + 4) {
            d.copy_from_slice(&v.to_le_bytes());
        }
    };
    // Uncompressed: the pitch of one row. Compressed: the whole top level (its "linear size").
    let (size_flag, pitch_or_size) = if store.is_block() { (DDSD_LINEARSIZE, Some(top)) } else { (DDSD_PITCH, store.row_bytes(width as usize)) };
    let pitch_or_size = pitch_or_size
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| encode_err(format!("a {width}x{height} level is too large for a DDS header")))?;
    let mipmapped = levels > 1;
    put(4, 124);
    put(8, DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT | size_flag | if mipmapped { DDSD_MIPMAPCOUNT } else { 0 });
    put(12, height);
    put(16, width);
    put(20, pitch_or_size);
    put(28, levels);
    let (flags, fourcc, bits, masks) = store.pixel_format();
    put(76, 32);
    put(80, flags);
    put(84, u32::from_le_bytes(fourcc));
    put(88, bits);
    for (i, mask) in masks.into_iter().enumerate() {
        put(92 + 4 * i, mask);
    }
    put(108, DDSCAPS_TEXTURE | if mipmapped { DDSCAPS_COMPLEX | DDSCAPS_MIPMAP } else { 0 });
    b[..4].copy_from_slice(b"DDS ");
    Ok(b)
}

/// Runs `f` on each `len`-byte row of `out` with its index, in parallel with the `parallel`
/// feature.
fn rows(out: &mut [u8], len: usize, f: impl Fn(usize, &mut [u8]) + Send + Sync) {
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        out.par_chunks_mut(len.max(1)).enumerate().for_each(|(i, row)| f(i, row));
    }
    #[cfg(not(feature = "parallel"))]
    for (i, row) in out.chunks_mut(len.max(1)).enumerate() {
        f(i, row);
    }
}

/// Writes one level of `width` × `height` pixels (in the image's layout) into `dst`.
fn write_level(store: Store, pixels: &[u8], (width, height): (usize, usize), dst: &mut [u8]) -> Result<(), DdsError> {
    let row_bytes = store.row_bytes(width).ok_or_else(|| encode_err("texture size overflow"))?;
    match store {
        Store::Luminance | Store::LuminanceAlpha => {
            // The image's gray (and alpha) samples, in the same order.
            let src = pixels.get(..dst.len()).ok_or_else(|| encode_err("the level is smaller than its size"))?;
            dst.copy_from_slice(src);
        }
        Store::Bgrx | Store::Bgra => {
            let channels = if store == Store::Bgra { 4 } else { 3 };
            rows(dst, row_bytes, |y, row| {
                let src = pixels.get(y * width * channels..).unwrap_or_default();
                for (d, s) in row.as_chunks_mut::<4>().0.iter_mut().zip(src.chunks_exact(channels)) {
                    let byte = |i: usize| s.get(i).copied().unwrap_or(255);
                    *d = [byte(2), byte(1), byte(0), byte(3)];
                }
            });
        }
        Store::Bc1 | Store::Bc2 | Store::Bc3 => {
            let (texels, _) = pixels.as_chunks::<4>();
            rows(dst, row_bytes, |by, row| {
                for (bx, d) in row.chunks_exact_mut(store.unit()).enumerate() {
                    let (block, inside) = gather(texels, (width, height), bx, by);
                    let encoded: &[u8] = match store {
                        Store::Bc1 => &bc::encode_bc1(&block, inside),
                        Store::Bc2 => &bc::encode_bc2(&block, inside),
                        _ => &bc::encode_bc3(&block, inside),
                    };
                    for (d, s) in d.iter_mut().zip(encoded) {
                        *d = *s;
                    }
                }
            });
        }
    }
    Ok(())
}

/// Block (`bx`, `by`)'s texels, and which of them lie inside the image.
fn gather(texels: &[[u8; 4]], (width, height): (usize, usize), bx: usize, by: usize) -> (Block, Mask) {
    let mut block = [[0u8; 4]; 16];
    // A block wholly inside the image: four rows of four texels.
    let (x0, y0) = (bx * 4, by * 4);
    if x0 + 4 <= width && y0 + 4 <= height {
        let mut copied = 0;
        for (ty, row) in block.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            if let Some(src) = texels.get((y0 + ty) * width + x0..).and_then(|s| s.first_chunk::<4>()) {
                *row = *src;
                copied += 1;
            }
        }
        if copied == 4 {
            return (block, Mask::MAX);
        }
    }
    let mut inside: Mask = 0;
    for (t, texel) in block.iter_mut().enumerate() {
        let (x, y) = (bx * 4 + t % 4, by * 4 + t / 4);
        if x < width
            && y < height
            && let Some(p) = texels.get(y * width + x)
        {
            *texel = *p;
            inside |= 1 << t;
        }
    }
    (block, inside)
}

/// The next mip level: each pixel the average of the 2×2 pixels it covers, or of 2×3, 3×2 or
/// 3×3 along the last column and row of an odd-sized level, so no pixel is dropped.
fn downsample(src: &[u8], (w, h): (usize, usize), (nw, nh): (usize, usize), channels: usize) -> Result<Vec<u8>, DdsError> {
    let len = nw.checked_mul(nh).and_then(|n| n.checked_mul(channels)).ok_or_else(|| encode_err("mip level size overflow"))?;
    let mut dst = alloc_zeroed(len)?;
    let span = |i: usize, n: usize, size: usize| 2 * i..=if i + 1 == n { size.saturating_sub(1) } else { 2 * i + 1 };
    let ch = channels.max(1);
    let line = w * ch;
    rows(&mut dst, nw * channels, |y, row| {
        let ys = span(y, nh, h);
        // Every pixel but those of the last column and row averages exactly 2×2: the same sum
        // and rounding as below, from two source rows read in step.
        let mut edge = 0;
        if y + 1 < nh
            && let (Some(r0), Some(r1)) = (src.get(2 * y * line..(2 * y + 1) * line), src.get((2 * y + 1) * line..(2 * y + 2) * line))
        {
            let interior = nw.saturating_sub(1);
            for ((px, a), b) in row.chunks_exact_mut(ch).zip(r0.chunks_exact(2 * ch)).zip(r1.chunks_exact(2 * ch)).take(interior) {
                for (c, v) in px.iter_mut().enumerate() {
                    let sum = u32::from(a[c]) + u32::from(a[ch + c]) + u32::from(b[c]) + u32::from(b[ch + c]);
                    *v = ((sum + 2) / 4) as u8;
                }
            }
            edge = interior;
        }
        for (x, px) in row.chunks_exact_mut(ch).enumerate().skip(edge) {
            let xs = span(x, nw, w);
            for (c, v) in px.iter_mut().enumerate() {
                let (mut sum, mut n) = (0u32, 0u32);
                for sy in ys.clone() {
                    for sx in xs.clone() {
                        if let Some(&s) = src.get((sy * w + sx) * channels + c) {
                            sum += u32::from(s);
                            n += 1;
                        }
                    }
                }
                *v = ((sum + n / 2) / n.max(1)) as u8;
            }
        }
    });
    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pixel_buffer_of_the_wrong_size_is_refused() {
        let options = DdsOptions::default();
        assert!(matches!(encode(2, 2, Layout::Rgb, &[0; 11], options), Err(DdsError::Encode(_))));
        assert!(matches!(encode(2, 2, Layout::Rgb, &[0; 13], options), Err(DdsError::Encode(_))));
        assert!(encode(2, 2, Layout::Rgb, &[0; 12], options).is_ok());
    }

    #[test]
    fn block_compressions_take_rgba_only() {
        let options = DdsOptions { compression: DdsCompression::Bc3, mipmaps: false };
        assert_eq!(encode(1, 1, Layout::Rgb, &[1, 2, 3], options), Err(DdsError::Encode("Rgb pixels can't be written as `bc3`".into())));
    }

    #[test]
    fn the_header_is_read_before_the_pixels() {
        // A header that promises more pixel data than the file holds: its size and layout are
        // known, and only decoding fails.
        let file = encode(8, 8, Layout::Gray, &[7; 64], DdsOptions::default()).unwrap();
        let texture = Texture::parse(&file[..HEADER + 10]).unwrap();
        assert_eq!((texture.width(), texture.height(), texture.layout()), (8, 8, Layout::Gray));
        assert!(matches!(texture.decode(), Err(DdsError::Malformed(_))));
        assert_eq!(Texture::parse(&file).and_then(|t| t.decode()), Ok(vec![7; 64]));
    }
}
