//! Differential tests on streams built by hand: a setup header written here, with floor 0 and
//! floor 1 side by side, residues 0, 1 and 2, several submaps and channel coupling, followed by
//! random audio packets. Every packet of random bits is decodable, so the two decoders (ours and
//! ffmpeg's, the oracle; skipped when ffmpeg is absent, a failure with
//! `CRAFT_MEDIA_REQUIRE_ORACLES=1`) must return the same samples, and no encoder's habits narrow
//! what is exercised. Floor 0, residue 0 and multi-submap mappings are the parts no encoder here
//! writes.

use craft_audio::vorbis::decoder::{Decoder, decode_ogg};
use craft_media::ogg::{PageWriter, vorbis_comment};
use std::process::Command;

fn xorshift(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}

/// A least-significant-bit-first bit writer.
#[derive(Default)]
struct W {
    bytes: Vec<u8>,
    bits: usize,
}

impl W {
    fn put(&mut self, v: u64, n: u32) {
        for i in 0..n {
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let last = self.bytes.len() - 1;
            self.bytes[last] |= (((v >> i) & 1) as u8) << (self.bits % 8);
            self.bits += 1;
        }
    }
    fn flag(&mut self, b: bool) {
        self.put(u64::from(b), 1);
    }
}

fn float_bits(v: f64) -> u64 {
    // The Vorbis float32 packing: sign, 10-bit exponent biased by 788, 21-bit mantissa.
    if v == 0.0 {
        return 0;
    }
    let (sign, a) = if v < 0.0 { (1u64 << 31, -v) } else { (0, v) };
    let mut exp = 0i32;
    let mut m = a;
    while m >= (1 << 21) as f64 {
        m /= 2.0;
        exp += 1;
    }
    while m < (1 << 20) as f64 {
        m *= 2.0;
        exp -= 1;
    }
    sign | (((exp + 788) as u64) << 21) | (m.round() as u64 & 0x1F_FFFF)
}

/// A complete codebook: `entries` codewords all of length `len` (`entries == 2^len`), optionally
/// with a lookup table: `(type, minimum, delta, value_bits, multiplicands)`.
fn book(w: &mut W, dims: u32, entries: u32, len: u32, lookup: Option<(u32, f64, f64, u32, Vec<u32>)>) {
    w.put(0x56_4342, 24);
    w.put(u64::from(dims), 16);
    w.put(u64::from(entries), 24);
    w.flag(false); // not ordered
    w.flag(false); // not sparse
    for _ in 0..entries {
        w.put(u64::from(len - 1), 5);
    }
    match lookup {
        None => w.put(0, 4),
        Some((ty, min, delta, bits, mult)) => {
            w.put(u64::from(ty), 4);
            w.put(float_bits(min), 32);
            w.put(float_bits(delta), 32);
            w.put(u64::from(bits - 1), 4);
            w.flag(ty == 1); // sequence_p: cumulative for the floor 0 book
            for m in mult {
                w.put(u64::from(m), bits);
            }
        }
    }
}

struct Layout {
    channels: u8,
    /// The residue type of each of the two residues.
    residue_types: [u32; 2],
    /// The submap of each channel (0 or 1): submap 0 uses floor 0 and residue 0, submap 1 uses
    /// floor 1 and residue 1.
    mux: Vec<u8>,
    coupling: Vec<(u8, u8)>,
}

fn identification(channels: u8) -> Vec<u8> {
    let mut w = W::default();
    w.put(1, 8);
    for c in b"vorbis" {
        w.put(u64::from(*c), 8);
    }
    w.put(0, 32);
    w.put(u64::from(channels), 8);
    w.put(16_000, 32);
    w.put(0, 32);
    w.put(0, 32);
    w.put(0, 32);
    w.put(8, 4); // short blocks: 256
    w.put(10, 4); // long blocks: 1024
    w.flag(true);
    w.bytes
}

