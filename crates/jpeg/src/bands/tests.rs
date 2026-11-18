use super::*;
use crate::{Decoded, Samples, decode};

// --- Helpers ----------------------------------------------------------------------------------

/// The transform byte of the stream's APP14 `Adobe` segment, as an app would pass it.
fn adobe(b: &[u8]) -> Option<u8> {
    let at = b.windows(9).position(|w| w[..2] == [0xFF, 0xEE] && &w[4..9] == b"Adobe")?;
    b.get(at + 4 + 11).copied()
}

/// The whole-image decode of `b`, with the transform its own APP14 segment states.
fn whole(b: &[u8]) -> Result<Decoded, Error> {
    decode(b, &Limits::default(), adobe(b))
}

/// The 8-bit samples of a decode.
fn eight(d: &Decoded) -> &[u8] {
    match &d.samples {
        Samples::U8(v) => v,
        Samples::U16(_) => panic!("16-bit samples"),
    }
}

/// Every band, read in order from band 0, joined; and whether the reader saw damage.
fn in_order(b: &Bands) -> Result<(Vec<u8>, bool), Error> {
    let mut r = b.reader();
    let mut out = Vec::new();
    let mut expect = 0;
    while let Some(band) = r.next_band()? {
        assert_eq!(band.index, expect);
        assert_eq!(band.y, expect as u32 * b.band_rows());
        assert_eq!(band.samples.len(), band.rows as usize * b.width() as usize * usize::from(b.components()));
        out.extend_from_slice(band.samples);
        expect += 1;
    }
    assert_eq!(expect, b.bands());
    Ok((out, r.damaged()))
}

/// Every band, read from each seek point to the next, the stretches taken in a scrambled order
/// by two readers that take turns; joined in image order.
fn by_seeking(b: &Bands) -> Result<(Vec<u8>, bool), Error> {
    let points: Vec<usize> = b.seek_points().collect();
    let band_bytes = b.band_rows() as usize * b.width() as usize * usize::from(b.components());
    let total = b.width() as usize * b.height() as usize * usize::from(b.components());
    let mut out = vec![0u8; total];
    let mut readers = [b.reader(), b.reader()];
    let mut order: Vec<usize> = (0..points.len()).collect();
    order.reverse();
    order.rotate_left(points.len() / 3);
    for (turn, &k) in order.iter().enumerate() {
        let r = &mut readers[turn % 2];
        r.seek(points[k])?;
        let end = points.get(k + 1).copied().unwrap_or(b.bands());
        for index in points[k]..end {
            let band = r.next_band()?.expect("a band");
            assert_eq!(band.index, index);
            let at = index * band_bytes;
            out[at..at + band.samples.len()].copy_from_slice(band.samples);
        }
    }
    let damaged = readers.iter().any(Reader::damaged);
    Ok((out, damaged))
}

struct Rng(u64);

impl Rng {
    /// The next value of the xorshift* sequence.
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value in `lo..=hi`.
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next() % (hi - lo + 1) as u64) as i32
    }
}

/// A test picture: gradients, edges and noise in every channel.
fn picture(w: usize, h: usize, channels: usize, seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed | 1);
    let mut px = Vec::with_capacity(w * h * channels);
    for y in 0..h {
        for x in 0..w {
            for c in 0..channels {
                let base = match c {
                    0 => x * 255 / w.max(1),
                    1 => y * 255 / h.max(1),
                    2 => {
                        if (x / 5 + y / 3) % 2 == 0 {
                            40
                        } else {
                            220
                        }
                    }
                    _ => (x * 3 + y * 7) % 256,
                };
                px.push((base as i32 + rng.range(-20, 20)).clamp(0, 255) as u8);
            }
        }
    }
    px
}

/// A `w`×`h` picture encoded by `jpeg-encoder` at quality 85, with a restart interval of
/// `restart` MCUs (0 for none).
fn jpeg_encoder_stream(w: u16, h: u16, color: jpeg_encoder::ColorType, sampling: jpeg_encoder::SamplingFactor, restart: u16) -> Vec<u8> {
    let channels = match color {
        jpeg_encoder::ColorType::Luma => 1,
        jpeg_encoder::ColorType::Rgb => 3,
        _ => 4,
    };
    let px = picture(usize::from(w), usize::from(h), channels, u64::from(w) * 31 + u64::from(h));
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, 85);
    enc.set_sampling_factor(sampling);
    if restart > 0 {
        enc.set_restart_interval(restart);
    }
    enc.encode(&px, w, h, color).unwrap();
    out
}

