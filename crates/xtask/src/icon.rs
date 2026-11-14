//! Icon packing: PNG images into a Windows `.ico` ([`ico`]) or a macOS `.icns` ([`icns`]).
//!
//! Both formats store each PNG as is, so no image is decoded. The app renders the PNGs, chooses the
//! sizes, the input and output paths and the command name, and calls [`ico::pack`] or
//! [`icns::pack`]:
//!
//! ```no_run
//! use std::path::Path;
//! use craft_xtask::icon::{self, Limits, ico};
//!
//! fn cmd_ico(out: &Path, inputs: &[&Path]) -> Result<(), icon::Error> {
//!     let limits = Limits::default();
//!     let pngs = icon::read_pngs(inputs, &limits)?;
//!     let file = ico::pack(&pngs, &limits)?;
//!     std::fs::write(out, &file).map_err(|e| icon::Error::io(out, &e))?;
//!     Ok(())
//! }
//! ```
//!
//! Every PNG is checked before anything is written: the signature, every chunk's length and CRC,
//! a first `IHDR` chunk of 13 bytes with valid fields, at least one `IDAT` chunk and a final `IEND`
//! chunk with nothing after it. The pixel data is not decompressed. Images are written in increasing
//! size, whatever the order of the input, and two images of one size are an error.

pub mod icns;
pub mod ico;

use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// The PNG file signature.
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// The largest chunk length and image side the PNG format allows (2^31 - 1).
const PNG_MAX_U31: u32 = 0x7fff_ffff;

/// Size limits for the images and the packed file. Every input over a limit is an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most images in one icon file. Default 16.
    pub max_images: usize,
    /// The most bytes in one PNG. Default 16 MiB.
    pub max_png_bytes: usize,
    /// The most bytes in the packed file, and in the PNGs that [`read_pngs`] reads together.
    /// Default 128 MiB. The formats' 32-bit offsets and lengths also cap a packed file at
    /// 4 GiB - 1 byte.
    pub max_total_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_images: 16, max_png_bytes: 16 << 20, max_total_bytes: 128 << 20 }
    }
}

/// Why an icon could not be packed, read or checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No images were given.
    Empty,
    /// More images than [`Limits::max_images`], or than the format can hold.
    TooManyImages {
        /// The number of images given.
        count: usize,
        /// The most allowed.
        max: usize,
    },
    /// The bytes are not a complete, well-formed PNG, or not the PNG kind the format needs.
    InvalidPng(String),
    /// The image is not square, or a side is 0.
    NotSquare {
        /// The width from `IHDR`.
        width: u32,
        /// The height from `IHDR`.
        height: u32,
    },
    /// The format has no entry for an image of this size.
    UnsupportedSize {
        /// `"ICO"` or `"ICNS"`.
        format: &'static str,
        /// The image's side in pixels.
        size: u32,
    },
    /// Two images have the same size.
    DuplicateSize(u32),
    /// An input or the packed file is over a limit.
    TooLarge(String),
    /// The memory for the packed file could not be reserved.
    Allocation(usize),
    /// An icon file that was read is not well formed.
    InvalidIcon(String),
    /// A file could not be read or written.
    Io {
        /// The file.
        path: PathBuf,
        /// The error the OS returned.
        cause: String,
    },
}

