//! Unit and property tests for the bitstream helpers.

use super::*;
use proptest::prelude::*;

#[test]
fn exp_golomb_known() {
    // 1 | 010 | 011 | 00100 | 00101 -> 0,1,2,3,4
    let mut w = BitWriter::new();
    for v in 0..5 {
        w.write_ue(v);
    }
    let b = w.finish();
    let mut r = BitReader::new(&b);
    for v in 0..5 {
        assert_eq!(r.read_ue().unwrap(), v);
    }
    let mut w = BitWriter::new();
    for v in [0, 1, -1, 2, -2, 1000, -1000] {
        w.write_se(v);
    }
    let b = w.finish();
    let mut r = BitReader::new(&b);
    for v in [0, 1, -1, 2, -2, 1000, -1000] {
        assert_eq!(r.read_se().unwrap(), v);
    }
}

proptest! {
    #[test]
    fn roundtrip(vals in proptest::collection::vec((0u32..u32::MAX, 1u32..=32), 0..200), ues in proptest::collection::vec(0u32..u32::MAX - 1, 0..50)) {
        let mut w = BitWriter::new();
        for &(v, n) in &vals { w.write_bits(v, n); }
        for &u in &ues { w.write_ue(u); }
        let b = w.finish();
        let mut r = BitReader::new(&b);
        for &(v, n) in &vals {
            let mask = if n == 32 { u32::MAX } else { (1 << n) - 1 };
            prop_assert_eq!(r.read_bits(n).unwrap(), v & mask);
        }
        for &u in &ues { prop_assert_eq!(r.read_ue().unwrap(), u); }
    }

    #[test]
    fn escape_roundtrip(data in proptest::collection::vec(0u8..4, 0..300)) {
        let e = escape_rbsp(&data);
        prop_assert_eq!(unescape_rbsp(&e), data);
        for w in e.windows(3) { prop_assert!(!(w[0] == 0 && w[1] == 0 && w[2] < 3)); }
    }
}

// Additional coverage ported from the Smacker PR's inline bitstream tests.
#[test]
fn reads_bits() {
    let d = [0b1010_1100, 0b0101_0011];
    let mut r = BitReader::new(&d);
    assert_eq!(r.read_bits(1).unwrap(), 1);
    assert_eq!(r.read_bits(3).unwrap(), 0b010);
    assert_eq!(r.read_bits(8).unwrap(), 0b1100_0101);
    assert_eq!(r.bits_left(), 4);
    assert_eq!(r.read_bits(5), Err(BitError::Eof));
}

#[test]
fn signed_exp_golomb_extremes_roundtrip() {
    let vals = [0, 1, -1, i32::MAX, i32::MIN];
    let mut w = BitWriter::new();
    for v in vals {
        w.write_se(v);
    }
    let b = w.finish();
    let mut r = BitReader::new(&b);
    for v in vals {
        assert_eq!(r.read_se(), Ok(v));
    }
}

#[test]
fn signed_exp_golomb_reads_minimum_from_spec_bits() {
    // 32 zero bits, 1, then the 32-bit suffix 1: codeNum = 2^32 -> -2^31.
    let bytes = [0, 0, 0, 0, 0x80, 0, 0, 0, 0x80];
    assert_eq!(BitReader::new(&bytes).read_se(), Ok(i32::MIN));
}

#[test]
fn signed_exp_golomb_rejects_positive_overflow() {
    // 32 zero bits, 1, then 32 zero bits: codeNum = 2^32 - 1 -> +2^31, not an i32.
    let bytes = [0, 0, 0, 0, 0x80, 0, 0, 0, 0];
    assert_eq!(BitReader::new(&bytes).read_se(), Err(BitError::InvalidExpGolomb));
}

#[test]
fn unsigned_exp_golomb_keeps_u32_range() {
    // codeNum = 2^32 is a valid se(v) code but does not fit ue(v)'s u32.
    let bytes = [0, 0, 0, 0, 0x80, 0, 0, 0, 0x80];
    assert_eq!(BitReader::new(&bytes).read_ue(), Err(BitError::InvalidExpGolomb));
}

