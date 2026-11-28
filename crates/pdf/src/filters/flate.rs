//! FlateDecode (ISO 32000-2 §7.4.4): zlib (RFC 1950) wrapping deflate (RFC 1951).
//!
//! Real-world quirks handled:
//! - the Adler-32 trailer is not required (some producers omit or corrupt it), because the
//!   deflate stream is inflated raw after the 2-byte zlib header;
//! - streams without a valid zlib header are inflated as raw deflate;
//! - data after the end of the deflate stream is ignored.
//!
//! A back-reference to before the start of the output points at nothing, and is damage, as zlib
//! has it: read strictly the stream is refused, and read tolerantly what came before it is kept.
//! (miniz_oxide, flate2's backend here before zlib-rs, copied zeros and went on.) So is a dynamic
//! block whose code has no end-of-block symbol, and nothing of that block is kept.

use std::io::Write;

use flate2::{Compression, Decompress, FlushDecompress, Status};

use craft_stream_codecs::{Failure, Step};

use super::FilterError;

const NAME: &str = "FlateDecode";

/// Checks the RFC 1950 header: CM = 8, CINFO ≤ 7 and the FCHECK multiple-of-31 rule.
fn zlib_header(data: &[u8]) -> Option<(u8, u8)> {
    let (&cmf, &flg) = (data.first()?, data.get(1)?);
    let ok = cmf & 0x0f == 8 && cmf >> 4 <= 7 && (u16::from(cmf) << 8 | u16::from(flg)) % 31 == 0;
    ok.then_some((cmf, flg))
}

/// `data` inflated within `max`, and whether it was read twice: a zlib header that only looked
/// valid is read again as raw deflate, within `again`, what a caller paying for what is read has
/// left for it once it has paid for the second reading. `None`, that reading can't be paid for,
/// and is [`FilterError::Spent`].
pub(crate) fn decode(data: &[u8], max: usize, again: Option<usize>) -> (Step, bool) {
    match zlib_header(data) {
        Some((_, flg)) if flg & 0x20 != 0 => (Err(Failure::corrupt(NAME, "zlib preset dictionary is not allowed in PDF", Vec::new())), false),
        Some(_) => match inflate_raw(&data[2..], max) {
            // A header that only looked valid: retry the whole input as raw deflate.
            Err(f) if f.partial.is_empty() && matches!(f.error, FilterError::Corrupt { .. }) => {
                let Some(again) = again else {
                    return (Err(Failure { error: FilterError::Spent, partial: Vec::new(), discarded: 0 }), false);
                };
                let step = match inflate_raw(data, again) {
                    // Keep the original corruption diagnostic only if the retry is corrupt
                    // too. A limit error must remain fatal even for tolerant decoding. What the
                    // retry decoded isn't kept, but a caller charging for work has to hear of it.
                    Err(retry) if matches!(retry.error, FilterError::Corrupt { .. }) => Err(Failure { discarded: retry.partial.len(), ..f }),
                    retry => retry,
                };
                (step, true)
            }
            r => (r, false),
        },
        None => (inflate_raw(data, max), false),
    }
}

