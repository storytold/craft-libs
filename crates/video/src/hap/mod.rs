//! Hap video (Vidvox Hap): frames of GPU block-compressed textures, with an optional Snappy
//! second stage, written from the public Hap specification, the Snappy format description and
//! the S3TC / RGTC block formats.
//!
//! A Hap frame is one top-level section: a header (3-byte little-endian size and a type byte, or a
//! zero size and an 8-byte header with a 4-byte size) and its data. The type byte names the
//! texture format (low nibble) and the second-stage compressor (high nibble: none, Snappy, or
//! "chunked", where a table says how the texture is cut into chunks and how each is compressed).
//! A frame of type `0x0D` holds two such sections, the scaled YCoCg and alpha textures of Hap Q
//! Alpha.
//!
//! - [`Decoder`] reads frames. [`Decoder::decode_textures`] gives the block data as stored, ready
//!   for upload to a GPU; [`Decoder::decode_rgba`] decodes DXT1, DXT5, scaled YCoCg-DXT5 (Hap Q),
//!   Hap Q Alpha and the alpha-only format to RGBA pictures in software. BC7 (Hap R) and BC6
//!   (Hap HDR) frames are read, and their blocks returned, but not decoded to pictures.
//! - [`Encoder`] writes Hap, Hap Alpha, Hap Q, Hap Q Alpha and Hap Alpha-Only frames, optionally
//!   Snappy-compressed and cut into chunks.
//!
//! The block compressors are range fits, not the slowest-and-best search a GPU-texture tool does:
//! the result is correct and decodes anywhere, but a little coarser than an exhaustive encoder's.
//! Decoding checks every size against the frame's dimensions before allocating, and a damaged frame
//! is an error, never a panic.

mod dxt;
mod snappy;

use crate::budget::DECODE_MEMORY_BUDGET;

/// Codec errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The frame is damaged or not Hap.
    #[error("invalid Hap frame: {0}")]
    Invalid(String),
    /// A valid frame this module does not decode to pictures.
    #[error("unsupported Hap feature: {0}")]
    Unsupported(String),
    /// The dimensions or settings given cannot be used.
    #[error("invalid Hap parameters: {0}")]
    Input(String),
}

/// `Result` of this module.
pub type Result<T> = std::result::Result<T, Error>;

/// The largest width or height, in pixels.
pub const MAX_DIMENSION: u32 = 32768;

/// The texture formats a Hap frame can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    /// RGB, DXT1 / BC1: 8 bytes per 4x4 block.
    Dxt1,
    /// RGBA, DXT5 / BC3: 16 bytes per block.
    Dxt5,
    /// Scaled YCoCg in DXT5 / BC3 (Hap Q): 16 bytes per block.
    YcocgDxt5,
    /// Alpha only, RGTC1 / BC4: 8 bytes per block.
    Bc4Alpha,
    /// RGBA, BPTC / BC7 (Hap R): 16 bytes per block.
    Bc7,
    /// Unsigned float RGB, BC6H (Hap HDR): 16 bytes per block.
    Bc6Unsigned,
    /// Signed float RGB, BC6H: 16 bytes per block.
    Bc6Signed,
}

impl TextureFormat {
    const fn nibble(self) -> u8 {
        match self {
            Self::Dxt1 => 0xB,
            Self::Dxt5 => 0xE,
            Self::YcocgDxt5 => 0xF,
            Self::Bc4Alpha => 0x1,
            Self::Bc7 => 0xC,
            Self::Bc6Unsigned => 0x2,
            Self::Bc6Signed => 0x3,
        }
    }

    fn from_nibble(n: u8) -> Option<Self> {
        Some(match n {
            0xB => Self::Dxt1,
            0xE => Self::Dxt5,
            0xF => Self::YcocgDxt5,
            0x1 => Self::Bc4Alpha,
            0xC => Self::Bc7,
            0x2 => Self::Bc6Unsigned,
            0x3 => Self::Bc6Signed,
            _ => return None,
        })
    }

