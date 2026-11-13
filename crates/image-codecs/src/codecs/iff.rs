//! Amiga IFF/ILBM reader and writer (EA "Interchange File Format", "InterLeaved BitMap" and the
//! chunky "PBM " variant). Written from the published IFF/ILBM layout: a `FORM` chunk of four-byte
//! chunks (`BMHD` bitmap header, `CMAP` palette, `CAMG` Amiga viewport mode, `BODY` pixel data).
//!
//! Read: 1–8 bitplanes with a palette (or a gray ramp when there is none) with masking 0/1/2, and
//! 24-bitplane "deep" ILBMs (8 planes each of red, green and blue, least significant bit first,
//! no palette) with masking 0/1, both stored (0) and ByteRun1/PackBits (1) compressed. Anything
//! else is a clear `Unsupported`/`Malformed` error, never a panic.
//!
//! Write: a 24-bitplane ILBM, ByteRun1 compressed one plane row at a time, with a mask plane
//! (masking 1) when the image has alpha. The mask plane is one bit per pixel: alpha of 128 or
//! more is opaque.

use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::format::Format;
use crate::image::{ChannelLayout, Image, SampleType};
use crate::options::Limits;

const F: Format = Format::Iff;

fn bad(msg: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, msg)
}

fn unsupported(msg: impl Into<String>) -> CodecError {
    CodecError::unsupported(F, msg)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_be_bytes(s))
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_be_bytes(s))
}

/// `FORM` + a bitmap type (`ILBM` interleaved, `PBM ` chunky) at offset 8.
pub(crate) fn looks_like_iff(b: &[u8]) -> bool {
    b.get(0..4) == Some(b"FORM") && matches!(b.get(8..12), Some(b"ILBM" | b"PBM "))
}

/// Bitplanes of a "deep" ILBM: red, green and blue, 8 each, least significant bit first.
const DEEP_PLANES: u8 = 24;

/// The bitmap header fields decoding needs.
struct Header {
    width: u32,
    height: u32,
    planes: u8,
    masking: u8,
    compression: u8,
    transparent: u16,
    /// `PBM ` stores one palette index per byte, padded to even row lengths.
    chunky: bool,
}

/// One four-byte chunk's id and byte range.
struct Chunk<'a> {
    id: [u8; 4],
    data: &'a [u8],
}

/// Walk the `FORM`'s chunks from offset 12 to `end` (the form length), stopping at the first
/// malformed length so a hostile file can't run past the buffer.
fn chunks(b: &[u8]) -> impl Iterator<Item = Chunk<'_>> {
    let end = be32(b, 4).map_or(b.len(), |n| (n as usize).saturating_add(8).min(b.len()));
    let mut pos = 12usize;
    std::iter::from_fn(move || {
        if pos + 8 > end {
            return None;
        }
        let id = b.get(pos..pos + 4)?.try_into().ok()?;
        let len = be32(b, pos + 4)? as usize;
        let start = pos + 8;
        let data = b.get(start..start.checked_add(len)?)?;
        // Chunks are padded to an even length.
        pos = start.checked_add(len)?.checked_add(len & 1)?;
        Some(Chunk { id, data })
    })
}

fn parse_header(b: &[u8], chunky: bool) -> Result<Header, CodecError> {
    let bmhd = chunks(b).find(|c| &c.id == b"BMHD").ok_or_else(|| bad("no BMHD chunk"))?;
    let d = bmhd.data;
    if d.len() < 20 {
        return Err(bad("BMHD chunk is too short"));
    }
    let width = u32::from(be16(d, 0).ok_or_else(|| bad("BMHD width"))?);
    let height = u32::from(be16(d, 2).ok_or_else(|| bad("BMHD height"))?);
    let planes = *d.get(8).ok_or_else(|| bad("BMHD planes"))?;
    let masking = *d.get(9).ok_or_else(|| bad("BMHD masking"))?;
    let compression = *d.get(10).ok_or_else(|| bad("BMHD compression"))?;
    if width == 0 || height == 0 {
        return Err(bad("zero-sized bitmap"));
    }
    if !(1..=8).contains(&planes) && planes != DEEP_PLANES {
        return Err(unsupported(format!("{planes} bitplanes (1–8 with a palette, or 24)")));
    }
    if masking > 2 {
        return Err(unsupported(format!("masking {masking}")));
    }
    if planes == DEEP_PLANES && masking == 2 {
        return Err(unsupported("a transparent color in a 24-bitplane ILBM"));
    }
    if compression > 1 {
        return Err(unsupported(format!("compression {compression} (only stored and ByteRun1)")));
    }
    if chunky && (planes != 8 || masking == 1) {
        return Err(unsupported("PBM requires 8-bit indices without a mask plane"));
    }
    let transparent = be16(d, 12).ok_or_else(|| bad("BMHD transparent colour"))?;
    Ok(Header { width, height, planes, masking, compression, transparent, chunky })
}