fn setup(l: &Layout) -> Vec<u8> {
    let mut w = W::default();
    w.put(5, 8);
    for c in b"vorbis" {
        w.put(u64::from(*c), 8);
    }
    // Books: 0 floor 0 vectors, 1 class book, 2 and 3 residue vectors, 4 floor 1 values.
    w.put(4, 8); // five books
    book(&mut w, 4, 16, 4, Some((1, 0.7, 0.2, 1, vec![0, 1]))); // lookup1_values(16, 4) = 2
    book(&mut w, 1, 4, 2, None);
    book(&mut w, 2, 16, 4, Some((1, -0.4, 0.25, 2, vec![0, 1, 2, 3]))); // lookup1_values(16, 2) = 4
    book(&mut w, 4, 16, 4, Some((1, -0.3, 0.6, 1, vec![0, 1])));
    book(&mut w, 1, 128, 7, None);
    // One time domain placeholder.
    w.put(0, 6);
    w.put(0, 16);
    // Floors: 0 and 1.
    w.put(1, 6); // two floors
    w.put(0, 16);
    w.put(8, 8); // order
    w.put(16_000, 16); // rate
    w.put(64, 16); // bark map size
    w.put(8, 6); // amplitude bits
    w.put(2, 8); // amplitude offset
    w.put(0, 4); // one book
    w.put(0, 8);
    w.put(1, 16);
    w.put(1, 5); // one partition
    w.put(0, 4); // class 0
    w.put(2, 3); // dimension 3
    w.put(0, 2); // no subclasses
    w.put(4 + 1, 8); // subclass book: book 4 (stored plus one)
    w.put(1, 2); // multiplier 2
    w.put(8, 4); // rangebits
    for x in [10, 40, 130] {
        w.put(x, 8);
    }
    // Residues.
    w.put(1, 6); // two residues
    for ty in l.residue_types {
        w.put(u64::from(ty), 16);
        w.put(0, 24); // begin
        w.put(if ty == 2 { 96 } else { 48 }, 24); // end
        w.put(7, 24); // partition size 8
        w.put(3, 6); // four classifications
        w.put(1, 8); // class book
        for _ in 0..4 {
            w.put(0b011, 3); // passes 0 and 1
            w.flag(false);
        }
        for class in 0..4 {
            w.put(2 + (class % 2), 8); // pass 0 uses book 2 or 3
            w.put(3 - (class % 2), 8); // pass 1 uses book 3 or 2
        }
    }
    // One mapping, two submaps.
    w.put(0, 6);
    w.put(0, 16);
    w.flag(true);
    w.put(1, 4); // two submaps
    w.flag(!l.coupling.is_empty());
    if !l.coupling.is_empty() {
        w.put(l.coupling.len() as u64 - 1, 8);
        let bits = if l.channels <= 1 { 0 } else { 8 - (l.channels - 1).leading_zeros() };
        for &(m, a) in &l.coupling {
            w.put(u64::from(m), bits);
            w.put(u64::from(a), bits);
        }
    }
    w.put(0, 2);
    for &m in &l.mux {
        w.put(u64::from(m), 4);
    }
    for sub in 0..2u64 {
        w.put(0, 8);
        w.put(sub, 8); // floor
        w.put(sub, 8); // residue
    }
    // Two modes over the one mapping.
    w.put(1, 6);
    for long in [false, true] {
        w.flag(long);
        w.put(0, 16);
        w.put(0, 16);
        w.put(0, 8);
    }
    w.flag(true);
    w.bytes
}

