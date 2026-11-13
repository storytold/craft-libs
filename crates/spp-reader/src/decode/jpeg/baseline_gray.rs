//! Baseline grayscale JPEG, the common form of images stored in compiled graphs. Decoded here rather than by a library for
//! its inverse DCT: the original's output is nearer to the exact transform, rounded to the nearest level (99.0 % of the pixels
//! of a 2048 x 2048 image, against 97.8 % for the fixed-point transforms of libjpeg or stb_image, measured on tapped nodes;
//! the rest are one level off at values within .05 of a half). Those one-level differences, carried through a graph, were what
//! kept many bitmap graphs under 99 % within one step.

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29,
    22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// Canonical Huffman decoding: for each code length the first code, how many there are and where their symbols start.
#[derive(Default, Clone)]
struct Huffman {
    first: [i32; 17],
    count: [i32; 17],
    offset: [i32; 17],
    symbols: Vec<u8>,
}

impl Huffman {
    fn new(bits: &[u8], symbols: &[u8]) -> Huffman {
        let mut h = Huffman { symbols: symbols.to_vec(), ..Huffman::default() };
        let (mut code, mut at) = (0i32, 0i32);
        // code lengths 1 to 16 go in entries 1 to 16
        let lengths = h.first.iter_mut().zip(h.count.iter_mut()).zip(h.offset.iter_mut()).skip(1);
        for (((first, count), offset), c) in lengths.zip(bits.iter().take(16)) {
            *first = code;
            *count = i32::from(*c);
            *offset = at;
            code = (code + i32::from(*c)) << 1;
            at += i32::from(*c);
        }
        h
    }
}

/// Entropy-coded bits, with the stuffed zeros taken out.
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    acc: u32,
    n: u32,
}

impl Bits<'_> {
    fn bit(&mut self) -> Option<u32> {
        if self.n == 0 {
            let b = *self.data.get(self.at)?;
            if b == 0xff {
                match self.data.get(self.at + 1)? {
                    0 => self.at += 2,
                    // a marker: nothing more in this interval, as a decoder pads with zeros
                    _ => {
                        self.acc = 0;
                        self.n = 8;
                        self.n -= 1;
                        return Some(0);
                    }
                }
            } else {
                self.at += 1;
            }
            self.acc = u32::from(b);
            self.n = 8;
        }
        self.n -= 1;
        Some((self.acc >> self.n) & 1)
    }

    fn take(&mut self, k: u32) -> Option<i32> {
        let mut v = 0;
        for _ in 0..k {
            v = v << 1 | self.bit()? as i32;
        }
        Some(v)
    }

    fn symbol(&mut self, h: &Huffman) -> Option<u8> {
        let mut code = 0i32;
        for ((first, count), offset) in h.first.iter().zip(&h.count).zip(&h.offset).skip(1) {
            code = code << 1 | self.bit()? as i32;
            let k = code - first;
            if k >= 0 && k < *count {
                return h.symbols.get((offset + k) as usize).copied();
            }
        }
        None
    }

    /// A value of `k` bits as JPEG signs it.
    fn signed(&mut self, k: u32) -> Option<i32> {
        if k == 0 {
            return Some(0);
        }
        // a coefficient takes at most 16 bits; a larger size is a damaged stream (and would overflow the shifts)
        if k > 16 {
            return None;
        }
        let v = self.take(k)?;
        Some(if v < 1 << (k - 1) { v - (1 << k) + 1 } else { v })
    }

    /// Skips to just after the next restart marker.
    fn restart(&mut self) -> Option<()> {
        self.n = 0;
        loop {
            if *self.data.get(self.at)? == 0xff && (0xd0..=0xd7).contains(self.data.get(self.at + 1)?) {
                self.at += 2;
                return Some(());
            }
            self.at += 1;
        }
    }
}

// Format data: JPEG markers (ITU T.81, table B.1).
/// Start of image, the first two bytes of every JPEG.
pub(crate) const SOI: [u8; 2] = [0xff, 0xd8];
/// Every marker starts with this byte.
const MARKER: u8 = 0xff;
/// Quantisation tables.
const DQT: u8 = 0xdb;
/// Huffman tables.
const DHT: u8 = 0xc4;
/// Baseline frame header.
const SOF0: u8 = 0xc0;
/// Restart interval.
const DRI: u8 = 0xdd;
/// Start of scan.
const SOS: u8 = 0xda;