impl Error {
    /// An [`Error::Io`] for `path`.
    pub fn io(path: &Path, e: &io::Error) -> Self {
        Error::Io { path: path.to_path_buf(), cause: e.to_string() }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Empty => f.write_str("need at least one PNG"),
            Error::TooManyImages { count, max } => write!(f, "{count} images given, at most {max} allowed"),
            Error::InvalidPng(m) => write!(f, "not a valid PNG: {m}"),
            Error::NotSquare { width, height } => write!(f, "icon images must be square and at least 1 px, got {width}x{height}"),
            Error::UnsupportedSize { format: "ICO", size } => write!(f, "ICO images must be 1 to 256 px, got {size}"),
            Error::UnsupportedSize { format, size } => {
                write!(f, "{format} images must be 16, 32, 64, 128, 256, 512 or 1024 px, got {size}")
            }
            Error::DuplicateSize(size) => write!(f, "two images are {size} px; give each size once"),
            Error::TooLarge(m) => write!(f, "too large: {m}"),
            Error::Allocation(n) => write!(f, "could not reserve {n} bytes for the icon file"),
            Error::InvalidIcon(m) => write!(f, "not a valid icon file: {m}"),
            Error::Io { path, cause } => write!(f, "{}: {cause}", path.display()),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for String {
    fn from(e: Error) -> Self {
        e.to_string()
    }
}

/// The `IHDR` fields of a PNG that [`png_info`] checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngInfo {
    /// Width in pixels, 1 to 2^31 - 1.
    pub width: u32,
    /// Height in pixels, 1 to 2^31 - 1.
    pub height: u32,
    /// Bits per sample or per palette index.
    pub bit_depth: u8,
    /// 0 gray, 2 RGB, 3 palette, 4 gray and alpha, 6 RGBA.
    pub color_type: u8,
}

impl PngInfo {
    /// 8-bit RGBA, the 32 bits per pixel that a PNG entry of an ICO file holds.
    pub fn is_rgba8(&self) -> bool {
        self.color_type == 6 && self.bit_depth == 8
    }
}

/// Width and height from a PNG's `IHDR` chunk, after [`png_info`] has checked the whole PNG.
pub fn png_size(png: &[u8]) -> Result<(u32, u32), Error> {
    png_info(png).map(|i| (i.width, i.height))
}

/// Checks the structure of a whole PNG and returns its `IHDR` fields.
///
/// The checks are the signature; every chunk's length (at most 2^31 - 1), type (four ASCII letters,
/// the third uppercase) and CRC; a first `IHDR` chunk of 13 bytes with a width and height of 1 to 2^31 - 1, a valid
/// color type and bit depth pair, compression and filter method 0 and interlace method 0 or 1; at
/// most one `PLTE` chunk, of 1 to 256 entries (2^bit depth in a palette image), before the first
/// `IDAT` chunk, required in a palette image and absent from a gray one; at least one `IDAT` chunk,
/// all of them consecutive; no critical chunk other than these four; and an empty `IEND` chunk at the
/// end with nothing after it.
pub fn png_info(png: &[u8]) -> Result<PngInfo, Error> {
    let bad = |m: &str| Error::InvalidPng(m.to_string());
    if png.get(..8) != Some(PNG_SIGNATURE.as_slice()) {
        return Err(bad("no PNG signature"));
    }
    let mut at = 8usize;
    let mut info = None;
    // `idat_done` is set by the first chunk after a run of IDAT chunks.
    let (mut plte, mut idat, mut idat_done) = (false, false, false);
    loop {
        let header = png.get(at..at + 8).ok_or_else(|| bad("truncated chunk header"))?;
        let len = be32(&header[..4]);
        let kind: [u8; 4] = [header[4], header[5], header[6], header[7]];
        if len > PNG_MAX_U31 {
            return Err(bad("chunk length over 2^31 - 1"));
        }
        if !kind.iter().all(u8::is_ascii_alphabetic) || !kind[2].is_ascii_uppercase() {
            return Err(bad("chunk type is not four ASCII letters with an uppercase third letter"));
        }
        let data_end = (at + 8).checked_add(len as usize).ok_or_else(|| bad("chunk length overflows"))?;
        let crc_end = data_end.checked_add(4).ok_or_else(|| bad("chunk length overflows"))?;
        let (Some(body), Some(stored)) = (png.get(at + 4..data_end), png.get(data_end..crc_end)) else {
            return Err(bad("truncated chunk"));
        };
        if crc32(body) != be32(stored) {
            return Err(Error::InvalidPng(format!("bad CRC in the {} chunk", String::from_utf8_lossy(&kind))));
        }
        let data = &body[4..];
        if idat && &kind != b"IDAT" {
            idat_done = true;
        }
        match (&kind, info) {
            (b"IHDR", None) => info = Some(ihdr(data)?),
            (_, None) => return Err(bad("the first chunk is not IHDR")),
            (b"IHDR", Some(_)) => return Err(bad("a second IHDR chunk")),
            (b"PLTE", Some(i)) => {
                if plte || idat {
                    return Err(bad("a second PLTE chunk, or a PLTE chunk after IDAT"));
                }
                if matches!(i.color_type, 0 | 4) {
                    return Err(bad("a PLTE chunk in a gray image"));
                }
                // A palette image indexes at most 2^bit_depth entries.
                let max_entries = if i.color_type == 3 { 1usize << i.bit_depth } else { 256 };
                if data.is_empty() || data.len() > 3 * max_entries || data.len() % 3 != 0 {
                    return Err(Error::InvalidPng(format!("PLTE chunk is not 1 to {max_entries} entries of 3 bytes")));
                }
                plte = true;
            }
            (b"IDAT", Some(i)) => {
                if i.color_type == 3 && !plte {
                    return Err(bad("a palette image without a PLTE chunk before its IDAT"));
                }
                if idat_done {
                    return Err(bad("IDAT chunks are not consecutive"));
                }
                idat = true;
            }
            (b"IEND", Some(i)) => {
                if len != 0 {
                    return Err(bad("IEND chunk with data"));
                }
                if !idat {
                    return Err(bad("no IDAT chunk"));
                }
                if crc_end != png.len() {
                    return Err(bad("bytes after the IEND chunk"));
                }
                return Ok(i);
            }
            // An uppercase first letter marks a critical chunk, which a decoder must understand.
            (k, Some(_)) if k[0].is_ascii_uppercase() => {
                return Err(Error::InvalidPng(format!("unknown critical chunk {}", String::from_utf8_lossy(k))));
            }
            _ => {}
        }
        at = crc_end;
    }
}

/// The checked fields of an `IHDR` chunk's 13 data bytes.
fn ihdr(data: &[u8]) -> Result<PngInfo, Error> {
    let bad = |m: &str| Error::InvalidPng(m.to_string());
    let [w0, w1, w2, w3, h0, h1, h2, h3, bit_depth, color_type, compression, filter, interlace] = data else {
        return Err(bad("IHDR chunk is not 13 bytes"));
    };
    let (width, height) = (u32::from_be_bytes([*w0, *w1, *w2, *w3]), u32::from_be_bytes([*h0, *h1, *h2, *h3]));
    if width == 0 || height == 0 || width > PNG_MAX_U31 || height > PNG_MAX_U31 {
        return Err(Error::InvalidPng(format!("image size {width}x{height} is outside 1 to 2^31 - 1")));
    }
    let depths: &[u8] = match color_type {
        0 => &[1, 2, 4, 8, 16],
        3 => &[1, 2, 4, 8],
        2 | 4 | 6 => &[8, 16],
        _ => return Err(Error::InvalidPng(format!("color type {color_type}"))),
    };
    if !depths.contains(bit_depth) {
        return Err(Error::InvalidPng(format!("bit depth {bit_depth} with color type {color_type}")));
    }
    if *compression != 0 || *filter != 0 || *interlace > 1 {
        return Err(bad("unknown compression, filter or interlace method"));
    }
    Ok(PngInfo { width, height, bit_depth: *bit_depth, color_type: *color_type })
}

/// Reads the PNGs at `paths`, in order, within `limits`: at most [`Limits::max_images`] files, each
/// at most [`Limits::max_png_bytes`] and together at most [`Limits::max_total_bytes`]. A file over a
/// limit is an error before more than one byte past the limit is read. The bytes are not checked;
/// [`ico::pack`] and [`icns::pack`] check them.
pub fn read_pngs<P: AsRef<Path>>(paths: &[P], limits: &Limits) -> Result<Vec<Vec<u8>>, Error> {
    if paths.len() > limits.max_images {
        return Err(Error::TooManyImages { count: paths.len(), max: limits.max_images });
    }
    let mut out = Vec::with_capacity(paths.len());
    let mut left = limits.max_total_bytes;
    for path in paths {
        let path = path.as_ref();
        // Each read stops at the smaller of the file limit and what the total limit has left.
        let png = match read_bounded(path, limits.max_png_bytes.min(left)) {
            Err(Error::TooLarge(_)) if left < limits.max_png_bytes => {
                return Err(Error::TooLarge(format!("the PNGs hold more than {} bytes together", limits.max_total_bytes)));
            }
            r => r?,
        };
        left -= png.len();
        out.push(png);
    }
    Ok(out)
}

/// The bytes of the file at `path`, or [`Error::TooLarge`] when it holds more than `max` bytes.
pub(crate) fn read_bounded(path: &Path, max: usize) -> Result<Vec<u8>, Error> {
    let too_large = || Error::TooLarge(format!("{} holds more than {max} bytes", path.display()));
    let file = File::open(path).map_err(|e| Error::io(path, &e))?;
    let len = file.metadata().map_err(|e| Error::io(path, &e))?.len();
    if len > max as u64 {
        return Err(too_large());
    }
    let mut buf = Vec::new();
    // `len` is at most `max`, a usize. The file can grow after the size check, so the read stops one
    // byte past the limit.
    buf.try_reserve_exact(len as usize).map_err(|_| Error::Allocation(len as usize))?;
    file.take((max as u64).saturating_add(1)).read_to_end(&mut buf).map_err(|e| Error::io(path, &e))?;
    if buf.len() > max {
        return Err(too_large());
    }
    Ok(buf)
}

/// Checks every PNG against `limits`, returns each one's side in pixels with its index, sorted by
/// size, and rejects two images of one size. `check` checks each square image's `IHDR` fields and
/// side for the format; `format_max` is the most images the format can hold.
fn sorted_sizes<P: AsRef<[u8]>>(
    pngs: &[P],
    limits: &Limits,
    format_max: usize,
    check: impl Fn(&PngInfo) -> Result<(), Error>,
) -> Result<Vec<(u32, usize)>, Error> {
    if pngs.is_empty() {
        return Err(Error::Empty);
    }
    let max = limits.max_images.min(format_max);
    if pngs.len() > max {
        return Err(Error::TooManyImages { count: pngs.len(), max });
    }
    let mut sizes = Vec::with_capacity(pngs.len());
    for (i, png) in pngs.iter().enumerate() {
        let png = png.as_ref();
        if png.len() > limits.max_png_bytes {
            return Err(Error::TooLarge(format!("image {} holds {} bytes, more than {}", i + 1, png.len(), limits.max_png_bytes)));
        }
        let info = png_info(png)?;
        if info.width != info.height {
            return Err(Error::NotSquare { width: info.width, height: info.height });
        }
        check(&info)?;
        sizes.push((info.width, i));
    }
    sizes.sort_unstable();
    if let Some(w) = sizes.windows(2).find(|w| w[0].0 == w[1].0) {
        return Err(Error::DuplicateSize(w[0].0));
    }
    Ok(sizes)
}

/// Checks a packed file's size against `limits` and the formats' 32-bit fields, and reserves it.
fn reserve(total: u64, limits: &Limits) -> Result<Vec<u8>, Error> {
    if total > u64::from(u32::MAX) {
        return Err(Error::TooLarge(format!("the icon file would hold {total} bytes, more than 32-bit offsets reach")));
    }
    if total > limits.max_total_bytes as u64 {
        return Err(Error::TooLarge(format!("the icon file would hold {total} bytes, more than {}", limits.max_total_bytes)));
    }
    let total = total as usize;
    let mut out = Vec::new();
    out.try_reserve_exact(total).map_err(|_| Error::Allocation(total))?;
    Ok(out)
}

/// A big-endian u32 from the first four bytes of `b`, which holds at least four.
fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// The CRC-32 table of the PNG specification (polynomial 0xedb88320).
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
};