#[test]
fn emulation_prevention() {
    let raw = [0, 0, 1, 0, 0, 0, 5, 0, 0, 3];
    let esc = escape_rbsp(&raw);
    assert_eq!(esc, vec![0, 0, 3, 1, 0, 0, 3, 0, 5, 0, 0, 3, 3]);
    assert_eq!(unescape_rbsp(&esc), raw);
}

/// Verbatim copy of the byte-at-a-time `unescape_rbsp` that this one replaced.
fn old_unescape_rbsp(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut zeros = 0;
    for &b in data {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        out.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
    }
    out
}

#[test]
fn unescape_edge_cases() {
    let cases: &[(&[u8], &[u8])] = &[
        (&[], &[]),
        (&[3], &[3]),
        (&[0, 0], &[0, 0]),
        (&[0, 0, 3], &[0, 0]),
        (&[0, 0, 3, 3], &[0, 0, 3]),
        (&[0, 0, 3, 0, 0, 3], &[0, 0, 0, 0]),
        (&[0, 0, 0, 3], &[0, 0, 0]),
        (&[0, 0, 0, 0, 3, 1], &[0, 0, 0, 0, 1]),
        (&[0, 3, 0, 0, 3, 7], &[0, 3, 0, 0, 7]),
        (&[1, 0, 0, 3, 3, 0, 0, 3], &[1, 0, 0, 3, 0, 0]),
        (&[0, 0, 3, 0, 3], &[0, 0, 0, 3]),
    ];
    for (input, want) in cases {
        assert_eq!(unescape_rbsp(input), *want, "input {input:?}");
        assert_eq!(old_unescape_rbsp(input), *want, "input {input:?}");
    }
}

#[test]
fn unescape_matches_the_pre_change_function_on_random_buffers() {
    // Fixed-seed xorshift64*, no new dependencies.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    };
    for round in 0..20_000u32 {
        let len = (next() % 300) as usize;
        // Mostly zeros and threes, some small values, some random bytes.
        let zero_bias = next() % 4;
        let data: Vec<u8> = (0..len)
            .map(|_| match next() % 8 {
                0..=3 if zero_bias > 0 => 0,
                4 | 5 => 3,
                6 => (next() % 5) as u8,
                _ => (next() & 0xFF) as u8,
            })
            .collect();
        assert_eq!(unescape_rbsp(&data), old_unescape_rbsp(&data), "round {round}: {data:?}");
    }
}

#[test]
fn annexb_split() {
    let s = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 9, 9];
    let n = annexb_nals(&s);
    assert_eq!(n, vec![&[0x67, 1, 2][..], &[0x68, 3][..], &[0x65, 9, 9][..]]);
    let l = [0, 0, 0, 2, 0x67, 1, 0, 0, 0, 1, 0x68];
    assert_eq!(length_prefixed_nals(&l, 4).unwrap(), vec![&[0x67, 1][..], &[0x68][..]]);
}

#[test]
fn more_rbsp() {
    let d = [0b1100_0000];
    let mut r = BitReader::new(&d);
    assert!(r.more_rbsp_data());
    r.read_bit().unwrap();
    assert!(!r.more_rbsp_data());
}

#[test]
fn more_rbsp_data_finds_the_stop_bit_before_a_zero_suffix() {
    let mut d = vec![0b1010_0000];
    d.resize(1 << 16, 0);
    let mut r = BitReader::new(&d);
    assert_eq!(r.rbsp_stop, Some(2));
    assert!(r.more_rbsp_data());
    r.skip(2).unwrap();
    assert!(!r.more_rbsp_data());
    assert_eq!(BitReader::new(&[0; 4]).rbsp_stop, None);
    assert!(!BitReader::new(&[0; 4]).more_rbsp_data());
}

#[test]
fn crc() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
}

