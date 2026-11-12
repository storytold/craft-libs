//! Krita's tiled pixel files and the LZF compression inside them.
//!
//! A tiled file is a short text header (`VERSION 2`, `TILEWIDTH`, `TILEHEIGHT`, `PIXELSIZE`,
//! `DATA <tiles>`) followed by, per tile, a `x,y,LZF,<bytes>` line and that many bytes: a flag
//! (1 = LZF-compressed, 0 = raw) and the tile's pixels. LZF tiles hold byte planes (every pixel's
//! first byte, then every pixel's second byte, ...); raw tiles are interleaved.

use crate::{Error, MAX_SIDE, Result, invalid};

/// The pixel layouts this crate knows, by Krita colour-space id. Samples are straight
/// (unpremultiplied).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    /// `RGBA`: B, G, R, A bytes.
    Rgba8,
    /// `RGBA16`: B, G, R, A as little-endian u16.
    Rgba16,
    /// `GRAYA`: gray, alpha bytes.
    Gray8,
    /// `GRAYAU16`: gray, alpha as little-endian u16.
    Gray16,
    /// Masks and selections: one byte.
    Alpha8,
}

impl Space {
    /// The layout of a `colorspacename` attribute; `None` for a space this crate does not read.
    /// Masks have no such attribute: their files are [`Space::Alpha8`].
    pub fn from_name(name: &str) -> Option<Space> {
        Some(match name {
            "RGBA" => Space::Rgba8,
            "RGBA16" => Space::Rgba16,
            "GRAYA" => Space::Gray8,
            "GRAYAU16" => Space::Gray16,
            _ => return None,
        })
    }
    /// Bytes of one pixel in a tile file.
    pub fn pixel_size(self) -> usize {
        match self {
            Space::Rgba8 => 4,
            Space::Rgba16 => 8,
            Space::Gray8 => 2,
            Space::Gray16 => 4,
            Space::Alpha8 => 1,
        }
    }
    /// Channels per pixel: 4 (RGBA), 2 (gray, alpha) or 1 (alpha).
    pub fn channels(self) -> usize {
        match self {
            Space::Rgba8 | Space::Rgba16 => 4,
            Space::Gray8 | Space::Gray16 => 2,
            Space::Alpha8 => 1,
        }
    }
    /// Bytes per sample: 1 or 2.
    pub fn bytes_per_sample(self) -> usize {
        match self {
            Space::Rgba16 | Space::Gray16 => 2,
            _ => 1,
        }
    }
    /// One interleaved Krita pixel as straight RGBA (0..1). Gray is repeated over R, G and B; a
    /// mask byte is the gray value with alpha 1.
    pub fn rgba(self, px: &[u8]) -> [f32; 4] {
        let b = |i: usize| f32::from(px.get(i).copied().unwrap_or(0)) / 255.0;
        let w = |i: usize| f32::from(u16::from_le_bytes([px.get(i).copied().unwrap_or(0), px.get(i + 1).copied().unwrap_or(0)])) / 65535.0;
        match self {
            Space::Rgba8 => [b(2), b(1), b(0), b(3)],
            Space::Rgba16 => [w(4), w(2), w(0), w(6)],
            Space::Gray8 => [b(0), b(0), b(0), b(1)],
            Space::Gray16 => [w(0), w(0), w(0), w(2)],
            Space::Alpha8 => [b(0), b(0), b(0), 1.0],
        }
    }
    /// Interleaved Krita pixels as interleaved samples in channel order (R, G, B, A; gray, alpha;
    /// or alpha), 16-bit samples in native byte order. `out` is cleared first.
    pub fn samples(self, src: &[u8], out: &mut Vec<u8>) {
        out.clear();
        for px in src.chunks_exact(self.pixel_size()) {
            let word = |i: usize| u16::from_le_bytes([px.get(i).copied().unwrap_or(0), px.get(i + 1).copied().unwrap_or(0)]).to_ne_bytes();
            match self {
                Space::Rgba8 => out.extend([2, 1, 0, 3].map(|i| px.get(i).copied().unwrap_or(0))),
                Space::Rgba16 => [4, 2, 0, 6].into_iter().for_each(|i| out.extend(word(i))),
                Space::Gray16 => [0, 2].into_iter().for_each(|i| out.extend(word(i))),
                Space::Gray8 | Space::Alpha8 => out.extend_from_slice(px),
            }
        }
    }
}