/// The CRC-32 that a PNG chunk stores over its type and data.
fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(u32::MAX, |c, &b| CRC_TABLE[((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A complete PNG of `width` x `height` transparent pixels, with its pixel data in stored
    /// (uncompressed) deflate blocks.
    pub(crate) fn png_of(width: u32, height: u32, bit_depth: u8, color_type: u8) -> Vec<u8> {
        let channels: usize = match color_type {
            2 => 3,
            4 => 2,
            6 => 4,
            _ => 1,
        };
        let row = (width as usize * channels * usize::from(bit_depth)).div_ceil(8) + 1;
        let raw = vec![0u8; row * height as usize];
        let mut z = vec![0x78, 0x01];
        let mut blocks = raw.chunks(0xffff).peekable();
        while let Some(block) = blocks.next() {
            let n = block.len() as u16;
            z.push(u8::from(blocks.peek().is_none()));
            z.extend_from_slice(&n.to_le_bytes());
            z.extend_from_slice(&(!n).to_le_bytes());
            z.extend_from_slice(block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &x in &raw {
            a = (a + u32::from(x)) % 65521;
            b = (b + a) % 65521;
        }
        z.extend_from_slice(&((b << 16) | a).to_be_bytes());
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[bit_depth, color_type, 0, 0, 0]);
        let mut png = PNG_SIGNATURE.to_vec();
        let mut chunks = vec![(*b"IHDR", ihdr)];
        if color_type == 3 {
            chunks.push((*b"PLTE", vec![0, 0, 0]));
        }
        chunks.push((*b"IDAT", z));
        chunks.push((*b"IEND", Vec::new()));
        for (kind, data) in chunks {
            push_chunk(&mut png, &kind, &data);
        }
        png
    }

    /// A square 8-bit RGBA PNG of `size` px.
    pub(crate) fn rgba(size: u32) -> Vec<u8> {
        png_of(size, size, 8, 6)
    }

    pub(crate) fn push_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        let crc = crc32(&png[start..]);
        png.extend_from_slice(&crc.to_be_bytes());
    }

    /// A 1x1 transparent RGBA PNG written by an independent encoder (Python's `struct`, `zlib.crc32`
    /// and `zlib.compress` at level 0) from the layout in the W3C PNG specification.
    const ONE_PIXEL_PNG: [u8; 73] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41, 0x54, 0x78, 0x01, 0x01, 0x05, 0x00,
        0xfa, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x01, 0x64, 0x78, 0x95, 0x38, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44,
        0xae, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn crc_matches_the_standard_check_values() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
    }

    #[test]
    fn fixtures_match_an_independent_encoder() {
        assert_eq!(rgba(1), ONE_PIXEL_PNG);
        assert_eq!(png_info(&ONE_PIXEL_PNG), Ok(PngInfo { width: 1, height: 1, bit_depth: 8, color_type: 6 }));
    }

    #[test]
    fn rejects_non_png() {
        assert!(matches!(png_size(b"GIF89a"), Err(Error::InvalidPng(_))));
        assert!(matches!(png_size(&[]), Err(Error::InvalidPng(_))));
        assert_eq!(png_size(&rgba(48)), Ok((48, 48)));
        assert_eq!(png_size(&png_of(3, 5, 1, 3)), Ok((3, 5)));
    }

    #[test]
    fn rejects_a_header_only_png() {
        // The 33-byte prefix the app copies' tests used: signature and IHDR, without the IHDR's CRC,
        // an IDAT or an IEND.
        let full = rgba(16);
        for cut in [24, 29, 33, full.len() - 12, full.len() - 1] {
            assert!(matches!(png_info(&full[..cut]), Err(Error::InvalidPng(_))), "accepted {cut} of {} bytes", full.len());
        }
    }

    #[test]
    fn rejects_a_wrong_ihdr_length() {
        let mut png = PNG_SIGNATURE.to_vec();
        push_chunk(&mut png, b"IHDR", &[0, 0, 0, 16, 0, 0, 0, 16, 8, 6, 0, 0]);
        push_chunk(&mut png, b"IDAT", &[0]);
        push_chunk(&mut png, b"IEND", &[]);
        assert_eq!(png_info(&png), Err(Error::InvalidPng("IHDR chunk is not 13 bytes".into())));
    }

    #[test]
    fn rejects_a_bad_crc_and_trailing_bytes() {
        let mut png = rgba(16);
        png[29] ^= 1; // the IHDR's CRC
        assert!(matches!(png_info(&png), Err(Error::InvalidPng(m)) if m.contains("CRC")));
        let mut png = rgba(16);
        png.push(0);
        assert_eq!(png_info(&png), Err(Error::InvalidPng("bytes after the IEND chunk".into())));
    }

    #[test]
    fn rejects_bad_ihdr_fields() {
        let mut zero = PNG_SIGNATURE.to_vec();
        push_chunk(&mut zero, b"IHDR", &[0, 0, 0, 0, 0, 0, 0, 1, 8, 6, 0, 0, 0]);
        assert!(matches!(png_info(&zero), Err(Error::InvalidPng(_))));
        for (depth, color) in [(8, 1), (4, 6), (16, 3), (8, 7)] {
            let mut png = PNG_SIGNATURE.to_vec();
            push_chunk(&mut png, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, depth, color, 0, 0, 0]);
            assert!(matches!(png_info(&png), Err(Error::InvalidPng(_))), "accepted depth {depth} color {color}");
        }
        let mut no_plte = PNG_SIGNATURE.to_vec();
        push_chunk(&mut no_plte, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 3, 0, 0, 0]);
        push_chunk(&mut no_plte, b"IDAT", &[0]);
        push_chunk(&mut no_plte, b"IEND", &[]);
        assert!(matches!(png_info(&no_plte), Err(Error::InvalidPng(m)) if m.contains("PLTE")));
    }

    #[test]
    fn rejects_chunks_out_of_order() {
        let build = |ihdr: [u8; 13], chunks: &[(&[u8; 4], &[u8])]| {
            let mut png = PNG_SIGNATURE.to_vec();
            push_chunk(&mut png, b"IHDR", &ihdr);
            for (kind, data) in chunks {
                push_chunk(&mut png, kind, data);
            }
            push_chunk(&mut png, b"IEND", &[]);
            png
        };
        let rgba = [0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0];
        let gray = [0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0];
        let palette = [0, 0, 0, 1, 0, 0, 0, 1, 8, 3, 0, 0, 0];
        assert!(png_info(&build(rgba, &[(b"IDAT", &[0]), (b"IDAT", &[0])])).is_ok());
        assert!(png_info(&build(palette, &[(b"PLTE", &[0, 0, 0]), (b"IDAT", &[0])])).is_ok());
        assert!(png_info(&build([0, 0, 0, 1, 0, 0, 0, 1, 1, 3, 0, 0, 0], &[(b"PLTE", &[0; 6]), (b"IDAT", &[0])])).is_ok());
        assert!(png_info(&build(rgba, &[(b"zzZz", &[]), (b"IDAT", &[0])])).is_ok());
        let bad = [
            build(rgba, &[(b"IDAT", &[0]), (b"tEXt", b"a\0b"), (b"IDAT", &[0])]),
            build(rgba, &[(b"IDAT", &[0]), (b"PLTE", &[0, 0, 0])]),
            build(rgba, &[(b"PLTE", &[0, 0, 0]), (b"PLTE", &[0, 0, 0]), (b"IDAT", &[0])]),
            build(rgba, &[(b"PLTE", &[0, 0]), (b"IDAT", &[0])]),
            build(gray, &[(b"PLTE", &[0, 0, 0]), (b"IDAT", &[0])]),
            build([0, 0, 0, 1, 0, 0, 0, 1, 1, 3, 0, 0, 0], &[(b"PLTE", &[0; 9]), (b"IDAT", &[0])]),
            build(rgba, &[(b"IDAT", &[0]), (b"ZZZZ", &[])]),
            build(rgba, &[(b"zzzz", &[]), (b"IDAT", &[0])]),
        ];
        for (n, png) in bad.iter().enumerate() {
            assert!(matches!(png_info(png), Err(Error::InvalidPng(_))), "accepted case {n}");
        }
    }

    #[test]
    fn read_pngs_stops_at_the_limits() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let (a, b) = (dir.path().join("a.png"), dir.path().join("b.png"));
        std::fs::write(&a, rgba(16)).expect("write a");
        std::fs::write(&b, rgba(32)).expect("write b");
        let limits = Limits::default();
        assert_eq!(read_pngs(&[&a, &b], &limits), Ok(vec![rgba(16), rgba(32)]));

        let small = Limits { max_png_bytes: rgba(16).len(), ..limits };
        assert!(matches!(read_pngs(&[&a, &b], &small), Err(Error::TooLarge(m)) if m.contains("b.png")));
        let one = Limits { max_images: 1, ..limits };
        assert_eq!(read_pngs(&[&a, &b], &one), Err(Error::TooManyImages { count: 2, max: 1 }));
        let total = Limits { max_total_bytes: rgba(16).len() + 1, ..limits };
        assert!(matches!(read_pngs(&[&a, &b], &total), Err(Error::TooLarge(m)) if m.contains("together")));
        let first = Limits { max_total_bytes: rgba(16).len() - 1, ..limits };
        assert!(matches!(read_pngs(&[&a], &first), Err(Error::TooLarge(m)) if m.contains("together")));
        assert!(matches!(read_pngs(&[dir.path().join("missing.png")], &limits), Err(Error::Io { .. })));
    }

    #[test]
    fn reserve_rejects_sizes_past_32_bits_and_the_limit() {
        let limits = Limits { max_total_bytes: usize::MAX, ..Limits::default() };
        assert!(matches!(reserve(u64::from(u32::MAX) + 1, &limits), Err(Error::TooLarge(_))));
        assert!(matches!(reserve(101, &Limits { max_total_bytes: 100, ..limits }), Err(Error::TooLarge(_))));
        assert_eq!(reserve(100, &Limits { max_total_bytes: 100, ..limits }).map(|v| v.capacity() >= 100), Ok(true));
    }
}