/// Samples of a JPEG if it is baseline, 8 bits and one channel: (width, height, samples); `None` for anything else.
pub fn decode(jpeg: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    if jpeg.get(..2)? != SOI {
        return None;
    }
    let (mut at, mut restart) = (2usize, 0usize);
    let mut quant = [[0i32; 64]; 4];
    let mut tables: [Huffman; 8] = Default::default();
    let (mut w, mut h, mut tq) = (0usize, 0usize, 0usize);
    let scan = loop {
        if *jpeg.get(at)? != MARKER {
            return None;
        }
        let marker = *jpeg.get(at + 1)?;
        let len = usize::from(*jpeg.get(at + 2)?) << 8 | usize::from(*jpeg.get(at + 3)?);
        let body = jpeg.get(at + 4..at + 2 + len)?;
        match marker {
            DQT => {
                let mut p = 0;
                while p < body.len() {
                    let spec = *body.get(p)?;
                    if spec >> 4 != 0 {
                        return None;
                    }
                    let table = quant.get_mut(usize::from(spec & 3))?;
                    for (slot, v) in table.iter_mut().zip(body.get(p + 1..p + 65)?) {
                        *slot = i32::from(*v);
                    }
                    p += 65;
                }
            }
            DHT => {
                let mut p = 0;
                while p < body.len() {
                    let spec = *body.get(p)?;
                    let bits = body.get(p + 1..p + 17)?;
                    let n: usize = bits.iter().map(|b| usize::from(*b)).sum();
                    *tables.get_mut(usize::from((spec >> 4 & 1) * 4 + (spec & 3)))? = Huffman::new(bits, body.get(p + 17..p + 17 + n)?);
                    p += 17 + n;
                }
            }
            SOF0 => {
                // one 8-bit channel, sampled 1 x 1
                if *body.first()? != 8 || *body.get(5)? != 1 {
                    return None;
                }
                h = usize::from(*body.get(1)?) << 8 | usize::from(*body.get(2)?);
                w = usize::from(*body.get(3)?) << 8 | usize::from(*body.get(4)?);
                tq = usize::from(*body.get(8)? & 3);
            }
            DRI => restart = usize::from(*body.first()?) << 8 | usize::from(*body.get(1)?),
            SOS => break (at + 2 + len, *body.get(2)?),
            // other frame types: not this decoder's business
            0xc1..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => return None,
            _ => {}
        }
        at += 2 + len;
    };
    if w == 0 || h == 0 {
        return None;
    }
    let (start, selector) = scan;
    let dc = tables.get(usize::from(selector >> 4 & 3))?;
    let ac = tables.get(4 + usize::from(selector & 3))?;
    let q = *quant.get(tq)?;
    let basis: [[f64; 8]; 8] = std::array::from_fn(|x| {
        std::array::from_fn(|u| {
            (std::f64::consts::PI * (2 * x + 1) as f64 * u as f64 / 16.0).cos() * if u == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 } * 0.5
        })
    });
    let (columns, rows) = (w.div_ceil(8), h.div_ceil(8));
    let mut out = vec![0u8; w * h];
    let mut bits = Bits { data: jpeg, at: start, acc: 0, n: 0 };
    let mut pred = 0i32;
    for block in 0..columns * rows {
        if restart != 0 && block != 0 && block % restart == 0 {
            bits.restart()?;
            pred = 0;
        }
        let mut c = [0f64; 64];
        let s = u32::from(bits.symbol(dc)?);
        pred = pred.checked_add(bits.signed(s)?)?;
        c[0] = f64::from(pred.checked_mul(q[0])?);
        let mut k = 1;
        let mut flat = true;
        while k < 64 {
            let rs = bits.symbol(ac)?;
            let (run, size) = (usize::from(rs >> 4), u32::from(rs & 15));
            if size == 0 {
                if run == 15 {
                    k += 16;
                    continue;
                }
                break;
            }
            k += run;
            if k > 63 {
                return None;
            }
            let value = f64::from(bits.signed(size)?.checked_mul(*q.get(k)?)?);
            *c.get_mut(*ZIGZAG.get(k)?)? = value;
            flat = false;
            k += 1;
        }
        let (bx, by) = (block % columns, block / columns);
        let mut rows_done = [0f64; 64];
        if !flat {
            for (row, basis_y) in rows_done.as_chunks_mut::<8>().0.iter_mut().zip(&basis) {
                for (u, out) in row.iter_mut().enumerate() {
                    *out = basis_y.iter().zip(c.as_chunks::<8>().0).map(|(b, cv)| b * cv.get(u).copied().unwrap_or(0.0)).sum();
                }
            }
        }
        for (y, row) in rows_done.as_chunks::<8>().0.iter().enumerate() {
            let py = by * 8 + y;
            if py >= h {
                break;
            }
            for (x, basis_x) in basis.iter().enumerate() {
                let px = bx * 8 + x;
                if px >= w {
                    break;
                }
                let v: f64 = if flat { c[0] / 8.0 } else { basis_x.iter().zip(row).map(|(b, r)| b * r).sum() };
                *out.get_mut(py * w + px)? = (v + 128.0 + 0.5).floor().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Some((w, h, out))
}

#[cfg(test)]
mod tests {
    use super::decode;

    /// A 16 x 8 image of two blocks, by hand: DC only, table of ones. Huffman: DC symbols 0 and 4 and 6, AC only end of block.
    #[test]
    fn two_flat_blocks_come_out_at_their_levels() {
        let mut j = vec![0xff, 0xd8, 0xff, 0xdb, 0, 67, 0];
        j.extend([1u8; 64]);
        // frame: 8 bits, 8 rows, 16 columns, one channel
        j.extend([0xff, 0xc0, 0, 11, 8, 0, 8, 0, 16, 1, 1, 0x11, 0]);
        // DC table: codes 0 -> 0 and 10 -> 8 (2 bits), AC table: code 0 -> end of block
        j.extend([0xff, 0xc4, 0, 21, 0x00, 0, 2]);
        j.extend([0u8; 14]);
        j.extend([0, 8]);
        j.extend([0xff, 0xc4, 0, 20, 0x10, 1]);
        j.extend([0u8; 15]);
        j.push(0);
        j.extend([0xff, 0xda, 0, 8, 1, 1, 0x00, 0, 63, 0]);
        // block 1: DC symbol 8 = code 01, value 0b11001000 = 200 -> 200 / 8 = 25 above 128; AC end of block 0.
        // block 2: difference symbol 0 = code 00, AC 0. Bits: 01 11001000 0 00 0, padded.
        j.extend([0b0111_0010, 0b0000_0000, 0xff, 0xd9]);
        let (w, h, s) = decode(&j).unwrap();
        assert_eq!((w, h), (16, 8));
        assert_eq!((s[0], s[15], s[127]), (153, 153, 153));
    }

    #[test]
    fn anything_else_is_left_to_the_library() {
        assert!(decode(&[0xff, 0xd8, 0xff, 0xc2, 0, 2]).is_none());
        assert!(decode(b"nope").is_none());
    }
}
