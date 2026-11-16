//! Single-part scanline OpenEXR output from RGBA8 pixels.

/// Scanline chunk compression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    /// Uncompressed.
    None,
    /// OpenEXR RLE.
    Rle,
    /// zlib per scanline.
    Zips,
    /// zlib per 16 scanlines.
    Zip,
}
impl Compression {
    fn code(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Rle => 1,
            Self::Zips => 2,
            Self::Zip => 3,
        }
    }
    fn lines(self) -> usize {
        if self == Self::Zip { 16 } else { 1 }
    }
}
/// Why a frame cannot be encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Empty, overflowing or mismatched RGBA dimensions.
    FrameSize,
    /// Dimensions exceed the OpenEXR signed-size fields.
    TooLarge,
}
fn checked_len(rgba: &[u8], w: u32, h: u32) -> Result<usize, Error> {
    let n = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(4)).ok_or(Error::FrameSize)?;
    if w == 0 || h == 0 || rgba.len() != n {
        return Err(Error::FrameSize);
    }
    Ok(n)
}

/// IEEE 754 binary16 bits of `v`, rounded to nearest even (overflow → infinity, NaN → quiet NaN).
pub fn half(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xFF) as i32;
    let man = b & 0x7F_FFFF;
    if exp == 0xFF {
        return sign | 0x7C00 | if man != 0 { 0x200 } else { 0 };
    }
    // rounds `m >> shift` to nearest, ties to even (`shift` is 1..=24)
    let round = |m: u32, shift: u32| {
        let (q, rem, mid) = (m >> shift, m & ((1 << shift) - 1), 1 << (shift - 1));
        if rem > mid || (rem == mid && q & 1 == 1) { q + 1 } else { q }
    };
    let e = exp - 127 + 15;
    if e >= 0x1F {
        sign | 0x7C00
    } else if e <= 0 {
        if e < -10 {
            return sign;
        }
        // subnormal half: the 24-bit significand scaled by 2^(e - 14); rounding up may carry
        // into the smallest normal, which is the right encoding
        sign | round(man | 0x80_0000, (14 - e) as u32) as u16
    } else {
        // a carry out of the mantissa bumps the exponent (up to infinity), as it should
        sign | round(((e as u32) << 23) | man, 13) as u16
    }
}