// --- A test-only baseline Huffman encoder (T.81 F.1.2), for layouts other encoders don't write ---

/// A component of [`Stream`]: identifier, sampling factors and quantization table.
#[derive(Clone, Copy)]
struct C {
    id: u8,
    h: usize,
    v: usize,
    tq: u8,
}

/// A sequential Huffman stream with one interleaved scan (or a single component), random
/// coefficients and canonical code tables in which every symbol has the same length.
struct Stream {
    w: usize,
    h: usize,
    comps: Vec<C>,
    restart: usize,
    seed: u64,
    /// Number the restart markers from this instead of 0.
    rst_base: u8,
    /// Instead of random coefficients, blocks with only a DC term: every sample of component
    /// `i` reconstructs to `flat[i]`.
    flat: Option<[u8; 4]>,
}

/// The AC symbols a baseline scan uses (F.1.2.2): EOB, ZRL and run/size 0..15/1..10.
fn ac_symbols() -> Vec<u8> {
    let mut v = vec![0x00, 0xF0];
    for r in 0..16u8 {
        for s in 1..=10u8 {
            v.push(r << 4 | s);
        }
    }
    v
}

struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    /// Appends the `len` low bits of `v`, stuffing a 0 byte after every `FF` (F.1.2.3).
    fn put(&mut self, v: u32, len: u32) {
        if len == 0 {
            return;
        }
        self.acc = (self.acc << len) | (u64::from(v) & ((1u64 << len) - 1));
        self.n += len;
        while self.n >= 8 {
            let b = (self.acc >> (self.n - 8)) as u8;
            self.out.push(b);
            if b == 0xFF {
                self.out.push(0);
            }
            self.n -= 8;
        }
    }

    /// Pads with 1 bits to a byte boundary (F.1.2.3).
    fn flush(&mut self) {
        let pad = (8 - self.n % 8) % 8;
        if pad > 0 {
            self.put((1 << pad) - 1, pad);
        }
        self.acc = 0;
    }
}

/// The size category of a coefficient or DC difference (F.1.2.1): its magnitude's bit length.
fn category(v: i32) -> u32 {
    32 - v.unsigned_abs().leading_zeros()
}

/// The `s` low bits that code `v` of category `s` (F.1.2.1).
fn magnitude(v: i32, s: u32) -> u32 {
    let v = if v < 0 { v - 1 } else { v };
    (v as u32) & ((1u32 << s) - 1)
}

impl Stream {
    /// A `w`×`h` stream whose components have the given (H, V) sampling factors; the first uses
    /// quantization table 0, the others table 1.
    fn new(w: usize, h: usize, comps: &[(usize, usize)], restart: usize, seed: u64) -> Self {
        let comps = comps.iter().enumerate().map(|(i, &(h, v))| C { id: i as u8 + 1, h, v, tq: (i > 0) as u8 }).collect();
        Stream { w, h, comps, restart, seed, rst_base: 0, flat: None }
    }

    /// Quantization table `t` (zigzag order): `Q0` is `1 + t`.
    fn quant(t: u8) -> [u8; 64] {
        std::array::from_fn(|k| 1 + (k as u8) / 4 + t)
    }

