#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Truevision Targa (`.tga`), read and written from the Truevision TGA File Format Specification 2.0.
//!
//! Reading ([`decode`]): image types 1, 2 and 3 (colour-mapped, true-colour, grayscale) and their
//! run-length encoded forms 9, 10 and 11; 15- and 16-bit (5-5-5, the top bit as alpha when the
//! descriptor says one attribute bit), 24-bit BGR and 32-bit BGRA true-colour, 8-bit grayscale and
//! 16-bit grayscale with alpha, colour maps of 15-, 16-, 24- or 32-bit entries indexed by 8 or 16
//! bits; any origin corner. Run-length packets may cross scan lines (older writers do it). The
//! result is straight-alpha RGBA8, top row first, with [`Picture::layout`] naming what the file
//! stored.
//!
//! Writing ([`encode`]): 8-bit grayscale, grayscale with alpha, RGB or RGBA, run-length encoded
//! (types 10 and 11), rows stored bottom-up (the bottom-left origin every reader assumes, also
//! those that ignore the origin bits), alpha bits in the descriptor and the TGA 2.0 footer. A
//! packet never crosses a scan line: TGA 2.0 forbids it, and readers that decode a row at a time
//! misread files that do.
//!
//! Input is treated as hostile: the picture size is capped by [`Limits`], and a run-length file
//! must be long enough to hold the pixels its header declares before anything is allocated.

#[cfg(test)]
mod tests;

/// Length of the fixed header.
const HEADER: usize = 18;
/// TGA 2.0 files end with this signature (after two 32-bit area offsets).
const SIGNATURE: &[u8] = b"TRUEVISION-XFILE.\0";
/// The TGA 2.0 footer written: no extension or developer area, then the signature.
const FOOTER: &[u8; 26] = b"\0\0\0\0\0\0\0\0TRUEVISION-XFILE.\0";
/// Most pixels one packet holds (a 7-bit count, plus one).
const MAX_PACKET: usize = 128;

/// Why a file could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The header or the pixel data is damaged or cut short.
    #[error("Targa: {0}")]
    Malformed(&'static str),
    /// A Targa image type this reader does not cover.
    #[error("unsupported Targa: image type {0}")]
    Unsupported(u8),
    /// The file declares more than [`Limits`] allow.
    #[error("Targa exceeds a limit: {0}")]
    LimitExceeded(&'static str),
    /// The picture handed to the writer cannot be stored.
    #[error("Targa: cannot write: {0}")]
    Encode(String),
}

/// Result of this crate's functions.
pub type Result<T> = std::result::Result<T, Error>;

/// Caps on what a header may ask the reader to allocate, checked before anything is allocated.
/// `#[non_exhaustive]`: start from [`Limits::default`] and set the fields you change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Most pixels (width × height) a picture may have. Default 2^28 (16384 × 16384).
    pub max_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_pixels: 1 << 28 }
    }
}

/// What a picture's pixels are: what a file stored, or what the writer is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// One byte a pixel.
    Gray,
    /// Two bytes a pixel: gray, alpha.
    GrayAlpha,
    /// Three bytes a pixel.
    Rgb,
    /// Four bytes a pixel, straight alpha.
    Rgba,
}

impl Layout {
    /// Bytes of one pixel.
    pub fn channels(self) -> usize {
        match self {
            Layout::Gray => 1,
            Layout::GrayAlpha => 2,
            Layout::Rgb => 3,
            Layout::Rgba => 4,
        }
    }
}

/// A decoded picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// Straight-alpha RGBA8, top row first, whatever the file stored.
    pub rgba: Vec<u8>,
    /// What the file stored (a colour-mapped file reports its map entries' layout).
    pub layout: Layout,
    /// Whether any pixel is not fully opaque.
    pub has_alpha: bool,
}

