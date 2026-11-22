//! DEFLATE compression (RFC 1951) and its zlib wrapper (RFC 1950), for the PNGs the decompiler embeds. Decompression is
//! [`crate::inflate`].
//!
//! LZ77 with hash chains (`lz77`) feeds blocks (`block`) that are written as dynamic Huffman, fixed Huffman or stored,
//! whichever is smallest.

mod bits;
mod block;
mod huffman;
mod lz77;
mod tables;

use super::checksum::adler32;
use bits::BitWriter;
use block::Block;
use lz77::{Params, TokenSink};

/// Compression effort. The PNG writer uses [`Level::Fast`]; the other levels are exercised by round-trip tests.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Level {
    /// No compression: stored blocks only.
    Store,
    /// Greedy parse, short hash chains (about zlib level 1-2).
    Fast,
    /// Lazy parse (about zlib level 6).
    #[default]
    Default,
    /// Lazy parse with long chains (about zlib level 9).
    Best,
}

impl Level {
    fn params(self) -> Option<Params> {
        match self {
            Level::Store => None,
            Level::Fast => Some(Params { max_chain: 8, nice_len: 32, good_len: 0, max_lazy: 0, max_insert: 6 }),
            Level::Default => Some(Params { max_chain: 128, nice_len: 128, good_len: 8, max_lazy: 16, max_insert: 0 }),
            Level::Best => Some(Params { max_chain: 4096, nice_len: 258, good_len: 32, max_lazy: 258, max_insert: 0 }),
        }
    }

    /// The FLEVEL hint of the zlib header.
    fn zlib_flevel(self) -> u8 {
        match self {
            Level::Store => 0,
            Level::Fast => 1,
            Level::Default => 2,
            Level::Best => 3,
        }
    }
}

/// Tokens per block: bounds memory and lets the Huffman codes adapt to changing data.
const BLOCK_TOKENS: usize = 16 * 1024;

/// Turns the parse into blocks as they fill up.
struct BlockSink<'a> {
    data: &'a [u8],
    out: BitWriter,
    block: Block,
    /// Offset in `data` where the current block starts.
    start: usize,
}

impl BlockSink<'_> {
    fn flush(&mut self, last: bool) {
        let end = self.start + self.block.bytes();
        // `start..end` is always inside `data`: the blocks cover the parsed prefix.
        self.block.write(&mut self.out, self.data.get(self.start..end).unwrap_or(&[]), last);
        self.start = end;
    }
}

impl TokenSink for BlockSink<'_> {
    fn literal(&mut self, byte: u8) {
        self.block.literal(byte);
        if self.block.len() >= BLOCK_TOKENS {
            self.flush(false);
        }
    }

    fn copy(&mut self, len: usize, dist: usize) {
        self.block.copy(len, dist);
        if self.block.len() >= BLOCK_TOKENS {
            self.flush(false);
        }
    }
}

/// Compress `data` to a raw DEFLATE stream.
pub fn compress(data: &[u8], level: Level) -> Vec<u8> {
    let Some(params) = level.params() else {
        let mut w = BitWriter::with_capacity(data.len() + data.len() / 65535 * 5 + 5);
        block::write_stored(&mut w, data, true);
        return w.finish();
    };
    let mut sink = BlockSink { data, out: BitWriter::with_capacity(data.len() / 3 + 64), block: Block::with_capacity(BLOCK_TOKENS), start: 0 };
    lz77::parse(data, params, &mut sink);
    // Always end with a final block, even if it is empty.
    sink.flush(true);
    sink.out.finish()
}

