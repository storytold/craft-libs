//! Block encoding: collects LZ77 tokens with their symbol statistics and writes each block in whichever of the
//! three DEFLATE block types (stored, fixed Huffman, dynamic Huffman) is smallest.

use super::bits::BitWriter;
use super::huffman::{canonical_codes, code_lengths};
use super::tables::{
    CODE_LENGTH_ORDER, DIST_BASE, DIST_EXTRA, END_OF_BLOCK, LENGTH_BASE, LENGTH_EXTRA, dist_code, fixed_literal_lengths, length_code,
};

/// A literal byte (`dist == 0`) or a back-reference of `len` bytes at distance `dist`.
#[derive(Clone, Copy)]
pub(super) struct Token {
    len_or_byte: u16,
    dist: u16,
}

/// Number of literal/length and distance symbols the format defines (286 and 30, of which 286/287 never occur).
const NUM_LIT: usize = 286;
const NUM_DIST: usize = 30;
const MAX_STORED: usize = 65535;

/// Tokens of the block being built, with their symbol frequencies.
pub(super) struct Block {
    tokens: Vec<Token>,
    lit_freq: [u32; NUM_LIT],
    dist_freq: [u32; NUM_DIST],
    /// Uncompressed bytes the tokens cover.
    bytes: usize,
}

