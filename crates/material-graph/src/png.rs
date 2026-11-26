// SPDX-License-Identifier: MIT OR Apache-2.0
//! Dependency-free PNG codec: 8/16-bit gray, gray+alpha, RGB and RGBA encoding with filtering and DEFLATE
//! compression; decoding of all standard formats to float RGBA (`decode`) or losslessly to raw samples (`decode_raw`).
use crate::checksum::Crc32;
use crate::deflate::{self, Level};

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = Crc32::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

/// Encode RGBA float pixels (0..1). `depth` must be 8 or 16 (anything else becomes 8).
pub fn encode(w: usize, h: usize, px: &[[f32; 4]], depth: u8) -> Vec<u8> {
    let depth = if depth == 16 { 16 } else { 8 };
    let opaque = px.iter().all(|p| p[3] >= 0.9999);
    let ch = if opaque { 3 } else { 4 };
    let mut samples = Vec::with_capacity(w * h * ch * (depth / 8) as usize);
    for i in 0..w * h {
        let p = px.get(i).copied().unwrap_or([0.0; 4]);
        for c in p.iter().take(ch) {
            let v = c.clamp(0.0, 1.0);
            if depth == 16 {
                samples.extend_from_slice(&((v * 65535.0 + 0.5) as u16).to_be_bytes());
            } else {
                samples.push((v * 255.0 + 0.5) as u8);
            }
        }
    }
    encode_raw(w, h, if opaque { ColorType::Rgb } else { ColorType::Rgba }, depth, &samples, Level::Default)
}

/// PNG colour types without palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorType {
    Gray,
    GrayAlpha,
    Rgb,
    Rgba,
}

impl ColorType {
    pub fn channels(self) -> usize {
        match self {
            ColorType::Gray => 1,
            ColorType::GrayAlpha => 2,
            ColorType::Rgb => 3,
            ColorType::Rgba => 4,
        }
    }

    /// The IHDR colour type code.
    fn code(self) -> u8 {
        match self {
            ColorType::Gray => 0,
            ColorType::GrayAlpha => 4,
            ColorType::Rgb => 2,
            ColorType::Rgba => 6,
        }
    }

    fn from_code(c: u8) -> Option<Self> {
        match c {
            0 => Some(ColorType::Gray),
            4 => Some(ColorType::GrayAlpha),
            2 => Some(ColorType::Rgb),
            6 => Some(ColorType::Rgba),
            _ => None,
        }
    }
}

/// Raw image samples exactly as stored: row-major, interleaved channels, 16-bit samples big-endian.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawImage {
    pub width: usize,
    pub height: usize,
    pub color: ColorType,
    /// 8 or 16.
    pub bit_depth: u8,
    pub samples: Vec<u8>,
}

/// Largest IDAT chunk written; big images are split over several.
const IDAT_MAX: usize = 1 << 20;

