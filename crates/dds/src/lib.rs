//! DirectDraw Surface (DDS) textures, after Microsoft's public DDS documentation ("DDS",
//! "DDS_HEADER", "DDS_PIXELFORMAT", "DDS_HEADER_DXT10").
//!
//! Reading: the top mip level of a 2D texture, either uncompressed (8 to 32 bits per pixel
//! described by channel masks, RGB or luminance, or 16 bits per channel: `R16_UNORM`,
//! `R16G16_UNORM`, `R16G16B16A16_UNORM`) or block-compressed BC1/BC2/BC3
//! (DXT1/DXT3/DXT5) or unsigned BC4/BC5 (ATI1/ATI2), with or without the DX10 header. BC4 opens
//! as gray, and BC5 and `R16G16_UNORM` as RGB with blue 0: they store only red and green, and a
//! normal map's Z is left for the caller to reconstruct. Cubemaps, texture arrays and volume
//! textures are refused rather than flattened to one face or slice, and signed (SNORM) BC4/BC5 is
//! refused too.
//!
//! Writing: a 2D texture, uncompressed (BGRA or BGRX, luminance with or without alpha),
//! BC1/BC2/BC3 or BC4/BC5, optionally with its full mip chain. The legacy header describes all
//! but BC4, BC5 and the 16-bit formats, which are written with the DX10 header (`BC4_UNORM`, `BC5_UNORM`): their
//! legacy FourCCs are vendor names that programs read differently (ATI2's two blocks are swapped
//! by some). BC6H, BC7 and sRGB formats aren't written.
//!
//! Pixels are 8 bits per channel ([`encode`], [`Texture::decode`]) or 16 bits per channel
//! ([`encode16`], [`Texture::decode16`]), interleaved, top row first, in one of the [`Layout`]s.
//! Uncompressed 16-bit pixels are written with the DX10 header as `R16_UNORM` (gray),
//! `R16G16_UNORM` (two channels, such as a normal map's X and Y, read back as red and green) or
//! `R16G16B16A16_UNORM` (RGB, with alpha 65535, and RGBA). BC4 and BC5 fit their 8-bit endpoints
//! and their indices to the 16-bit values; BC1 to BC3 round the pixels to 8 bits first. The
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

/// The channels of a pixel, 8 or 16 bits each, in this order.
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
    /// The decoded output exceeds the caller-selected per-buffer byte limit.
    LimitExceeded { required: usize, limit: u64 },
}

impl fmt::Display for DdsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DdsError::Malformed(reason) => write!(f, "malformed DDS: {reason}"),
            DdsError::Unsupported(reason) => write!(f, "unsupported DDS: {reason}"),
            DdsError::Encode(reason) => write!(f, "DDS encode failed: {reason}"),
            DdsError::Alloc(len) => write!(f, "cannot allocate {len} bytes for the texture"),
            DdsError::LimitExceeded { required, limit } => write!(f, "decoded texture requires {required} bytes, exceeding limit {limit}"),
        }
    }
}

impl std::error::Error for DdsError {}

/// How a DDS texture stores its pixels. More compressions may be added, so a `match` on it needs
/// a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum DdsCompression {
    /// Uncompressed, 8 bits per channel: BGRA (BGRX without alpha), or luminance with or
    /// without alpha for grayscale. From [`encode16`], 16 bits per channel instead. Lossless.
    #[default]
    None,
    /// BC1 (DXT1): 4 bits per pixel. Alpha is on or off: texels under half opacity become
    /// transparent black.
    Bc1,
    /// BC2 (DXT3): 8 bits per pixel, with 4 bits of explicit alpha per texel.
    Bc2,
    /// BC3 (DXT5): 8 bits per pixel, with smooth interpolated alpha.
    Bc3,
    /// BC4 (ATI1): 4 bits per pixel, one channel: gray, for roughness, metallic, height or
    /// ambient-occlusion maps. Written from gray pixels.
    Bc4,
    /// BC5 (ATI2): 8 bits per pixel, two channels: red and green, for tangent-space normal maps
    /// (X and Y; Z is reconstructed by the renderer). Written from RGB pixels, blue dropped.
    Bc5,
}

impl DdsCompression {
    pub const ALL: [DdsCompression; 6] =
        [DdsCompression::None, DdsCompression::Bc1, DdsCompression::Bc2, DdsCompression::Bc3, DdsCompression::Bc4, DdsCompression::Bc5];

