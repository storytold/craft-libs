//! Pixar image (`.pxr`) reader — the 1986 Pixar Image Computer format. Written from the
//! published layout: a 512-byte little-endian header (magic `80 E8 00 00`, version, picture and
//! tile dimensions, a channel-presence bitmask), a table of (address, length) tile pointers, and
//! uncompressed 8-bit tiles. Read-only.
//!
//! Supported: one to four channels (a colour bitmask of R/G/B, optionally A, else a single grey
//! channel) at 8 bits, in one tile or a tile grid. Anything malformed (bad magic, a tile that
//! runs past the file, a tile shorter than its pixels) errors rather than panicking.

use crate::error::CodecError;
use crate::format::Format;
use crate::image::{ChannelLayout, Image, SampleType};
use crate::options::Limits;

const F: Format = Format::Pixar;
const HEADER: usize = 512;
/// Widest tile grid we accept (guards a hostile header against a huge pointer table).
const MAX_TILES_PER_AXIS: u32 = 4096;

fn bad(msg: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, msg)
}

fn unsupported(msg: impl Into<String>) -> CodecError {
    CodecError::unsupported(F, msg)
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_le_bytes(s))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_le_bytes(s))
}

/// The Pixar magic number (`80 E8 00 00`).
pub(crate) fn looks_like_pixar(b: &[u8]) -> bool {
    b.get(0..4) == Some(&[0x80, 0xE8, 0x00, 0x00])
}