/// An OpenEXR file: half-float scene-linear Rec.709 RGB, or RGBA with `alpha` (premultiplied, as
/// EXR defines alpha; the frame's straight alpha is multiplied in after linearisation).
pub fn write_rgba8(rgba: &[u8], w: u32, h: u32, alpha: bool, compression: Compression, linear: &[f32; 256]) -> Result<Vec<u8>, Error> {
    let n = checked_len(rgba, w, h)?;
    let too_large = || Error::TooLarge;
    let (Ok(x_max), Ok(y_max)) = (i32::try_from(w - 1), i32::try_from(h - 1)) else {
        return Err(too_large());
    };
    // channels in the alphabetical order the file layout requires
    let names: &[u8] = if alpha { b"ABGR" } else { b"BGR" };
    let line = (w as usize).checked_mul(names.len() * 2).ok_or(Error::FrameSize)?;
    // a chunk's byte count is a signed 32-bit field
    let lines = compression.lines();
    if line.checked_mul(lines).and_then(|c| i32::try_from(c).ok()).is_none() {
        return Err(too_large());
    }

    let mut hd = Vec::with_capacity(512);
    hd.extend_from_slice(&[0x76, 0x2F, 0x31, 0x01]); // magic number 20000630
    hd.extend_from_slice(&[2, 0, 0, 0]); // version 2, single-part scan-line file, short names
    let attr = |hd: &mut Vec<u8>, name: &str, ty: &str, value: &[u8]| {
        for s in [name, ty] {
            hd.extend_from_slice(s.as_bytes());
            hd.push(0);
        }
        hd.extend_from_slice(&(value.len() as i32).to_le_bytes());
        hd.extend_from_slice(value);
    };
    let le = |v: &[f32]| v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>();
    let mut channels = Vec::new();
    for &c in names {
        channels.extend_from_slice(&[c, 0]);
        channels.extend_from_slice(&1_i32.to_le_bytes()); // pixel type HALF
        channels.extend_from_slice(&[0, 0, 0, 0]); // pLinear 0, reserved
        channels.extend_from_slice(&1_i32.to_le_bytes()); // x sampling
        channels.extend_from_slice(&1_i32.to_le_bytes()); // y sampling
    }
    channels.push(0);
    let window: Vec<u8> = [0, 0, x_max, y_max].iter().flat_map(|v| v.to_le_bytes()).collect();
    attr(&mut hd, "channels", "chlist", &channels);
    attr(&mut hd, "chromaticities", "chromaticities", &le(&[0.64, 0.33, 0.30, 0.60, 0.15, 0.06, 0.3127, 0.3290]));
    attr(&mut hd, "compression", "compression", &[compression.code()]);
    attr(&mut hd, "dataWindow", "box2i", &window);
    attr(&mut hd, "displayWindow", "box2i", &window);
    attr(&mut hd, "lineOrder", "lineOrder", &[0]); // INCREASING_Y
    attr(&mut hd, "pixelAspectRatio", "float", &le(&[1.0]));
    attr(&mut hd, "screenWindowCenter", "v2f", &le(&[0.0, 0.0]));
    attr(&mut hd, "screenWindowWidth", "float", &le(&[1.0]));
    hd.push(0); // end of header

    // offset table (one entry per chunk), then the chunks: first y, data size, data
    let chunks = (h as usize).div_ceil(lines);
    let table = chunks.checked_mul(8);
    let body = (h as usize).checked_mul(line + 8);
    let size = table.zip(body).and_then(|(t, b)| hd.len().checked_add(t)?.checked_add(b)).ok_or(Error::FrameSize)?;
    // the uncompressed size: compressed chunks only shrink it
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&hd);
    let table_at = out.len();
    out.resize(table_at + chunks * 8, 0);
    let lin = linear;
    let opaque: [u16; 256] = std::array::from_fn(|i| half(lin[i]));
    let row = w as usize * 4;
    let wn = w as usize;
    let mut px = vec![0_u16; wn * names.len()];
    let mut raw = Vec::with_capacity(line * lines);
    // the deflate hash tables, shared by the frame's chunks instead of zeroed for each one
    let mut tables = crate::zlib::Tables::default();
    for (k, block) in rgba[..n].chunks(row * lines).enumerate() {
        raw.clear();
        for src in block.chunks_exact(row) {
            // planar per line: every pixel of the first channel, then the next channel …
            for (x, &[r, g, b, a]) in src.as_chunks::<4>().0.iter().enumerate() {
                let rgb = [b, g, r];
                if alpha {
                    let k = f32::from(a) / 255.0;
                    px[x] = half(k);
                    for (c, &v) in rgb.iter().enumerate() {
                        px[(c + 1) * wn + x] = if a == 255 { opaque[v as usize] } else { half(lin[v as usize] * k) };
                    }
                } else {
                    for (c, &v) in rgb.iter().enumerate() {
                        px[c * wn + x] = opaque[v as usize];
                    }
                }
            }
            raw.extend(px.iter().flat_map(|v| v.to_le_bytes()));
        }
        let packed = match compression {
            Compression::None => None,
            Compression::Rle => Some(rle(&predict(&raw))),
            Compression::Zips | Compression::Zip => Some(crate::zlib::zlib(&predict(&raw), &mut tables)),
        };
        // a chunk that does not shrink is stored as it is (readers compare the sizes)
        let data = packed.as_deref().filter(|p| p.len() < raw.len()).unwrap_or(&raw);
        let offset = (out.len() as u64).to_le_bytes();
        out[table_at + k * 8..table_at + k * 8 + 8].copy_from_slice(&offset);
        out.extend_from_slice(&((k * lines) as i32).to_le_bytes());
        out.extend_from_slice(&(data.len() as i32).to_le_bytes());
        out.extend_from_slice(data);
    }
    Ok(out)
}

/// The OpenEXR RLE / ZIP predictor: bytes split into even and odd halves, then each byte replaced
/// by its difference from the previous one, plus 128.
fn predict(raw: &[u8]) -> Vec<u8> {
    let half = raw.len().div_ceil(2);
    let mut t = vec![0_u8; raw.len()];
    for (i, &b) in raw.iter().enumerate() {
        t[if i % 2 == 0 { i / 2 } else { half + i / 2 }] = b;
    }
    let mut p = t.first().copied().unwrap_or(0);
    for v in t.iter_mut().skip(1) {
        let d = v.wrapping_sub(p).wrapping_add(128);
        p = *v;
        *v = d;
    }
    t
}

/// OpenEXR run-length coding: a signed count byte, then either `count + 1` copies of one byte
/// (count ≥ 0, runs of 3 to 128) or `-count` literal bytes (1 to 127).
pub fn rle(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() + src.len() / 127 + 1);
    let mut i = 0;
    while i < src.len() {
        let b = src[i];
        let run = src[i..].iter().take(128).take_while(|&&x| x == b).count();
        if run >= 3 {
            out.extend_from_slice(&[(run - 1) as u8, b]);
            i += run;
            continue;
        }
        // literals up to the next run of three (or 127 bytes)
        let s = i;
        while i < src.len() && i - s < 127 {
            if src.get(i..i + 3).is_some_and(|t| t[0] == t[1] && t[1] == t[2]) {
                break;
            }
            i += 1;
        }
        out.push(((i - s) as u8).wrapping_neg());
        out.extend_from_slice(&src[s..i]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writes_all_scanline_compressions() {
        let linear = std::array::from_fn(|i| i as f32 / 255.0);
        for c in [Compression::None, Compression::Rle, Compression::Zips, Compression::Zip] {
            let bytes = write_rgba8(&[128; 16 * 16 * 4], 16, 16, true, c, &linear).unwrap();
            assert_eq!(&bytes[..4], &[0x76, 0x2f, 0x31, 1]);
            assert!(bytes.len() > 100);
        }
        assert_eq!(write_rgba8(&[], 1, 1, false, Compression::None, &linear), Err(Error::FrameSize));
    }
}