/// Encode raw samples (row-major, interleaved, 16-bit big-endian). `bit_depth` is 8 or 16 (anything else becomes 8).
/// Missing samples are written as zero, extra ones ignored. Each row gets the PNG filter with the smallest sum of
/// absolute (signed) residuals, then the image is DEFLATE-compressed at `level`.
pub fn encode_raw(w: usize, h: usize, color: ColorType, bit_depth: u8, samples: &[u8], level: Level) -> Vec<u8> {
    let depth = if bit_depth == 16 { 16 } else { 8 };
    let bpp = color.channels() * (depth / 8) as usize;
    let stride = w * bpp;
    let mut filtered = Vec::with_capacity(h * (stride + 1));
    let mut cur = vec![0u8; stride];
    let mut prev = vec![0u8; stride];
    let (mut trial, mut best) = (vec![0u8; stride], vec![0u8; stride]);
    for y in 0..h {
        let src = samples.get((y * stride).min(samples.len())..).unwrap_or(&[]);
        let n = src.len().min(stride);
        let (head, tail) = cur.split_at_mut(n);
        head.copy_from_slice(src.get(..n).unwrap_or(&[]));
        tail.fill(0);
        let (mut best_f, mut best_cost) = (0u8, u64::MAX);
        for f in 0..5u8 {
            filter_row(f, &cur, &prev, bpp, &mut trial);
            let cost = trial.iter().map(|&b| (b as i8).unsigned_abs() as u64).sum::<u64>();
            if cost < best_cost {
                (best_f, best_cost) = (f, cost);
                std::mem::swap(&mut trial, &mut best);
            }
        }
        filtered.push(best_f);
        filtered.extend_from_slice(&best);
        std::mem::swap(&mut cur, &mut prev);
    }
    let z = deflate::zlib_compress(&filtered, level);

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[depth, color.code(), 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    for part in z.chunks(IDAT_MAX) {
        chunk(&mut out, b"IDAT", part);
    }
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Apply PNG filter `f` (0 none, 1 sub, 2 up, 3 average, 4 Paeth) to `cur` given the previous row.
/// `cur`, `prev` and `out` all have the row length.
fn filter_row(f: u8, cur: &[u8], prev: &[u8], bpp: usize, out: &mut [u8]) {
    let predict = |a: u8, b: u8, c: u8| match f {
        0 => 0,
        1 => a,
        2 => b,
        3 => ((a as u16 + b as u16) / 2) as u8,
        _ => paeth(a, b, c),
    };
    // The first pixel has no left neighbour (a = c = 0).
    for ((o, &x), &b) in out.iter_mut().zip(cur).zip(prev).take(bpp) {
        *o = x.wrapping_sub(predict(0, b, 0));
    }
    // Then byte i is predicted from cur[i - bpp], prev[i] and prev[i - bpp].
    let rest = out.iter_mut().zip(cur).zip(prev).skip(bpp).zip(cur.iter().zip(prev));
    for (((o, &x), &b), (&a, &c)) in rest {
        *o = x.wrapping_sub(predict(a, b, c));
    }
}

// ======================= decoder =======================
use crate::tex::Tex;

/// DEFLATE decompression lives in [`crate::deflate`]; re-exported here because `png::inflate` is public API.
pub use crate::deflate::inflate;

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (a as i32, b as i32, c as i32);
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// The chunks of a PNG the decoders use.
struct Parsed {
    w: usize,
    h: usize,
    depth: u8,
    ctype: u8,
    inter: u8,
    /// Channels per pixel (1 for palette).
    ch: usize,
    plte: Vec<u8>,
    trns: Vec<u8>,
    idat: Vec<u8>,
}

fn parse(b: &[u8]) -> Result<Parsed, String> {
    if b.get(..8) != Some(b"\x89PNG\r\n\x1a\n".as_slice()) {
        return Err("not a PNG file".into());
    }
    let mut pos = 8;
    let (mut w, mut h, mut depth, mut ctype, mut inter) = (0usize, 0usize, 0u8, 0u8, 0u8);
    let mut plte: Vec<u8> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat: Vec<u8> = Vec::new();
    // Each chunk: 4-byte length, 4-byte type, data, 4-byte CRC. `pos <= b.len()` throughout, so `pos + 8` cannot
    // overflow; the 32-bit length can (on 32-bit targets), hence the checked sums.
    while let Some(&[l0, l1, l2, l3, k0, k1, k2, k3]) = b.get(pos..pos + 8) {
        let len = u32::from_be_bytes([l0, l1, l2, l3]) as usize;
        let kind = [k0, k1, k2, k3];
        let end = (pos + 8).checked_add(len).filter(|e| e.checked_add(4).is_some_and(|e4| e4 <= b.len())).ok_or("truncated chunk")?;
        let data = b.get(pos + 8..end).ok_or("truncated chunk")?;
        match &kind {
            b"IHDR" if data.len() >= 13 => {
                if let Some(&[w0, w1, w2, w3, h0, h1, h2, h3, d, c, _, _, i]) = data.get(..13) {
                    w = u32::from_be_bytes([w0, w1, w2, w3]) as usize;
                    h = u32::from_be_bytes([h0, h1, h2, h3]) as usize;
                    (depth, ctype, inter) = (d, c, i);
                }
            }
            b"PLTE" => plte = data.to_vec(),
            b"tRNS" => trns = data.to_vec(),
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        pos = end + 4;
    }
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return Err("bad image size".into());
    }
    let ch = match (ctype, depth) {
        (0, 1 | 2 | 4 | 8 | 16) => 1,
        (2, 8 | 16) => 3,
        (3, 1 | 2 | 4 | 8) => 1,
        (4, 8 | 16) => 2,
        (6, 8 | 16) => 4,
        _ => return Err(format!("unsupported PNG format (colour type {ctype}, {depth}-bit)")),
    };
    if idat.len() < 6 {
        return Err("missing image data".into());
    }
    Ok(Parsed { w, h, depth, ctype, inter, ch, plte, trns, idat })
}

/// One unfiltered Adam7 pass (or the whole image when not interlaced): pixel `(i, j)` of the pass lands at
/// `(x0 + i * dx, y0 + j * dy)`; `img` holds `ph` rows of `stride` bytes.
struct Pass {
    x0: usize,
    y0: usize,
    dx: usize,
    dy: usize,
    pw: usize,
    ph: usize,
    stride: usize,
    img: Vec<u8>,
}

/// Inflate the image data and undo the row filters of every pass.
fn unfilter_passes(p: &Parsed) -> Result<Vec<Pass>, String> {
    let raw = inflate(p.idat.get(2..).ok_or("missing image data")?)?;
    let bits_pp = p.ch * p.depth as usize;
    let bpp = bits_pp.div_ceil(8).max(1);
    let layout: &[(usize, usize, usize, usize)] = if p.inter == 0 {
        &[(0, 0, 1, 1)]
    } else {
        &[(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]
    };
    let mut passes = Vec::new();
    let mut off = 0usize;
    for &(x0, y0, dx, dy) in layout {
        if x0 >= p.w || y0 >= p.h {
            continue;
        }
        // w, h <= 16384 and at most 64 bits per pixel: these products stay below 2^31 even with a 32-bit usize.
        let pw = (p.w - x0).div_ceil(dx);
        let ph = (p.h - y0).div_ceil(dy);
        let stride = (pw * bits_pp).div_ceil(8);
        let need = ph * (stride + 1);
        let end = off.checked_add(need).ok_or("image data too short")?;
        let Some(data) = raw.get(off..end) else {
            return Err("image data too short".into());
        };
        let mut img = Vec::with_capacity(ph * stride);
        // The previous unfiltered row (zeros above the first one) and the row being unfiltered.
        let (mut up, mut cur) = (vec![0u8; stride], vec![0u8; stride]);
        for line in data.chunks_exact(stride + 1) {
            let Some((&f, src)) = line.split_first() else {
                return Err("image data too short".into());
            };
            unfilter_row(f, src, &up, bpp, &mut cur)?;
            img.extend_from_slice(&cur);
            std::mem::swap(&mut up, &mut cur);
        }
        off = end;
        passes.push(Pass { x0, y0, dx, dy, pw, ph, stride, img });
    }
    Ok(passes)
}

/// Undo filter `f` on one row: `src` is the filtered row, `up` the unfiltered row above, all of `cur`'s length.
fn unfilter_row(f: u8, src: &[u8], up: &[u8], bpp: usize, cur: &mut [u8]) -> Result<(), String> {
    // Byte i depends on byte i - bpp of this row (`left`) and of the row above (`upleft`): kept per byte of a pixel
    // (bpp <= 8), zero for the first pixel.
    let (mut left, mut upleft) = ([0u8; 8], [0u8; 8]);
    for ((cpx, spx), upx) in cur.chunks_mut(bpp).zip(src.chunks(bpp)).zip(up.chunks(bpp)) {
        for ((((o, &x), &u), l), ul) in cpx.iter_mut().zip(spx).zip(upx).zip(left.iter_mut()).zip(upleft.iter_mut()) {
            let (a, c) = (*l, *ul);
            let v = match f {
                0 => x,
                1 => x.wrapping_add(a),
                2 => x.wrapping_add(u),
                3 => x.wrapping_add(((a as u16 + u as u16) / 2) as u8),
                4 => x.wrapping_add(paeth(a, u, c)),
                _ => return Err("bad PNG filter".into()),
            };
            *o = v;
            *l = v;
            *ul = u;
        }
    }
    Ok(())
}

/// `n` copies of `v`, or an error instead of an abort when the allocation is impossible (a 16384 x 16384 float image
/// is 4 GiB, more than a 32-bit target can address).
fn filled<T: Clone>(n: usize, v: T) -> Result<Vec<T>, String> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| format!("image too large ({n} pixels or samples)"))?;
    out.resize(n, v);
    Ok(out)
}

/// Decode a PNG: gray, gray+alpha, RGB, RGBA and palette images (1-16 bit, tRNS palette alpha, Adam7 interlacing).
pub fn decode(b: &[u8]) -> Result<Tex, String> {
    let p = parse(b)?;
    let (w, h, depth, ch) = (p.w, p.h, p.depth, p.ch);
    let maxv = ((1u32 << depth) - 1) as f32;
    let mut px = filled(w * h, [0.0f32; 4])?;
    for pass in unfilter_passes(&p)? {
        for (py, row) in pass.img.chunks_exact(pass.stride.max(1)).take(pass.ph).enumerate() {
            let sample = |idx: usize| -> u32 {
                match depth {
                    16 => match row.get(idx * 2..idx * 2 + 2) {
                        Some(&[hi, lo]) => u16::from_be_bytes([hi, lo]) as u32,
                        _ => 0,
                    },
                    8 => row.get(idx).copied().unwrap_or(0) as u32,
                    d => {
                        let bit = idx * d as usize;
                        let shift = 8 - d as usize - (bit % 8);
                        row.get(bit / 8).map_or(0, |&byte| ((byte >> shift) & ((1u8 << d) - 1)) as u32)
                    }
                }
            };
            for pxi in 0..pass.pw {
                let v = |k: usize| sample(pxi * ch + k) as f32 / maxv;
                let c = match p.ctype {
                    0 => [v(0), v(0), v(0), 1.0],
                    4 => [v(0), v(0), v(0), v(1)],
                    2 => [v(0), v(1), v(2), 1.0],
                    6 => [v(0), v(1), v(2), v(3)],
                    _ => {
                        let i = sample(pxi) as usize;
                        let Some(&[r, g, b]) = p.plte.get(i * 3..i * 3 + 3) else {
                            return Err("palette index out of range".into());
                        };
                        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, p.trns.get(i).map(|a| *a as f32 / 255.0).unwrap_or(1.0)]
                    }
                };
                // The pass geometry keeps every pixel inside the image.
                if let Some(o) = px.get_mut((pass.y0 + py * pass.dy) * w + pass.x0 + pxi * pass.dx) {
                    *o = c;
                }
            }
        }
    }
    Ok(Tex { w, h, px })
}

