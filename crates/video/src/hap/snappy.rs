//! Snappy raw (unframed) compression, written from the public Snappy format description: a varint
//! of the uncompressed length, then literal and copy elements. Hap uses it as the second-stage
//! compressor of its texture data.

use super::Error;

/// Decompress `src`, which must expand to exactly `expected` bytes. The length in the preamble is
/// checked before anything is allocated, so a hostile stream cannot ask for more than `expected`.
pub(super) fn decompress(src: &[u8], expected: usize) -> Result<Vec<u8>, Error> {
    let bad = || Error::Invalid("Snappy data is damaged".into());
    // The varint preamble.
    let (mut len, mut shift, mut pos) = (0u64, 0u32, 0usize);
    loop {
        let b = *src.get(pos).ok_or_else(bad)?;
        pos += 1;
        len |= u64::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 28 {
            return Err(bad());
        }
    }
    if len != expected as u64 {
        return Err(Error::Invalid(format!("Snappy data expands to {len} bytes, expected {expected}")));
    }
    let mut out: Vec<u8> = Vec::with_capacity(expected);
    while pos < src.len() {
        let tag = src[pos];
        pos += 1;
        match tag & 3 {
            0 => {
                let mut n = usize::from(tag >> 2);
                if n >= 60 {
                    let extra = n - 59;
                    let bytes = src.get(pos..pos + extra).ok_or_else(bad)?;
                    n = bytes.iter().rev().fold(0usize, |a, &b| (a << 8) | usize::from(b));
                    pos += extra;
                }
                n += 1;
                let lit = src.get(pos..pos.checked_add(n).ok_or_else(bad)?).ok_or_else(bad)?;
                if out.len() + n > expected {
                    return Err(bad());
                }
                out.extend_from_slice(lit);
                pos += n;
            }
            kind => {
                let (n, offset) = match kind {
                    1 => {
                        let b = *src.get(pos).ok_or_else(bad)?;
                        pos += 1;
                        (4 + usize::from((tag >> 2) & 7), (usize::from(tag >> 5) << 8) | usize::from(b))
                    }
                    2 => {
                        let b = src.get(pos..pos + 2).ok_or_else(bad)?;
                        pos += 2;
                        (usize::from(tag >> 2) + 1, usize::from(u16::from_le_bytes([b[0], b[1]])))
                    }
                    _ => {
                        let b = src.get(pos..pos + 4).ok_or_else(bad)?;
                        pos += 4;
                        (usize::from(tag >> 2) + 1, u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
                    }
                };
                if offset == 0 || offset > out.len() || out.len() + n > expected {
                    return Err(bad());
                }
                // Copies may overlap their own output, so go byte by byte.
                let start = out.len() - offset;
                for i in 0..n {
                    let b = out[start + i];
                    out.push(b);
                }
            }
        }
    }
    if out.len() != expected {
        return Err(bad());
    }
    Ok(out)
}

fn push_literal(out: &mut Vec<u8>, lit: &[u8]) {
    let mut rest = lit;
    while !rest.is_empty() {
        let n = rest.len().min(1 << 24);
        let (now, later) = rest.split_at(n);
        let m = n - 1;
        if m < 60 {
            out.push((m as u8) << 2);
        } else if m < 1 << 8 {
            out.extend_from_slice(&[60 << 2, m as u8]);
        } else if m < 1 << 16 {
            out.extend_from_slice(&[61 << 2, m as u8, (m >> 8) as u8]);
        } else {
            out.extend_from_slice(&[62 << 2, m as u8, (m >> 8) as u8, (m >> 16) as u8]);
        }
        out.extend_from_slice(now);
        rest = later;
    }
}

fn push_copy(out: &mut Vec<u8>, offset: usize, mut len: usize) {
    while len > 0 {
        // Copies of 4 to 11 bytes at offsets below 2048 take two bytes; the rest take three.
        let n = if len >= 68 {
            64
        } else if len > 64 {
            60
        } else {
            len
        };
        if (4..=11).contains(&n) && offset < 2048 {
            out.push(1 | (((n - 4) as u8) << 2) | (((offset >> 8) as u8) << 5));
            out.push(offset as u8);
        } else {
            out.push(2 | (((n - 1) as u8) << 2));
            out.extend_from_slice(&(offset as u16).to_le_bytes());
        }
        len -= n;
    }
}

/// Compress `src` into a Snappy stream: greedy matching over a hash of four bytes, offsets up to
/// 64 KiB.
pub(super) fn compress(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() / 2 + 16);
    let mut n = src.len();
    loop {
        let b = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
    const BITS: u32 = 15;
    let mut table = vec![usize::MAX; 1 << BITS];
    let hash = |i: usize| -> usize {
        let v = u32::from_le_bytes([src[i], src[i + 1], src[i + 2], src[i + 3]]);
        (v.wrapping_mul(0x1E35_A7BD) >> (32 - BITS)) as usize
    };
    let (mut i, mut lit_start) = (0usize, 0usize);
    while i + 4 <= src.len() {
        let h = hash(i);
        let cand = table[h];
        table[h] = i;
        if cand != usize::MAX && i - cand <= 65535 && src[cand..cand + 4] == src[i..i + 4] {
            let mut len = 4;
            while i + len < src.len() && src[cand + len] == src[i + len] {
                len += 1;
            }
            push_literal(&mut out, &src[lit_start..i]);
            push_copy(&mut out, i - cand, len);
            i += len;
            lit_start = i;
        } else {
            i += 1;
        }
    }
    push_literal(&mut out, &src[lit_start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_assorted_data() {
        let mut seed = 1u32;
        let mut noise = Vec::new();
        for _ in 0..5000 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            noise.push(seed as u8);
        }
        let cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![7],
            vec![1, 2, 3],
            vec![0; 100_000],
            (0..70_000u32).map(|i| (i % 251) as u8).collect(),
            noise.clone(),
            [noise.clone(), noise].concat(),
            b"abcabcabcabcabcabcabcabcabcabcabcabcabcabcabc".repeat(20),
        ];
        for data in cases {
            let packed = compress(&data);
            assert_eq!(decompress(&packed, data.len()).expect("decompress"), data, "len {}", data.len());
        }
    }

    #[test]
    fn repetitive_data_gets_smaller() {
        assert!(compress(&vec![5u8; 50_000]).len() < 3_000);
    }

    #[test]
    fn decodes_the_format_description_forms() {
        // A literal, then a 1-byte-offset copy that overlaps its own output.
        let s = [9, 0x0C, b'a', b'b', b'c', b'd', 0b0000_0101, 4];
        // "abcd" + copy len 4+... : length field (5>>2)&7 = 1 -> 5 bytes, offset 4 -> total 9.
        assert_eq!(decompress(&s, 9).expect("decode"), b"abcdabcda");
        // A literal of 61 bytes uses the extra-length form.
        let mut s = vec![61, 60 << 2, 60];
        s.extend_from_slice(&[3; 61]);
        assert_eq!(decompress(&s, 61).expect("decode"), vec![3; 61]);
    }

    #[test]
    fn hostile_streams_are_errors() {
        // Wrong length in the preamble.
        assert!(decompress(&[5, 0x00, b'x'], 10).is_err());
        // Copy before any output.
        assert!(decompress(&[4, 0b0000_0001, 1], 4).is_err());
        // Offset of zero.
        assert!(decompress(&[8, 0x0C, b'a', b'b', b'c', b'd', 0b0000_0001, 0], 8).is_err());
        // Literal longer than the stream.
        assert!(decompress(&[4, 0x0C, b'a'], 4).is_err());
        // Overlong varint.
        assert!(decompress(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01], 0).is_err());
        // Output past the stated length.
        assert!(decompress(&[2, 0x0C, b'a', b'b', b'c', b'd'], 2).is_err());
        // Too short.
        assert!(decompress(&[4, 0x00, b'a'], 4).is_err());
        assert!(decompress(&[], 0).is_err());
    }
}
