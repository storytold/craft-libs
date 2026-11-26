//! Per-block compression schemes: NONE, RLE (read only), ZIPS and ZIP. RLE and ZIP share the same preprocessing:
//! bytes are split into even/odd halves and delta-coded before the entropy stage.

use crate::deflate::{self, Level};

/// Block compression of a scanline file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    /// Run-length encoding (decode only).
    Rle,
    /// zlib, one scanline per block.
    Zips,
    /// zlib, sixteen scanlines per block.
    Zip,
}

impl Compression {
    pub(super) fn code(self) -> u8 {
        match self {
            Compression::None => 0,
            Compression::Rle => 1,
            Compression::Zips => 2,
            Compression::Zip => 3,
        }
    }

    pub(super) fn from_code(c: u8) -> Result<Self, String> {
        match c {
            0 => Ok(Compression::None),
            1 => Ok(Compression::Rle),
            2 => Ok(Compression::Zips),
            3 => Ok(Compression::Zip),
            4 => Err("PIZ compression is not supported".into()),
            5 => Err("PXR24 compression is not supported".into()),
            6 | 7 => Err("B44 compression is not supported".into()),
            8 | 9 => Err("DWA compression is not supported".into()),
            _ => Err(format!("unknown compression {c}")),
        }
    }

    /// Scanlines per block.
    pub(super) fn lines_per_block(self) -> usize {
        match self {
            Compression::Zip => 16,
            _ => 1,
        }
    }

    /// Upper bound of uncompressed/compressed size, used to reject absurd headers before allocating.
    pub(super) fn max_expansion(self) -> usize {
        match self {
            Compression::None => 1,
            Compression::Rle => 128,
            Compression::Zips | Compression::Zip => 1032,
        }
    }

    /// Compress one block of raw (little-endian, planar per scanline) bytes. Returns the raw bytes when compressing
    /// would not make them smaller, as the format requires.
    pub(super) fn compress(self, raw: &[u8]) -> Vec<u8> {
        let packed = match self {
            Compression::None | Compression::Rle => return raw.to_vec(),
            Compression::Zips | Compression::Zip => deflate::zlib_compress(&predict(&interleave(raw)), Level::Default),
        };
        if packed.len() < raw.len() { packed } else { raw.to_vec() }
    }

    /// Inverse of `compress`; `expected` is the raw block size.
    pub(super) fn decompress(self, data: &[u8], expected: usize) -> Result<Vec<u8>, String> {
        if data.len() == expected || self == Compression::None {
            return if data.len() == expected { Ok(data.to_vec()) } else { Err("block size mismatch".into()) };
        }
        let mut t = match self {
            Compression::Rle => rle_decode(data, expected)?,
            _ => match (data.first(), data.get(2..)) {
                (Some(&cmf), Some(body)) if data.len() >= 6 && cmf & 0x0f == 8 => crate::deflate::inflate(body)?,
                _ => return Err("bad zlib block".into()),
            },
        };
        if t.len() != expected {
            return Err("block decompressed to the wrong size".into());
        }
        unpredict(&mut t);
        Ok(deinterleave(&t))
    }
}

/// First half: bytes 0, 2, 4...; second half: bytes 1, 3, 5...
fn interleave(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    out.extend(raw.iter().step_by(2));
    out.extend(raw.iter().skip(1).step_by(2));
    out
}

fn deinterleave(t: &[u8]) -> Vec<u8> {
    // Even output bytes come from the first half, odd ones from the second (one shorter for odd lengths).
    let (even, odd) = t.split_at(t.len().div_ceil(2));
    let mut out = Vec::with_capacity(t.len());
    let mut odd = odd.iter();
    for &e in even {
        out.push(e);
        out.extend(odd.next());
    }
    out
}

fn predict(t: &[u8]) -> Vec<u8> {
    let mut out = t.to_vec();
    for (o, (&prev, &cur)) in out.iter_mut().skip(1).zip(t.iter().zip(t.iter().skip(1))) {
        *o = cur.wrapping_sub(prev).wrapping_add(128);
    }
    out
}

fn unpredict(t: &mut [u8]) {
    let Some((first, rest)) = t.split_first_mut() else {
        return;
    };
    let mut prev = *first;
    for x in rest {
        *x = prev.wrapping_add(*x).wrapping_sub(128);
        prev = *x;
    }
}

/// OpenEXR RLE: a negative count -n is followed by n literal bytes; a count n >= 0 by one byte repeated n + 1 times.
fn rle_decode(data: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(expected);
    let mut i = 0;
    while let Some(&count) = data.get(i) {
        let n = count as i8;
        i += 1;
        if n < 0 {
            let run = data.get(i..i + (-(n as i32)) as usize).ok_or("truncated RLE block")?;
            out.extend_from_slice(run);
            i += run.len();
        } else {
            let b = *data.get(i).ok_or("truncated RLE block")?;
            out.extend(std::iter::repeat_n(b, n as usize + 1));
            i += 1;
        }
        if out.len() > expected {
            return Err("RLE block too long".into());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocessing_inverts() {
        for n in [0usize, 1, 2, 7, 100] {
            let raw: Vec<u8> = (0..n).map(|i| (i * 37 % 256) as u8).collect();
            let mut p = predict(&interleave(&raw));
            unpredict(&mut p);
            assert_eq!(deinterleave(&p), raw);
        }
    }

    #[test]
    fn rle_decodes() {
        // Reference RLE of [1,1,1,1,2,3]: run of 4 ones (count 3), then two literals (count -2).
        let mut packed = vec![3u8, 1, (-2i8) as u8, 2, 3];
        let pre = rle_decode(&packed, 6).unwrap();
        assert_eq!(pre, [1, 1, 1, 1, 2, 3]);
        packed.pop();
        assert!(rle_decode(&packed, 6).is_err());
    }

    #[test]
    fn zip_block_round_trip() {
        let raw: Vec<u8> = (0..4096).map(|i| ((i / 64) as u8).wrapping_mul(3)).collect();
        for c in [Compression::Zips, Compression::Zip] {
            let z = c.compress(&raw);
            assert!(z.len() < raw.len() / 4);
            assert_eq!(c.decompress(&z, raw.len()).unwrap(), raw);
        }
    }
}
