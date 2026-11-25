//! A small PNG writer for the images a compiled graph stores, which the decompiler embeds in the graph text as PNG
//! resources: 8/16-bit grey, RGB and RGBA, each row with the PNG filter of the smallest residual sum, then DEFLATE.
//!
//! Keep encoding deterministic because the PNG bytes form part of the decompiled graph's cache key.

mod checksum;
mod deflate;

use crc32fast::Hasher;
use deflate::Level;

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = Hasher::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
}

/// PNG colour types the decompiler writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColorType {
    Gray,
    Rgb,
    Rgba,
}

impl ColorType {
    fn channels(self) -> usize {
        match self {
            ColorType::Gray => 1,
            ColorType::Rgb => 3,
            ColorType::Rgba => 4,
        }
    }

    /// The IHDR colour type code.
    fn code(self) -> u8 {
        match self {
            ColorType::Gray => 0,
            ColorType::Rgb => 2,
            ColorType::Rgba => 6,
        }
    }
}

/// Largest IDAT chunk written; big images are split over several.
const IDAT_MAX: usize = 1 << 20;

/// Encode raw samples (row-major, interleaved, 16-bit big-endian). `bit_depth` is 8 or 16 (anything else becomes 8).
/// Missing samples are written as zero, extra ones ignored. Each row gets the PNG filter with the smallest sum of
/// absolute (signed) residuals, then the image is DEFLATE-compressed with fast matching.
pub(crate) fn encode_raw(w: usize, h: usize, color: ColorType, bit_depth: u8, samples: &[u8]) -> Vec<u8> {
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
    let z = deflate::zlib_compress(&filtered, Level::Fast);

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

#[cfg(test)]
/// The samples of a PNG this module wrote: chunks checked, IDAT inflated and unfiltered.
pub(crate) fn decode(png: &[u8]) -> (u32, u32, u8, u8, Vec<u8>) {
    assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let (mut at, mut ihdr, mut idat) = (8, Vec::new(), Vec::new());
    while at < png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let kind = &png[at + 4..at + 8];
        let data = &png[at + 8..at + 8 + len];
        let crc = u32::from_be_bytes(png[at + 8 + len..at + 12 + len].try_into().unwrap());
        assert_eq!(crc, checksum::crc32(&png[at + 4..at + 8 + len]));
        match kind {
            b"IHDR" => ihdr = data.to_vec(),
            b"IDAT" => idat.extend_from_slice(data),
            _ => {}
        }
        at += 12 + len;
    }
    let w = u32::from_be_bytes(ihdr[0..4].try_into().unwrap());
    let h = u32::from_be_bytes(ihdr[4..8].try_into().unwrap());
    let (depth, color) = (ihdr[8], ihdr[9]);
    let raw = crate::inflate::inflate(&idat[2..idat.len() - 4]).unwrap();
    assert_eq!(&idat[idat.len() - 4..], &checksum::adler32(&raw).to_be_bytes());
    let channels = match color {
        0 => 1,
        2 => 3,
        _ => 4,
    };
    let bpp = channels * usize::from(depth / 8);
    let stride = w as usize * bpp;
    let mut out: Vec<u8> = Vec::new();
    let mut prev = vec![0u8; stride];
    for row in raw.chunks(stride + 1) {
        let (f, line) = (row[0], &row[1..]);
        let mut cur = vec![0u8; stride];
        for i in 0..stride {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let (b, c) = (prev[i], if i >= bpp { prev[i - bpp] } else { 0 });
            let p = match f {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((a as u16 + b as u16) / 2) as u8,
                _ => paeth(a, b, c),
            };
            cur[i] = line[i].wrapping_add(p);
        }
        out.extend_from_slice(&cur);
        prev = cur;
    }
    (w, h, depth, color, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_survive_every_filter_and_depth() {
        let rgba: Vec<u8> = (0..7 * 5 * 4).map(|i| (i * 37 % 251) as u8).collect();
        assert_eq!(decode(&encode_raw(7, 5, ColorType::Rgba, 8, &rgba)), (7, 5, 8, 6, rgba));
        let grey16: Vec<u8> = (0..3 * 4 * 2).map(|i| (i * 91 % 256) as u8).collect();
        assert_eq!(decode(&encode_raw(3, 4, ColorType::Gray, 16, &grey16)), (3, 4, 16, 0, grey16));
    }

    #[test]
    fn missing_samples_are_zero() {
        let (.., samples) = decode(&encode_raw(2, 2, ColorType::Rgb, 8, &[9, 8, 7]));
        assert_eq!(samples, [9, 8, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }
}