#[test]
fn read_ue_plus_rejects_overflow() {
    let mut w = BitWriter::new();
    w.write_ue(7);
    w.write_ue(u32::MAX - 1);
    w.rbsp_trailing();
    let b = w.finish();
    let mut r = BitReader::new(&b);
    assert_eq!(r.read_ue_plus(1).unwrap(), 8);
    assert_eq!(r.read_ue_plus(2), Err(BitError::InvalidExpGolomb));
}

#[test]
fn length_prefixed_rejects_invalid_width_and_oversized_length() {
    for size in [0, 5, 8, usize::MAX] {
        assert_eq!(length_prefixed_nals(&[], size), Err(BitError::InvalidNalLengthSize), "size {size}");
        assert_eq!(length_prefixed_nals(&[0; 16], size), Err(BitError::InvalidNalLengthSize), "size {size}");
    }
    // `(lengthSizeMinusOne & 3) + 1` can be 3; such samples still split.
    assert_eq!(length_prefixed_nals(&[0, 0, 1, 7], 3).unwrap(), vec![&[7u8][..]]);
    assert_eq!(length_prefixed_nals(&[0xFF, 0xFF, 0xFF, 0xFF, 1], 4), Err(BitError::Eof));
    assert_eq!(length_prefixed_nals(&[0, 0, 0, 1, 9], 4).unwrap(), vec![&[9u8][..]]);
}

#[test]
fn widths_above_32_bits_do_not_panic() {
    let mut w = BitWriter::new();
    w.write_bits(0xABCD_EF01, 40);
    w.write_bits(1, 60);
    let b = w.finish();
    assert_eq!(b.len(), 13);
    assert_eq!(&b[..5], &[0x00, 0xAB, 0xCD, 0xEF, 0x01]);
    let mut r = BitReader::new(&b);
    assert_eq!(r.peek(40), 0xABCD_EF01);
    assert_eq!(r.read_bits(40).unwrap(), 0xABCD_EF01);
    assert_eq!(r.peek(60), 1);
    assert_eq!(r.read_bits_u64(60).unwrap(), 1);
    assert_eq!(r.bits_left(), 4);
    assert_eq!(r.read_bits(40), Err(BitError::Eof));
}