/// Decode an 8- or 16-bit gray, gray+alpha, RGB or RGBA PNG (interlaced or not) to its exact stored samples.
/// Palette and 1/2/4-bit images are rejected; ancillary chunks (gamma, colour profiles, tRNS) are ignored.
pub fn decode_raw(b: &[u8]) -> Result<RawImage, String> {
    let p = parse(b)?;
    let color = ColorType::from_code(p.ctype)
        .filter(|_| p.depth >= 8)
        .ok_or_else(|| format!("decode_raw: unsupported PNG format (colour type {}, {}-bit)", p.ctype, p.depth))?;
    let bpp = p.ch * (p.depth / 8) as usize;
    let mut samples = filled(p.w * p.h * bpp, 0u8)?;
    for pass in unfilter_passes(&p)? {
        for (py, row) in pass.img.chunks_exact(pass.stride.max(1)).take(pass.ph).enumerate() {
            let y = pass.y0 + py * pass.dy;
            if pass.dx == 1 {
                if let Some(dst) = samples.get_mut(y * p.w * bpp..(y + 1) * p.w * bpp) {
                    dst.copy_from_slice(row);
                }
                continue;
            }
            for (i, px) in row.chunks_exact(bpp).enumerate() {
                let o = (y * p.w + pass.x0 + i * pass.dx) * bpp;
                if let Some(dst) = samples.get_mut(o..o + bpp) {
                    dst.copy_from_slice(px);
                }
            }
        }
    }
    Ok(RawImage { width: p.w, height: p.h, color, bit_depth: p.depth, samples })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(w: usize, h: usize, color: ColorType, depth: u8) -> Vec<u8> {
        let n = w * h * color.channels() * (depth / 8) as usize;
        (0..n).map(|i| ((i * 7 + i / 13 * 3 + (i * i) % 5) % 251) as u8).collect()
    }

    #[test]
    fn raw_round_trips_all_formats() {
        for color in [ColorType::Gray, ColorType::GrayAlpha, ColorType::Rgb, ColorType::Rgba] {
            for depth in [8u8, 16] {
                for (w, h) in [(1, 1), (3, 2), (17, 9), (64, 64)] {
                    let s = pattern(w, h, color, depth);
                    let png = encode_raw(w, h, color, depth, &s, Level::Default);
                    let r = decode_raw(&png).unwrap();
                    assert_eq!(r, RawImage { width: w, height: h, color, bit_depth: depth, samples: s.clone() });
                    // The float decoder agrees on the first channel of the first pixel.
                    let t = decode(&png).unwrap();
                    let first = if depth == 16 { u16::from_be_bytes([s[0], s[1]]) as f32 / 65535.0 } else { s[0] as f32 / 255.0 };
                    assert!((t.px[0][0] - first).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn compresses_smooth_images() {
        let (w, h) = (256, 256);
        let s: Vec<u8> = (0..w * h * 3).map(|i| ((i / 3) % w + (i / 3) / w) as u8).collect();
        let png = encode_raw(w, h, ColorType::Rgb, 8, &s, Level::Default);
        assert!(png.len() < s.len() / 20, "{}", png.len());
        assert_eq!(decode_raw(&png).unwrap().samples, s);
    }

    #[test]
    fn encode_float_is_lossless_at_8_bit() {
        let px: Vec<[f32; 4]> = (0..64).map(|i| [i as f32 / 63.0, (i % 8) as f32 / 255.0, 0.5, if i == 5 { 0.5 } else { 1.0 }]).collect();
        let t = decode(&encode(8, 8, &px, 8)).unwrap();
        for (a, b) in px.iter().zip(&t.px) {
            for c in 0..4 {
                assert_eq!((a[c] * 255.0 + 0.5) as u8, (b[c] * 255.0 + 0.5) as u8);
            }
        }
    }

    #[test]
    fn decode_raw_handles_interlaced() {
        // A 3x3 8-bit gray Adam7 image built by hand: passes 1,4,6,7 are the only non-empty ones for this size
        // except pass 1 (0,0), pass 4 (2,0), pass 5 (0,2),(2,2), pass 6 (1,0),(1,2), pass 7 rows 1.
        let img = [[10u8, 11, 12], [13, 14, 15], [16, 17, 18]];
        let mut raw = Vec::new();
        let layout = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];
        for (x0, y0, dx, dy) in layout {
            let mut y = y0;
            while y < 3 && x0 < 3 {
                raw.push(0);
                let mut x = x0;
                while x < 3 {
                    raw.push(img[y][x]);
                    x += dx;
                }
                y += dy;
            }
        }
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        chunk(&mut out, b"IHDR", &[0, 0, 0, 3, 0, 0, 0, 3, 8, 0, 0, 0, 1]);
        chunk(&mut out, b"IDAT", &deflate::zlib_compress(&raw, Level::Default));
        chunk(&mut out, b"IEND", &[]);
        let r = decode_raw(&out).unwrap();
        assert_eq!(r.samples, img.concat());
    }

    #[test]
    fn truncations_error_without_panicking() {
        let png = encode_raw(5, 4, ColorType::Rgba, 16, &pattern(5, 4, ColorType::Rgba, 16), Level::Default);
        for n in 0..png.len() - 12 {
            assert!(decode_raw(&png[..n]).is_err(), "prefix {n}");
            assert!(decode(&png[..n]).is_err(), "prefix {n}");
        }
        assert!(decode_raw(&encode_raw(0, 0, ColorType::Gray, 8, &[], Level::Fast)).is_err());
    }

    #[test]
    fn inflate_rejects_every_truncation() {
        let data: Vec<u8> = (0..5000u32).map(|i| (i % 97 + i / 300) as u8).collect();
        for level in [Level::Store, Level::Fast, Level::Default] {
            let z = deflate::compress(&data, level);
            assert_eq!(inflate(&z).unwrap(), data);
            for n in 0..z.len() {
                assert!(inflate(&z[..n]).is_err(), "{level:?} prefix {n}");
            }
        }
    }
}
