//! DEFLATE decompression (RFC 1951), the counterpart of [`super::compress`]. Used by PNG, ZIP and OpenEXR.

use super::tables::{CODE_LENGTH_ORDER, DIST_BASE as DBASE, DIST_EXTRA as DEXT, LENGTH_BASE as LBASE, LENGTH_EXTRA as LEXT};

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
    count: [u16; 16],
    symbol: Vec<u16>,
}

fn build(lengths: &[u8]) -> Huff {
    let mut count = [0u16; 16];
    for &l in lengths {
        if let Some(c) = count.get_mut(l as usize & 15) {
            *c += 1;
        }
    }
    // offs[i + 1] = offs[i] + count[i] for i in 1..15 (offs[0] = offs[1] = 0).
    let mut offs = [0u16; 16];
    let mut acc = 0u16;
    for (o, &c) in offs.iter_mut().skip(2).zip(count.iter().skip(1)) {
        acc += c;
        *o = acc;
    }
    let mut symbol = vec![0u16; lengths.len()];
    for (s, &l) in lengths.iter().enumerate() {
        if l != 0
            && let Some(o) = offs.get_mut(l as usize & 15)
        {
            if let Some(slot) = symbol.get_mut(*o as usize) {
                *slot = s as u16;
            }
            *o += 1;
        }
    }
    Huff { count, symbol }
}

fn decode_sym(br: &mut BitR, h: &Huff) -> Result<u16, String> {
    let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
    for &count in h.count.iter().skip(1) {
        code |= br.bits(1)? as i32;
        let count = count as i32;
        if code - count < first {
            return h.symbol.get((index + (code - first)) as usize).copied().ok_or_else(|| "bad huffman code".to_string());
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
            let (Some(&lbase), Some(&lext)) = (LBASE.get(s), LEXT.get(s)) else {
                return Err("bad length symbol".into());
            };
            let len = lbase as usize + br.bits(lext as u32)? as usize;
            let ds = decode_sym(br, dh)? as usize;
            let (Some(&dbase), Some(&dext)) = (DBASE.get(ds), DEXT.get(ds)) else {
                return Err("bad distance symbol".into());
            };
            let dist = dbase as usize + br.bits(dext as u32)? as usize;
            if dist > out.len() {
                return Err("distance too far back".into());
            }
            for _ in 0..len {
                // dist <= out.len() (checked above) and out only grows, so this is always in range.
                let b = *out.get(out.len() - dist).ok_or("distance too far back")?;
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
                let p = br.pos;
                let Some(&[l0, l1, n0, n1]) = data.get(p..p.saturating_add(4)) else {
                    return Err("truncated stored block".into());
                };
                let len = u16::from_le_bytes([l0, l1]) as usize;
                let nlen = u16::from_le_bytes([n0, n1]) as usize;
                // p + 4 cannot overflow: the four bytes above exist.
                let end = (p + 4).checked_add(len).ok_or("bad stored block")?;
                let stored = data.get(p + 4..end).filter(|_| len == (!nlen & 0xffff)).ok_or("bad stored block")?;
                out.extend_from_slice(stored);
                br.pos = end;
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
                codes(&mut br, &mut out, &build(&l), &build(&[5u8; 30]))?;
            }
            2 => {
                let nlen = br.bits(5)? as usize + 257;
                let ndist = br.bits(5)? as usize + 1;
                let ncode = br.bits(4)? as usize + 4;
                if nlen > 286 || ndist > 30 {
                    return Err("bad dynamic header".into());
                }
                let mut lengths = [0u8; 320];
                for &o in CODE_LENGTH_ORDER.iter().take(ncode) {
                    let v = br.bits(3)? as u8;
                    if let Some(l) = lengths.get_mut(o) {
                        *l = v;
                    }
                }
                let lc = build(&lengths[..19]);
                let mut idx = 0;
                let total = nlen + ndist;
                let mut ll = [0u8; 320];
                while idx < total {
                    let sym = decode_sym(&mut br, &lc)?;
                    if sym < 16 {
                        *ll.get_mut(idx).ok_or("too many lengths")? = sym as u8;
                        idx += 1;
                    } else {
                        let (prev, rep) = match sym {
                            16 => {
                                let Some(&prev) = idx.checked_sub(1).and_then(|i| ll.get(i)) else {
                                    return Err("bad repeat".into());
                                };
                                (prev, 3 + br.bits(2)? as usize)
                            }
                            17 => (0, 3 + br.bits(3)? as usize),
                            _ => (0, 11 + br.bits(7)? as usize),
                        };
                        if idx + rep > total {
                            return Err("too many lengths".into());
                        }
                        ll.get_mut(idx..idx + rep).ok_or("too many lengths")?.fill(prev);
                        idx += rep;
                    }
                }
                let (Some(lit), Some(dist)) = (ll.get(..nlen), ll.get(nlen..total)) else {
                    return Err("bad dynamic header".into());
                };
                codes(&mut br, &mut out, &build(lit), &build(dist))?;
            }
            _ => return Err("bad block type".into()),
        }
        if last == 1 {
            return Ok(out);
        }
    }
}
