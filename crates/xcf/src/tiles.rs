//! One tile's bytes: uncompressed, zlib-coded or RLE-coded.

use std::io::Read;

use crate::{Compression, Result, invalid};

/// What one tile's bytes decode to, for a drawable of `bpp` bytes per pixel: `pixels × bpp`
/// interleaved bytes.
pub(crate) fn decode(input: &[u8], compression: Compression, bpp: usize, pixels: usize) -> Result<Vec<u8>> {
    let expected = pixels.checked_mul(bpp).ok_or_else(|| invalid("tile size overflow"))?;
    match compression {
        Compression::None => input.get(..expected).map(<[u8]>::to_vec).ok_or_else(|| invalid("an uncompressed tile is cut short")),
        Compression::Zlib => {
            let mut out = Vec::with_capacity(expected);
            flate2::read::ZlibDecoder::new(input).take(expected as u64).read_to_end(&mut out).map_err(|e| invalid(format!("zlib tile: {e}")))?;
            if out.len() != expected {
                return Err(invalid("a zlib tile is cut short"));
            }
            Ok(out)
        }
        Compression::Rle => rle_decode(input, bpp, pixels),
    }
}

/// RLE tile data: one run-length stream per byte position of the pixel, each of `pixels` bytes.
pub(crate) fn rle_decode(input: &[u8], bpp: usize, pixels: usize) -> Result<Vec<u8>> {
    let short = || invalid("an RLE tile is cut short");
    let mut out = vec![0u8; pixels * bpp];
    let mut pos = 0usize;
    let mut plane = Vec::with_capacity(pixels);
    for b in 0..bpp {
        plane.clear();
        while plane.len() < pixels {
            let op = *input.get(pos).ok_or_else(short)?;
            pos += 1;
            let mut long = || -> Result<usize> {
                let (p, q) = (input.get(pos).copied().ok_or_else(short)?, input.get(pos + 1).copied().ok_or_else(short)?);
                pos += 2;
                Ok(usize::from(p) * 256 + usize::from(q))
            };
            let (run, verbatim) = match op {
                0..=126 => (usize::from(op) + 1, false),
                127 => (long()?, false),
                128 => (long()?, true),
                _ => (256 - usize::from(op), true),
            };
            if plane.len() + run > pixels {
                return Err(invalid("an RLE run crosses the end of its tile"));
            }
            // A long run of length 0 consumes input without producing output: a tile of them
            // (reachable from many tile pointers) is work the pixel budget never charges.
            if run == 0 {
                return Err(invalid("an RLE run has no length"));
            }
            if verbatim {
                let data = input.get(pos..pos + run).ok_or_else(short)?;
                pos += run;
                plane.extend_from_slice(data);
            } else {
                let v = *input.get(pos).ok_or_else(short)?;
                pos += 1;
                plane.resize(plane.len() + run, v);
            }
        }
        for (slot, v) in out.iter_mut().skip(b).step_by(bpp.max(1)).zip(&plane) {
            *slot = *v;
        }
    }
    Ok(out)
}