    /// Bytes of one 4x4 block.
    #[must_use]
    pub const fn block_bytes(self) -> usize {
        match self {
            Self::Dxt1 | Self::Bc4Alpha => 8,
            _ => 16,
        }
    }
}

/// One texture of a frame: the block data as a GPU takes it (rows of 4x4 blocks, top to bottom,
/// the size rounded up to whole blocks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    pub format: TextureFormat,
    pub data: Vec<u8>,
}

/// A decoded picture: RGBA, 8 bits per channel, rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn check_dimensions(width: u32, height: u32) -> Result<()> {
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(Error::Input(format!("{width}x{height} is not a size from 1 to {MAX_DIMENSION} pixels a side")));
    }
    if u64::from(width) * u64::from(height) * 4 > DECODE_MEMORY_BUDGET / 4 {
        return Err(Error::Input(format!("{width}x{height} is above the memory budget")));
    }
    Ok(())
}

fn blocks(width: u32, height: u32) -> (usize, usize) {
    (width.div_ceil(4) as usize, height.div_ceil(4) as usize)
}

fn texture_bytes(format: TextureFormat, width: u32, height: u32) -> usize {
    let (bx, by) = blocks(width, height);
    bx * by * format.block_bytes()
}

// ------------------------------------------------------------------------------------ sections

/// Split a section off the front of `data`: its type, its payload and what follows it.
fn section(data: &[u8]) -> Result<(u8, &[u8], &[u8])> {
    let bad = || Error::Invalid("section header or size out of range".into());
    let head = data.get(..4).ok_or_else(bad)?;
    let (mut size, mut start) = (usize::from(head[0]) | usize::from(head[1]) << 8 | usize::from(head[2]) << 16, 4usize);
    if size == 0 {
        let ext = data.get(4..8).ok_or_else(bad)?;
        size = u32::from_le_bytes([ext[0], ext[1], ext[2], ext[3]]) as usize;
        start = 8;
    }
    let end = start.checked_add(size).ok_or_else(bad)?;
    let payload = data.get(start..end).ok_or_else(bad)?;
    Ok((head[3], payload, &data[end..]))
}

fn write_section(out: &mut Vec<u8>, kind: u8, payload: &[u8]) {
    if payload.len() < 1 << 24 && !payload.is_empty() {
        out.extend_from_slice(&[payload.len() as u8, (payload.len() >> 8) as u8, (payload.len() >> 16) as u8, kind]);
    } else {
        out.extend_from_slice(&[0, 0, 0, kind]);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    }
    out.extend_from_slice(payload);
}

const SECTION_MULTIPLE: u8 = 0x0D;
const SECTION_INSTRUCTIONS: u8 = 0x01;
const SECTION_COMPRESSORS: u8 = 0x02;
const SECTION_SIZES: u8 = 0x03;
const SECTION_OFFSETS: u8 = 0x04;
const CHUNK_NONE: u8 = 0x0A;
const CHUNK_SNAPPY: u8 = 0x0B;

/// The uncompressed length a Snappy stream declares.
fn snappy_len(src: &[u8]) -> Option<usize> {
    let (mut len, mut shift) = (0u64, 0u32);
    for &b in src.iter().take(5) {
        len |= u64::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            return usize::try_from(len).ok();
        }
        shift += 7;
    }
    None
}

/// Decode one texture section holding `expected` bytes of the given format; returns the texture
/// and the bytes after the section.
fn decode_section(data: &[u8], width: u32, height: u32) -> Result<(Texture, &[u8])> {
    let (kind, payload, rest) = section(data)?;
    let format = TextureFormat::from_nibble(kind & 0x0F).ok_or_else(|| Error::Unsupported(format!("texture type {:#04x}", kind)))?;
    let expected = texture_bytes(format, width, height);
    let bytes = match kind >> 4 {
        0xA => {
            if payload.len() != expected {
                return Err(Error::Invalid(format!("texture of {} bytes, expected {expected}", payload.len())));
            }
            payload.to_vec()
        }
        0xB => snappy::decompress(payload, expected)?,
        0xC => decode_chunks(payload, expected)?,
        _ => return Err(Error::Unsupported(format!("second-stage compressor in type {:#04x}", kind))),
    };
    Ok((Texture { format, data: bytes }, rest))
}

