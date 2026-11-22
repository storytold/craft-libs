//! DEFLATE decompression (RFC 1951), for the zlib-compressed parts of the reference app's records (the mesh scene).
//! A copy of our own decoder (`src/deflate`), kept here so the reader depends on no application crate.

mod tables;

use tables::{CODE_LENGTH_ORDER, DIST_BASE as DBASE, DIST_EXTRA as DEXT, LENGTH_BASE as LBASE, LENGTH_EXTRA as LEXT};

struct BitR<'a> {
    d: &'a [u8],
    pos: usize,
    bit: u32,
    cnt: u32,
}

impl<'a> BitR<'a> {
    fn bits(&mut self, n: u32) -> Result<u32, String> {
        while self.cnt < n {
            let b = *self.d.get(self.pos).ok_or_else(|| "unexpected end of compressed data".to_string())?;
            self.bit |= (b as u32) << self.cnt;
            self.pos += 1;
            self.cnt += 8;
        }
        let v = self.bit & ((1u32 << n) - 1);
        self.bit >>= n;
        self.cnt -= n;
        Ok(v)
    }
}

struct Huff {
    /// Number of codes of each bit length (index 0 unused).
    count: [u16; 16],
    /// Symbols ordered by code.
    symbol: Vec<u16>,
}

fn build(lengths: &[u8]) -> Result<Huff, String> {
    let mut count = [0u16; 16];
    for &l in lengths {
        *count.get_mut(usize::from(l)).ok_or("bad code length")? += 1;
    }
    // First position in `symbol` of the codes of each length.
    let mut offs = [0u16; 16];
    let mut running = 0u16;
    for (off, &n) in offs.iter_mut().zip(&count).skip(1) {
        *off = running;
        running += n;
    }
    let mut symbol = vec![0u16; lengths.len()];
    for (s, &l) in lengths.iter().enumerate() {
        if l != 0 {
            let off = offs.get_mut(usize::from(l)).ok_or("bad code length")?;
            *symbol.get_mut(usize::from(*off)).ok_or("bad code lengths")? = s as u16;
            *off += 1;
        }
    }
    Ok(Huff { count, symbol })
}

fn decode_sym(br: &mut BitR, h: &Huff) -> Result<u16, String> {
    let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
    // Code lengths 1 to 15.
    for &count in h.count.iter().skip(1) {
        code |= br.bits(1)? as i32;
        let count = i32::from(count);
        if code - count < first {
            return usize::try_from(index + (code - first)).ok().and_then(|i| h.symbol.get(i)).copied().ok_or_else(|| "bad huffman code".to_string());
        }
        index += count;
        first += count;
        first <<= 1;
        code <<= 1;
    }
    Err("bad huffman code".into())
}

fn codes(br: &mut BitR, out: &mut Vec<u8>, lh: &Huff, dh: &Huff) -> Result<(), String> {
    loop {
        let sym = decode_sym(br, lh)? as usize;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let s = sym - 257;
            let (&base, &extra) = LBASE.get(s).zip(LEXT.get(s)).ok_or("bad length symbol")?;
            let len = base as usize + br.bits(u32::from(extra))? as usize;
            let ds = decode_sym(br, dh)? as usize;
            let (&base, &extra) = DBASE.get(ds).zip(DEXT.get(ds)).ok_or("bad distance symbol")?;
            let dist = base as usize + br.bits(u32::from(extra))? as usize;
            let from = out.len().checked_sub(dist).ok_or("distance too far back")?;
            // Byte by byte: the copy may overlap the bytes it appends.
            for i in from..from + len {
                let b = *out.get(i).ok_or("distance too far back")?;
                out.push(b);
            }
        }
        if out.len() > (1 << 30) {
            return Err("decompressed data too large".into());
        }
    }
}