#[test]
fn huge_write_width_is_capped_at_64_bits() {
    let mut w = BitWriter::new();
    w.write_bits(0xFFFF_FFFF, u32::MAX);
    assert_eq!(w.finish(), [0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
}

/// Bit `p` of `d`, MSB first, zero past the end.
fn bit_at(d: &[u8], p: usize) -> u32 {
    d.get(p / 8).map_or(0, |&b| u32::from(b >> (7 - p % 8)) & 1)
}

/// Bit-by-bit reference for `peek`: the low 32 bits of the `n`-bit field at `pos`.
fn reference_peek(d: &[u8], pos: usize, n: u32) -> u32 {
    let start = pos + n.saturating_sub(32) as usize;
    (0..n.min(32) as usize).fold(0, |v, i| (v << 1) | bit_at(d, start + i))
}

proptest! {
    #[test]
    fn peek_and_read_bit_match_a_bitwise_reference(data in proptest::collection::vec(any::<u8>(), 0..24)) {
        // Every position, including the last 7 bytes (byte loads) and past the end.
        for pos in 0..=data.len() * 8 + 9 {
            let mut r = BitReader::new(&data);
            r.seek_bits(pos);
            for n in 0..=40 {
                prop_assert_eq!(r.peek(n), reference_peek(&data, pos, n), "pos {} n {}", pos, n);
            }
            let bit = r.read_bit();
            if pos < data.len() * 8 {
                prop_assert_eq!(bit, Ok(bit_at(&data, pos) == 1));
                prop_assert_eq!(r.position(), pos + 1);
            } else {
                prop_assert_eq!(bit, Err(BitError::Eof));
                prop_assert_eq!(r.position(), pos);
            }
        }
    }
}

/// The reader as it was before the 8-byte load, kept verbatim as the reference.
struct OldReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl OldReader<'_> {
    fn bits_left(&self) -> usize {
        self.data.len().saturating_mul(8).saturating_sub(self.pos)
    }
    fn peek(&self, n: u32) -> u32 {
        if n > 32 {
            let r = OldReader { data: self.data, pos: self.pos.saturating_add((n - 32) as usize) };
            return r.peek(32);
        }
        if n == 0 {
            return 0;
        }
        let byte = self.pos / 8;
        let shift = self.pos % 8;
        let mut v: u64 = 0;
        for i in 0..5 {
            v = (v << 8) | *self.data.get(byte.saturating_add(i)).unwrap_or(&0) as u64;
        }
        ((v << (24 + shift)) >> (64 - n)) as u32
    }
    fn read_bits(&mut self, n: u32) -> Result<u32> {
        if n as usize > self.bits_left() {
            return Err(BitError::Eof);
        }
        if n > 32 {
            self.pos += (n - 32) as usize;
            return self.read_bits(32);
        }
        let v = self.peek(n);
        self.pos += n as usize;
        Ok(v)
    }
    fn read_bit(&mut self) -> Result<bool> {
        self.read_bits(1).map(|b| b == 1)
    }
    fn skip(&mut self, n: usize) -> Result<()> {
        if n > self.bits_left() {
            return Err(BitError::Eof);
        }
        self.pos += n;
        Ok(())
    }
}

/// Fixed-seed xorshift64*, so the differential test is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn reader_matches_the_pre_change_reader_on_random_operations() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..3000 {
        // Lengths cluster around the 8-byte boundary of the fast path; some buffers are zero heavy.
        let len = if rng.below(2) == 0 { rng.below(20) } else { rng.below(200) } as usize;
        let zero_heavy = rng.below(3) == 0;
        let data: Vec<u8> = (0..len).map(|_| if zero_heavy && rng.below(4) != 0 { 0 } else { rng.next() as u8 }).collect();
        let mut new = BitReader::new(&data);
        let mut old = OldReader { data: &data, pos: 0 };
        for step in 0..200 {
            let n = match rng.below(8) {
                0 => [31, 32, 33, 63, 64, 65, u32::MAX][rng.below(7) as usize],
                _ => rng.below(33) as u32,
            };
            let ctx = format!("len {len} step {step} pos {} n {n}", old.pos);
            match rng.below(8) {
                0 | 1 => assert_eq!(new.read_bits(n), old.read_bits(n), "read_bits {ctx}"),
                2 => assert_eq!(new.read_bit(), old.read_bit(), "read_bit {ctx}"),
                3 | 4 => assert_eq!(new.peek(n), old.peek(n), "peek {ctx}"),
                5 => {
                    let k = [rng.below(70) as usize, len * 8 + rng.below(10) as usize, usize::MAX - rng.below(8) as usize][rng.below(3) as usize];
                    assert_eq!(new.skip(k), old.skip(k), "skip {k} {ctx}");
                }
                6 => {
                    let p = [
                        rng.below(len as u64 * 8 + 12) as usize,
                        len * 8,
                        len.saturating_sub(7) * 8 + rng.below(8) as usize,
                        usize::MAX - rng.below(4) as usize,
                    ][rng.below(4) as usize];
                    new.seek_bits(p);
                    old.pos = p;
                }
                // `byte_align` itself overflows within 7 bits of `usize::MAX`, before and after this change.
                _ if old.pos > usize::MAX - 8 => {}
                _ => {
                    new.byte_align();
                    old.pos = old.pos.div_ceil(8) * 8;
                }
            }
            assert_eq!(new.position(), old.pos, "position {ctx}");
            assert_eq!(new.bits_left(), old.bits_left(), "bits_left {ctx}");
        }
    }
}