fn decode_chunks(payload: &[u8], expected: usize) -> Result<Vec<u8>> {
    let bad = |m: &str| Error::Invalid(format!("decode instructions: {m}"));
    let (kind, instructions, data) = section(payload)?;
    if kind != SECTION_INSTRUCTIONS {
        return Err(bad("not a decode instructions container"));
    }
    let (mut compressors, mut sizes, mut offsets) = (None, None, None);
    let mut rest = instructions;
    while !rest.is_empty() {
        let (k, body, next) = section(rest)?;
        match k {
            SECTION_COMPRESSORS => compressors = Some(body),
            SECTION_SIZES => sizes = Some(body),
            SECTION_OFFSETS => offsets = Some(body),
            _ => {} // unknown sub-sections are skipped
        }
        rest = next;
    }
    let (compressors, sizes) = compressors.zip(sizes).ok_or_else(|| bad("missing the compressor or size table"))?;
    let count = compressors.len();
    if sizes.len() != count * 4 || offsets.is_some_and(|o| o.len() != count * 4) {
        return Err(bad("the tables differ in length"));
    }
    let word = |t: &[u8], i: usize| u32::from_le_bytes([t[4 * i], t[4 * i + 1], t[4 * i + 2], t[4 * i + 3]]) as usize;
    let mut out = Vec::with_capacity(expected);
    let mut running = 0usize;
    for (i, &compressor) in compressors.iter().enumerate() {
        let size = word(sizes, i);
        let offset = offsets.map_or(running, |o| word(o, i));
        running = offset.checked_add(size).ok_or_else(|| bad("chunk out of range"))?;
        let chunk = data.get(offset..running).ok_or_else(|| bad("chunk out of range"))?;
        match compressor {
            CHUNK_NONE => {
                if out.len() + chunk.len() > expected {
                    return Err(bad("chunks exceed the texture"));
                }
                out.extend_from_slice(chunk);
            }
            CHUNK_SNAPPY => {
                let n = snappy_len(chunk).ok_or_else(|| bad("damaged Snappy chunk"))?;
                if out.len() + n > expected {
                    return Err(bad("chunks exceed the texture"));
                }
                out.extend_from_slice(&snappy::decompress(chunk, n)?);
            }
            other => return Err(Error::Unsupported(format!("chunk compressor {other:#04x}"))),
        }
    }
    if out.len() != expected {
        return Err(bad("chunks do not fill the texture"));
    }
    Ok(out)
}

// ------------------------------------------------------------------------------------ decoder

/// A Hap decoder for frames of one size. Every frame is independent.
#[derive(Debug, Clone)]
pub struct Decoder {
    width: u32,
    height: u32,
}