#[cfg(test)]
thread_local! {
    /// Bytes inflating gave back on this thread, its partial output included.
    pub(crate) static INFLATED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn inflate_raw(input: &[u8], max: usize) -> Step {
    let step = inflate(input, max);
    #[cfg(test)]
    INFLATED.with(|c| c.set(c.get() + step.as_ref().map_or_else(|f| f.partial.len(), Vec::len)));
    step
}

fn inflate(input: &[u8], max: usize) -> Step {
    let mut d = Decompress::new(false);
    let mut out: Vec<u8> = Vec::new();
    // One byte beyond the limit lets us tell "exactly max" from "more than max".
    let cap = max.saturating_add(1);
    loop {
        if out.len() == out.capacity() {
            if out.len() >= cap {
                return Err(Failure::limit(max));
            }
            let grow = out.capacity().max(input.len().saturating_mul(2)).max(4096).min(cap - out.len());
            out.reserve_exact(grow);
        }
        let consumed = d.total_in() as usize;
        let produced = out.len();
        let res = d.decompress_vec(&input[consumed.min(input.len())..], &mut out, FlushDecompress::None);
        if out.len() > max {
            return Err(Failure::limit(max));
        }
        match res {
            Err(e) => return Err(Failure::corrupt(NAME, e.to_string(), out)),
            Ok(Status::StreamEnd) => return Ok(out),
            Ok(Status::Ok | Status::BufError) => {
                let progressed = d.total_in() as usize != consumed || out.len() != produced;
                let room_left = out.len() < out.capacity();
                if room_left && (d.total_in() as usize >= input.len() || !progressed) {
                    return Err(Failure::corrupt(NAME, "unexpected end of deflate data", out));
                }
            }
        }
    }
}

/// The least of zlib `data`, counted from its start (its 2-byte header included), a reader
/// inflating it takes in to give `output` bytes, or, where that gives fewer, the header and the
/// whole deflate stream, its Adler-32 checksum not counted; and, told or not, how many bytes it
/// inflated. Not told for data without a zlib header or with a preset dictionary (a reader may
/// refuse it outright), where the data is damaged first, or where it inflates more than `max`
/// bytes before either (`max` itself is within the limit, as for decoding). Data that ends before
/// either is all taken in.
pub fn input_for(data: &[u8], output: usize, max: usize) -> (Option<usize>, usize) {
    match zlib_header(data) {
        Some((_, flg)) if flg & 0x20 == 0 => {
            let (taken, made) = least_input(data.get(2..).unwrap_or_default(), output, max);
            (taken.map(|taken| taken + 2), made)
        }
        _ => (None, 0),
    }
}

/// [`input_for`] of raw deflate. The input is fed as many bytes at a time as can't give the bytes
/// still wanted (a byte of deflate gives at most 258 bytes for each of its bits, and the bits held
/// from bytes already fed a few matches more), then a byte at a time, so what is taken in when the
/// output is reached is the least that reaches it. Once it is all fed, steps go on with none while
/// they give more.
fn least_input(input: &[u8], output: usize, max: usize) -> (Option<usize>, usize) {
    const MOST_PER_BYTE: usize = 258 * 8;
    const HELD: usize = 16 << 10;
    let mut d = Decompress::new(false);
    let mut scratch = vec![0u8; 64 << 10];
    let counts = |d: &Decompress| (usize::try_from(d.total_in()).unwrap_or(usize::MAX), usize::try_from(d.total_out()).unwrap_or(usize::MAX));
    loop {
        let (taken, made) = counts(&d);
        if made > max {
            return (None, made);
        }
        if made >= output {
            return (Some(taken), made);
        }
        let left = output - made;
        let feed = (left.saturating_sub(HELD) / MOST_PER_BYTE).max(1);
        // One byte past `max` tells "exactly max" from "more than max", as in `inflate`.
        let room = left.min((max - made).saturating_add(1)).min(scratch.len());
        let Some(rest) = input.get(taken..) else { return (None, made) };
        let (Some(fed), Some(out)) = (rest.get(..feed.min(rest.len())), scratch.get_mut(..room)) else { return (None, made) };
        let Ok(status) = d.decompress(fed, out, FlushDecompress::None) else {
            // A step can give the last of the output and then meet the damage: the output came first.
            let (now_taken, now_made) = counts(&d);
            return ((now_made >= output && now_made <= max).then_some(now_taken), now_made);
        };
        let (now_taken, now_made) = counts(&d);
        match status {
            Status::StreamEnd if now_made > max => return (None, now_made),
            Status::StreamEnd => return (Some(now_taken), now_made),
            // With the data all taken in, zlib may still hold what a step had no room for; the data
            // ends once a step gives nothing more.
            _ if rest.is_empty() && now_made == made => return (Some(input.len()), made),
            // A step that takes and gives nothing would be repeated forever.
            _ if now_taken == taken && now_made == made => return (None, made),
            _ => {}
        }
    }
}

/// zlib at level 6.
// The documented never-crash exception: writing into a `Vec` cannot fail.
#[allow(clippy::expect_used)]
pub(crate) fn encode(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::with_capacity(data.len() / 2 + 64), Compression::new(6));
    e.write_all(data).expect("in-memory write");
    e.finish().expect("in-memory write")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(d: &[u8]) -> Result<Vec<u8>, FilterError> {
        decode(d, 1 << 24, Some(1 << 24)).0.map_err(|f| f.error)
    }

    /// The least of `data` whose prefix inflates to `output` bytes or ends the deflate stream (its
    /// checksum unread), or `data.len()`: what [`input_for`] tells, found the slow way, a prefix
    /// at a time.
    fn least_prefix(data: &[u8], output: usize) -> usize {
        (2..=data.len())
            .find(|&n| {
                let mut d = Decompress::new(false);
                let mut out = vec![0u8; output];
                let status = d.decompress(&data[2..n], &mut out, FlushDecompress::Finish);
                d.total_out() as usize >= output || matches!(status, Ok(Status::StreamEnd))
            })
            .unwrap_or(data.len())
    }

    /// A reader inflating zlib data takes in the least that gives the output asked for, checked
    /// against a prefix search over streams of each kind: stored, fixed and dynamic blocks, with
    /// sync flushes and an empty final block, for outputs short of, at and past each stream's;
    /// and long runs, where a byte gives so much that the input is fed many bytes at a time
    /// until the output is near.
    /// Not told past what may be inflated, without a zlib header, with a preset dictionary, or
    /// for damaged data; data that ends first is all taken in.
    #[test]
    fn the_input_for_an_output_is_the_least_that_gives_it() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut streams = Vec::new();
        for level in [0, 1, 6, 9] {
            for flushes in [0, 2] {
                let len = 200 + (next() % 600) as usize;
                let data: Vec<u8> = (0..len).map(|i| if next() % 3 == 0 { (next() % 256) as u8 } else { (i % 7) as u8 }).collect();
                let mut e = flate2::write::ZlibEncoder::new(Vec::new(), Compression::new(level));
                for (i, chunk) in data.chunks(len / (flushes + 1) + 1).enumerate() {
                    e.write_all(chunk).unwrap();
                    if i < flushes {
                        e.flush().unwrap();
                    }
                }
                streams.push((e.finish().unwrap(), len));
            }
        }
        for level in [1, 9] {
            let len = 300_000;
            let data: Vec<u8> = (0..len).map(|i| (i / 40_000 % 3) as u8).collect();
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), Compression::new(level));
            e.write_all(&data).unwrap();
            streams.push((e.finish().unwrap(), len));
        }
        for (stream, len) in &streams {
            for output in [1, len / 3, len / 2, len - 1, *len, len + 10] {
                let (taken, made) = input_for(stream, output, 1 << 20);
                assert_eq!(taken, Some(least_prefix(stream, output)), "output {output} of a stream of {len}");
                assert!(made >= output.min(*len), "{made}");
            }
        }
        let (data, _) = &streams[2];
        assert_eq!(input_for(data, 1 << 20, 100), (None, 101), "past what may be inflated, by a byte");
        assert_eq!(input_for(&data[2..], 10, 1000).0, None, "no zlib header");
        assert_eq!(input_for(&[0x78, 0xbb, 0, 0], 10, 1000).0, None, "a preset dictionary");
        assert_eq!(input_for(&[0x78, 0x9c, 0xff, 0xff, 0xff], 10, 1000).0, None, "damaged");
        assert_eq!(input_for(&data[..20], 1 << 20, 1 << 20).0, Some(20), "ends before either: all taken in");
    }

    #[test]
    fn known_zlib_vector() {
        // zlib("a"): header 78 9C, fixed-Huffman block, Adler-32 0x00620062.
        assert_eq!(dec(&[0x78, 0x9c, 0x4b, 0x04, 0x00, 0x00, 0x62, 0x00, 0x62]).unwrap(), b"a");
    }

    #[test]
    fn hand_built_stored_block() {
        // RFC 1951 §3.2.4 stored block: BFINAL=1 BTYPE=00, LEN=5, NLEN=!5, "hello",
        // then Adler-32("hello") = 0x062C0215.
        let v = [0x78, 0x01, 0x01, 0x05, 0x00, 0xfa, 0xff, b'h', b'e', b'l', b'l', b'o', 0x06, 0x2c, 0x02, 0x15];
        assert_eq!(dec(&v).unwrap(), b"hello");
        // Missing Adler-32 is tolerated.
        assert_eq!(dec(&v[..12]).unwrap(), b"hello");
        // Wrong Adler-32 is tolerated (as in other readers).
        let mut bad = v;
        bad[15] ^= 0xff;
        assert_eq!(dec(&bad).unwrap(), b"hello");
        // Trailing garbage after the stream is ignored.
        let mut trailing = v.to_vec();
        trailing.extend_from_slice(b"\r\nendstream");
        assert_eq!(dec(&trailing).unwrap(), b"hello");
    }

    #[test]
    fn raw_deflate_fallback() {
        let data = b"raw deflate without any zlib header, repeated repeated repeated".repeat(4);
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), Compression::new(6));
        e.write_all(&data).unwrap();
        let raw = e.finish().unwrap();
        assert!(zlib_header(&raw).is_none());
        assert_eq!(dec(&raw).unwrap(), data);
    }

    #[test]
    fn round_trip_and_missing_checksum() {
        let data: Vec<u8> = (0..50_000u32).map(|i| (i * i % 251) as u8).collect();
        let z = encode(&data);
        assert_eq!(&z[..2], &[0x78, 0x9c]);
        assert_eq!(dec(&z).unwrap(), data);
        assert_eq!(dec(&z[..z.len() - 4]).unwrap(), data);
        assert_eq!(dec(&z[..z.len() - 2]).unwrap(), data);
    }

    #[test]
    fn empty_input() {
        assert_eq!(dec(&encode(b"")).unwrap(), b"");
        assert!(matches!(dec(b""), Err(FilterError::Corrupt { .. })));
    }

    #[test]
    fn corrupt_and_truncated() {
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 13) as u8).collect();
        let z = encode(&data);
        match decode(&z[..z.len() / 2], 1 << 24, Some(1 << 24)).0 {
            Err(f) => {
                assert!(matches!(f.error, FilterError::Corrupt { .. }));
                assert_eq!(&data[..f.partial.len()], &f.partial[..]);
            }
            Ok(_) => panic!("truncated stream decoded"),
        }
        // BTYPE = 11 is reserved (RFC 1951 §3.2.3).
        assert!(matches!(dec(&[0x78, 0x9c, 0x07, 0, 0, 0]), Err(FilterError::Corrupt { .. })));
        // Preset dictionary.
        assert!(matches!(dec(&[0x78, 0x20, 0, 0, 0, 0]), Err(FilterError::Corrupt { .. })));
        // One whose header is followed by data that inflates: refused all the same.
        assert!(matches!(dec(&[0x78, 0xBB, 0x4b, 0x04, 0x00]), Err(FilterError::Corrupt { .. })));
    }

    #[test]
    fn bomb_is_bounded() {
        let z = encode(&vec![0u8; 16 << 20]);
        assert!(z.len() < 32 * 1024);
        assert!(matches!(dec_limit(&z, 1 << 20), Err(FilterError::LimitExceeded(n)) if n == 1 << 20));
        assert_eq!(dec_limit(&z, 16 << 20).unwrap().len(), 16 << 20);
        assert!(matches!(dec_limit(&z, (16 << 20) - 1), Err(FilterError::LimitExceeded(_))));
        assert!(matches!(dec_limit(&z, 0), Err(FilterError::LimitExceeded(0))));
    }

    fn dec_limit(d: &[u8], max: usize) -> Result<Vec<u8>, FilterError> {
        decode(d, max, Some(max)).0.map_err(|f| f.error)
    }

    /// Deflate's bits: header fields and extra bits least significant first, Huffman codes most
    /// significant first (RFC 1951 §3.1.1).
    #[derive(Default)]
    struct Bits {
        out: Vec<u8>,
        acc: u32,
        n: u32,
    }

    impl Bits {
        fn put(&mut self, value: u32, len: u32) {
            for i in 0..len {
                self.acc |= ((value >> i) & 1) << self.n;
                self.n += 1;
                if self.n == 8 {
                    self.out.push(self.acc as u8);
                    (self.acc, self.n) = (0, 0);
                }
            }
        }

        fn code(&mut self, code: u32, len: u32) {
            for i in (0..len).rev() {
                self.put((code >> i) & 1, 1);
            }
        }

        fn finish(mut self) -> Vec<u8> {
            if self.n > 0 {
                self.out.push(self.acc as u8);
            }
            self.out
        }
    }

    /// A dynamic block whose literal and length code has no end-of-block symbol can't end (RFC 1951
    /// §3.2.5): it is refused, and nothing of it is kept.
    #[test]
    fn a_dynamic_code_without_an_end_is_damage() {
        let mut bits = Bits::default();
        bits.put(1, 1); // the final block,
        bits.put(2, 2); // dynamic Huffman codes (§3.2.7)
        bits.put(0, 5); // 257 literal and length codes
        bits.put(0, 5); // 1 distance code
        bits.put(14, 4); // 18 code length codes, in their order up to the one for 1
        for symbol in [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1] {
            bits.put(u32::from(symbol == 18 || symbol == 1), 3);
        }
        // Code length codes: 1 is 0, 18 is 1. 'A' and 'B' get a bit each, 256 none.
        let zeros = |bits: &mut Bits, n: u32| {
            bits.code(1, 1);
            bits.put(n - 11, 7);
        };
        zeros(&mut bits, 65);
        bits.code(0, 1);
        bits.code(0, 1);
        zeros(&mut bits, 138);
        zeros(&mut bits, 52);
        bits.code(0, 1); // the distance code: a bit
        for _ in 0..4 {
            bits.code(0, 1); // 'A'
        }
        let data = bits.finish();
        assert!(zlib_header(&data).is_none(), "read as raw deflate");
        match decode(&data, 1 << 20, Some(1 << 20)).0 {
            Err(f) => {
                assert!(matches!(f.error, FilterError::Corrupt { .. }), "{}", f.error);
                assert_eq!(f.partial, b"");
            }
            Ok(out) => panic!("read as {out:?}"),
        }
    }

    /// A back-reference to before the start of the output points at nothing: read strictly the
    /// stream is refused, and what came before it is what a tolerant reading keeps. miniz_oxide,
    /// flate2's backend here before zlib-rs, copied zeros and went on, so this also shows which
    /// backend inflates (the price of reading Flate, `FLATE_READ_WORK`, is set for zlib-rs).
    #[test]
    fn a_copy_from_before_the_start_is_damage() {
        let literal = |bits: &mut Bits, b: u8| bits.code(0x30 + u32::from(b), 8);
        let mut bits = Bits::default();
        bits.put(1, 1); // the final block,
        bits.put(1, 2); // fixed Huffman codes (§3.2.6)
        literal(&mut bits, b'A');
        literal(&mut bits, b'B');
        bits.code(1, 7); // length 3
        bits.code(4, 5); // distance 5 (code 4 and an extra bit of 0), three bytes before the start
        bits.put(0, 1);
        literal(&mut bits, b'C');
        bits.code(0, 7); // end of block
        let data = bits.finish();
        assert!(zlib_header(&data).is_none(), "read as raw deflate");
        match decode(&data, 1 << 20, Some(1 << 20)).0 {
            Err(f) => {
                assert!(matches!(f.error, FilterError::Corrupt { .. }), "{}", f.error);
                assert_eq!(f.partial, b"AB");
            }
            Ok(out) => panic!("read as {out:?}"),
        }
    }

    /// zlib data of one fixed block, one literal and 255 copies of 258: 65,791 bytes, more than a
    /// step has room for. With its end of block, or without, when the data ends first.
    fn a_run_of_copies(end: bool) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.put(1, 1); // the final block,
        bits.put(1, 2); // fixed Huffman codes (§3.2.6)
        bits.code(0x30 + u32::from(b'A'), 8);
        for _ in 0..255 {
            bits.code(0xc5, 8); // length 258
            bits.code(0, 5); // distance 1
        }
        if end {
            bits.code(0, 7);
        }
        let mut data = vec![0x78, 0x9c];
        data.extend(bits.finish());
        data
    }

    /// A run of copies can be all taken in before what it gives has room: what zlib holds of it is
    /// still inflated, and counts toward the limit, once the data is all taken in.
    #[test]
    fn what_is_held_once_the_data_is_taken_in_is_still_inflated() {
        let data = a_run_of_copies(false);
        assert_eq!(input_for(&data, 1 << 20, 1 << 20), (Some(data.len()), 65_791), "ends first: all taken in");
        assert_eq!(input_for(&data, 1 << 20, 65_537), (None, 65_538), "past what may be inflated");
    }

    /// `max` bytes are within what may be inflated, as for decoding, whatever the output asked for
    /// past them: data that gives exactly `max` is told, and data that gives one more isn't.
    #[test]
    fn data_that_inflates_exactly_max_is_told_for_any_output() {
        for end in [true, false] {
            let data = a_run_of_copies(end);
            for output in [65_792, 66_000, 70_000, 100_000, 148_000, 1 << 20] {
                assert_eq!(input_for(&data, output, 65_791), (Some(data.len()), 65_791), "{end}, {output}");
                assert_eq!(input_for(&data, output, 65_790), (None, 65_791), "{end}, {output}");
            }
            // An output of one byte past `max`, reached by the byte that is past it.
            assert_eq!(input_for(&data, 65_791, 65_790), (None, 65_791), "{end}");
        }
        let whole = a_run_of_copies(true);
        assert_eq!(dec_limit(&whole, 65_791).ok().map(|out| out.len()), Some(65_791), "decoded within the limit");
        assert!(dec_limit(&whole, 65_790).is_err(), "past the limit");
        // An empty final block, and a header alone, with nothing to be inflated.
        assert_eq!(input_for(&[0x78, 0x9c, 0x03, 0x00], 1, 0), (Some(4), 0));
        assert_eq!(dec_limit(&[0x78, 0x9c, 0x03, 0x00], 0).ok(), Some(Vec::new()));
        assert_eq!(input_for(&[0x78, 0x9c], 1, 0), (Some(2), 0));
    }

    /// zlib data of a dynamic block of `count` 'A's, a bit each, and its end, then a block of the
    /// reserved type (§3.2.3): damage, five bits after the last 'A'.
    fn letters_then_damage(count: usize) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.put(0, 1); // a block that isn't the last,
        bits.put(2, 2); // of dynamic Huffman codes (§3.2.7)
        bits.put(0, 5); // 257 literal and length codes
        bits.put(0, 5); // 1 distance code
        bits.put(14, 4); // 18 code length codes, in their order up to the one for 1
        for symbol in [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1] {
            bits.put(
                match symbol {
                    18 => 1,
                    1 | 2 => 2,
                    _ => 0,
                },
                3,
            );
        }
        // Code length codes: 18 is 0, 1 is 10, 2 is 11. 'A' gets a bit (0), 'B' and 256 two (10, 11).
        let zeros = |bits: &mut Bits, n: u32| {
            bits.code(0, 1);
            bits.put(n - 11, 7);
        };
        zeros(&mut bits, 65);
        bits.code(2, 2);
        bits.code(3, 2);
        zeros(&mut bits, 138);
        zeros(&mut bits, 51);
        bits.code(3, 2);
        bits.code(2, 2); // the distance code: a bit
        for _ in 0..count {
            bits.code(0, 1); // 'A'
        }
        bits.code(3, 2); // the end of the block
        bits.put(1, 1); // the last block,
        bits.put(3, 2); // of the reserved type
        [&[0x78, 0x9c][..], &bits.finish()].concat()
    }

    /// The step that gives the last of the output can meet damage after it in the same byte (here
    /// for 2, 3, 4, 10, 11 and 12 'A's): the output came first, so it is told, as it is where the
    /// damage is in the next byte.
    #[test]
    fn damage_after_the_output_in_the_same_byte_is_after_it() {
        for count in 1..=16 {
            let data = letters_then_damage(count);
            assert_eq!(decode(&data, 1 << 20, Some(1 << 20)).0.map_err(|f| f.partial), Err(vec![b'A'; count]), "{count}");
            let least = least_prefix(&data, count);
            assert_eq!(input_for(&data, count, 1 << 20), (Some(least), count), "{count}");
            assert_eq!(input_for(&data, count, count), (Some(least), count), "exactly max, {count}");
            assert_eq!(input_for(&data, count + 1, 1 << 20), (None, count), "damage first, {count}");
            assert_eq!(input_for(&data, count, count - 1), (None, count), "past what may be inflated, {count}");
        }
    }
}