/// Whether `bytes` looks like a Targa file: a plausible header and either a TGA 2.0 footer or,
/// for the original format (which has no signature), a Targa file extension on `name`.
pub fn sniff(name: &str, bytes: &[u8]) -> bool {
    let header_ok = || {
        let Some(h) = bytes.first_chunk::<HEADER>() else { return false };
        let (map_type, kind, depth) = (h[1], h[2], h[16]);
        let (w, ht) = (u16::from_le_bytes([h[12], h[13]]), u16::from_le_bytes([h[14], h[15]]));
        map_type <= 1 && matches!(kind, 1 | 2 | 3 | 9 | 10 | 11) && w > 0 && ht > 0 && matches!(depth, 8 | 15 | 16 | 24 | 32)
    };
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    let named = matches!(ext.as_deref(), Some("tga" | "targa" | "icb" | "vda" | "vst"));
    header_ok() && (named || bytes.ends_with(SIGNATURE))
}

/// Pixel layout of the stored values (after the colour-map lookup, for mapped images).
#[derive(Clone, Copy)]
enum Stored {
    /// 15/16-bit `ARRRRRGG GGGBBBBB`, little-endian; `alpha`: the top bit is an alpha bit.
    Rgb555 {
        alpha: bool,
    },
    Bgr24,
    Bgra32,
    Gray8,
    GrayAlpha16,
}

impl Stored {
    fn true_colour(depth: u8, attr_bits: u8) -> Option<Self> {
        Some(match depth {
            15 => Stored::Rgb555 { alpha: false },
            16 => Stored::Rgb555 { alpha: attr_bits == 1 },
            24 => Stored::Bgr24,
            32 => Stored::Bgra32,
            _ => return None,
        })
    }

    fn bytes(self) -> usize {
        match self {
            Stored::Rgb555 { .. } | Stored::GrayAlpha16 => 2,
            Stored::Bgr24 => 3,
            Stored::Bgra32 => 4,
            Stored::Gray8 => 1,
        }
    }

    fn layout(self) -> Layout {
        match self {
            Stored::Rgb555 { alpha: false } | Stored::Bgr24 => Layout::Rgb,
            Stored::Rgb555 { alpha: true } | Stored::Bgra32 => Layout::Rgba,
            Stored::Gray8 => Layout::Gray,
            Stored::GrayAlpha16 => Layout::GrayAlpha,
        }
    }

    /// RGBA of one stored value (`p` holds exactly [`bytes`](Self::bytes) bytes).
    fn rgba(self, p: &[u8]) -> [u8; 4] {
        let b = |i: usize| p.get(i).copied().unwrap_or(0);
        match self {
            Stored::Rgb555 { alpha } => {
                let v = u16::from_le_bytes([b(0), b(1)]);
                let five = |s: u16| {
                    let c = ((v >> s) & 31) as u8;
                    (c << 3) | (c >> 2)
                };
                let a = if !alpha || v & 0x8000 != 0 { 255 } else { 0 };
                [five(10), five(5), five(0), a]
            }
            Stored::Bgr24 => [b(2), b(1), b(0), 255],
            Stored::Bgra32 => [b(2), b(1), b(0), b(3)],
            Stored::Gray8 => [b(0), b(0), b(0), 255],
            Stored::GrayAlpha16 => [b(0), b(0), b(0), b(1)],
        }
    }
}