/// LZF decompression (the liblzf format) into exactly `expected` bytes.
pub fn lzf_decompress(src: &[u8], expected: usize) -> Result<Vec<u8>> {
    let bad = || invalid("corrupt LZF tile data");
    let mut out = Vec::with_capacity(expected);
    let mut ip = 0;
    while ip < src.len() {
        let ctrl = usize::from(*src.get(ip).ok_or_else(bad)?);
        ip += 1;
        if ctrl < 32 {
            let len = ctrl + 1;
            let lit = src.get(ip..ip + len).ok_or_else(bad)?;
            if out.len() + len > expected {
                return Err(bad());
            }
            out.extend_from_slice(lit);
            ip += len;
        } else {
            let mut len = ctrl >> 5;
            if len == 7 {
                len += usize::from(*src.get(ip).ok_or_else(bad)?);
                ip += 1;
            }
            len += 2;
            let back = ((ctrl & 0x1f) << 8) + usize::from(*src.get(ip).ok_or_else(bad)?) + 1;
            ip += 1;
            let start = out.len().checked_sub(back).ok_or_else(bad)?;
            if out.len() + len > expected {
                return Err(bad());
            }
            // Back-references may overlap the bytes they produce, so copy one at a time.
            for i in start..start + len {
                let b = *out.get(i).ok_or_else(bad)?;
                out.push(b);
            }
        }
    }
    if out.len() != expected {
        return Err(invalid(format!("LZF tile data decodes to {} bytes, expected {expected}", out.len())));
    }
    Ok(out)
}

/// One tile of a [`TileFile`]: where it sits and its stored bytes.
#[derive(Clone, Copy, Debug)]
pub struct Tile<'a> {
    /// The tile's left edge, in the pixel file's own coordinates (before any node offset).
    pub x: i32,
    /// The tile's top edge.
    pub y: i32,
    data: &'a [u8],
}

/// A parsed tiled pixel file (`VERSION 2`), borrowing the tile payloads.
#[derive(Debug)]
pub struct TileFile<'a> {
    /// Tile width in pixels, 1 to 1024.
    pub width: usize,
    /// Tile height in pixels, 1 to 1024.
    pub height: usize,
    /// Bytes per pixel, 1 to 64.
    pub pixel_size: usize,
    tiles: Vec<Tile<'a>>,
}

fn line<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a str> {
    let rest = bytes.get(*pos..).ok_or_else(|| invalid("truncated tile file"))?;
    let end = rest.iter().take(256).position(|b| *b == b'\n').ok_or_else(|| invalid("truncated or malformed tile file header"))?;
    *pos += end + 1;
    std::str::from_utf8(rest.get(..end).unwrap_or_default()).map_err(|_| invalid("tile file header is not text"))
}

fn header_value(bytes: &[u8], pos: &mut usize, key: &str, max: usize) -> Result<usize> {
    let l = line(bytes, pos)?;
    let v = l.strip_prefix(key).map(str::trim).ok_or_else(|| invalid(format!("tile file: expected {key}, found `{l}`")))?;
    v.parse::<usize>().ok().filter(|v| (1..=max).contains(v)).ok_or_else(|| invalid(format!("tile file: invalid {key} `{v}`")))
}

/// Tile records reserved before any is read; the table grows as records are found.
const FIRST_RESERVATION: usize = 1 << 16;