    /// The name scripts and the CLI use: `none`, `bc1`, `bc2`, `bc3`, `bc4` or `bc5`.
    pub fn name(self) -> &'static str {
        match self {
            DdsCompression::None => "none",
            DdsCompression::Bc1 => "bc1",
            DdsCompression::Bc2 => "bc2",
            DdsCompression::Bc3 => "bc3",
            DdsCompression::Bc4 => "bc4",
            DdsCompression::Bc5 => "bc5",
        }
    }

    /// [`Self::name`] back to a compression, also taking the DXT and ATI names (`dxt1`, `dxt3`,
    /// `dxt5`, `ati1`, `ati2`), in any case. Anything else is `None`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "none" => Some(DdsCompression::None),
            "bc1" | "dxt1" => Some(DdsCompression::Bc1),
            "bc2" | "dxt3" => Some(DdsCompression::Bc2),
            "bc3" | "dxt5" => Some(DdsCompression::Bc3),
            "bc4" | "ati1" => Some(DdsCompression::Bc4),
            "bc5" | "ati2" => Some(DdsCompression::Bc5),
            _ => None,
        }
    }

    /// The pixel layout the compression is written from: gray for BC4, RGB for BC5, RGBA for
    /// the other block compressions. `None` for uncompressed textures, which take every layout.
    pub fn layout(self) -> Option<Layout> {
        match self {
            DdsCompression::None => None,
            DdsCompression::Bc4 => Some(Layout::Gray),
            DdsCompression::Bc5 => Some(Layout::Rgb),
            DdsCompression::Bc1 | DdsCompression::Bc2 | DdsCompression::Bc3 => Some(Layout::Rgba),
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
/// The legacy FourCC (a number, not text) of 16-bit RGBA.
const D3DFMT_A16B16G16R16: u32 = 36;

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

    fn get16(self, pixel: u32) -> u16 {
        let v = u64::from((pixel >> self.shift) & self.max);
        let max = u64::from(self.max);
        ((v * 65535 + max / 2) / max) as u16
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
    /// 16 bits per channel, little-endian: gray, red and green (read as RGB with blue 0), or
    /// RGBA.
    Unorm16 {
        channels: usize,
    },
    /// One value block: gray.
    Bc4,
    /// Two value blocks: red, then green.
    Bc5,
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
            Encoding::Unorm16 { channels: 1 } => Layout::Gray,
            Encoding::Unorm16 { channels: 2 } => Layout::Rgb,
            Encoding::Bc4 => Layout::Gray,
            Encoding::Bc5 => Layout::Rgb,
            _ => Layout::Rgba,
        }
    }

    /// Bytes per 4×4 block; 0 for uncompressed pixels.
    fn block_size(&self) -> usize {
        match self {
            Encoding::Masks { .. } | Encoding::Unorm16 { .. } => 0,
            Encoding::Bc1 | Encoding::Bc4 => 8,
            Encoding::Bc2 | Encoding::Bc3 | Encoding::Bc5 => 16,
        }
    }

    /// Stored row stride and byte size of the top mip level (block rows for BC).
    fn level_layout(&self, width: u32, height: u32, pitch: Option<u32>) -> Result<(usize, usize), DdsError> {
        let overflow = || bad("pixel data size overflow");
        let (w, h) = (usize::try_from(width).map_err(|_| overflow())?, usize::try_from(height).map_err(|_| overflow())?);
        let pixel_bytes = match self {
            Encoding::Masks { bytes, .. } => Some(*bytes),
            Encoding::Unorm16 { channels } => Some(2 * channels),
            _ => None,
        };
        let (row_bytes, rows) = match pixel_bytes {
            Some(bytes) => {
                let packed = w.checked_mul(bytes).ok_or_else(overflow)?;
                let row_bytes = match pitch {
                    Some(pitch) => usize::try_from(pitch).map_err(|_| overflow())?,
                    None => packed,
                };
                if row_bytes < packed {
                    return Err(bad(format!("row pitch {row_bytes} is smaller than the {packed}-byte pixel row")));
                }
                (row_bytes, h)
            }
            None => (w.div_ceil(4).checked_mul(self.block_size()).ok_or_else(overflow)?, h.div_ceil(4)),
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
        // D3DFMT_A16B16G16R16, the legacy name of R16G16B16A16_UNORM (DirectXTex writes it so).
        if fourcc == D3DFMT_A16B16G16R16 {
            return Ok(Encoding::Unorm16 { channels: 4 });
        }
        return match &fourcc.to_le_bytes() {
            b"DXT1" => Ok(Encoding::Bc1),
            b"DXT3" => Ok(Encoding::Bc2),
            b"DXT5" => Ok(Encoding::Bc3),
            // Microsoft's mapping (DirectXTex reads ATI2 as BC5_UNORM, as Pillow does): red in
            // the first block, green in the second. FFmpeg's reader swaps the two blocks of
            // ATI2, after the old ATI 3Dc tools; this reader doesn't.
            b"ATI1" | b"BC4U" => Ok(Encoding::Bc4),
            b"ATI2" | b"BC5U" => Ok(Encoding::Bc5),
            b"BC4S" | b"BC5S" => Err(unsupported(format!("signed (SNORM) `{}` textures are not supported", fourcc_name(fourcc)))),
            b"DXT2" | b"DXT4" => Err(unsupported("premultiplied-alpha DXT2/DXT4 textures are not supported")),
            _ => Err(unsupported(format!("compression `{}` is not supported (DXT1, DXT3, DXT5, ATI1/BC4U and ATI2/BC5U are)", fourcc_name(fourcc)))),
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
    // miscFlags2 bits 0-2 are the alpha mode; 2 is premultiplied. Only the formats with a real
    // alpha channel are affected, and the legacy path refuses their DXT2/DXT4 equivalents too.
    if le32(b, HEADER + 16)? & 0x7 == 2 && matches!(dxgi, 9 | 11 | 27..=29 | 73..=78 | 87 | 90 | 91) {
        return Err(unsupported(format!("premultiplied-alpha DX10 textures are not supported (DXGI format {dxgi})")));
    }
    const RGBA: [u32; 4] = [0xff, 0xff00, 0xff_0000, 0xff00_0000];
    const BGRA: [u32; 4] = [0xff_0000, 0xff00, 0xff, 0xff00_0000];
    const BGRX: [u32; 4] = [0xff_0000, 0xff00, 0xff, 0];
    match dxgi {
        27..=29 => Encoding::masks(32, RGBA, false),
        // Typeless, then UNORM, as for BC4 and BC5 below.
        9 | 11 => Ok(Encoding::Unorm16 { channels: 4 }),
        33 | 35 => Ok(Encoding::Unorm16 { channels: 2 }),
        53 | 56 => Ok(Encoding::Unorm16 { channels: 1 }),
        87 | 90 | 91 => Encoding::masks(32, BGRA, false),
        88 | 92 | 93 => Encoding::masks(32, BGRX, false),
        70..=72 => Ok(Encoding::Bc1),
        73..=75 => Ok(Encoding::Bc2),
        76..=78 => Ok(Encoding::Bc3),
        // Typeless, then UNORM: a typeless texture's view is unsigned unless a program says
        // otherwise, and Pillow writes BC5 as typeless.
        79 | 80 => Ok(Encoding::Bc4),
        82 | 83 => Ok(Encoding::Bc5),
        81 | 84 => Err(unsupported(format!("signed (SNORM) BC4 and BC5 textures are not supported (DXGI format {dxgi})"))),
        _ => Err(unsupported(format!("DXGI format {dxgi} is not supported (BC1 to BC5, 8-bit RGBA or BGRA and 16-bit R, RG or RGBA are)"))),
    }
}

/// Default per-buffer decoded output budget (1 GiB). This is not a total process memory budget.
pub const DEFAULT_MAX_DECODE_BYTES: u64 = 1024 * 1024 * 1024;

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
    levels: u32,
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
        if b.len() < start {
            return Err(bad("truncated header"));
        }
        if width == 0 || height == 0 {
            return Err(bad("texture dimensions must be nonzero"));
        }
        let levels = le32(b, 28)?.max(1).min(mip_levels(width, height));
        Ok(Texture { bytes: b, flags, width, height, encoding, start, levels })
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

    /// Number of declared mip levels, capped by the chain down to one pixel.
    pub fn levels(&self) -> u32 {
        self.levels
    }

    /// Bits in the widest channel the file stores: 16 for the 16-bit formats, the widest mask
    /// field for other uncompressed pixels, 8 for block compressions. Above 8,
    /// [`Texture::decode`] rounds samples that [`Texture::decode16`] keeps.
    pub fn bits_per_channel(&self) -> u32 {
        match self.encoding {
            Encoding::Unorm16 { .. } => 16,
            Encoding::Masks { fields, .. } => fields.iter().flatten().map(|f| u32::BITS - f.max.leading_zeros()).max().unwrap_or(0),
            Encoding::Bc1 | Encoding::Bc2 | Encoding::Bc3 | Encoding::Bc4 | Encoding::Bc5 => 8,
        }
    }

    /// The encoding named by the header, including unsigned BC4 and BC5.
    pub fn compression(&self) -> DdsCompression {
        match self.encoding {
            Encoding::Masks { .. } | Encoding::Unorm16 { .. } => DdsCompression::None,
            Encoding::Bc1 => DdsCompression::Bc1,
            Encoding::Bc2 => DdsCompression::Bc2,
            Encoding::Bc3 => DdsCompression::Bc3,
            Encoding::Bc4 => DdsCompression::Bc4,
            Encoding::Bc5 => DdsCompression::Bc5,
        }
    }

    /// A borrowed view of one mip level, without allocating or decoding pixels.
    /// Lower levels are tightly packed; only the top level uses the header's row pitch.
    pub fn mip_level(&self, index: u32) -> Result<Texture<'a>, DdsError> {
        if index >= self.levels {
            return Err(bad("mip level index is outside the declared chain"));
        }
        let (mut width, mut height, mut start) = (self.width, self.height, self.start);
        for i in 0..index {
            let pitch = if i == 0 && self.flags & DDSD_PITCH != 0 { Some(le32(self.bytes, 20)?) } else { None };
            let (_, size) = self.encoding.level_layout(width, height, pitch)?;
            start = start.checked_add(size).ok_or_else(|| bad("mip offset overflow"))?;
            if start > self.bytes.len() {
                return Err(bad("truncated mip data"));
            }
            width = (width / 2).max(1);
            height = (height / 2).max(1);
        }
        Ok(Texture {
            bytes: self.bytes,
            flags: if index == 0 { self.flags } else { self.flags & !DDSD_PITCH },
            width,
            height,
            encoding: self.encoding,
            start,
            levels: self.levels - index,
        })
    }

    /// The top mip level's pixels in [`Texture::layout`], bounded by [`DEFAULT_MAX_DECODE_BYTES`].
    pub fn decode(&self) -> Result<Vec<u8>, DdsError> {
        self.decode_with_limit(DEFAULT_MAX_DECODE_BYTES)
    }

    /// Decode with an explicit per-buffer output byte budget; allocation remains fallible.
    pub fn decode_with_limit(&self, max_bytes: u64) -> Result<Vec<u8>, DdsError> {
        let encoding = self.encoding;
        let (data, row_pitch, total) = self.top_level(1, max_bytes)?;
        let (w, channels) = (self.width as usize, encoding.layout().channels());
        let row_size = w * channels;
        let mut pixels = alloc_zeroed(total)?;
        match encoding {
            Encoding::Masks { bytes, gray, fields } => decode_masks(Masks { bytes, gray, fields }, data, row_pitch, &mut pixels, row_size),
            // Each 16-bit sample rounded to the nearest 8-bit level.
            Encoding::Unorm16 { channels: stored } => rows(&mut pixels, row_size, |y, dst| {
                unorm16_row(data.get(y * row_pitch..).unwrap_or_default(), stored, dst, to8);
            }),
            Encoding::Bc1 | Encoding::Bc2 | Encoding::Bc3 | Encoding::Bc4 | Encoding::Bc5 => {
                decode_blocks(encoding, data, (w, channels), &mut pixels)
            }
        }
        Ok(pixels)
    }

    /// The top mip level's pixels in [`Texture::layout`] at 16 bits per channel, bounded by
    /// [`DEFAULT_MAX_DECODE_BYTES`]. 16-bit formats come back exactly; channel-mask fields are
    /// rescaled from their own width to 0..=65535 (a 16-bit luminance field exactly, an 8-bit one
    /// times 257); BC4 and BC5 values are interpolated at 16 bits; BC1 to BC3 are their 8-bit
    /// values times 257 (so 255 is 65535).
    pub fn decode16(&self) -> Result<Vec<u16>, DdsError> {
        self.decode16_with_limit(DEFAULT_MAX_DECODE_BYTES)
    }

    /// [`Texture::decode16`] with an explicit per-buffer output byte budget (two bytes a sample).
    pub fn decode16_with_limit(&self, max_bytes: u64) -> Result<Vec<u16>, DdsError> {
        let encoding = self.encoding;
        let (data, row_pitch, total) = self.top_level(2, max_bytes)?;
        let (w, channels) = (self.width as usize, encoding.layout().channels());
        let row_size = w * channels;
        let mut pixels = alloc_zeroed(total / 2)?;
        match encoding {
            Encoding::Masks { bytes, gray, fields } => decode_masks16(Masks { bytes, gray, fields }, data, row_pitch, &mut pixels, row_size),
            Encoding::Unorm16 { channels: stored } => rows(&mut pixels, row_size, |y, dst| {
                unorm16_row(data.get(y * row_pitch..).unwrap_or_default(), stored, dst, |v| v);
            }),
            Encoding::Bc4 | Encoding::Bc5 => decode_value_blocks16(encoding, data, (w, channels), &mut pixels),
            Encoding::Bc1 | Encoding::Bc2 | Encoding::Bc3 => {
                // The limit was checked above for the 16-bit buffer, which is twice this one.
                let narrow = self.decode_with_limit(u64::MAX)?;
                for (d, v) in pixels.iter_mut().zip(narrow) {
                    *d = u16::from(v) * 257;
                }
            }
        }
        Ok(pixels)
    }

    /// The top level's stored bytes and row stride, and the decoded size in bytes at
    /// `sample_bytes` a sample, checked against `max_bytes` before the stored data is.
    fn top_level(&self, sample_bytes: usize, max_bytes: u64) -> Result<(&'a [u8], usize, usize), DdsError> {
        let (b, flags, encoding) = (self.bytes, self.flags, self.encoding);
        let (width, height) = (self.width, self.height);
        // The same header field is a total level size for BC, but a row stride when DDSD_PITCH is set.
        let pitch = if flags & DDSD_PITCH != 0 { Some(le32(b, 20)?) } else { None };
        let (row_pitch, size) = encoding.level_layout(width, height, pitch)?;
        let (w, h) = (width as usize, height as usize);
        let channels = encoding.layout().channels();
        let row_size = w.checked_mul(channels).and_then(|n| n.checked_mul(sample_bytes)).ok_or_else(|| bad("pixel size overflow"))?;
        let total = row_size.checked_mul(h).ok_or_else(|| bad("pixel size overflow"))?;
        if total as u64 > max_bytes {
            return Err(DdsError::LimitExceeded { required: total, limit: max_bytes });
        }
        let data = b.get(self.start..).and_then(|d| d.get(..size)).ok_or_else(|| bad(format!("truncated pixel data: {size} bytes needed")))?;
        Ok((data, row_pitch, total))
    }
}

/// One row of 16-bit samples, `stored` a pixel and little-endian, into `dst` through `convert`.
/// Two stored channels are red and green: they fill RGB pixels with blue 0.
fn unorm16_row<T: Copy + Default>(src: &[u8], stored: usize, dst: &mut [T], convert: impl Fn(u16) -> T) {
    let src = src.as_chunks::<2>().0;
    if stored == 2 {
        for (d, s) in dst.as_chunks_mut::<3>().0.iter_mut().zip(src.as_chunks::<2>().0) {
            *d = [convert(u16::from_le_bytes(s[0])), convert(u16::from_le_bytes(s[1])), T::default()];
        }
    } else {
        for (d, s) in dst.iter_mut().zip(src) {
            *d = convert(u16::from_le_bytes(*s));
        }
    }
}

/// A 16-bit sample rounded to the nearest 8-bit level.
fn to8(v: u16) -> u8 {
    ((u32::from(v) + 128) / 257) as u8
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

/// [`decode_masks`] at 16 bits: every field scaled to 0..=65535, so fields wider than 8 bits
/// (luminance or red and green of 16 bits) keep their precision.
fn decode_masks16(m: Masks, data: &[u8], row_pitch: usize, pixels: &mut [u16], row_size: usize) {
    let Masks { bytes, gray, fields } = m;
    let channels = if gray { 1 + usize::from(fields[3].is_some()) } else { 3 + usize::from(fields[3].is_some()) };
    rows(pixels, row_size, |y, dst_row| {
        let src_row = data.get(y * row_pitch..).unwrap_or_default();
        for (src, dst) in src_row.chunks_exact(bytes).zip(dst_row.chunks_exact_mut(channels)) {
            let pixel = src.iter().rev().fold(0u32, |v, &byte| v << 8 | u32::from(byte));
            let channel = |i: usize| fields[i].map_or(0, |f| f.get16(pixel));
            let values = if gray { [channel(0), channel(3), 0, 0] } else { [channel(0), channel(1), channel(2), channel(3)] };
            dst.copy_from_slice(&values[..channels]);
        }
    });
}

/// BC4 or BC5 blocks into 16-bit `pixels` of `width` × `channels` samples a row: gray, or red and
/// green with blue 0, interpolated at 16 bits.
fn decode_value_blocks16(encoding: Encoding, data: &[u8], (width, channels): (usize, usize), pixels: &mut [u16]) {
    let block_size = encoding.block_size().max(1);
    let blocks_x = width.div_ceil(4);
    let row_size = width * channels;
    rows(pixels, row_size.saturating_mul(4), |by, band| {
        let src = data.get(by * blocks_x * block_size..).unwrap_or_default();
        for (bx, block) in src.chunks_exact(block_size).take(blocks_x).enumerate() {
            let red = bc::decode_values16(block);
            let green = if matches!(encoding, Encoding::Bc5) { bc::decode_values16(block.get(8..).unwrap_or_default()) } else { [0; 16] };
            let x0 = bx * 4;
            for (ty, line) in band.chunks_exact_mut(row_size).enumerate() {
                let dst = line.get_mut(x0 * channels..).unwrap_or_default();
                for (tx, d) in dst.chunks_exact_mut(channels).take(4).enumerate() {
                    let t = ty * 4 + tx;
                    for (c, v) in d.iter_mut().enumerate() {
                        *v = match c {
                            0 => red[t],
                            1 => green[t],
                            _ => 0,
                        };
                    }
                }
            }
        }
    });
}

/// Block-compressed data into `pixels` of `width` × `channels` bytes a row, one row of blocks
/// per task. Blocks wholly inside the image copy whole rows of texels; only those on the right
/// edge check each texel.
fn decode_blocks(encoding: Encoding, data: &[u8], (width, channels): (usize, usize), pixels: &mut [u8]) {
    let block_size = encoding.block_size().max(1);
    let blocks_x = width.div_ceil(4);
    let row_size = width * channels;
    rows(pixels, row_size.saturating_mul(4), |by, band| {
        let src = data.get(by * blocks_x * block_size..).unwrap_or_default();
        for (bx, block) in src.chunks_exact(block_size).take(blocks_x).enumerate() {
            let second = block.get(8..).unwrap_or_default();
            let texels = match encoding {
                Encoding::Bc1 => bc::decode_colour(block, true),
                Encoding::Bc2 => with_alpha(bc::decode_colour(second, false), bc::decode_bc2_alpha(block)),
                Encoding::Bc4 => values(&[bc::decode_values(block)]),
                Encoding::Bc5 => values(&[bc::decode_values(block), bc::decode_values(second)]),
                _ => with_alpha(bc::decode_colour(second, false), bc::decode_values(block)),
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

/// Texels whose first channels are `channels`' values, the rest 0: gray for BC4, red and green
/// (and blue 0) for BC5.
fn values(channels: &[[u8; 16]]) -> [[u8; 4]; 16] {
    std::array::from_fn(|t| std::array::from_fn(|c| channels.get(c).map_or(0, |v| v[t])))
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
const DXGI_BC4_UNORM: u32 = 80;
const DXGI_BC5_UNORM: u32 = 83;
const DXGI_R16G16B16A16_UNORM: u32 = 11;
const DXGI_R16G16_UNORM: u32 = 35;
const DXGI_R16_UNORM: u32 = 56;

fn encode_err(reason: impl fmt::Display) -> DdsError {
    DdsError::Encode(reason.to_string())
}

/// A zeroed buffer of `len` samples, or [`DdsError::Alloc`] (in bytes) when it can't be reserved.
fn alloc_zeroed<T: Sample>(len: usize) -> Result<Vec<T>, DdsError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| DdsError::Alloc(len.saturating_mul(size_of::<T>())))?;
    v.resize(len, T::default());
    Ok(v)
}

/// A sample of 8 or 16 bits: what the mip chain averages and the buffers hold.
trait Sample: Copy + Default + Send + Sync {
    fn wide(self) -> u32;
    /// An average already within the sample's range.
    fn from_average(v: u32) -> Self;
}

impl Sample for u8 {
    fn wide(self) -> u32 {
        u32::from(self)
    }
    fn from_average(v: u32) -> u8 {
        v as u8
    }
}

impl Sample for u16 {
    fn wide(self) -> u32 {
        u32::from(self)
    }
    fn from_average(v: u32) -> u16 {
        v as u16
    }
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
    Bc4,
    Bc5,
    /// 16-bit gray (`R16_UNORM`).
    R16,
    /// 16-bit gray and alpha, or any two channels (`R16G16_UNORM`).
    Rg16,
    /// 16-bit RGB with alpha 65535 (`R16G16B16A16_UNORM`).
    Rgb16,
    /// 16-bit RGBA (`R16G16B16A16_UNORM`).
    Rgba16,
}

impl Store {
    /// The storage for pixels of `layout`. Block compressions take their
    /// [`DdsCompression::layout`] only.
    fn new(compression: DdsCompression, layout: Layout) -> Result<Store, DdsError> {
        Ok(match (compression, layout) {
            (DdsCompression::None, Layout::Gray) => Store::Luminance,
            (DdsCompression::None, Layout::GrayA) => Store::LuminanceAlpha,
            (DdsCompression::None, Layout::Rgb) => Store::Bgrx,
            (DdsCompression::None, Layout::Rgba) => Store::Bgra,
            (DdsCompression::Bc1, Layout::Rgba) => Store::Bc1,
            (DdsCompression::Bc2, Layout::Rgba) => Store::Bc2,
            (DdsCompression::Bc3, Layout::Rgba) => Store::Bc3,
            (DdsCompression::Bc4, Layout::Gray) => Store::Bc4,
            (DdsCompression::Bc5, Layout::Rgb) => Store::Bc5,
            (_, layout) => return Err(encode_err(format!("{layout:?} pixels can't be written as `{}`", compression.name()))),
        })
    }

    /// [`Store::new`] for 16-bit pixels: uncompressed ones keep 16 bits per channel.
    fn new16(compression: DdsCompression, layout: Layout) -> Result<Store, DdsError> {
        Ok(match (compression, layout) {
            (DdsCompression::None, Layout::Gray) => Store::R16,
            (DdsCompression::None, Layout::GrayA) => Store::Rg16,
            (DdsCompression::None, Layout::Rgb) => Store::Rgb16,
            (DdsCompression::None, Layout::Rgba) => Store::Rgba16,
            _ => Store::new(compression, layout)?,
        })
    }

    /// Bytes per pixel, or per 4×4 block for a block compression.
    fn unit(self) -> usize {
        match self {
            Store::Luminance => 1,
            Store::LuminanceAlpha | Store::R16 => 2,
            Store::Bgrx | Store::Bgra | Store::Rg16 => 4,
            Store::Rgb16 | Store::Rgba16 => 8,
            Store::Bc1 | Store::Bc4 => 8,
            Store::Bc2 | Store::Bc3 | Store::Bc5 => 16,
        }
    }

    fn is_block(self) -> bool {
        matches!(self, Store::Bc1 | Store::Bc2 | Store::Bc3 | Store::Bc4 | Store::Bc5)
    }

    /// The DXGI format of the DX10 header, for the formats the legacy header can't name.
    fn dxgi(self) -> Option<u32> {
        match self {
            Store::Bc4 => Some(DXGI_BC4_UNORM),
            Store::Bc5 => Some(DXGI_BC5_UNORM),
            Store::R16 => Some(DXGI_R16_UNORM),
            Store::Rg16 => Some(DXGI_R16G16_UNORM),
            Store::Rgb16 | Store::Rgba16 => Some(DXGI_R16G16B16A16_UNORM),
            _ => None,
        }
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
            Store::Bc4 | Store::Bc5 | Store::R16 | Store::Rg16 | Store::Rgb16 | Store::Rgba16 => (DDPF_FOURCC, *b"DX10", 0, [0; 4]),
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
/// Block compressions take the layout [`DdsCompression::layout`] names (RGBA for BC1 to BC3,
/// gray for BC4, RGB for BC5); uncompressed textures take every layout.
pub fn encode(width: u32, height: u32, layout: Layout, pixels: &[u8], options: DdsOptions) -> Result<Vec<u8>, DdsError> {
    if width == 0 || height == 0 {
        return Err(encode_err("texture dimensions must be nonzero"));
    }
    let store = Store::new(options.compression, layout)?;
    encode_levels((width, height), layout, pixels, store, options.mipmaps, write_level)
}

/// [`encode`] for 16-bit pixels. Uncompressed, they keep 16 bits per channel: gray as
/// `R16_UNORM`, gray and alpha (or any two channels, such as a normal map's X and Y) as
/// `R16G16_UNORM`, RGB (with alpha 65535) and RGBA as `R16G16B16A16_UNORM`, all with the DX10
/// header. `R16G16_UNORM` holds red and green, so two channels read back as RGB with blue 0. BC4
/// and BC5 pick their endpoints and indices against the 16-bit values; BC1 to BC3
/// round the pixels to 8 bits and write what [`encode`] writes from them.
pub fn encode16(width: u32, height: u32, layout: Layout, pixels: &[u16], options: DdsOptions) -> Result<Vec<u8>, DdsError> {
    if width == 0 || height == 0 {
        return Err(encode_err("texture dimensions must be nonzero"));
    }
    let store = Store::new16(options.compression, layout)?;
    check_len((width, height), layout, pixels)?;
    if matches!(store, Store::Bc1 | Store::Bc2 | Store::Bc3) {
        let mut narrow = alloc_zeroed(pixels.len())?;
        for (d, &v) in narrow.iter_mut().zip(pixels) {
            *d = to8(v);
        }
        return encode(width, height, layout, &narrow, options);
    }
    encode_levels((width, height), layout, pixels, store, options.mipmaps, write_level16)
}

/// Refuses a pixel buffer that isn't `width` × `height` pixels of `layout`.
fn check_len<T: Sample>((width, height): (u32, u32), layout: Layout, pixels: &[T]) -> Result<(), DdsError> {
    let expected = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(layout.channels()));
    if expected != Some(pixels.len()) {
        let unit = if size_of::<T>() == 1 { "bytes" } else { "16-bit samples" };
        return Err(encode_err(format!("{} {unit} of pixels for a {width}x{height} {layout:?} texture", pixels.len())));
    }
    Ok(())
}

/// A level writer: one level of `width` × `height` pixels (in the image's layout) into `dst`.
type WriteLevel<T> = fn(Store, &[T], (usize, usize), &mut [u8]) -> Result<(), DdsError>;

/// The header and every level of `pixels`, each written by `write`.
fn encode_levels<T: Sample>(
    (width, height): (u32, u32),
    layout: Layout,
    pixels: &[T],
    store: Store,
    mipmaps: bool,
    write: WriteLevel<T>,
) -> Result<Vec<u8>, DdsError> {
    check_len((width, height), layout, pixels)?;
    let levels = if mipmaps { mip_levels(width, height) } else { 1 };
    let top = (width as usize, height as usize);
    // Every level's size, checked before anything is allocated.
    let mut sizes = Vec::new();
    let mut size = top;
    for _ in 0..levels {
        sizes.push(store.level_bytes(size.0, size.1).ok_or_else(|| encode_err("texture size overflow"))?);
        size = half(size);
    }
    let head = if store.dxgi().is_some() { HEADER + DX10_HEADER } else { HEADER };
    let total = sizes.iter().try_fold(head, |total, &s| total.checked_add(s)).ok_or_else(|| encode_err("texture size overflow"))?;
    let header = header(width, height, store, levels, sizes.first().copied().unwrap_or(0))?;
    let mut out = alloc_zeroed::<u8>(total)?;
    let (Some(dst), Some(src)) = (out.get_mut(..head), header.get(..head)) else {
        return Err(encode_err("output buffer too small"));
    };
    dst.copy_from_slice(src);

    let channels = layout.channels();
    let mut level = Cow::Borrowed(pixels);
    let (mut size, mut at) = (top, head);
    for (i, &bytes) in sizes.iter().enumerate() {
        let dst = out.get_mut(at..at + bytes).ok_or_else(|| encode_err("output buffer too small"))?;
        write(store, &level, size, dst)?;
        at += bytes;
        if i + 1 < sizes.len() {
            let next = half(size);
            level = Cow::Owned(downsample(&level, size, next, channels)?);
            size = next;
        }
    }
    Ok(out)
}

/// The `DDS ` signature, DDS_HEADER and, for a format only DXGI names, DDS_HEADER_DXT10 (the
/// caller keeps the first [`HEADER`] bytes otherwise). `top` is the top level's size in bytes.
fn header(width: u32, height: u32, store: Store, levels: u32, top: usize) -> Result<[u8; HEADER + DX10_HEADER], DdsError> {
    let mut b = [0u8; HEADER + DX10_HEADER];
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
    if let Some(dxgi) = store.dxgi() {
        // One 2D texture, no misc flags, alpha mode unknown (straight for the one format with
        // alpha, RGBA16, as readers take it).
        put(HEADER, dxgi);
        put(HEADER + 4, DIMENSION_TEXTURE2D);
        put(HEADER + 12, 1);
    }
    b[..4].copy_from_slice(b"DDS ");
    Ok(b)
}

/// Runs `f` on each `len`-byte row of `out` with its index, in parallel with the `parallel`
/// feature.
fn rows<T: Send>(out: &mut [T], len: usize, f: impl Fn(usize, &mut [T]) + Send + Sync) {
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
        // 16-bit stores are written by `write_level16`.
        Store::R16 | Store::Rg16 | Store::Rgb16 | Store::Rgba16 => return Err(encode_err("16-bit storage needs 16-bit pixels")),
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
        Store::Bc4 | Store::Bc5 => {
            // BC4: the gray channel; BC5: red, then green, of RGB pixels.
            let channels = if store == Store::Bc4 { 1 } else { 3 };
            rows(dst, row_bytes, |by, row| {
                for (bx, d) in row.chunks_exact_mut(store.unit()).enumerate() {
                    for (c, d) in d.as_chunks_mut::<8>().0.iter_mut().enumerate() {
                        let (values, inside) = gather_channel(pixels, (width, height), channels, c, (bx, by));
                        *d = bc::value_block(&values, inside);
                    }
                }
            });
        }
    }
    Ok(())
}

/// [`write_level`] for 16-bit pixels: little-endian samples, or BC4/BC5 blocks fitted to them.
fn write_level16(store: Store, pixels: &[u16], (width, height): (usize, usize), dst: &mut [u8]) -> Result<(), DdsError> {
    let row_bytes = store.row_bytes(width).ok_or_else(|| encode_err("texture size overflow"))?;
    match store {
        Store::R16 | Store::Rg16 | Store::Rgba16 => {
            // The image's samples, in the same order.
            for (d, s) in dst.as_chunks_mut::<2>().0.iter_mut().zip(pixels) {
                *d = s.to_le_bytes();
            }
        }
        Store::Rgb16 => rows(dst, row_bytes, |y, row| {
            let src = pixels.get(y * width * 3..).unwrap_or_default();
            for (d, s) in row.as_chunks_mut::<8>().0.iter_mut().zip(src.as_chunks::<3>().0) {
                let [r, g, b, a] = [s[0], s[1], s[2], u16::MAX].map(u16::to_le_bytes);
                *d = [r[0], r[1], g[0], g[1], b[0], b[1], a[0], a[1]];
            }
        }),
        Store::Bc4 | Store::Bc5 => {
            let channels = if store == Store::Bc4 { 1 } else { 3 };
            rows(dst, row_bytes, |by, row| {
                for (bx, d) in row.chunks_exact_mut(store.unit()).enumerate() {
                    for (c, d) in d.as_chunks_mut::<8>().0.iter_mut().enumerate() {
                        let (values, inside) = gather_channel(pixels, (width, height), channels, c, (bx, by));
                        *d = bc::value_block16(&values, inside);
                    }
                }
            });
        }
        // `encode16` rounds BC1 to BC3 to 8 bits; the 8-bit stores take 8-bit pixels.
        _ => return Err(encode_err("8-bit storage needs 8-bit pixels")),
    }
    Ok(())
}

/// Channel `c` of block (`bx`, `by`) of pixels with `channels` channels, and which of its texels
/// lie inside the image.
fn gather_channel<T: Sample>(pixels: &[T], (width, height): (usize, usize), channels: usize, c: usize, (bx, by): (usize, usize)) -> ([T; 16], Mask) {
    let mut values = [T::default(); 16];
    let mut inside: Mask = 0;
    for (t, v) in values.iter_mut().enumerate() {
        let (x, y) = (bx * 4 + t % 4, by * 4 + t / 4);
        if x < width
            && y < height
            && let Some(&p) = pixels.get((y * width + x) * channels + c)
        {
            *v = p;
            inside |= 1 << t;
        }
    }
    (values, inside)
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
fn downsample<T: Sample>(src: &[T], (w, h): (usize, usize), (nw, nh): (usize, usize), channels: usize) -> Result<Vec<T>, DdsError> {
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
                    let sum = a[c].wide() + a[ch + c].wide() + b[c].wide() + b[ch + c].wide();
                    *v = T::from_average((sum + 2) / 4);
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
                            sum += s.wide();
                            n += 1;
                        }
                    }
                }
                *v = T::from_average((sum + n / 2) / n.max(1));
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
    fn block_compressions_take_their_layout_only() {
        let options = DdsOptions { compression: DdsCompression::Bc3, mipmaps: false };
        assert_eq!(encode(1, 1, Layout::Rgb, &[1, 2, 3], options), Err(DdsError::Encode("Rgb pixels can't be written as `bc3`".into())));
        for compression in DdsCompression::ALL {
            for layout in [Layout::Gray, Layout::GrayA, Layout::Rgb, Layout::Rgba] {
                let options = DdsOptions { compression, mipmaps: false };
                let written = encode(1, 1, layout, &[9; 4][..layout.channels()], options);
                let takes = compression.layout().is_none_or(|l| l == layout);
                assert_eq!(written.is_ok(), takes, "{compression:?} from {layout:?}");
            }
        }
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

    #[test]
    fn bits_per_channel_is_the_widest_stored_channel() {
        let options = |compression| DdsOptions { compression, mipmaps: false };
        let gray = encode(4, 4, Layout::Gray, &[7; 16], options(DdsCompression::None)).unwrap();
        let bc4 = encode(4, 4, Layout::Gray, &[7; 16], options(DdsCompression::Bc4)).unwrap();
        let r16 = encode16(4, 4, Layout::Gray, &[7; 16], options(DdsCompression::None)).unwrap();
        let bits = |file: &[u8]| Texture::parse(file).unwrap().bits_per_channel();
        assert_eq!((bits(&gray), bits(&bc4), bits(&r16)), (8, 8, 16));
        // A2R10G10B10 as masks: 10 bits.
        let mut a2r10 = gray[..HEADER].to_vec();
        a2r10[80..84].copy_from_slice(&(DDPF_RGB | DDPF_ALPHAPIXELS).to_le_bytes());
        a2r10[88..92].copy_from_slice(&32u32.to_le_bytes());
        for (i, mask) in [0x3ff0_0000u32, 0xf_fc00, 0x3ff, 0xc000_0000].into_iter().enumerate() {
            a2r10[92 + 4 * i..96 + 4 * i].copy_from_slice(&mask.to_le_bytes());
        }
        assert_eq!(bits(&a2r10), 10);
    }

    /// Samples spread over the whole 16-bit range, `channels` a pixel.
    fn spread16(width: usize, height: usize, channels: usize) -> Vec<u16> {
        (0..width * height * channels).map(|i| (i as u32 * 7919 % 65536) as u16).collect()
    }

    #[test]
    fn uncompressed_16_bit_pixels_round_trip_exactly() {
        let (w, h) = (7, 5);
        for (layout, dxgi, read_as, pixel_bytes) in [
            (Layout::Gray, 56, Layout::Gray, 2),
            (Layout::GrayA, 35, Layout::Rgb, 4),
            (Layout::Rgb, 11, Layout::Rgba, 8),
            (Layout::Rgba, 11, Layout::Rgba, 8),
        ] {
            let pixels = spread16(w, h, layout.channels());
            let want: Vec<u16> = match layout {
                // R16G16 holds red and green: the two channels read back with blue 0.
                Layout::GrayA => pixels.chunks(2).flat_map(|p| [p[0], p[1], 0]).collect(),
                Layout::Rgb => pixels.chunks(3).flat_map(|p| [p[0], p[1], p[2], 65535]).collect(),
                _ => pixels.clone(),
            };
            for mipmaps in [false, true] {
                let file = encode16(w as u32, h as u32, layout, &pixels, DdsOptions { compression: DdsCompression::None, mipmaps }).unwrap();
                assert_eq!(le32(&file, HEADER).unwrap(), dxgi, "{layout:?}");
                // 7×5, then 3×2 and 1×1.
                let samples = if mipmaps { 7 * 5 + 3 * 2 + 1 } else { 7 * 5 };
                assert_eq!(file.len(), HEADER + DX10_HEADER + samples * pixel_bytes);
                let texture = Texture::parse(&file).unwrap();
                assert_eq!(texture.layout(), read_as);
                assert_eq!(texture.decode16().unwrap(), want);
                assert_eq!(texture.decode().unwrap(), want.iter().map(|&v| to8(v)).collect::<Vec<_>>());
                if layout == Layout::Rgba {
                    let mut premultiplied = file.clone();
                    premultiplied[HEADER + 16] = 2;
                    assert!(matches!(Texture::parse(&premultiplied), Err(DdsError::Unsupported(_))));
                }
            }
        }
    }

    #[test]
    fn mip_levels_of_16_bit_pixels_average_at_16_bits() {
        // 1000 to 1003 average to 1002 (rounded), which 8 bits can't hold.
        let file = encode16(2, 2, Layout::Gray, &[1000, 1001, 1002, 1003], DdsOptions { compression: DdsCompression::None, mipmaps: true }).unwrap();
        assert_eq!(le32(&file, 28).unwrap(), 2);
        assert_eq!(file[file.len() - 2..], 1002u16.to_le_bytes());
    }

    #[test]
    fn legacy_16_bit_textures_decode_at_16_bits() {
        // D3DFMT_A16B16G16R16: the legacy header's numeric FourCC 36, the same pixels as the
        // DX10 file.
        let pixels = spread16(3, 2, 4);
        let file = encode16(3, 2, Layout::Rgba, &pixels, DdsOptions::default()).unwrap();
        let mut legacy = file[..HEADER].to_vec();
        legacy[84..88].copy_from_slice(&36u32.to_le_bytes());
        legacy.extend_from_slice(&file[HEADER + DX10_HEADER..]);
        let texture = Texture::parse(&legacy).unwrap();
        assert_eq!((texture.layout(), texture.decode16().unwrap()), (Layout::Rgba, pixels));
        // L16: a 16-bit luminance mask keeps its 16 bits through `decode16`.
        let mut l16 = encode(2, 1, Layout::Gray, &[0, 0], DdsOptions::default()).unwrap();
        l16[20..24].copy_from_slice(&4u32.to_le_bytes());
        l16[88..92].copy_from_slice(&16u32.to_le_bytes());
        l16[92..96].copy_from_slice(&0xffffu32.to_le_bytes());
        l16.truncate(HEADER);
        l16.extend([300u16, 65000].iter().flat_map(|v| v.to_le_bytes()));
        let texture = Texture::parse(&l16).unwrap();
        assert_eq!(texture.decode16().unwrap(), [300, 65000]);
        assert_eq!(texture.decode().unwrap(), [1, 253]);
        assert_eq!(texture.bits_per_channel(), 16);
        // G16R16 as masks (red low, green high) reads as RGB at 16 bits, blue 0.
        let mut g16r16 = l16[..HEADER].to_vec();
        g16r16[80..84].copy_from_slice(&DDPF_RGB.to_le_bytes());
        g16r16[20..24].copy_from_slice(&8u32.to_le_bytes());
        g16r16[88..92].copy_from_slice(&32u32.to_le_bytes());
        g16r16[96..100].copy_from_slice(&0xffff_0000u32.to_le_bytes());
        g16r16.extend([300u16, 40000, 65535, 7].iter().flat_map(|v| v.to_le_bytes()));
        let texture = Texture::parse(&g16r16).unwrap();
        assert_eq!((texture.layout(), texture.decode16().unwrap()), (Layout::Rgb, vec![300, 40000, 0, 65535, 7, 0]));
    }

    /// A DX10 `R16G16` file (DXGI 35, or 33 typeless) of `levels` levels: red then green,
    /// little-endian, each level packed after the one above.
    fn rg16_file(width: u32, height: u32, dxgi: u32, levels: u32, samples: &[u16]) -> Vec<u8> {
        let mut file = header(width, height, Store::R16, levels, 0).unwrap().to_vec();
        file[HEADER..HEADER + 4].copy_from_slice(&dxgi.to_le_bytes());
        file[20..24].copy_from_slice(&(width * 4).to_le_bytes());
        file.extend(samples.iter().flat_map(|v| v.to_le_bytes()));
        file
    }

    #[test]
    fn r16g16_textures_read_as_red_and_green_with_blue_0() {
        // R16G16 stores red and green and no alpha: blue is 0, as for BC5 and G16R16 masks.
        let (w, h) = (3, 2);
        let top = spread16(w, h, 2);
        let rgb: Vec<u16> = top.chunks(2).flat_map(|p| [p[0], p[1], 0]).collect();
        for dxgi in [35, 33] {
            // 3×2, then 1×1 with red 11 and green 22.
            let file = rg16_file(w as u32, h as u32, dxgi, 2, &[top.clone(), vec![11, 22]].concat());
            let texture = Texture::parse(&file).unwrap();
            assert_eq!((texture.layout(), texture.bits_per_channel()), (Layout::Rgb, 16), "DXGI {dxgi}");
            assert_eq!(texture.decode16().unwrap(), rgb, "DXGI {dxgi}");
            assert_eq!(texture.decode().unwrap(), rgb.iter().map(|&v| to8(v)).collect::<Vec<_>>(), "DXGI {dxgi}");
            let level = texture.mip_level(1).unwrap();
            assert_eq!(level.decode16().unwrap(), [11, 22, 0]);
            assert!(matches!(level.decode16_with_limit(5), Err(DdsError::LimitExceeded { required: 6, limit: 5 })));
            assert!(Texture::parse(&file[..file.len() - 5]).unwrap().decode16().is_err());
        }
    }

    #[test]
    fn typeless_and_padded_16_bit_textures_are_read() {
        // Rows padded to 8 bytes, as other writers may store them: the padding is skipped.
        let file = encode16(3, 2, Layout::Gray, &[1, 2, 3, 4, 5, 6], DdsOptions::default()).unwrap();
        let mut padded = file[..HEADER + DX10_HEADER].to_vec();
        padded[20..24].copy_from_slice(&8u32.to_le_bytes());
        for row in file[HEADER + DX10_HEADER..].chunks(6) {
            padded.extend_from_slice(row);
            padded.extend_from_slice(&[0xee, 0xee]);
        }
        assert_eq!(Texture::parse(&padded).unwrap().decode16().unwrap(), [1, 2, 3, 4, 5, 6]);
        // The typeless forms read as the UNORM ones (R16G16's in its own test); typeless RGBA
        // refuses premultiplied alpha too.
        for (unorm, typeless, layout) in [(56u32, 53u32, Layout::Gray), (11, 9, Layout::Rgba)] {
            let pixels = spread16(3, 2, layout.channels());
            let file = encode16(3, 2, layout, &pixels, DdsOptions::default()).unwrap();
            assert_eq!(le32(&file, HEADER).unwrap(), unorm);
            let mut retyped = file.clone();
            retyped[HEADER..HEADER + 4].copy_from_slice(&typeless.to_le_bytes());
            let texture = Texture::parse(&retyped).unwrap();
            assert_eq!((texture.layout(), texture.decode16().unwrap()), (layout, pixels));
            retyped[HEADER + 16] = 2;
            assert_eq!(Texture::parse(&retyped).is_err(), layout == Layout::Rgba);
        }
    }

    #[test]
    fn bc4_and_bc5_from_16_bits_beat_the_8_bit_rounded_source() {
        // Smooth ramps, about half an 8-bit level across a block, as in height and normal maps.
        // 62×61 leaves partial blocks on the right and bottom edges.
        let (w, h) = (62usize, 61usize);
        let at = |i: usize| (i % w, i / w);
        let height: Vec<u16> = (0..w * h).map(|i| (20_000 + at(i).0 * 29 + at(i).1 * 11) as u16).collect();
        let normal: Vec<u16> =
            (0..w * h).flat_map(|i| [(10_000 + at(i).0 * 23 + at(i).1 * 7) as u16, (40_000 + at(i).1 * 27 + at(i).0 * 5) as u16, 65535]).collect();
        for (compression, layout, pixels) in [(DdsCompression::Bc4, Layout::Gray, &height), (DdsCompression::Bc5, Layout::Rgb, &normal)] {
            let options = DdsOptions { compression, mipmaps: true };
            let rounded: Vec<u8> = pixels.iter().map(|&v| to8(v)).collect();
            let (file16, file8) =
                (encode16(w as u32, h as u32, layout, pixels, options).unwrap(), encode(w as u32, h as u32, layout, &rounded, options).unwrap());
            let from16 = Texture::parse(&file16).unwrap();
            let from8 = Texture::parse(&file8).unwrap();
            let channels = layout.channels();
            // Root-mean-square error of the stored channels (BC5: red and green), in 16-bit units.
            let rms = |got: &[u16]| {
                let stored = pixels.iter().zip(got).enumerate().filter(|(i, _)| i % channels < 2);
                let (sum, n) = stored.fold((0f64, 0f64), |(sum, n), (_, (&a, &b))| (sum + (f64::from(a) - f64::from(b)).powi(2), n + 1.0));
                (sum / n).sqrt()
            };
            let (ours, theirs) = (rms(&from16.decode16().unwrap()), rms(&from8.decode16().unwrap()));
            eprintln!("{compression:?}: RMS error {ours:.1} from 16 bits, {theirs:.1} from the 8-bit rounded source (16-bit units)");
            assert!(ours * 2.0 < theirs, "{compression:?}: {ours} vs {theirs}");
            // `decode` and `decode16` read the same blocks: 8 bits are the 16-bit values rounded,
            // give or take the 8-bit palette's own rounding.
            for texture in [&from16, &from8] {
                let (narrow, wide) = (texture.decode().unwrap(), texture.decode16().unwrap());
                assert!(narrow.iter().zip(&wide).all(|(&a, &b)| a.abs_diff(to8(b)) <= 1));
            }
        }
    }

    #[test]
    fn bc1_to_bc3_from_16_bits_write_what_the_rounded_pixels_write() {
        let pixels = spread16(6, 5, 4);
        let rounded: Vec<u8> = pixels.iter().map(|&v| to8(v)).collect();
        for compression in [DdsCompression::Bc1, DdsCompression::Bc2, DdsCompression::Bc3] {
            for mipmaps in [false, true] {
                let options = DdsOptions { compression, mipmaps };
                assert_eq!(encode16(6, 5, Layout::Rgba, &pixels, options), encode(6, 5, Layout::Rgba, &rounded, options));
            }
        }
        // `encode16` takes the layouts `encode` takes, and checks the buffer size the same way.
        for compression in DdsCompression::ALL {
            for layout in [Layout::Gray, Layout::GrayA, Layout::Rgb, Layout::Rgba] {
                let options = DdsOptions { compression, mipmaps: false };
                let n = layout.channels();
                assert_eq!(
                    encode16(1, 1, layout, &[9; 4][..n], options).is_ok(),
                    encode(1, 1, layout, &[9; 4][..n], options).is_ok(),
                    "{compression:?} from {layout:?}"
                );
            }
        }
        assert!(matches!(encode16(2, 2, Layout::Rgb, &[0; 11], DdsOptions::default()), Err(DdsError::Encode(_))));
        let bc1 = DdsOptions { compression: DdsCompression::Bc1, mipmaps: false };
        assert_eq!(encode16(1, 1, Layout::Rgba, &[0; 3], bc1), Err(DdsError::Encode("3 16-bit samples of pixels for a 1x1 Rgba texture".into())));
        // Reading BC1 to BC3 at 16 bits: their 8-bit values times 257.
        let file = encode16(6, 5, Layout::Rgba, &pixels, bc1).unwrap();
        let texture = Texture::parse(&file).unwrap();
        let narrow = texture.decode().unwrap();
        assert_eq!(texture.decode16().unwrap(), narrow.iter().map(|&v| u16::from(v) * 257).collect::<Vec<_>>());
        assert!(matches!(encode16(0, 2, Layout::Rgb, &[], DdsOptions::default()), Err(DdsError::Encode(_))));
    }

    #[test]
    fn decode16_checks_its_budget_in_bytes() {
        let file = encode16(4, 4, Layout::Gray, &[7; 16], DdsOptions::default()).unwrap();
        let texture = Texture::parse(&file).unwrap();
        assert!(matches!(texture.decode16_with_limit(31), Err(DdsError::LimitExceeded { required: 32, limit: 31 })));
        assert_eq!(texture.decode16_with_limit(32).unwrap(), [7; 16]);
    }
}