fn stream(l: &Layout, packets: usize, seed: &mut u32) -> Vec<u8> {
    let mut pages = PageWriter::new(7);
    let mut out = pages.header(&identification(l.channels));
    out.extend(pages.header(&vorbis_comment("synthetic", &[("TITLE", "hand built")])));
    out.extend(pages.header(&setup(l)));
    // The window flags of a long block say whether its neighbours are long, so choose the block
    // sizes first.
    let longs: Vec<bool> = (0..packets).map(|_| !xorshift(seed).is_multiple_of(3)).collect();
    let mut granule = 0i64;
    for i in 0..packets {
        let long = longs[i];
        // Packet type 0, the mode bit, and for a long block the two neighbour flags.
        let prev = i == 0 || longs[i - 1];
        let next = i + 1 >= packets || longs[i + 1];
        let mut w = W::default();
        w.flag(false);
        w.flag(long);
        if long {
            w.flag(prev);
            w.flag(next);
        }
        // Floor 0 needs a codebook number that exists, which random bits would not give, so the
        // channels that use it (the first ones) are written properly: an amplitude, book 0 and two
        // codewords of its vectors (a full tree of 4-bit codes: the code is the entry number).
        for _ in 0..l.mux.iter().take_while(|&&m| m == 0).count() {
            w.put(1 + u64::from(xorshift(seed)) % 24, 8);
            w.put(0, 1);
            for _ in 0..2 {
                let entry = xorshift(seed) % 16;
                for bit in (0..4).rev() {
                    w.put(u64::from(entry >> bit) & 1, 1);
                }
            }
        }
        // The rest is random bits, enough that no residue runs out of packet.
        let mut p = w.bytes;
        let fixed = p.len();
        p.extend((0..200 + xorshift(seed) as usize % 200).map(|_| xorshift(seed) as u8));
        let _ = fixed;
        // The bits after the fixed fields start mid-byte: the last fixed byte keeps its own bits and
        // takes random ones above them.
        let used = w.bits % 8;
        if used != 0 {
            let last = fixed - 1;
            p[last] |= (xorshift(seed) as u8) << used;
        }
        if i > 0 {
            let size = |long: bool| if long { 1024 } else { 256 };
            granule += (size(longs[i - 1]) / 4 + size(long) / 4) as i64;
        }
        out.extend(pages.packet(&p, granule));
    }
    out.extend(pages.finish(None));
    out
}