/// ByteRun1 / PackBits decode of one plane row into `out`; returns the bytes consumed. A `0x80`
/// control byte is a no-op, as the layout specifies.
fn unpack_byte_run1(src: &[u8], out: &mut [u8]) -> Result<usize, CodecError> {
    let (mut i, mut o) = (0usize, 0usize);
    while o < out.len() {
        let n = *src.get(i).ok_or_else(|| bad("truncated ByteRun1 data"))? as i8;
        i += 1;
        if n >= 0 {
            let count = n as usize + 1;
            let lit = src.get(i..i.checked_add(count).ok_or_else(|| bad("ByteRun1 overflow"))?).ok_or_else(|| bad("truncated ByteRun1 run"))?;
            if count > out.len() - o {
                return Err(bad("ByteRun1 run crosses the plane row"));
            }
            out[o..o + count].copy_from_slice(lit);
            i += count;
            o += count;
        } else if n != -128 {
            let count = (1 - n as i32) as usize;
            let value = *src.get(i).ok_or_else(|| bad("truncated ByteRun1 repeat"))?;
            i += 1;
            if count > out.len() - o {
                return Err(bad("ByteRun1 repeat crosses the plane row"));
            }
            out[o..o + count].fill(value);
            o += count;
        }
    }
    Ok(i)
}

pub(crate) fn decode(b: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    if !looks_like_iff(b) {
        return Err(bad("not an IFF/ILBM file (no FORM ... ILBM)"));
    }
    let h = parse_header(b, b.get(8..12) == Some(b"PBM "))?;
    if let Some(camg) = chunks(b).find(|c| &c.id == b"CAMG") {
        let mode = be32(camg.data, 0).ok_or_else(|| bad("short CAMG chunk"))?;
        if mode & (0x80 | 0x800) != 0 {
            return Err(unsupported("EHB and HAM viewport modes"));
        }
    }
    let layout = if h.masking == 0 { ChannelLayout::Rgb } else { ChannelLayout::Rgba };
    limits.check(h.width, h.height, layout, SampleType::U8)?;

    // Palette: three bytes per colour; missing entries read as black.
    let cmap = chunks(b).find(|c| &c.id == b"CMAP").map(|c| c.data).filter(|p| !p.is_empty());
    let colors = 1usize << h.planes;
    let palette = |idx: usize| -> [u8; 3] {
        if let Some(p) = cmap {
            p.get(idx * 3..idx * 3 + 3).map(|c| [c[0], c[1], c[2]]).unwrap_or([0; 3])
        } else {
            let v = ((idx as u32 * 255) / (colors as u32 - 1).max(1)) as u8;
            [v, v, v]
        }
    };

    let body = chunks(b).find(|c| &c.id == b"BODY").map(|c| c.data).ok_or_else(|| bad("no BODY chunk"))?;
    let (w, hh) = (h.width as usize, h.height as usize);
    // Each plane row is stored as a whole number of 16-bit words.
    let plane_row = if h.chunky { w.div_ceil(2) * 2 } else { w.div_ceil(16) * 2 };
    // A masking type 1 bitmap carries one extra (mask) plane per row.
    let rows_planes: usize = if h.chunky { 1 } else { usize::from(h.planes) + usize::from(h.masking == 1) };

    // Decode every stored plane row up front, then recombine.
    let total_rows = hh.checked_mul(rows_planes).ok_or_else(|| CodecError::LimitExceeded("bitmap too large".into()))?;
    let total_bytes = total_rows.checked_mul(plane_row).ok_or_else(|| CodecError::LimitExceeded("bitmap too large".into()))?;
    if total_bytes as u64 > limits.max_alloc {
        return Err(CodecError::LimitExceeded("bitmap storage exceeds max_alloc".into()));
    }
    Limits::check_plausible(total_bytes as u64, body.len(), if h.compression == 0 { 1 } else { 128 })?;
    let mut stored = crate::options::alloc_zeroed::<u8>(total_bytes)?;
    let mut cursor = 0usize;
    for row in stored.chunks_mut(plane_row.max(1)).take(total_rows) {
        match h.compression {
            0 => {
                let src = body.get(cursor..cursor.checked_add(plane_row).ok_or_else(|| bad("BODY overflow"))?).ok_or_else(|| bad("truncated BODY data"))?;
                let n = row.len().min(src.len());
                row[..n].copy_from_slice(&src[..n]);
                cursor += plane_row;
            }
            _ => {
                let tail = body.get(cursor..).ok_or_else(|| bad("truncated BODY data"))?;
                cursor += unpack_byte_run1(tail, row)?;
            }
        }
    }

    let mut out = Vec::new();
    out.try_reserve_exact(w.saturating_mul(hh).saturating_mul(layout.channels())).map_err(|_| CodecError::LimitExceeded("cannot allocate the image".into()))?;
    let planes = usize::from(h.planes);
    for y in 0..hh {
        for x in 0..w {
            let index = if h.chunky {
                usize::from(*stored.get(y * plane_row + x).ok_or_else(|| bad("PBM pixel outside BODY"))?)
            } else {
                (0..planes).filter(|&p| bit_at(&stored, (y * rows_planes + p) * plane_row, x)).fold(0usize, |acc, p| acc | (1 << p))
            };
            let c = if h.planes == DEEP_PLANES { [index as u8, (index >> 8) as u8, (index >> 16) as u8] } else { palette(index) };
            out.extend_from_slice(&c);
            if h.masking != 0 {
                let opaque = if h.masking == 1 { bit_at(&stored, (y * rows_planes + planes) * plane_row, x) } else { index != usize::from(h.transparent) };
                out.push(if opaque { 255 } else { 0 });
            }
        }
    }
    Image::from_u8(h.width, h.height, layout, out)
}