pub fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut br = BitR { d: data, pos: 0, bit: 0, cnt: 0 };
    let mut out = Vec::new();
    loop {
        let last = br.bits(1)?;
        match br.bits(2)? {
            0 => {
                br.bit = 0;
                br.cnt = 0;
                let (header, rest) = data.get(br.pos..).and_then(<[u8]>::split_first_chunk::<4>).ok_or("truncated stored block")?;
                let [l0, l1, n0, n1] = *header;
                let len = usize::from(u16::from_le_bytes([l0, l1]));
                let nlen = usize::from(u16::from_le_bytes([n0, n1]));
                let stored = rest.get(..len).filter(|_| len == (!nlen & 0xffff)).ok_or("bad stored block")?;
                out.extend_from_slice(stored);
                br.pos += 4 + len;
            }
            1 => {
                let mut l = [0u8; 288];
                for (i, v) in l.iter_mut().enumerate() {
                    *v = if i < 144 {
                        8
                    } else if i < 256 {
                        9
                    } else if i < 280 {
                        7
                    } else {
                        8
                    };
                }
                codes(&mut br, &mut out, &build(&l)?, &build(&[5u8; 30])?)?;
            }
            2 => {
                let nlen = br.bits(5)? as usize + 257;
                let ndist = br.bits(5)? as usize + 1;
                let ncode = br.bits(4)? as usize + 4;
                if nlen > 286 || ndist > 30 {
                    return Err("bad dynamic header".into());
                }
                let mut lengths = [0u8; 19];
                for &o in CODE_LENGTH_ORDER.iter().take(ncode) {
                    *lengths.get_mut(o).ok_or("bad dynamic header")? = br.bits(3)? as u8;
                }
                let lc = build(&lengths)?;
                let total = nlen + ndist;
                let mut ll = Vec::with_capacity(total);
                while ll.len() < total {
                    let sym = decode_sym(&mut br, &lc)?;
                    if sym < 16 {
                        ll.push(sym as u8);
                    } else {
                        let (prev, rep) = match sym {
                            16 => (*ll.last().ok_or("bad repeat")?, 3 + br.bits(2)? as usize),
                            17 => (0, 3 + br.bits(3)? as usize),
                            _ => (0, 11 + br.bits(7)? as usize),
                        };
                        if ll.len() + rep > total {
                            return Err("too many lengths".into());
                        }
                        ll.resize(ll.len() + rep, prev);
                    }
                }
                let (lit, dist) = ll.split_at_checked(nlen).ok_or("bad dynamic header")?;
                codes(&mut br, &mut out, &build(lit)?, &build(dist)?)?;
            }
            _ => return Err("bad block type".into()),
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::inflate;

    #[test]
    fn raw_deflate_round_trips_and_bad_input_is_an_error() {
        let compressed: &[u8] = &[
            11, 201, 72, 85, 72, 203, 204, 73, 85, 200, 78, 77, 45, 40, 86, 200, 44, 41, 86, 200, 77, 45, 206, 80, 72, 73, 77, 203, 73, 44, 73, 77,
            177, 86, 8, 25, 85, 49, 170, 98, 192, 85, 0, 0,
        ];
        let expected = b"The file keeps its mesh deflated; ".repeat(20);
        assert_eq!(inflate(compressed).as_deref(), Ok(expected.as_slice()));
        assert!(inflate(&[0xff, 0xff]).is_err());
        assert!(inflate(&[]).is_err());
    }

    #[test]
    fn stored_blocks_are_copied_and_checked() {
        assert_eq!(inflate(&[0x01, 3, 0, 0xfc, 0xff, b'a', b'b', b'c']).as_deref(), Ok(&b"abc"[..]));
        // Length and its complement disagree.
        assert!(inflate(&[0x01, 3, 0, 0, 0, b'a', b'b', b'c']).is_err());
        // Fewer bytes than the length says.
        assert!(inflate(&[0x01, 5, 0, 0xfa, 0xff, b'a']).is_err());
        // Header cut short.
        assert!(inflate(&[0x01, 3, 0]).is_err());
    }

    #[test]
    fn bad_block_type_and_back_reference_before_the_start_are_errors() {
        assert_eq!(inflate(&[0x07]), Err("bad block type".to_string()));
        // Fixed block whose first symbol is a length (257) with distance 1, but nothing has been output yet.
        assert_eq!(inflate(&[0x03, 0x02, 0x00]), Err("distance too far back".to_string()));
    }

    #[test]
    fn every_truncation_of_a_valid_stream_is_an_error() {
        let compressed: &[u8] = &[
            11, 201, 72, 85, 72, 203, 204, 73, 85, 200, 78, 77, 45, 40, 86, 200, 44, 41, 86, 200, 77, 45, 206, 80, 72, 73, 77, 203, 73, 44, 73, 77,
            177, 86, 8, 25, 85, 49, 170, 98, 192, 85, 0, 0,
        ];
        for end in 0..compressed.len() - 1 {
            assert!(inflate(&compressed[..end]).is_err(), "prefix of {end} bytes");
        }
    }
}