impl Block {
    pub fn with_capacity(cap: usize) -> Self {
        Block { tokens: Vec::with_capacity(cap), lit_freq: [0; NUM_LIT], dist_freq: [0; NUM_DIST], bytes: 0 }
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    #[inline]
    pub fn literal(&mut self, b: u8) {
        self.tokens.push(Token { len_or_byte: b as u16, dist: 0 });
        if let Some(f) = self.lit_freq.get_mut(b as usize) {
            *f += 1;
        }
        self.bytes += 1;
    }

    #[inline]
    pub fn copy(&mut self, len: usize, dist: usize) {
        self.tokens.push(Token { len_or_byte: len as u16, dist: dist as u16 });
        if let Some(f) = self.lit_freq.get_mut(257 + length_code(len)) {
            *f += 1;
        }
        if let Some(f) = self.dist_freq.get_mut(dist_code(dist)) {
            *f += 1;
        }
        self.bytes += len;
    }

    /// Write the block; `raw` must be exactly the uncompressed bytes it covers. Clears the block afterwards.
    pub fn write(&mut self, w: &mut BitWriter, raw: &[u8], last: bool) {
        debug_assert_eq!(raw.len(), self.bytes);
        self.lit_freq[END_OF_BLOCK] = 1;
        let dynamic = DynamicHeader::new(&self.lit_freq, &self.dist_freq);
        let fixed_lit = fixed_literal_lengths();
        let fixed_dist = [5u8; NUM_DIST];
        let dynamic_bits = dynamic.cost() + self.data_cost(&dynamic.lit_lens, &dynamic.dist_lens);
        let fixed_bits = 3 + self.data_cost(&fixed_lit, &fixed_dist);
        // Header, alignment to a byte (worst case), LEN/NLEN, payload; repeated per 64 KiB chunk.
        let stored_bits = raw.len().div_ceil(MAX_STORED).max(1) as u64 * (3 + 7 + 32) + raw.len() as u64 * 8;

        if stored_bits <= fixed_bits.min(dynamic_bits) {
            write_stored(w, raw, last);
        } else if fixed_bits <= dynamic_bits {
            w.put(last as u32, 1);
            w.put(1, 2);
            // All 288 lengths: canonical codes depend on the counts of the unused symbols 286 and 287 too.
            self.write_tokens(w, &fixed_lit, &fixed_dist);
        } else {
            w.put(last as u32, 1);
            w.put(2, 2);
            dynamic.write(w);
            self.write_tokens(w, &dynamic.lit_lens, &dynamic.dist_lens);
        }
        self.tokens.clear();
        self.lit_freq = [0; NUM_LIT];
        self.dist_freq = [0; NUM_DIST];
        self.bytes = 0;
    }

    /// Bits for the block's symbols (with extra bits) under the given code lengths.
    fn data_cost(&self, lit_lens: &[u8], dist_lens: &[u8]) -> u64 {
        let mut bits = 0u64;
        for (s, (&f, &len)) in self.lit_freq.iter().zip(lit_lens).enumerate() {
            let extra = if s > END_OF_BLOCK { LENGTH_EXTRA.get(s - 257).copied().unwrap_or(0) as u64 } else { 0 };
            bits += f as u64 * (len as u64 + extra);
        }
        for ((&f, &len), &extra) in self.dist_freq.iter().zip(dist_lens).zip(&DIST_EXTRA) {
            bits += f as u64 * (len as u64 + extra as u64);
        }
        bits
    }

    fn write_tokens(&self, w: &mut BitWriter, lit_lens: &[u8], dist_lens: &[u8]) {
        let lit_codes = canonical_codes(lit_lens);
        let dist_codes = canonical_codes(dist_lens);
        // (code, length) of a symbol; every symbol written has both (the tables cover the whole alphabet).
        let lit = |s: usize| (lit_codes.get(s).copied().unwrap_or(0) as u32, lit_lens.get(s).copied().unwrap_or(0) as u32);
        let dist_sym = |s: usize| (dist_codes.get(s).copied().unwrap_or(0) as u32, dist_lens.get(s).copied().unwrap_or(0) as u32);
        for t in &self.tokens {
            if t.dist == 0 {
                let (code, n) = lit(t.len_or_byte as usize);
                w.put(code, n);
            } else {
                let len = t.len_or_byte as usize;
                let lc = length_code(len);
                let (code, n) = lit(257 + lc);
                w.put(code, n);
                let (base, extra) = (LENGTH_BASE.get(lc).copied().unwrap_or(0), LENGTH_EXTRA.get(lc).copied().unwrap_or(0));
                w.put((len - base as usize) as u32, extra as u32);
                let dist = t.dist as usize;
                let dc = dist_code(dist);
                let (code, n) = dist_sym(dc);
                w.put(code, n);
                let (base, extra) = (DIST_BASE.get(dc).copied().unwrap_or(0), DIST_EXTRA.get(dc).copied().unwrap_or(0));
                w.put((dist - base as usize) as u32, extra as u32);
            }
        }
        let (code, n) = lit(END_OF_BLOCK);
        w.put(code, n);
    }
}

/// Stored (uncompressed) block(s): one per 64 KiB of `raw`, at least one.
pub(super) fn write_stored(w: &mut BitWriter, raw: &[u8], last: bool) {
    let mut chunks = raw.chunks(MAX_STORED).peekable();
    let empty: &[u8] = &[];
    if chunks.peek().is_none() {
        write_stored_chunk(w, empty, last);
    }
    while let Some(c) = chunks.next() {
        write_stored_chunk(w, c, last && chunks.peek().is_none());
    }
}

fn write_stored_chunk(w: &mut BitWriter, chunk: &[u8], last: bool) {
    w.put(last as u32, 1);
    w.put(0, 2);
    w.align();
    let len = chunk.len() as u16;
    w.put(len as u32 | ((!len as u32) << 16), 32);
    w.align();
    w.put_bytes(chunk);
}

/// Code lengths of a dynamic block and their run-length-encoded transmission form.
struct DynamicHeader {
    lit_lens: Vec<u8>,
    dist_lens: Vec<u8>,
    hlit: usize,
    hdist: usize,
    /// (code-length symbol 0..18, extra-bit value)
    rle: Vec<(u8, u8)>,
    cl_lens: Vec<u8>,
    hclen: usize,
}

impl DynamicHeader {
    fn new(lit_freq: &[u32; NUM_LIT], dist_freq: &[u32; NUM_DIST]) -> Self {
        let lit_lens = code_lengths(lit_freq, 15);
        let dist_lens = code_lengths(dist_freq, 15);
        let hlit = 257 + lit_lens.iter().skip(257).rposition(|&l| l != 0).map_or(0, |p| p + 1);
        let hdist = 1 + dist_lens.iter().rposition(|&l| l != 0).unwrap_or(0);
        let mut all: Vec<u8> = lit_lens.iter().take(hlit).copied().collect();
        all.extend(dist_lens.iter().take(hdist));
        let rle = run_length_encode(&all);
        let mut cl_freq = [0u32; 19];
        for &(s, _) in &rle {
            if let Some(f) = cl_freq.get_mut(s as usize) {
                *f += 1;
            }
        }
        let cl_lens = code_lengths(&cl_freq, 7);
        let hclen = 4.max(1 + CODE_LENGTH_ORDER.iter().rposition(|&s| cl_lens.get(s).is_some_and(|&l| l != 0)).unwrap_or(0));
        DynamicHeader { lit_lens, dist_lens, hlit, hdist, rle, cl_lens, hclen }
    }