/// The bit for pixel `x` of a stored plane row at `base` (bits run MSB-first).
fn bit_at(stored: &[u8], base: usize, x: usize) -> bool {
    stored.get(base + x / 8).is_some_and(|byte| byte & (0x80 >> (x % 8)) != 0)
}

/// ByteRun1 / PackBits encode of one plane row, appended to `out`: three or more equal bytes
/// become a repeat (at most 128), everything else literal runs (at most 128 bytes).
fn pack_byte_run1(row: &[u8], out: &mut Vec<u8>) {
    let mut i = 0usize;
    while let Some(&value) = row.get(i) {
        let repeat = row[i..].iter().take(128).take_while(|&&b| b == value).count();
        if repeat >= 3 {
            out.push((1 - repeat as i32) as i8 as u8);
            out.push(value);
            i += repeat;
            continue;
        }
        let start = i;
        while i < row.len() && i - start < 128 {
            if row.get(i..i + 3).is_some_and(|r| r[0] == r[1] && r[1] == r[2]) {
                break;
            }
            i += 1;
        }
        out.push((i - start - 1) as u8);
        out.extend_from_slice(&row[start..i]);
    }
}

fn push_chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) -> Result<(), CodecError> {
    let len = u32::try_from(data.len()).map_err(|_| CodecError::encode(F, "chunk too large for an IFF file (4 GiB)"))?;
    out.extend_from_slice(id);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(data);
    // Chunks are padded to an even length.
    if data.len() % 2 == 1 {
        out.push(0);
    }
    Ok(())
}