/// Compress `data` to a zlib stream (2-byte header, DEFLATE data, Adler-32 of the input).
pub fn zlib_compress(data: &[u8], level: Level) -> Vec<u8> {
    // CMF: deflate with a 32 KiB window. FLG: level hint, FCHECK so that (CMF * 256 + FLG) % 31 == 0.
    let cmf = 0x78u8;
    let flg_base = level.zlib_flevel() << 6;
    let flg = flg_base + (31 - ((cmf as u16 * 256 + flg_base as u16) % 31) as u8) % 31;
    let mut out = vec![cmf, flg];
    out.extend_from_slice(&compress(data, level));
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inflate::inflate;

    const LEVELS: [Level; 4] = [Level::Store, Level::Fast, Level::Default, Level::Best];

    fn noise(n: usize, seed: u32) -> Vec<u8> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                s as u8
            })
            .collect()
    }

    fn text(n: usize) -> Vec<u8> {
        let words = ["material", "craft", "paint", "layer", "mask", "the", "of", "roughness", "normal", "height", " ", ", ", ".\n"];
        let r = noise(n, 7);
        let mut out = Vec::with_capacity(n + 16);
        let mut i = 0;
        while out.len() < n {
            out.extend_from_slice(words[r[i % r.len()] as usize % words.len()].as_bytes());
            i += 1;
        }
        out.truncate(n);
        out
    }

    fn roundtrip(data: &[u8]) {
        for level in LEVELS {
            let z = compress(data, level);
            assert_eq!(inflate(&z).expect("inflate"), data, "{level:?}, {} bytes", data.len());
        }
    }

    #[test]
    fn edge_cases_round_trip() {
        roundtrip(&[]);
        roundtrip(&[42]);
        roundtrip(&[1, 2]);
        roundtrip(b"aaa");
        roundtrip(b"abcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabc");
        roundtrip(&[0u8; 300_000]);
        roundtrip(&noise(100_000, 1));
        roundtrip(&text(300_000));
    }

    #[test]
    fn many_small_inputs_round_trip() {
        // Small inputs exercise the fixed-Huffman path and degenerate dynamic trees.
        let r = noise(3_000_000, 11);
        let mut k = 0;
        for i in 0..3000 {
            let n = r[k] as usize + (i % 3) * 200;
            let alphabet = r[k + 1] % 8 + 1;
            let d: Vec<u8> = r[k + 2..k + 2 + n].iter().map(|&b| (b % alphabet).wrapping_mul(37)).collect();
            k += n + 2;
            roundtrip(&d);
        }
    }

    #[test]
    fn long_range_repeats_round_trip() {
        // A 50 KiB random block repeated: matches at distances near and beyond the 32 KiB window.
        let mut data = noise(50_000, 3);
        data.extend_from_within(..);
        data.extend_from_slice(&noise(20_000, 4));
        data.extend_from_within(..40_000);
        roundtrip(&data);
        // Period exactly at the window size.
        let mut w = noise(32_768, 5);
        w.extend_from_within(..);
        w.extend_from_within(..);
        roundtrip(&w);
        let z = compress(&w, Level::Default);
        assert!(z.len() < 40_000, "window-sized period should be found: {}", z.len());
    }

    #[test]
    fn ratios_are_sensible() {
        assert!(compress(&[0u8; 1 << 20], Level::Default).len() < 2_000);
        let t = text(200_000);
        let (fast, def, best) = (compress(&t, Level::Fast).len(), compress(&t, Level::Default).len(), compress(&t, Level::Best).len());
        assert!(best <= def && def <= fast && fast < t.len() / 2, "{fast} {def} {best}");
        // Incompressible data falls back to stored blocks: tiny overhead.
        let r = noise(200_000, 9);
        assert!(compress(&r, Level::Default).len() <= r.len() + r.len() / 1000 + 16);
    }

    #[test]
    fn zlib_wrapper() {
        for level in LEVELS {
            let z = zlib_compress(b"hello hello hello", level);
            assert_eq!(z[0], 0x78);
            assert_eq!((z[0] as u16 * 256 + z[1] as u16) % 31, 0);
            assert_eq!(inflate(&z[2..z.len() - 4]).unwrap(), b"hello hello hello");
            assert_eq!(&z[z.len() - 4..], &adler32(b"hello hello hello").to_be_bytes());
        }
    }
}