#[test]
fn peek_and_read_bit_at_the_edges_of_the_fast_path() {
    // Exactly 8 bytes: the last position with an 8-byte load is bit 0 of byte 0.
    let d: Vec<u8> = (1..=9).collect();
    for len in [0, 1, 7, 8, 9] {
        let data = &d[..len];
        let end = len * 8;
        for pos in [0, 1, 7, 8, end.saturating_sub(64), end.saturating_sub(57), end.saturating_sub(33), end.saturating_sub(1), end, end + 1] {
            for n in [0, 1, 7, 8, 31, 32, 33, 40, 64, u32::MAX] {
                let mut new = BitReader::new(data);
                let mut old = OldReader { data, pos };
                new.seek_bits(pos);
                assert_eq!(new.peek(n), old.peek(n), "peek len {len} pos {pos} n {n}");
                assert_eq!(new.read_bits(n), old.read_bits(n), "read_bits len {len} pos {pos} n {n}");
                assert_eq!(new.position(), old.pos);
            }
            let mut new = BitReader::new(data);
            let mut old = OldReader { data, pos };
            new.seek_bits(pos);
            assert_eq!(new.read_bit(), old.read_bit(), "read_bit len {len} pos {pos}");
            assert_eq!(new.position(), old.pos);
        }
    }
    // Positions near usize::MAX must neither panic nor read.
    let mut r = BitReader::new(&d);
    r.seek_bits(usize::MAX);
    assert_eq!(r.peek(32), 0);
    assert_eq!(r.read_bit(), Err(BitError::Eof));
    assert_eq!(r.position(), usize::MAX);
}

/// Verbatim copy of the bit-by-bit `read_code_num` that the one-peek version replaced.
fn old_read_code_num(r: &mut BitReader) -> Result<u64> {
    let mut zeros = 0u32;
    while !r.read_bit()? {
        zeros += 1;
        if zeros > 32 {
            return Err(BitError::InvalidExpGolomb);
        }
    }
    if zeros == 0 {
        return Ok(0);
    }
    let v = r.read_bits_u64(zeros)?;
    Ok((1u64 << zeros) - 1 + v)
}

fn old_read_ue(r: &mut BitReader) -> Result<u32> {
    u32::try_from(old_read_code_num(r)?).map_err(|_| BitError::InvalidExpGolomb)
}

fn old_read_se(r: &mut BitReader) -> Result<i32> {
    let k = old_read_code_num(r)? as i64;
    let v = if k & 1 == 1 { (k + 1) / 2 } else { -(k / 2) };
    i32::try_from(v).map_err(|_| BitError::InvalidExpGolomb)
}

/// Verbatim copy of the `write_ue` / `write_code_num` before the one-call version.
fn old_write_code_num(w: &mut BitWriter, v: u64) {
    let x = v + 1;
    let len = 64 - x.leading_zeros();
    let zeros = len - 1;
    for _ in 0..zeros {
        w.write_bit(false);
    }
    if len > 32 {
        w.write_bits((x >> 32) as u32, len - 32);
        w.write_bits(x as u32, 32);
    } else {
        w.write_bits(x as u32, len);
    }
}

#[test]
fn exp_golomb_reader_matches_the_pre_change_reader() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for round in 0..20_000u32 {
        let len = rng.below(17) as usize;
        let mut data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        if round % 2 == 0 && len > 0 {
            // A zero run of 28..=40 bits, then a one bit, somewhere in the buffer.
            let start = rng.below(len as u64 * 8) as usize;
            let zeros = 28 + rng.below(13) as usize;
            for p in start..(start + zeros + 1).min(len * 8) {
                data[p / 8] &= !(0x80 >> (p % 8));
            }
            if start + zeros < len * 8 {
                let p = start + zeros;
                data[p / 8] |= 0x80 >> (p % 8);
            }
        }
        // Every start position, including the last bits and past the end.
        for pos in 0..=len * 8 + 2 {
            let mut old = BitReader::new(&data);
            let mut new = BitReader::new(&data);
            old.seek_bits(pos);
            new.seek_bits(pos);
            // Several codes in a row, so the position after each result is checked too.
            for step in 0..4 {
                if (round + step) % 2 == 0 {
                    assert_eq!(new.read_ue(), old_read_ue(&mut old), "ue {data:02x?} pos {pos} step {step}");
                } else {
                    assert_eq!(new.read_se(), old_read_se(&mut old), "se {data:02x?} pos {pos} step {step}");
                }
                assert_eq!(new.position(), old.position(), "position {data:02x?} pos {pos} step {step}");
                assert_eq!(new.bits_left(), old.bits_left());
            }
        }
    }
}