impl Decoder {
    /// A decoder for `width` x `height` frames.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        check_dimensions(width, height)?;
        Ok(Self { width, height })
    }

    /// The textures of one frame, as stored: one, or two for Hap Q Alpha.
    pub fn decode_textures(&self, frame: &[u8]) -> Result<Vec<Texture>> {
        let (kind, payload, _) = section(frame)?;
        if kind != SECTION_MULTIPLE {
            return Ok(vec![decode_section(frame, self.width, self.height)?.0]);
        }
        let mut textures = Vec::new();
        let mut rest = payload;
        while !rest.is_empty() {
            if textures.len() == 2 {
                return Err(Error::Invalid("more than two images".into()));
            }
            let (t, next) = decode_section(rest, self.width, self.height)?;
            textures.push(t);
            rest = next;
        }
        if textures.is_empty() {
            return Err(Error::Invalid("no images".into()));
        }
        Ok(textures)
    }

    /// Decode one frame to an RGBA picture.
    pub fn decode_rgba(&self, frame: &[u8]) -> Result<Image> {
        let textures = self.decode_textures(frame)?;
        let formats: Vec<TextureFormat> = textures.iter().map(|t| t.format).collect();
        let (w, h) = (self.width as usize, self.height as usize);
        let mut rgba = vec![0u8; w * h * 4];
        let (bx, by) = blocks(self.width, self.height);
        let alpha_tex = match formats.as_slice() {
            [TextureFormat::YcocgDxt5, TextureFormat::Bc4Alpha] => textures.get(1),
            [TextureFormat::Dxt1] | [TextureFormat::Dxt5] | [TextureFormat::YcocgDxt5] | [TextureFormat::Bc4Alpha] => None,
            [TextureFormat::Bc7 | TextureFormat::Bc6Unsigned | TextureFormat::Bc6Signed] => {
                return Err(Error::Unsupported("BC7 and BC6H textures are not decoded to pictures".into()));
            }
            _ => return Err(Error::Unsupported("this combination of textures".into())),
        };
        let main = &textures[0];
        let bb = main.format.block_bytes();
        for j in 0..by {
            for i in 0..bx {
                let at = (j * bx + i) * bb;
                let block = &main.data[at..at + bb];
                let mut px = match main.format {
                    TextureFormat::Dxt1 => dxt::decode_dxt1(block),
                    TextureFormat::Dxt5 => dxt::decode_dxt5(block),
                    TextureFormat::YcocgDxt5 => dxt::decode_ycocg(block),
                    _ => {
                        let a = dxt::decode_bc4(block);
                        std::array::from_fn(|k| [255, 255, 255, a[k]])
                    }
                };
                if let Some(t) = alpha_tex {
                    let a = dxt::decode_bc4(&t.data[(j * bx + i) * 8..(j * bx + i) * 8 + 8]);
                    for (p, a) in px.iter_mut().zip(a) {
                        p[3] = a;
                    }
                }
                for (k, p) in px.iter().enumerate() {
                    let (x, y) = (i * 4 + k % 4, j * 4 + k / 4);
                    if x < w && y < h {
                        rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(p);
                    }
                }
            }
        }
        Ok(Image { width: self.width, height: self.height, rgba })
    }
}

// ------------------------------------------------------------------------------------ encoder

/// The Hap variants the encoder writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variant {
    /// Hap: RGB in DXT1 (`Hap1`).
    Hap,
    /// Hap Alpha: RGBA in DXT5 (`Hap5`).
    HapAlpha,
    /// Hap Q: scaled YCoCg in DXT5 (`HapY`), higher quality, no alpha.
    HapQ,
    /// Hap Q Alpha: scaled YCoCg plus a BC4 alpha texture (`HapM`).
    HapQAlpha,
    /// Hap Alpha-Only: BC4 alpha (`HapA`).
    HapAlphaOnly,
}

impl Variant {
    /// The codec's FourCC.
    #[must_use]
    pub const fn fourcc(self) -> [u8; 4] {
        match self {
            Self::Hap => *b"Hap1",
            Self::HapAlpha => *b"Hap5",
            Self::HapQ => *b"HapY",
            Self::HapQAlpha => *b"HapM",
            Self::HapAlphaOnly => *b"HapA",
        }
    }
}

/// A Hap encoder for frames of one size and variant.
#[derive(Debug, Clone)]
pub struct Encoder {
    width: u32,
    height: u32,
    variant: Variant,
    snappy: bool,
    chunks: usize,
}

impl Encoder {
    /// An encoder with no second stage and no chunks.
    pub fn new(width: u32, height: u32, variant: Variant) -> Result<Self> {
        check_dimensions(width, height)?;
        Ok(Self { width, height, variant, snappy: false, chunks: 1 })
    }

    /// Compress the textures with Snappy.
    #[must_use]
    pub fn with_snappy(mut self, on: bool) -> Self {
        self.snappy = on;
        self
    }

    /// Cut each texture into `n` chunks (at least 1), which a decoder can decompress in parallel.
    #[must_use]
    pub fn with_chunks(mut self, n: usize) -> Self {
        self.chunks = n.clamp(1, 4096);
        self
    }