    /// The JPEG file: SOI, the tables, the frame, one scan and EOI.
    fn encode(&self) -> Vec<u8> {
        let hmax = self.comps.iter().map(|c| c.h).max().unwrap();
        let vmax = self.comps.iter().map(|c| c.v).max().unwrap();
        let single = self.comps.len() == 1;
        let (mcux, mcuy) = if single { (self.w.div_ceil(8), self.h.div_ceil(8)) } else { (self.w.div_ceil(8 * hmax), self.h.div_ceil(8 * vmax)) };
        let mut rng = Rng(self.seed | 1);
        let mut s = vec![0xFF, 0xD8];
        let seg = |s: &mut Vec<u8>, m: u8, body: &[u8]| {
            s.extend([0xFF, m]);
            s.extend(((body.len() + 2) as u16).to_be_bytes());
            s.extend(body);
        };
        for t in 0..2u8 {
            let mut b = vec![t];
            b.extend(Self::quant(t));
            seg(&mut s, 0xDB, &b);
        }
        let mut sof = vec![8];
        sof.extend((self.h as u16).to_be_bytes());
        sof.extend((self.w as u16).to_be_bytes());
        sof.push(self.comps.len() as u8);
        for c in &self.comps {
            sof.extend([c.id, (c.h << 4 | c.v) as u8, c.tq]);
        }
        seg(&mut s, 0xC0, &sof);
        // DC: 12 symbols of 4 bits; AC: 162 symbols of 8 bits.
        let mut dht = vec![0x00];
        dht.extend([0, 0, 0, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        dht.extend(0..12u8);
        let ac = ac_symbols();
        dht.push(0x10);
        dht.extend([0, 0, 0, 0, 0, 0, 0, ac.len() as u8, 0, 0, 0, 0, 0, 0, 0, 0]);
        dht.extend(&ac);
        seg(&mut s, 0xC4, &dht);
        if self.restart > 0 {
            seg(&mut s, 0xDD, &(self.restart as u16).to_be_bytes());
        }
        let mut sos = vec![self.comps.len() as u8];
        for c in &self.comps {
            sos.extend([c.id, 0x00]);
        }
        sos.extend([0, 63, 0]);
        seg(&mut s, 0xDA, &sos);
        let mut bw = BitWriter { out: s, acc: 0, n: 0 };
        let mut pred = vec![0i32; self.comps.len()];
        let total = mcux * mcuy;
        let blocks_per_mcu: Vec<usize> = self.comps.iter().map(|c| if single { 1 } else { c.h * c.v }).collect();
        for m in 0..total {
            if self.restart > 0 && m > 0 && m % self.restart == 0 {
                bw.flush();
                let n = (m / self.restart - 1) as u8;
                bw.out.extend([0xFF, 0xD0 + (n.wrapping_add(self.rst_base)) % 8]);
                pred.iter_mut().for_each(|p| *p = 0);
            }
            for (ci, &n) in blocks_per_mcu.iter().enumerate() {
                for _ in 0..n {
                    // A DC term d alone reconstructs to d × Q0 / 8 + 128 (A.3.3).
                    let dc = match self.flat {
                        Some(f) => (i32::from(f[ci]) - 128) * 8 / i32::from(Self::quant(self.comps[ci].tq)[0]),
                        None => rng.range(-300, 300),
                    };
                    let diff = dc - pred[ci];
                    pred[ci] = dc;
                    let t = category(diff);
                    bw.put(t, 4);
                    bw.put(magnitude(diff, t), t);
                    let mut run = 0;
                    let last = if self.flat.is_some() { 0 } else { rng.next() % 4 };
                    let last = match last {
                        0 => 0,
                        1 => 5,
                        2 => 20,
                        _ => 63,
                    };
                    for k in 1..64 {
                        let v = if k <= last && rng.next().is_multiple_of(3) { rng.range(-40, 40) / (1 + k / 8) } else { 0 };
                        if v == 0 {
                            run += 1;
                            continue;
                        }
                        while run > 15 {
                            bw.put(ac.iter().position(|&a| a == 0xF0).unwrap() as u32, 8);
                            run -= 16;
                        }
                        let sz = category(v);
                        let sym = (run << 4 | sz) as u8;
                        bw.put(ac.iter().position(|&a| a == sym).unwrap() as u32, 8);
                        bw.put(magnitude(v, sz), sz);
                        run = 0;
                    }
                    if run > 0 {
                        bw.put(0, 8);
                    }
                }
            }
        }
        bw.flush();
        let mut out = bw.out;
        out.extend([0xFF, 0xD9]);
        out
    }
}

// --- Tests ------------------------------------------------------------------------------------

/// Checks `stream` decodes in bands to exactly the whole-image decode's samples, in order and by
/// seeking. Returns the bands, or `None` when the stream isn't one bands decode.
fn check_equal<'a>(stream: &'a [u8], what: &str) -> Option<Bands<'a>> {
    let mut b = match Bands::new(stream, &Limits::default(), adobe(stream)) {
        Ok(b) => b,
        Err(Error::Unsupported(_)) => return None,
        Err(e) => panic!("{what}: {e}"),
    };
    let reference = whole(stream).unwrap_or_else(|e| panic!("{what}: {e}"));
    assert_eq!((b.width(), b.height(), b.components()), (reference.width, reference.height, reference.components), "{what}");
    let (seq, damaged) = in_order(&b).unwrap();
    assert!(!damaged, "{what}");
    assert!(seq == eight(&reference), "{what}: bands differ from the whole-image decode");
    b.find_seek_points();
    let (sought, damaged) = by_seeking(&b).unwrap();
    assert!(!damaged, "{what}");
    assert!(sought == seq, "{what}: seeking changes the samples");
    Some(b)
}