/// Decodes a Targa file to straight-alpha RGBA8, top row first.
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Picture> {
    let err = Error::Malformed;
    let h = bytes.first_chunk::<HEADER>().ok_or(err("truncated header"))?;
    let (id_len, map_type, kind) = (usize::from(h[0]), h[1], h[2]);
    let map_first = usize::from(u16::from_le_bytes([h[3], h[4]]));
    let map_len = usize::from(u16::from_le_bytes([h[5], h[6]]));
    let map_depth = h[7];
    let (w, ht) = (usize::from(u16::from_le_bytes([h[12], h[13]])), usize::from(u16::from_le_bytes([h[14], h[15]])));
    let (depth, descriptor) = (h[16], h[17]);
    let attr_bits = descriptor & 0x0F;
    let (right_to_left, top_to_bottom) = (descriptor & 0x10 != 0, descriptor & 0x20 != 0);
    if w == 0 || ht == 0 {
        return Err(err("empty picture"));
    }
    // Both are 16-bit, so the product fits.
    let pixels = w * ht;
    if pixels as u64 > limits.max_pixels {
        return Err(Error::LimitExceeded("pixel count"));
    }
    let rle = kind >= 9;
    // The stored value layout and, for colour-mapped images, the palette.
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let map_entry = if map_type == 1 {
        Some(Stored::true_colour(map_depth, if map_depth == 16 { attr_bits.min(1) } else { 0 }).ok_or(err("colour map entry size"))?)
    } else {
        None
    };
    let mut at = HEADER + id_len;
    if let Some(l) = map_entry {
        let end = at + map_len * l.bytes();
        let map = bytes.get(at..end).ok_or(err("truncated colour map"))?;
        palette = map.chunks_exact(l.bytes()).map(|e| l.rgba(e)).collect();
        at = end;
    }
    let (stored_as, mapped) = match kind & 7 {
        1 => {
            if map_type != 1 || !matches!(depth, 8 | 16) {
                return Err(err("colour-mapped picture without an 8/16-bit index and a colour map"));
            }
            (if depth == 8 { Stored::Gray8 } else { Stored::GrayAlpha16 }, true)
        }
        2 => (Stored::true_colour(depth, attr_bits).ok_or(err("true-colour pixel depth"))?, false),
        3 => match depth {
            8 => (Stored::Gray8, false),
            16 => (Stored::GrayAlpha16, false),
            _ => return Err(err("grayscale pixel depth")),
        },
        _ => return Err(Error::Unsupported(kind)),
    };
    let data = bytes.get(at..).unwrap_or_default();
    let bpp = stored_as.bytes();
    // The stored values, in file order.
    let stored: Vec<u8> = if rle {
        unpack_rle(data, pixels, bpp)?
    } else {
        let n = pixels.checked_mul(bpp).ok_or(Error::LimitExceeded("image size"))?;
        data.get(..n).ok_or(err("truncated pixel data"))?.to_vec()
    };
    let mut rgba = vec![0u8; pixels.checked_mul(4).ok_or(Error::LimitExceeded("image size"))?];
    for (i, v) in stored.chunks_exact(bpp).enumerate().take(pixels) {
        let (x, y) = (i % w, i / w);
        let x = if right_to_left { w - 1 - x } else { x };
        let y = if top_to_bottom { y } else { ht - 1 - y };
        let px = if mapped {
            let byte = |i: usize| v.get(i).copied().unwrap_or(0);
            let index = if bpp == 1 { usize::from(byte(0)) } else { usize::from(u16::from_le_bytes([byte(0), byte(1)])) };
            *index.checked_sub(map_first).and_then(|k| palette.get(k)).ok_or(err("colour index outside the colour map"))?
        } else {
            stored_as.rgba(v)
        };
        let o = (y * w + x) * 4;
        if let Some(d) = rgba.get_mut(o..o + 4) {
            d.copy_from_slice(&px);
        }
    }
    // 32-bit files from writers that leave the attribute bits unset often store zero alpha
    // everywhere: such a picture is opaque, not invisible.
    let alpha_used = match (stored_as, mapped) {
        (Stored::Bgra32, false) => attr_bits != 0 || rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 0),
        _ => true,
    };
    if !alpha_used {
        rgba.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
    }
    let has_alpha = rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 255);
    let layout = match (mapped, map_entry) {
        (true, Some(entry)) => entry.layout(),
        _ => stored_as.layout(),
    };
    Ok(Picture { width: w as u32, height: ht as u32, rgba, layout, has_alpha })
}