    /// Header size in bits, including the 3-bit block header.
    fn cost(&self) -> u64 {
        let mut bits = 3 + 5 + 5 + 4 + 3 * self.hclen as u64;
        for &(s, _) in &self.rle {
            bits += self.cl_lens.get(s as usize).copied().unwrap_or(0) as u64 + rle_extra_bits(s) as u64;
        }
        bits
    }

    fn write(&self, w: &mut BitWriter) {
        w.put((self.hlit - 257) as u32, 5);
        w.put((self.hdist - 1) as u32, 5);
        w.put((self.hclen - 4) as u32, 4);
        for &s in CODE_LENGTH_ORDER.iter().take(self.hclen) {
            w.put(self.cl_lens.get(s).copied().unwrap_or(0) as u32, 3);
        }
        let codes = canonical_codes(&self.cl_lens);
        for &(s, extra) in &self.rle {
            let i = s as usize;
            w.put(codes.get(i).copied().unwrap_or(0) as u32, self.cl_lens.get(i).copied().unwrap_or(0) as u32);
            w.put(extra as u32, rle_extra_bits(s));
        }
    }
}

fn rle_extra_bits(sym: u8) -> u32 {
    match sym {
        16 => 2,
        17 => 3,
        18 => 7,
        _ => 0,
    }
}

/// Encode a code-length sequence with the repeat symbols 16 (previous, 3-6), 17 (zeros, 3-10), 18 (zeros, 11-138).
fn run_length_encode(lens: &[u8]) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rest) = lens.get(i..)
        && let Some(&v) = rest.first()
    {
        let run = rest.iter().take_while(|&&x| x == v).count();
        let mut left = run;
        if v == 0 {
            while left >= 11 {
                let n = left.min(138);
                out.push((18, (n - 11) as u8));
                left -= n;
            }
            if left >= 3 {
                out.push((17, (left - 3) as u8));
                left = 0;
            }
        } else {
            out.push((v, 0));
            left -= 1;
            while left >= 3 {
                let n = left.min(6);
                out.push((16, (n - 3) as u8));
                left -= n;
            }
        }
        out.extend(std::iter::repeat_n((v, 0), left));
        i += run;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand(rle: &[(u8, u8)]) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        for &(s, e) in rle {
            match s {
                16 => {
                    let p = *out.last().unwrap();
                    out.extend(std::iter::repeat_n(p, 3 + e as usize));
                }
                17 => out.extend(std::iter::repeat_n(0, 3 + e as usize)),
                18 => out.extend(std::iter::repeat_n(0, 11 + e as usize)),
                v => out.push(v),
            }
        }
        out
    }

    #[test]
    fn rle_round_trips() {
        let cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![0; 2],
            vec![0; 3],
            vec![0; 11],
            vec![0; 150],
            vec![0; 300],
            vec![5; 1],
            vec![5; 4],
            vec![5; 7],
            vec![5; 20],
            vec![1, 2, 2, 2, 2, 0, 0, 0, 0, 3, 3],
        ];
        for c in cases {
            assert_eq!(expand(&run_length_encode(&c)), c);
        }
    }
}