#[test]
fn bands_equal_the_whole_image_decode() {
    use jpeg_encoder::{ColorType, SamplingFactor};
    let mut checked = 0;
    for color in [ColorType::Luma, ColorType::Rgb, ColorType::Cmyk, ColorType::CmykAsYcck] {
        for sampling in [SamplingFactor::R_4_4_4, SamplingFactor::R_4_2_2, SamplingFactor::R_4_2_0, SamplingFactor::R_4_4_0] {
            for (w, h) in [(37u16, 29u16), (1, 1), (64, 64), (100, 9), (9, 70)] {
                let plain = jpeg_encoder_stream(w, h, color, sampling, 0);
                let what = format!("{color:?} {sampling:?} {w}x{h}");
                let Some(b) = check_equal(&plain, &what) else { panic!("{what}: not decoded in bands") };
                let row = b.mcux as u16;
                for restart in [1, 7, row, 2 * row] {
                    let s = jpeg_encoder_stream(w, h, color, sampling, restart);
                    let b = check_equal(&s, &format!("{what} restart {restart}")).unwrap();
                    if restart % row == 0 {
                        // Every band (or every other one) begins a restart interval.
                        let expect = b.bands().div_ceil(usize::from(restart / row));
                        assert_eq!(b.seek_points().len(), expect, "{what} restart {restart}");
                    }
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 4 * 4 * 5 * 4);
}

/// Interleaved layouts `jpeg-encoder` doesn't write (factors of 3 and 4 across and down), from
/// the test encoder, with and without restart intervals.
#[test]
fn interleaved_layouts_with_any_sampling_factors() {
    let layouts: [&[(usize, usize)]; 8] = [
        &[(4, 1), (1, 1), (1, 1)],
        &[(4, 2), (1, 1), (1, 1)],
        &[(1, 4), (1, 1), (1, 1)],
        &[(2, 4), (1, 2), (1, 2)],
        &[(3, 1), (1, 1), (1, 1)],
        &[(1, 3), (1, 1), (1, 1)],
        &[(2, 2), (1, 1), (1, 1), (2, 2)],
        &[(2, 2)],
    ];
    for (n, comps) in layouts.iter().enumerate() {
        for (w, h) in [(53, 47), (8, 8), (200, 33)] {
            let plain = Stream::new(w, h, comps, 0, n as u64 * 7 + w as u64).encode();
            let what = format!("{comps:?} {w}x{h}");
            let b = check_equal(&plain, &what).expect("decoded in bands");
            let down = comps.len() > 1 && comps.iter().any(|c| c.1 < comps.iter().map(|c| c.1).max().unwrap_or(1));
            assert_eq!(b.halo(), u32::from(down), "{what}");
            let row = b.mcux;
            for restart in [1, 5, row, 3 * row] {
                let s = Stream { restart, ..Stream::new(w, h, comps, 0, n as u64 * 7 + w as u64) }.encode();
                let b = check_equal(&s, &format!("{what} restart {restart}")).unwrap();
                if restart % row == 0 {
                    assert_eq!(b.seek_points().len(), b.bands().div_ceil(restart / row), "{what} restart {restart}");
                }
            }
        }
    }
}

/// The test encoder's streams decode as `zune-jpeg` decodes them, within the difference between
/// its integer inverse transform and upsampling and these (mean under 1.5, as for
/// `decode`).
#[test]
fn bands_match_zune_jpeg() {
    use jpeg_encoder::{ColorType, SamplingFactor};
    for sampling in [SamplingFactor::R_4_4_4, SamplingFactor::R_4_2_0, SamplingFactor::R_4_2_2] {
        let s = jpeg_encoder_stream(91, 67, ColorType::Rgb, sampling, 4);
        let b = Bands::new(&s, &Limits::default(), None).unwrap();
        let (ours, _) = in_order(&b).unwrap();
        let mut dec = zune_jpeg::JpegDecoder::new(zune_jpeg::zune_core::bytestream::ZCursor::new(&s));
        let zune = dec.decode().unwrap();
        assert_eq!(ours.len(), zune.len());
        let diffs: Vec<u8> = ours.iter().zip(&zune).map(|(a, b)| a.abs_diff(*b)).collect();
        let mean = diffs.iter().map(|&d| f32::from(d)).sum::<f32>() / diffs.len() as f32;
        let max = diffs.iter().copied().max().unwrap();
        let (bound_mean, bound_max) = if sampling == SamplingFactor::R_4_4_4 { (0.75, 4) } else { (1.5, 255) };
        assert!(mean < bound_mean && max <= bound_max, "{sampling:?}: mean {mean}, max {max}");
    }
}

/// Without restart intervals, damaged data leaves the rest of the scan at zero, exactly as the
/// whole-image decode does. Data that fails in the first MCU, which the whole-image decode
/// refuses, is a damaged (grey) image here.
#[test]
fn damage_without_restarts_matches_the_whole_image_decode() {
    let s = Stream::new(61, 45, &[(2, 2), (1, 1), (1, 1)], 0, 3).encode();
    let sos = s.windows(2).rposition(|w| w == [0xFF, 0xDA]).unwrap();
    let data = sos + 2 + 12;
    let mut cuts: Vec<usize> = (data..s.len()).step_by(37).collect();
    cuts.extend([data, data + 1, s.len() - 2]);
    for cut in cuts {
        let t = &s[..cut];
        let reference = decode(t, &Limits::default(), None);
        let ours = Bands::new(t, &Limits::default(), None).and_then(|b| in_order(&b));
        let (o, damaged) = ours.unwrap_or_else(|e| panic!("cut at {cut}: {e}"));
        match reference {
            Ok(r) => {
                assert!(o == eight(&r), "cut at {cut}");
                assert_eq!(damaged, r.damaged, "cut at {cut}");
            }
            Err(_) => assert!(damaged, "cut at {cut}"),
        }
    }
    // Corrupt data in the middle of the scan.
    let mut t = s.clone();
    for b in &mut t[data + 200..data + 230] {
        *b = 0xFE;
    }
    let r = decode(&t, &Limits::default(), None).unwrap();
    let (o, damaged) = in_order(&Bands::new(&t, &Limits::default(), None).unwrap()).unwrap();
    assert!(r.damaged && damaged);
    assert!(o == eight(&r));
}

/// With restart intervals, damage stays inside its interval: the next one decodes as in the
/// undamaged file, whether read in order or by seeking.
#[test]
fn damage_with_restarts_resumes_at_the_next_interval() {
    let (w, h) = (80, 96);
    let s = Stream { restart: 10, ..Stream::new(w, h, &[(2, 2), (1, 1), (1, 1)], 0, 9) }.encode();
    let clean = in_order(&Bands::new(&s, &Limits::default(), None).unwrap()).unwrap().0;
    // 4:2:0, 5 MCUs a line: each interval of 10 MCUs is two bands of 16 rows. Corrupt the
    // second interval (rows 32..64) with bytes no code begins with.
    let markers: Vec<usize> = s.windows(2).enumerate().filter(|(_, p)| p[0] == 0xFF && (0xD0..=0xD7).contains(&p[1])).map(|(i, _)| i).collect();
    assert_eq!(markers.len(), 2);
    let mut t = s.clone();
    for b in &mut t[markers[0] + 20..markers[0] + 40] {
        *b = 0xFE;
    }
    let mut b = Bands::new(&t, &Limits::default(), None).unwrap();
    let (seq, damaged) = in_order(&b).unwrap();
    assert!(damaged);
    b.find_seek_points();
    assert_eq!(b.seek_points().collect::<Vec<_>>(), [0, 2, 4]);
    let (sought, damaged) = by_seeking(&b).unwrap();
    assert!(damaged);
    assert!(sought == seq);
    // Row 31 interpolates chroma with the damaged interval's first row and row 64 with its last;
    // the rows before and after decode as in the undamaged file.
    let row = w * 3;
    assert!(seq[..31 * row] == clean[..31 * row]);
    assert!(seq[32 * row..64 * row] != clean[32 * row..64 * row]);
    assert!(seq[65 * row..] == clean[65 * row..]);
    // The whole-image decode stops at the damage instead: the same up to it.
    let reference = decode(&t, &Limits::default(), None).unwrap();
    assert!(reference.damaged);
    assert!(seq[..31 * row] == eight(&reference)[..31 * row]);
    assert!(seq[65 * row..] != eight(&reference)[65 * row..]);

    // The second interval's data cut short (its last bytes gone, the marker after it kept): it
    // runs out of data, and the third still decodes as in the undamaged file.
    let mut t = s.clone();
    t.drain(markers[1] - 30..markers[1]);
    let mut b = Bands::new(&t, &Limits::default(), None).unwrap();
    let (seq, damaged) = in_order(&b).unwrap();
    assert!(damaged);
    b.find_seek_points();
    assert_eq!(b.seek_points().len(), 3);
    assert!(by_seeking(&b).unwrap().0 == seq);
    assert!(seq[..31 * row] == clean[..31 * row]);
    assert!(seq[65 * row..] == clean[65 * row..]);
}

/// Data that fails in the first MCU damages only its interval: with restarts the rest of the
/// image decodes as in the undamaged file.
#[test]
fn a_bad_first_interval_is_damage_not_an_error() {
    let (w, h) = (80, 96);
    let s = Stream { restart: 10, ..Stream::new(w, h, &[(2, 2), (1, 1), (1, 1)], 0, 9) }.encode();
    let clean = in_order(&Bands::new(&s, &Limits::default(), None).unwrap()).unwrap().0;
    let sos = s.windows(2).rposition(|p| p == [0xFF, 0xDA]).unwrap();
    let mut t = s.clone();
    for b in &mut t[sos + 14..sos + 30] {
        *b = 0xFE;
    }
    assert!(decode(&t, &Limits::default(), None).is_err(), "the whole-image decode refuses it");
    let mut b = Bands::new(&t, &Limits::default(), None).unwrap();
    let (seq, damaged) = in_order(&b).unwrap();
    assert!(damaged);
    b.find_seek_points();
    assert!(by_seeking(&b).unwrap().0 == seq);
    let row = w * 3;
    assert!(seq[33 * row..] == clean[33 * row..]);
}

/// Bytes added to an interval, or moved within it, leave more than its padding before the
/// restart marker: the image is damaged, whether or not a code failed, read in order or by
/// seeking.
#[test]
fn data_left_before_a_restart_marker_is_damage() {
    let (w, h) = (64, 64);
    let s = Stream { restart: 4, ..Stream::new(w, h, &[(1, 1), (1, 1), (1, 1)], 0, 21) }.encode();
    let markers: Vec<usize> = s.windows(2).enumerate().filter(|(_, p)| p[0] == 0xFF && (0xD0..=0xD7).contains(&p[1])).map(|(i, _)| i).collect();
    // Two bytes before the third marker; a stuffed FF 00 before the fifth.
    for insert in [&[0x12u8, 0x34][..], &[0xFF, 0x00]] {
        for k in [2, 4] {
            let mut t = s.clone();
            t.splice(markers[k]..markers[k], insert.iter().copied());
            let mut b = Bands::new(&t, &Limits::default(), None).unwrap();
            let (seq, damaged) = in_order(&b).unwrap();
            assert!(damaged, "{insert:?} before marker {k}");
            b.find_seek_points();
            let (sought, hurt) = by_seeking(&b).unwrap();
            assert!(hurt && sought == seq, "{insert:?} before marker {k}");
        }
    }
    // Fill bytes before a marker are not data.
    let mut t = s.clone();
    t.splice(markers[2]..markers[2], [0xFF, 0xFF]);
    assert!(!in_order(&Bands::new(&t, &Limits::default(), None).unwrap()).unwrap().1);
}

/// Bytes added after the last MCU leave more than its padding before EOI: damaged, with restart
/// intervals or without, read in order or by seeking, with the clean stream's samples. Fill
/// bytes there are not data.
#[test]
fn data_left_before_eoi_is_damage() {
    for restart in [0, 4] {
        let s = Stream { restart, ..Stream::new(64, 64, &[(2, 2), (1, 1), (1, 1)], 0, 23) }.encode();
        let eoi = s.len() - 2;
        assert_eq!(s[eoi..], [0xFF, 0xD9]);
        let (clean, hurt) = in_order(&Bands::new(&s, &Limits::default(), None).unwrap()).unwrap();
        assert!(!hurt, "restart {restart}");
        for (insert, damaged) in [(&[0x12u8, 0x34][..], true), (&[0xFF, 0x00], true), (&[0xFF, 0xFF], false)] {
            let mut t = s.clone();
            t.splice(eoi..eoi, insert.iter().copied());
            let mut b = Bands::new(&t, &Limits::default(), None).unwrap();
            let (seq, hurt) = in_order(&b).unwrap();
            assert_eq!(hurt, damaged, "{insert:?} before EOI, restart {restart}");
            // The added bytes follow the last MCU's codes: the samples are the clean stream's.
            assert!(seq == clean, "{insert:?} before EOI, restart {restart}");
            b.find_seek_points();
            let (sought, hurt) = by_seeking(&b).unwrap();
            assert!(hurt == damaged && sought == seq, "{insert:?} before EOI, restart {restart}");
        }
    }
}

/// Restart markers misnumbered, missing (the file cut short), added inside an interval or
/// removed: seek points follow the markers in the data as a reader in order meets them, so
/// reading from them gives the samples reading in order does.
#[test]
fn damaged_restart_markers_seek_as_reading_in_order_does() {
    let base = Stream { restart: 4, ..Stream::new(32, 40, &[(1, 1)], 0, 5) };
    let good = base.encode();
    let mut b = Bands::new(&good, &Limits::default(), None).unwrap();
    b.find_seek_points();
    assert_eq!(b.seek_points().len(), 5);
    let markers: Vec<usize> = good.windows(2).enumerate().filter(|(_, p)| p[0] == 0xFF && (0xD0..=0xD7).contains(&p[1])).map(|(i, _)| i).collect();
    let misnumbered = Stream { rst_base: 1, ..base }.encode();
    let cut = good[..markers[2] + 10].to_vec();
    let mut added = good.clone();
    added.splice(markers[1] - 7..markers[1] - 7, [0xFF, 0xD5]);
    let mut removed = good.clone();
    removed.drain(markers[2]..markers[2] + 2);
    for (what, t, points, damaged) in [
        ("misnumbered", misnumbered, vec![0, 1, 2, 3, 4], false),
        ("cut", cut, vec![0, 1, 2, 3], true),
        ("added", added, vec![0, 1, 2, 3, 4], true),
        ("removed", removed, vec![0, 1, 2, 3], true),
    ] {
        let mut b = Bands::new(&t, &Limits::default(), None).unwrap();
        let (seq, hurt) = in_order(&b).unwrap();
        assert_eq!(hurt, damaged, "{what}");
        b.find_seek_points();
        assert_eq!(b.seek_points().collect::<Vec<_>>(), points, "{what}");
        let (sought, hurt) = by_seeking(&b).unwrap();
        assert_eq!(hurt, damaged, "{what}");
        assert!(sought == seq, "{what}: seeking changes the samples");
    }
}

#[test]
fn other_processes_and_split_scans_are_unsupported() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/");
    for f in
        ["tests/data/testorig12.jpg", "tests/data/testimgari.jpg", "tests/data/testorig-ari-progressive.jpg", "fuzz/corpus/decode/sof1-12bit.jpg"]
    {
        let b = std::fs::read(format!("{dir}{f}")).unwrap();
        assert!(matches!(Bands::new(&b, &Limits::default(), None), Err(Error::Unsupported(_))), "{f}");
    }
    let mut progressive = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut progressive, 80);
    enc.set_progressive(true);
    enc.encode(&picture(16, 16, 3, 1), 16, 16, jpeg_encoder::ColorType::Rgb).unwrap();
    assert!(matches!(Bands::new(&progressive, &Limits::default(), None), Err(Error::Unsupported(_))));
    // jpeg-encoder writes each component in its own scan for factors of 4.
    let split = jpeg_encoder_stream(32, 32, jpeg_encoder::ColorType::Rgb, jpeg_encoder::SamplingFactor::R_4_1_1, 0);
    assert_eq!(split.windows(2).filter(|w| *w == [0xFF, 0xDA]).count(), 3);
    assert!(matches!(Bands::new(&split, &Limits::default(), None), Err(Error::Unsupported(_))));
}

/// A scan that names a component twice breaks T.81 (B.2.3: its components follow the frame's
/// order): malformed, not a valid feature left to the whole-image decode.
#[test]
fn a_component_named_twice_in_the_scan_is_malformed() {
    let mut s = Stream::new(16, 16, &[(1, 1), (1, 1), (1, 1)], 0, 3).encode();
    let sos = s.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    assert_eq!(s[sos + 4], 3);
    s[sos + 7] = s[sos + 5];
    assert!(matches!(Bands::new(&s, &Limits::default(), None), Err(Error::Malformed(_))));
}

#[test]
fn limits_are_checked_from_the_headers() {
    let s = Stream::new(40, 30, &[(2, 1), (1, 1), (1, 1)], 0, 2).encode();
    let b = Bands::new(&s, &Limits::default(), None).unwrap();
    // The chroma's column maps: 40 entries of 24 bytes each.
    let need = b.reader_bytes() + 2 * 40 * 24;
    for (max_pixels, max_alloc, ok) in [(1200, need, true), (1199, need, false), (1200, need - 1, false)] {
        let limits = Limits { max_pixels, max_alloc };
        assert_eq!(Bands::new(&s, &limits, None).is_ok(), ok, "{max_pixels} {max_alloc}");
    }
    // A frame of 65535² under the default limits: refused before anything is sized from it.
    let mut big = s.clone();
    let sof = big.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    big[sof + 5..sof + 9].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    assert!(matches!(Bands::new(&big, &Limits::default(), None), Err(Error::LimitExceeded(_))));
    let wide = Limits { max_pixels: u64::MAX, ..Limits::default() };
    let b = Bands::new(&big, &wide, None).unwrap();
    assert!(b.reader_bytes() < 64 << 20);
}

#[test]
fn readers_seek_only_to_seek_points() {
    let s = Stream { restart: 5, ..Stream::new(40, 40, &[(1, 1), (1, 1), (1, 1)], 0, 4) }.encode();
    let mut b = Bands::new(&s, &Limits::default(), None).unwrap();
    assert!(b.reader().seek(1).is_err());
    b.find_seek_points();
    assert_eq!(b.seek_points().collect::<Vec<_>>(), [0, 1, 2, 3, 4]);
    let mut r = b.reader();
    assert!(r.seek(5).is_err());
    r.seek(4).unwrap();
    assert_eq!(r.next_band().unwrap().map(|band| band.index), Some(4));
    assert!(r.next_band().unwrap().is_none());
}

/// CMYK and YCCK samples of known ink, checked against T.81's reconstruction rather than
/// against the whole-image decode (both renderers share the colour conversion). Adobe CMYK is
/// stored inverted (255 = no ink); YCCK's C, M and Y come out of YCbCr as ink, and K inverted.
#[test]
fn four_component_samples_are_ink_amounts() {
    let comps = [(1, 1), (1, 1), (1, 1), (1, 1)];
    // YCCK with neutral chroma gives C, M and Y equal to the luma. With Cb 150 and Cr 110 (T.871):
    // C = 100 + 1.402 × −18 = 74.76, M = 100 − 0.344136 × 22 − 0.714136 × −18 = 105.28 and
    // Y = 100 + 1.772 × 22 = 138.98.
    for (transform, stored, ink) in
        [(0, [228, 128, 28, 178], [27, 127, 227, 77]), (2, [100, 128, 128, 178], [100, 100, 100, 77]), (2, [100, 150, 110, 178], [75, 105, 139, 77])]
    {
        let s = Stream { flat: Some(stored), ..Stream::new(16, 16, &comps, 0, 1) }.encode();
        let b = Bands::new(&s, &Limits::default(), Some(transform)).unwrap();
        let (seq, damaged) = in_order(&b).unwrap();
        assert!(!damaged);
        assert!(seq.as_chunks::<4>().0.iter().all(|px| *px == ink), "transform {transform}: {:?}", &seq[..4]);
        let whole = decode(&s, &Limits::default(), Some(transform)).unwrap();
        assert!(eight(&whole) == seq, "transform {transform}");
    }
}