#[test]
fn exp_golomb_reader_edge_codes() {
    // 31-bit (15 zeros) and 32-bit (16 zeros) codes, 33-bit-wide codes, and 32 or 33 zeros.
    for zeros in 0..=34usize {
        for extra in 0..4usize {
            let total = (2 * zeros + 1 + extra).div_ceil(8) * 8;
            let mut data = vec![0u8; total / 8];
            data[zeros / 8] |= 0x80 >> (zeros % 8);
            for tail in [0u8, 0xFF] {
                let mut d = data.clone();
                for p in 2 * zeros + 1..total {
                    if tail != 0 {
                        d[p / 8] |= 0x80 >> (p % 8);
                    }
                }
                for cut in [d.len(), d.len().saturating_sub(1)] {
                    let d = &d[..cut];
                    let (mut o, mut n) = (BitReader::new(d), BitReader::new(d));
                    assert_eq!(n.read_ue(), old_read_ue(&mut o), "ue zeros {zeros} cut {cut}");
                    assert_eq!(n.position(), o.position());
                    let (mut o, mut n) = (BitReader::new(d), BitReader::new(d));
                    assert_eq!(n.read_se(), old_read_se(&mut o), "se zeros {zeros} cut {cut}");
                    assert_eq!(n.position(), o.position());
                }
            }
        }
    }
}

#[test]
fn write_ue_matches_the_pre_change_writer() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let mut values: Vec<u32> = vec![0, 1, 2, 3, 6, 7, 8, 65534, 65535, 65536, u32::MAX - 1, u32::MAX];
    values.extend((0..32).flat_map(|b| [(1u32 << b) - 1, 1 << b, (1u32 << b) + 1]));
    values.extend((0..5000).map(|_| (rng.next() as u32) >> rng.below(32)));
    for (i, &v) in values.iter().enumerate() {
        for lead in [0u32, 1, 3, 7] {
            let mut new = BitWriter::new();
            let mut old = BitWriter::new();
            new.write_bits(0x55, lead);
            old.write_bits(0x55, lead);
            new.write_ue(v);
            old_write_code_num(&mut old, u64::from(v));
            assert_eq!(new.bit_len(), old.bit_len(), "value {v} lead {lead} (#{i})");
            assert_eq!(new.finish(), old.finish(), "value {v} lead {lead} (#{i})");
        }
    }
    // write_se goes through the same code with codeNum 2^32 at i32::MIN.
    for v in [0, 1, -1, 2, -2, i32::MAX, i32::MIN, i32::MIN + 1] {
        let mut new = BitWriter::new();
        new.write_se(v);
        new.rbsp_trailing();
        let b = new.finish();
        assert_eq!(BitReader::new(&b).read_se(), Ok(v));
    }
}

#[test]
fn lsb_reader_takes_the_low_bits_first() {
    let data = [0b1010_0101, 0xFF, 0x00, 0x12, 0x34, 0x56];
    let mut r = LsbBitReader::new(&data);
    assert_eq!(r.read_bit(), Ok(true));
    assert_eq!(r.read_bits(3), Ok(0b010));
    assert_eq!(r.read_bits(4), Ok(0b1010));
    assert_eq!(r.read_bits(0), Ok(0));
    // 32 bits at a bit offset span five bytes
    assert_eq!(r.read_bits(3), Ok(0b111));
    assert_eq!(r.read_bits(32), Ok((0x56_3412_00FFu64 >> 3) as u32));
    assert_eq!((r.position(), r.bits_left()), (43, 5));
    assert_eq!(r.read_bits(6), Err(BitError::Eof));
    assert_eq!(r.read_bits(5), Ok(0x56 >> 3));
    assert_eq!(r.read_bit(), Err(BitError::Eof));
    assert_eq!(LsbBitReader::new(&[]).read_bits(1), Err(BitError::Eof));
}