impl<'a> TileFile<'a> {
    /// Reads the header and the tile table. No pixel is decoded.
    pub fn parse(bytes: &'a [u8]) -> Result<TileFile<'a>> {
        let mut pos = 0;
        let version = line(bytes, &mut pos)?;
        if version.trim() != "VERSION 2" {
            return Err(Error::Unsupported(format!("Krita tile file `{}` (only VERSION 2, Krita 2.0 and later)", version.trim())));
        }
        let width = header_value(bytes, &mut pos, "TILEWIDTH", 1024)?;
        let height = header_value(bytes, &mut pos, "TILEHEIGHT", 1024)?;
        let pixel_size = header_value(bytes, &mut pos, "PIXELSIZE", 64)?;
        let l = line(bytes, &mut pos)?;
        let count = l
            .strip_prefix("DATA")
            .map(str::trim)
            .and_then(|v| v.parse::<usize>().ok())
            .ok_or_else(|| invalid(format!("tile file: invalid DATA line `{l}`")))?;
        // Every tile needs at least a header line and a flag byte.
        if count > bytes.len() / 8 + 1 {
            return Err(invalid("tile count exceeds the file size"));
        }
        // Not `count` up front: a compressed member can declare far more records than it holds.
        let mut tiles = Vec::with_capacity(count.min(FIRST_RESERVATION));
        for _ in 0..count {
            let l = line(bytes, &mut pos)?;
            let mut f = l.split(',');
            let (x, y, method, len) = (f.next(), f.next(), f.next(), f.next());
            let coord = |v: Option<&str>| v.and_then(|v| v.trim().parse::<i32>().ok()).filter(|v| v.unsigned_abs() <= MAX_SIDE * 4);
            let (Some(x), Some(y), Some("LZF"), Some(len), None) =
                (coord(x), coord(y), method.map(str::trim), len.and_then(|v| v.trim().parse::<usize>().ok()), f.next())
            else {
                return Err(invalid(format!("tile file: malformed tile header `{l}`")));
            };
            let data =
                bytes.get(pos..pos.checked_add(len).ok_or_else(|| invalid("tile size overflow"))?).ok_or_else(|| invalid("truncated tile data"))?;
            pos += len;
            if tiles.len() == tiles.capacity() {
                // Grow by at most what is still declared, so the vector never overshoots `count`.
                tiles.try_reserve_exact(tiles.len().max(1).min(count - tiles.len())).map_err(|_| invalid("tile table allocation"))?;
            }
            tiles.push(Tile { x, y, data });
        }
        Ok(TileFile { width, height, pixel_size, tiles })
    }

    /// The tiles, in file order. Their coordinates are at most 4 × [`MAX_SIDE`] from the origin.
    pub fn tiles(&self) -> &[Tile<'a>] {
        &self.tiles
    }

    /// Bytes of one decoded tile: `width × height × pixel_size`, at most 64 MiB.
    pub fn tile_bytes(&self) -> usize {
        self.width * self.height * self.pixel_size
    }

    /// Decodes `tile` into `out` as interleaved pixels in the file's own layout
    /// ([`Space::rgba`] and [`Space::samples`] read them). `out` is resized to
    /// [`Self::tile_bytes`] and may be reused between tiles.
    pub fn decode(&self, tile: &Tile<'_>, out: &mut Vec<u8>) -> Result<()> {
        let n = self.width * self.height;
        let expected = n * self.pixel_size;
        let (flag, payload) = tile.data.split_first().ok_or_else(|| invalid("empty tile"))?;
        match flag {
            1 => {
                // LZF tiles hold byte planes: byte k of pixel i is at k * n + i.
                let planar = lzf_decompress(payload, expected)?;
                out.clear();
                out.resize(expected, 0);
                for (k, plane) in planar.chunks_exact(n).enumerate() {
                    for (i, b) in plane.iter().enumerate() {
                        if let Some(slot) = out.get_mut(i * self.pixel_size + k) {
                            *slot = *b;
                        }
                    }
                }
            }
            // Raw tiles (written when LZF doesn't shrink them) are already interleaved.
            0 if payload.len() == expected => {
                out.clear();
                out.extend_from_slice(payload);
            }
            0 => return Err(invalid("raw tile has the wrong size")),
            f => return Err(invalid(format!("unknown tile compression flag {f}"))),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file may declare one record per eight bytes (134 million in a 1 GiB member, 3 GiB of
    /// records) and hold none of them: the table is reserved as records are read.
    #[test]
    fn the_tile_table_is_not_reserved_from_the_declared_count() {
        let mut bytes = b"VERSION 2\nTILEWIDTH 4\nTILEHEIGHT 4\nPIXELSIZE 1\nDATA 200000\n".to_vec();
        bytes.resize(8 * 200_000, b' ');
        assert_eq!(TileFile::parse(&bytes).unwrap_err(), invalid("truncated or malformed tile file header"));
        // 70 000 real tiles: more than the first reservation, and no more capacity than tiles.
        let mut bytes = b"VERSION 2\nTILEWIDTH 4\nTILEHEIGHT 4\nPIXELSIZE 1\nDATA 70000\n".to_vec();
        for i in 0..70_000 {
            bytes.extend_from_slice(format!("{i},0,LZF,1\n\x07").as_bytes());
        }
        let file = TileFile::parse(&bytes).unwrap();
        assert_eq!((file.tiles.len(), file.tiles.capacity()), (70_000, 70_000));
        assert!(FIRST_RESERVATION * size_of::<Tile<'_>>() <= 2 << 20);
    }
}