/// Writes a 24-bitplane `FORM ILBM`: `BMHD` then a ByteRun1-compressed `BODY`, each row stored as
/// the red, green and blue planes (least significant bit first) and, for RGBA, a mask plane.
pub(crate) fn encode(src: &Image, plan: Plan) -> Result<Vec<u8>, CodecError> {
    let enc = |msg: String| CodecError::encode(F, msg);
    let img = src.converted(plan.layout, plan.sample);
    let masked = match img.layout() {
        ChannelLayout::Rgb => false,
        ChannelLayout::Rgba => true,
        l => return Err(enc(format!("IFF/ILBM cannot store {l:?}"))),
    };
    if img.sample_type() != SampleType::U8 {
        return Err(enc(format!("IFF/ILBM cannot store {:?} samples", img.sample_type())));
    }
    let (w, h) = img.dimensions();
    let (Ok(w16), Ok(h16)) = (u16::try_from(w), u16::try_from(h)) else {
        return Err(enc("IFF/ILBM stores at most 65535 × 65535 pixels".into()));
    };
    let (w, ch) = (w as usize, img.layout().channels());
    let data = img.data();
    let row_len = w.checked_mul(ch).ok_or_else(|| enc("image too large".into()))?;
    if data.len() != row_len.checked_mul(h as usize).ok_or_else(|| enc("image too large".into()))? {
        return Err(enc("the image has the wrong number of samples".into()));
    }
    let planes = usize::from(DEEP_PLANES) + usize::from(masked);
    let mut plane_row = vec![0u8; w.div_ceil(16) * 2];
    let mut body = Vec::new();
    for px in data.chunks_exact(row_len.max(1)) {
        for plane in 0..planes {
            plane_row.fill(0);
            for (x, p) in px.chunks_exact(ch).enumerate() {
                let on = if plane == usize::from(DEEP_PLANES) { p[3] >= 128 } else { (p[plane / 8] >> (plane % 8)) & 1 == 1 };
                if on {
                    plane_row[x / 8] |= 0x80 >> (x % 8);
                }
            }
            pack_byte_run1(&plane_row, &mut body);
        }
    }

    let mut bmhd = Vec::with_capacity(20);
    bmhd.extend_from_slice(&w16.to_be_bytes());
    bmhd.extend_from_slice(&h16.to_be_bytes());
    bmhd.extend_from_slice(&[0, 0, 0, 0]); // x/y origin
    bmhd.push(DEEP_PLANES);
    bmhd.push(u8::from(masked)); // masking: 1 = a mask plane follows the color planes
    bmhd.push(1); // ByteRun1
    bmhd.push(0); // pad
    bmhd.extend_from_slice(&[0, 0]); // transparent color (unused)
    bmhd.extend_from_slice(&[1, 1]); // square pixels
    bmhd.extend_from_slice(&w16.to_be_bytes()); // page size
    bmhd.extend_from_slice(&h16.to_be_bytes());

    let mut out = b"FORM".to_vec();
    out.extend_from_slice(&[0; 4]); // patched below
    out.extend_from_slice(b"ILBM");
    push_chunk(&mut out, b"BMHD", &bmhd)?;
    push_chunk(&mut out, b"BODY", &body)?;
    let form_len = u32::try_from(out.len() - 8).map_err(|_| enc("image too large for an IFF file (4 GiB)".into()))?;
    out[4..8].copy_from_slice(&form_len.to_be_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `FORM ILBM` with a `BMHD`, `CMAP` and `BODY`. `planes` plane rows per row,
    /// `compression` 0 (stored) or 1 (ByteRun1), MSB-first.
    fn ilbm(w: u16, h: u16, planes: u8, palette: &[[u8; 3]], compression: u8, body: &[u8]) -> Vec<u8> {
        let mut b = b"FORM".to_vec();
        b.extend_from_slice(&0u32.to_be_bytes()); // patched below
        b.extend_from_slice(b"ILBM");
        // BMHD (20 bytes)
        b.extend_from_slice(b"BMHD");
        b.extend_from_slice(&20u32.to_be_bytes());
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[0, 0, 0, 0]); // x/y origin
        b.push(planes);
        b.push(0); // masking
        b.push(compression);
        b.push(0); // pad
        b.extend_from_slice(&[0, 0]); // transparent colour
        b.extend_from_slice(&[1, 1]); // aspect
        b.extend_from_slice(&[0, 0, 0, 0]); // page size
        // CMAP
        b.extend_from_slice(b"CMAP");
        b.extend_from_slice(&((palette.len() * 3) as u32).to_be_bytes());
        for c in palette {
            b.extend_from_slice(c);
        }
        if palette.len() % 2 == 1 {
            b.push(0);
        }
        // BODY
        b.extend_from_slice(b"BODY");
        b.extend_from_slice(&(body.len() as u32).to_be_bytes());
        b.extend_from_slice(body);
        if body.len() % 2 == 1 {
            b.push(0);
        }
        let size = (b.len() - 8) as u32;
        b[4..8].copy_from_slice(&size.to_be_bytes());
        b
    }

    #[test]
    fn reads_a_stored_two_plane_bitmap() {
        // 2×1, 2 planes: pixel 0 index 0 (plane bits 0,0), pixel 1 index 3 (bits 1,1).
        // Plane 0 = 0b0100_0000, plane 1 = 0b0100_0000, each padded to 2 bytes.
        let body = [0b0100_0000, 0, 0b0100_0000, 0];
        let pal = [[10, 20, 30], [40, 50, 60], [70, 80, 90], [100, 110, 120]];
        let b = ilbm(2, 1, 2, &pal, 0, &body);
        assert!(looks_like_iff(&b));
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!((img.width(), img.height(), img.layout()), (2, 1, ChannelLayout::Rgb));
        assert_eq!(img.data(), &[10, 20, 30, 100, 110, 120]);
    }

    #[test]
    fn reads_byte_run1_compressed_rows() {
        // Same image, ByteRun1: literal run of two bytes (0x01) then the two plane bytes each row.
        let body = [0x01, 0b0100_0000, 0, 0x01, 0b0100_0000, 0];
        let pal = [[1, 2, 3], [4, 5, 6], [7, 8, 9], [250, 251, 252]];
        let b = ilbm(2, 1, 2, &pal, 1, &body);
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!(img.data(), &[1, 2, 3, 250, 251, 252]);
    }

    #[test]
    fn no_palette_falls_back_to_a_grey_ramp() {
        // 2×1, 1 plane, no CMAP: indices 0 and 1 → black and white.
        let body = [0b0100_0000, 0];
        let b = ilbm(2, 1, 1, &[], 0, &body);
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!(img.data(), &[0, 0, 0, 255, 255, 255]);
    }

    #[test]
    fn pbm_reads_chunky_indices_with_odd_width_padding() {
        let pal = [[10, 20, 30], [40, 50, 60], [70, 80, 90]];
        for (compression, body) in [(0, vec![2, 0, 1, 255, 1, 2, 0, 255]), (1, vec![3, 2, 0, 1, 255, 3, 1, 2, 0, 255])] {
            let mut b = ilbm(3, 2, 8, &pal, compression, &body);
            b[8..12].copy_from_slice(b"PBM ");
            let img = decode(&b, &Limits::default()).unwrap();
            assert_eq!(img.data(), &[70, 80, 90, 10, 20, 30, 40, 50, 60, 40, 50, 60, 70, 80, 90, 10, 20, 30]);
        }
    }

    #[test]
    fn transparency_uses_the_mask_plane_or_transparent_index() {
        for masking in [1, 2] {
            let mut b = ilbm(2, 1, 1, &[[10, 20, 30], [40, 50, 60]], 0, if masking == 1 { &[0x40, 0, 0x40, 0] } else { &[0x40, 0] });
            b[29] = masking;
            let img = decode(&b, &Limits::default()).unwrap();
            assert_eq!(img.layout(), ChannelLayout::Rgba);
            assert_eq!(img.data(), &[10, 20, 30, 0, 40, 50, 60, 255]);
        }
        let mut b = ilbm(2, 1, 8, &[[10, 20, 30], [40, 50, 60]], 0, &[0, 1]);
        b[8..12].copy_from_slice(b"PBM ");
        b[29] = 2;
        b[32..34].copy_from_slice(&1u16.to_be_bytes());
        assert_eq!(decode(&b, &Limits::default()).unwrap().data(), &[10, 20, 30, 255, 40, 50, 60, 0]);
    }

    #[test]
    fn rejects_ham_and_ehb_instead_of_returning_wrong_colours() {
        for mode in [0x80u32, 0x800] {
            let mut b = ilbm(1, 1, 1, &[], 0, &[0, 0]);
            b.extend_from_slice(b"CAMG");
            b.extend_from_slice(&4u32.to_be_bytes());
            b.extend_from_slice(&mode.to_be_bytes());
            let len = (b.len() - 8) as u32;
            b[4..8].copy_from_slice(&len.to_be_bytes());
            assert!(matches!(decode(&b, &Limits::default()), Err(CodecError::Unsupported { .. })));
        }
    }

    #[test]
    fn missing_palette_entries_are_black() {
        let b = ilbm(1, 1, 2, &[[10, 20, 30]], 0, &[0x80, 0, 0x80, 0]);
        assert_eq!(decode(&b, &Limits::default()).unwrap().data(), &[0, 0, 0]);
    }

    fn plan(layout: ChannelLayout) -> Plan {
        Plan { layout, sample: SampleType::U8 }
    }

    #[test]
    fn reads_a_stored_24_plane_bitmap() {
        // 2×1: pixel 0 = (1, 0, 128), pixel 1 = (0, 255, 0). Each plane row is 2 bytes; red bit 0
        // has pixel 0 set, green bits 0–7 have pixel 1 set, blue bit 7 has pixel 0 set.
        let mut body = Vec::new();
        for plane in 0..24 {
            let byte = match plane {
                0 | 23 => 0x80,
                8..=15 => 0x40,
                _ => 0,
            };
            body.extend_from_slice(&[byte, 0]);
        }
        let b = ilbm(2, 1, 24, &[], 0, &body);
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!((img.width(), img.height(), img.layout()), (2, 1, ChannelLayout::Rgb));
        assert_eq!(img.data(), &[1, 0, 128, 0, 255, 0]);
    }

    #[test]
    fn writes_24_plane_byte_run1_that_reads_back_exactly() {
        // 37 pixels wide: three plane-row words, the last one partly padding.
        let (w, h) = (37u32, 3u32);
        let data: Vec<u8> = (0..w * h * 3).map(|i| (i * 7 % 256) as u8).collect();
        let img = Image::from_u8(w, h, ChannelLayout::Rgb, data.clone()).unwrap();
        let bytes = encode(&img, plan(ChannelLayout::Rgb)).unwrap();
        assert!(looks_like_iff(&bytes));
        assert_eq!(be32(&bytes, 4).unwrap() as usize, bytes.len() - 8, "FORM length");
        let h = parse_header(&bytes, false).unwrap();
        assert_eq!((h.width, h.height, h.planes, h.masking, h.compression), (37, 3, 24, 0, 1));
        assert!(chunks(&bytes).all(|c| &c.id != b"CMAP"), "a deep ILBM has no palette");
        let back = decode(&bytes, &Limits::default()).unwrap();
        assert_eq!(back.layout(), ChannelLayout::Rgb);
        assert_eq!(back.data(), &data[..]);
    }

    #[test]
    fn alpha_is_written_as_a_mask_plane() {
        let data = [10, 20, 30, 0, 40, 50, 60, 127, 70, 80, 90, 128, 1, 2, 3, 255];
        let img = Image::from_u8(4, 1, ChannelLayout::Rgba, data.to_vec()).unwrap();
        let bytes = encode(&img, plan(ChannelLayout::Rgba)).unwrap();
        assert_eq!(parse_header(&bytes, false).unwrap().masking, 1);
        let back = decode(&bytes, &Limits::default()).unwrap();
        assert_eq!(back.layout(), ChannelLayout::Rgba);
        assert_eq!(back.data(), &[10, 20, 30, 0, 40, 50, 60, 0, 70, 80, 90, 255, 1, 2, 3, 255]);
    }

    #[test]
    fn byte_run1_round_trips_runs_literals_and_long_rows() {
        let mut rows: Vec<Vec<u8>> = vec![vec![], vec![5], vec![5, 5], vec![5, 5, 5], vec![1, 2, 2, 2, 3], vec![9; 300]];
        rows.push((0..300).map(|i| (i % 251) as u8).collect());
        rows.push([vec![1, 2], vec![7; 129], vec![3]].concat());
        for row in rows {
            let mut packed = Vec::new();
            pack_byte_run1(&row, &mut packed);
            let mut back = vec![0; row.len()];
            assert_eq!(unpack_byte_run1(&packed, &mut back).unwrap(), packed.len(), "{row:?}");
            assert_eq!(back, row);
        }
        // A long repeat packs into two-byte runs.
        let mut packed = Vec::new();
        pack_byte_run1(&[0; 256], &mut packed);
        assert_eq!(packed, [0x81, 0, 0x81, 0]);
    }

    #[test]
    fn rejects_what_ilbm_cannot_store() {
        let gray = Image::from_u8(1, 1, ChannelLayout::Gray, vec![0]).unwrap();
        assert!(encode(&gray, plan(ChannelLayout::Gray)).is_err());
        let wide = Image::from_u8(65536, 1, ChannelLayout::Rgb, vec![0; 65536 * 3]).unwrap();
        assert!(encode(&wide, plan(ChannelLayout::Rgb)).is_err());
        // A transparent color index has no meaning without a palette.
        let mut b = ilbm(1, 1, 24, &[], 0, &[0; 48]);
        b[29] = 2;
        assert!(matches!(decode(&b, &Limits::default()), Err(CodecError::Unsupported { .. })));
    }

    #[test]
    fn hostile_input_errors_not_panics() {
        // Truncations of a valid file never panic.
        let body = [0b0100_0000, 0, 0b0100_0000, 0];
        let full = ilbm(2, 1, 2, &[[1, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12]], 0, &body);
        for cut in [0, 4, 8, 12, 40, full.len() - 1] {
            assert!(decode(&full[..cut.min(full.len())], &Limits::default()).is_err(), "cut {cut}");
        }
        // Too many planes is unsupported.
        let bad = ilbm(1, 1, 9, &[], 0, &[0, 0]);
        assert!(matches!(decode(&bad, &Limits::default()), Err(CodecError::Unsupported { .. })));
    }
}