/// `tolerance`: the largest difference from ffmpeg, as a fraction of the peak output. A floor 0
/// curve is an exponential of a quotient that random coefficients can make very large, so where
/// two floors meet in one stream (ffmpeg computes in single precision, we in double) the results
/// agree only to about a thousandth of the peak.
fn run(name: &str, l: &Layout, seed: u32, tolerance: f32) {
    let mut s = seed;
    let ogg = stream(l, 40, &mut s);
    // Packet by packet our decoder must accept every one.
    let file = craft_media::ogg::open(ogg.as_slice()).expect("open");
    let st = &file.streams[0];
    let mut dec = Decoder::new(&st.headers[0], &st.headers[1], &st.headers[2]).expect("setup decodes");
    let mut total = vec![Vec::new(); usize::from(l.channels)];
    for i in 0..st.packets.len() {
        let p = file.read_packet(ogg.as_slice(), 0, i).expect("packet");
        let pcm = dec.decode_packet(&p).unwrap_or_else(|e| panic!("{name}: packet {i}: {e}"));
        for (t, c) in total.iter_mut().zip(pcm) {
            t.extend(c);
        }
    }
    let expected = usize::try_from(st.last_granule.expect("generated final granule")).expect("nonnegative granule");
    for channel in &total {
        assert_eq!(channel.len(), expected, "{name}: decoded sample count");
        assert!(channel.iter().all(|v| v.is_finite()), "{name}: non-finite output");
    }
    let whole = decode_ogg(&ogg).expect("whole file");
    assert_eq!(whole.channels, total, "{name}: packet and file decoding differ");
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("vorbis_synthetic");
    let path = dir.join(format!("{name}.ogg"));
    std::fs::write(&path, &ogg).expect("write fixture");
    let out = Command::new(&ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&path)
        .args(["-f", "f32le", "-acodec", "pcm_f32le", "-"])
        .output()
        .expect("run ffmpeg");
    assert!(out.status.success(), "{name}: ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr));
    let all: Vec<f32> = out.stdout.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    let ch = usize::from(l.channels);
    let theirs: Vec<Vec<f32>> = (0..ch).map(|c| all.iter().skip(c).step_by(ch).copied().collect()).collect();
    assert!(theirs.iter().flatten().all(|v| v.is_finite()), "{name}: non-finite ffmpeg output");
    for channel in &theirs {
        assert!(channel.len() >= expected, "{name}: ffmpeg returned fewer than the expected samples");
    }
    // ffmpeg puts the channels of surround streams in its own order, so each of ours is matched
    // with the channel of theirs it agrees with best, and every one of theirs is used once.
    let mut used = vec![false; ch];
    #[allow(clippy::needless_range_loop)]
    for c in 0..ch {
        let mut best: Option<(usize, f32, f32)> = None;
        for (k, t) in theirs.iter().enumerate().filter(|(k, _)| !used[*k]) {
            let n = total[c].len().min(t.len());
            assert!(n > 1000, "{name}: only {n} samples to compare");
            let scale = total[c][..n].iter().fold(1e-3f32, |m, v| m.max(v.abs()));
            let worst = (0..n).map(|i| (total[c][i] - t[i]).abs()).fold(0f32, f32::max);
            if best.is_none_or(|(_, w, sc)| worst / scale < w / sc) {
                best = Some((k, worst, scale));
            }
        }
        let (k, worst, scale) = best.expect("a free channel");
        used[k] = true;
        assert!(worst <= tolerance * scale + 1e-5, "{name}: channel {c} differs from ffmpeg's channel {k} by {worst} (peak {scale})");
    }
}

#[test]
fn mono_floor_zero_and_residue_zero() {
    run("mono_f0_r0", &Layout { channels: 1, residue_types: [0, 0], mux: vec![0], coupling: vec![] }, 11, 1e-4);
}

#[test]
fn mono_floor_one_and_residue_one() {
    run("mono_f1_r1", &Layout { channels: 1, residue_types: [1, 1], mux: vec![1], coupling: vec![] }, 12, 1e-4);
}

#[test]
fn two_submaps_floor_zero_and_one() {
    run("stereo_mixed", &Layout { channels: 2, residue_types: [0, 1], mux: vec![0, 1], coupling: vec![] }, 13, 3e-3);
}

#[test]
fn coupled_stereo_residue_two() {
    run("stereo_coupled_r2", &Layout { channels: 2, residue_types: [2, 2], mux: vec![1, 1], coupling: vec![(0, 1)] }, 14, 1e-4);
}

#[test]
fn six_channels_with_coupling_and_submaps() {
    let l = Layout { channels: 6, residue_types: [1, 2], mux: vec![0, 0, 0, 1, 1, 1], coupling: vec![(0, 3), (1, 2), (4, 5)] };
    run("six", &l, 15, 3e-3);
}

#[test]
fn mono_floor_one_and_residue_zero() {
    run("mono_f1_r0", &Layout { channels: 1, residue_types: [0, 0], mux: vec![1], coupling: vec![] }, 16, 1e-4);
}

#[test]
fn mono_floor_zero_and_residue_one() {
    run("mono_f0_r1", &Layout { channels: 1, residue_types: [1, 1], mux: vec![0], coupling: vec![] }, 17, 1e-4);
}

/// A packet that names a floor 0 codebook that does not exist cannot be decoded (§6.2.2); the
/// decoder says so and the stream goes on with the next packet.
#[test]
fn a_floor_zero_codebook_that_does_not_exist_is_a_packet_error() {
    let l = Layout { channels: 1, residue_types: [1, 1], mux: vec![0], coupling: vec![] };
    let mut seed = 5u32;
    let ogg = stream(&l, 6, &mut seed);
    let file = craft_media::ogg::open(ogg.as_slice()).expect("open");
    let st = &file.streams[0];
    let mut dec = Decoder::new(&st.headers[0], &st.headers[1], &st.headers[2]).expect("decoder");
    let good = file.read_packet(ogg.as_slice(), 0, 0).expect("packet");
    assert!(dec.decode_packet(&good).is_ok());
    // A short block: type 0, mode 0, then the 8 amplitude bits, then book number 1 (only 0 exists).
    let mut w = W::default();
    w.flag(false);
    w.flag(false);
    w.put(9, 8);
    w.put(1, 1);
    w.put(0, 64);
    assert!(matches!(dec.decode_packet(&w.bytes), Err(craft_audio::vorbis::decoder::Error::Packet(_))));
    assert!(dec.decode_packet(&good).is_ok(), "the decoder carries on");
}