    /// Encode one RGBA picture (`width * height * 4` bytes) as a Hap frame.
    pub fn encode_rgba(&self, rgba: &[u8]) -> Result<Vec<u8>> {
        let (w, h) = (self.width as usize, self.height as usize);
        if rgba.len() != w * h * 4 {
            return Err(Error::Input(format!("{} bytes of pixels for {w}x{h}", rgba.len())));
        }
        let (bx, by) = blocks(self.width, self.height);
        let block_at = |i: usize, j: usize| -> dxt::Block {
            std::array::from_fn(|k| {
                // Pixels past the edge repeat the last row and column.
                let (x, y) = ((i * 4 + k % 4).min(w - 1), (j * 4 + k / 4).min(h - 1));
                let p = (y * w + x) * 4;
                [rgba[p], rgba[p + 1], rgba[p + 2], rgba[p + 3]]
            })
        };
        let build = |format: TextureFormat, f: &dyn Fn(&dxt::Block) -> Vec<u8>| -> Texture {
            let mut data = Vec::with_capacity(bx * by * format.block_bytes());
            for j in 0..by {
                for i in 0..bx {
                    data.extend_from_slice(&f(&block_at(i, j)));
                }
            }
            Texture { format, data }
        };
        let mut out = Vec::new();
        match self.variant {
            Variant::Hap => self.write_texture(&mut out, &build(TextureFormat::Dxt1, &|b| dxt::encode_dxt1(b).to_vec())),
            Variant::HapAlpha => self.write_texture(&mut out, &build(TextureFormat::Dxt5, &|b| dxt::encode_dxt5(b).to_vec())),
            Variant::HapQ => self.write_texture(&mut out, &build(TextureFormat::YcocgDxt5, &|b| dxt::encode_ycocg(b).to_vec())),
            Variant::HapAlphaOnly => self.write_texture(&mut out, &build(TextureFormat::Bc4Alpha, &|b| dxt::encode_bc4(b).to_vec())),
            Variant::HapQAlpha => {
                let mut inner = Vec::new();
                self.write_texture(&mut inner, &build(TextureFormat::YcocgDxt5, &|b| dxt::encode_ycocg(b).to_vec()));
                self.write_texture(&mut inner, &build(TextureFormat::Bc4Alpha, &|b| dxt::encode_bc4(b).to_vec()));
                write_section(&mut out, SECTION_MULTIPLE, &inner);
            }
        }
        Ok(out)
    }

    fn write_texture(&self, out: &mut Vec<u8>, t: &Texture) {
        let nibble = t.format.nibble();
        if self.chunks <= 1 {
            if self.snappy {
                write_section(out, 0xB0 | nibble, &snappy::compress(&t.data));
            } else {
                write_section(out, 0xA0 | nibble, &t.data);
            }
            return;
        }
        // Chunks of whole blocks, as even as they go.
        let bb = t.format.block_bytes();
        let total_blocks = t.data.len() / bb;
        let n = self.chunks.min(total_blocks).max(1);
        let (mut compressors, mut sizes, mut data) = (Vec::new(), Vec::new(), Vec::new());
        for c in 0..n {
            let (from, to) = (c * total_blocks / n * bb, (c + 1) * total_blocks / n * bb);
            let raw = &t.data[from..to];
            let packed = if self.snappy { Some(snappy::compress(raw)).filter(|p| p.len() < raw.len()) } else { None };
            let (kind, body) = packed.as_deref().map_or((CHUNK_NONE, raw), |p| (CHUNK_SNAPPY, p));
            compressors.push(kind);
            sizes.extend_from_slice(&(body.len() as u32).to_le_bytes());
            data.extend_from_slice(body);
        }
        let mut instructions = Vec::new();
        write_section(&mut instructions, SECTION_COMPRESSORS, &compressors);
        write_section(&mut instructions, SECTION_SIZES, &sizes);
        let mut payload = Vec::new();
        write_section(&mut payload, SECTION_INSTRUCTIONS, &instructions);
        payload.extend_from_slice(&data);
        write_section(out, 0xC0 | nibble, &payload);
    }
}