pub(crate) fn decode(b: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    if !looks_like_pixar(b) {
        return Err(bad("not a Pixar file (missing the 80 E8 00 00 magic)"));
    }
    if b.len() < HEADER {
        return Err(bad("the header is truncated"));
    }
    // Version 0 is the channel-bitmask layout the synthetic tests write. Version 1 stores a
    // picture type instead. Published notes describe the file as an uncompressed 1986 raster,
    // grey or RGB at 8 bits, beginning 80 E8 (Wolfram Language PXR reference; Just Solve the
    // File Format Problem, "Pixar picture"). They do not give these field positions. The positions
    // are the layout this reader implements, and the tests write it. No third-party sample is committed.
    let version = le16(b, 4).ok_or_else(|| bad("format version"))?;
    if version > 1 {
        return Err(unsupported("an unknown Pixar format version"));
    }
    // height 416, width 418, tile size 420/422, storage 424, sample encoding 426.
    let height = u32::from(le16(b, 416).ok_or_else(|| bad("picture height"))?);
    let width = u32::from(le16(b, 418).ok_or_else(|| bad("picture width"))?);
    let tile_h = u32::from(le16(b, 420).ok_or_else(|| bad("tile height"))?);
    let tile_w = u32::from(le16(b, 422).ok_or_else(|| bad("tile width"))?);
    let storage = le16(b, 424).ok_or_else(|| bad("picture format"))?;
    let depth = le16(b, 426).unwrap_or(0);
    if width == 0 || height == 0 || tile_w == 0 || tile_h == 0 {
        return Err(bad("zero-sized picture or tile"));
    }
    // Version 1 stores a picture type (8 = grey, 14 = RGB) with sample encoding 2 (8-bit).
    // Version 0 uses a channel bitmask in the low nibble: R=1, G=2, B=4, A=8.
    let (layout, color) = match (storage, depth) {
        (8, 2) => (ChannelLayout::Gray, 1),
        (14, 2) => (ChannelLayout::Rgb, 3),
        (s, _) => match (s & 0x7, s & 0x8 != 0) {
            (0x7, false) => (ChannelLayout::Rgb, 3),
            (0x7, true) => (ChannelLayout::Rgba, 4),
            (0x0, false) => (ChannelLayout::Gray, 1),
            (0x0, true) => (ChannelLayout::GrayA, 2),
            (rgb, _) => return Err(unsupported(format!("channel bitmask {rgb:#x} (colour needs R, G and B)"))),
        },
    };
    limits.check(width, height, layout, SampleType::U8)?;

    let tiles_x = (width - 1) / tile_w + 1;
    let tiles_y = (height - 1) / tile_h + 1;
    if tiles_x > MAX_TILES_PER_AXIS || tiles_y > MAX_TILES_PER_AXIS {
        return Err(unsupported("more tiles than the reader accepts"));
    }
    let tiles = tiles_x.checked_mul(tiles_y).ok_or_else(|| CodecError::LimitExceeded("tile count overflow".into()))?;
    let table_end = HEADER + (tiles as usize).checked_mul(8).ok_or_else(|| CodecError::LimitExceeded("tile table overflow".into()))?;
    if b.len() < table_end {
        return Err(bad("the tile pointer table is truncated"));
    }

    let (w, h) = (width as usize, height as usize);
    let row = w.checked_mul(color).ok_or_else(|| CodecError::LimitExceeded("row too long".into()))?;
    let mut out = Vec::new();
    out.try_reserve_exact(row.checked_mul(h).ok_or_else(|| CodecError::LimitExceeded("picture too large".into()))?)
        .map_err(|_| CodecError::LimitExceeded("cannot allocate the image".into()))?;
    out.resize(row * h, 0);

    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let i = (ty * tiles_x + tx) as usize;
            let address = le32(b, HEADER + i * 8).ok_or_else(|| bad("tile address"))? as usize;
            let length = le32(b, HEADER + i * 8 + 4).ok_or_else(|| bad("tile length"))? as usize;
            let tile = b
                .get(address..address.checked_add(length).ok_or_else(|| CodecError::LimitExceeded("tile end overflow".into()))?)
                .ok_or_else(|| bad("a tile points past the end of the file"))?;
            let dw = tile_w.min(width - tx * tile_w) as usize;
            let dh = tile_h.min(height - ty * tile_h) as usize;
            let need = dw.checked_mul(dh).and_then(|n| n.checked_mul(color)).ok_or_else(|| CodecError::LimitExceeded("tile too large".into()))?;
            let pixels = tile.get(..need).ok_or_else(|| bad("a tile is shorter than its pixels"))?;
            let (ox, oy) = ((tx * tile_w) as usize, (ty * tile_h) as usize);
            for r in 0..dh {
                let dst = (oy + r) * row + ox * color;
                let src = r * dw * color;
                out.get_mut(dst..dst + dw * color).ok_or_else(|| bad("tile crosses the picture"))?.copy_from_slice(&pixels[src..src + dw * color]);
            }
        }
    }
    Image::from_u8(width, height, layout, out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// A little-endian Pixar file: one tile per `(tile_w × tile_h)` block, `format` the channel
    /// bitmask, `tiles` the raw tile payloads in tile order.
    fn pxr(width: u16, height: u16, tile_w: u16, tile_h: u16, format: u16, tiles: &[Vec<u8>]) -> Vec<u8> {
        let tx = (u32::from(width) - 1) / u32::from(tile_w) + 1;
        let ty = (u32::from(height) - 1) / u32::from(tile_h) + 1;
        assert_eq!((tx * ty) as usize, tiles.len());
        let table = tiles.len() * 8;
        let mut data_at = HEADER + table;
        let mut b = vec![0u8; HEADER + table];
        b[0..4].copy_from_slice(&[0x80, 0xE8, 0x00, 0x00]);
        b[416..418].copy_from_slice(&height.to_le_bytes());
        b[418..420].copy_from_slice(&width.to_le_bytes());
        b[420..422].copy_from_slice(&tile_h.to_le_bytes());
        b[422..424].copy_from_slice(&tile_w.to_le_bytes());
        b[424..426].copy_from_slice(&format.to_le_bytes());
        for (i, t) in tiles.iter().enumerate() {
            b[HEADER + i * 8..HEADER + i * 8 + 4].copy_from_slice(&(data_at as u32).to_le_bytes());
            b[HEADER + i * 8 + 4..HEADER + i * 8 + 8].copy_from_slice(&(t.len() as u32).to_le_bytes());
            data_at += t.len();
        }
        for t in tiles {
            b.extend_from_slice(t);
        }
        b
    }

    /// Version 1: picture type in `storage` (8 grey, 14 RGB) and sample encoding 2 (8-bit).
    fn pxr_v1(width: u16, height: u16, tile_w: u16, tile_h: u16, storage: u16, tiles: &[Vec<u8>]) -> Vec<u8> {
        let mut b = pxr(width, height, tile_w, tile_h, storage, tiles);
        b[4..6].copy_from_slice(&1u16.to_le_bytes());
        b[426..428].copy_from_slice(&2u16.to_le_bytes());
        b
    }

    #[test]
    fn reads_a_single_tile_rgb() {
        // 2×1 RGB: red then green.
        let b = pxr(2, 1, 2, 1, 0x7, &[vec![255, 0, 0, 0, 255, 0]]);
        assert!(looks_like_pixar(&b));
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!((img.width(), img.height(), img.layout()), (2, 1, ChannelLayout::Rgb));
        assert_eq!(img.data(), &[255, 0, 0, 0, 255, 0]);
    }

    #[test]
    fn reads_a_two_by_two_tile_grid() {
        // 4×2 RGBA in 2×1 tiles → 2 tiles across, 2 down = 4 tiles.
        let t = |v: u8| vec![v, v, v, 255, v, v, v, 255]; // 2 pixels RGBA
        let b = pxr(4, 2, 2, 1, 0xF, &[t(10), t(20), t(30), t(40)]);
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!((img.width(), img.height(), img.layout()), (4, 2, ChannelLayout::Rgba));
        let d = img.data();
        assert_eq!(&d[0..8], &[10, 10, 10, 255, 10, 10, 10, 255]);
        assert_eq!(&d[8..16], &[20, 20, 20, 255, 20, 20, 20, 255]);
        assert_eq!(&d[16..24], &[30, 30, 30, 255, 30, 30, 30, 255]); // second row
    }

    #[test]
    fn reads_a_single_channel_grey() {
        let b = pxr(3, 1, 3, 1, 0x0, &[vec![0, 128, 255]]);
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!((img.layout(), img.data()), (ChannelLayout::Gray, &[0u8, 128, 255][..]));
    }

    #[test]
    fn hostile_input_errors_not_panics() {
        let good = pxr(2, 1, 2, 1, 0x7, &[vec![1, 2, 3, 4, 5, 6]]);
        for cut in [0, 4, 100, 512, good.len() - 1] {
            assert!(decode(&good[..cut.min(good.len())], &Limits::default()).is_err(), "cut {cut}");
        }
        // An unsupported channel bitmask (only R) is Unsupported.
        let bad_mask = pxr(1, 1, 1, 1, 0x1, &[vec![1]]);
        assert!(matches!(decode(&bad_mask, &Limits::default()), Err(CodecError::Unsupported { .. })));
        // A tile whose length is shorter than its pixels errors.
        let mut short = pxr(2, 1, 2, 1, 0x7, &[vec![1, 2, 3]]);
        let n = short.len();
        short[n - 3..].fill(0);
        short[HEADER + 4..HEADER + 8].copy_from_slice(&1u32.to_le_bytes()); // lie: length 1
        assert!(decode(&short, &Limits::default()).is_err());
    }

    #[test]
    fn reads_version1_grey_and_rgb_tiles() {
        let grey = pxr_v1(3, 1, 3, 1, 8, &[vec![0, 128, 255]]);
        let img = decode(&grey, &Limits::default()).unwrap();
        assert_eq!((img.layout(), img.data()), (ChannelLayout::Gray, &[0u8, 128, 255][..]));

        let rgb = pxr_v1(2, 1, 2, 1, 14, &[vec![255, 0, 0, 0, 255, 0]]);
        let img = decode(&rgb, &Limits::default()).unwrap();
        assert_eq!((img.width(), img.height(), img.layout()), (2, 1, ChannelLayout::Rgb));
        assert_eq!(img.data(), &[255, 0, 0, 0, 255, 0]);
    }
}