/// Expands run-length packets into `pixels` stored values of `bpp` bytes each. Packets may span
/// scan lines (the TGA 2.0 spec discourages it; older writers do it).
fn unpack_rle(mut data: &[u8], pixels: usize, bpp: usize) -> Result<Vec<u8>> {
    let short = Error::Malformed("truncated run-length data");
    // A two-byte packet expands to at most 128 values: a header asking for more is lying.
    let bound = data.len().saturating_mul(64).saturating_add(128);
    if pixels > bound {
        return Err(short);
    }
    let total = pixels.checked_mul(bpp).ok_or(Error::LimitExceeded("image size"))?;
    let mut out = Vec::with_capacity(total);
    while out.len() < total {
        let (&head, rest) = data.split_first().ok_or(short.clone())?;
        let count = usize::from(head & 0x7F) + 1;
        let want = (total - out.len()).min(count * bpp);
        if head & 0x80 != 0 {
            let v = rest.get(..bpp).ok_or(short.clone())?;
            for _ in 0..want / bpp {
                out.extend_from_slice(v);
            }
            data = rest.get(bpp..).unwrap_or_default();
        } else {
            let n = count * bpp;
            let v = rest.get(..n).ok_or(short.clone())?;
            out.extend_from_slice(v.get(..want).unwrap_or(v));
            data = rest.get(n..).unwrap_or_default();
        }
    }
    Ok(out)
}

/// Encodes 8-bit pixels (`height` rows of `width × layout.channels()` bytes, top row first) as a
/// run-length encoded Targa.
pub fn encode(width: u32, height: u32, layout: Layout, data: &[u8]) -> Result<Vec<u8>> {
    let (bpp, image_type, alpha_bits) = match layout {
        Layout::Gray => (1usize, 11u8, 0u8),
        Layout::GrayAlpha => (2, 11, 8),
        Layout::Rgb => (3, 10, 0),
        Layout::Rgba => (4, 10, 8),
    };
    let (Ok(w16), Ok(h16)) = (u16::try_from(width), u16::try_from(height)) else {
        return Err(Error::Encode(format!("dimensions exceed 65535x65535: {width}x{height}")));
    };
    let row = usize::from(w16) * bpp;
    if data.len() != row * usize::from(h16) {
        return Err(Error::Encode("buffer size mismatch".into()));
    }
    let mut out = Vec::with_capacity(HEADER + data.len() + data.len() / MAX_PACKET + FOOTER.len());
    out.extend_from_slice(&[0, 0, image_type, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.extend_from_slice(&w16.to_le_bytes());
    out.extend_from_slice(&h16.to_le_bytes());
    // Bits per pixel; the descriptor holds the alpha bits, origin bits 0 (bottom-left).
    out.extend_from_slice(&[(bpp * 8) as u8, alpha_bits]);
    let mut px = vec![0u8; row];
    // `row` is 0 only for an empty picture, which has no rows to write.
    for line in data.chunks_exact(row.max(1)).rev() {
        px.copy_from_slice(line);
        // Targa stores colour as BGR.
        if bpp >= 3 {
            for p in px.chunks_exact_mut(bpp) {
                p.swap(0, 2);
            }
        }
        rle_row(&px, bpp, &mut out);
    }
    out.extend_from_slice(FOOTER);
    Ok(out)
}

/// One scan line as run-length packets that end with it: a run of two or more equal pixels is a
/// run packet, anything else goes out in raw packets.
fn rle_row(row: &[u8], bpp: usize, out: &mut Vec<u8>) {
    let px: Vec<&[u8]> = row.chunks_exact(bpp).collect();
    let n = px.len();
    let at = |i: usize| px.get(i).copied().unwrap_or_default();
    let mut i = 0;
    while i < n {
        let mut run = 1;
        while i + run < n && run < MAX_PACKET && at(i + run) == at(i) {
            run += 1;
        }
        if run >= 2 {
            out.push(0x80 | (run - 1) as u8);
            out.extend_from_slice(at(i));
            i += run;
            continue;
        }
        // Raw until two equal pixels start a run, the packet is full or the row ends. The first
        // pixel always goes in: it differs from the one after it, or it is the row's last.
        let start = i;
        while i < n && i - start < MAX_PACKET && !(i > start && i + 1 < n && at(i) == at(i + 1)) {
            i += 1;
        }
        out.push((i - start - 1) as u8);
        for p in px.get(start..i).unwrap_or_default() {
            out.extend_from_slice(p);
        }
    }
}
