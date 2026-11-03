use super::*;

/// The test image decoded by `zune-jpeg`, as interleaved 8-bit samples, and its size.
fn zune_decode(bytes: &[u8]) -> (Vec<u8>, (u32, u32)) {
    let mut dec = zune_jpeg::JpegDecoder::new(zune_jpeg::zune_core::bytestream::ZCursor::new(bytes));
    let px = dec.decode().unwrap();
    let (w, h) = dec.dimensions().unwrap();
    (px, (w as u32, h as u32))
}

fn decode_default(bytes: &[u8]) -> Result<Decoded, Error> {
    decode(bytes, &Limits::default(), None)
}

fn u16_samples(img: &Decoded) -> &[u16] {
    match &img.samples {
        Samples::U16(v) => v,
        Samples::U8(_) => panic!("8-bit samples"),
    }
}

// --- A test-only arithmetic encoder (T.81 D.1, F.1.4 and G.1.3), mirroring the decoder ---

struct Enc {
    out: Vec<u8>,
    c: u32,
    a: u32,
    ct: u32,
    stacked: u32,
    b: Option<u8>,
}

impl Enc {
    fn new() -> Self {
        Enc { out: Vec::new(), c: 0, a: 0x10000, ct: 11, stacked: 0, b: None }
    }

    fn emit(&mut self, v: u8) {
        self.out.push(v);
        if v == 0xFF {
            self.out.push(0);
        }
    }

    fn byte_out(&mut self) {
        let t = self.c >> 19;
        if t > 0xFF {
            let b = self.b.unwrap() + 1;
            self.emit(b);
            for _ in 0..self.stacked {
                self.emit(0);
            }
            self.stacked = 0;
            self.b = Some((t & 0xFF) as u8);
        } else if t == 0xFF {
            self.stacked += 1;
        } else {
            if let Some(b) = self.b {
                self.emit(b);
            }
            for _ in 0..self.stacked {
                self.emit(0xFF);
            }
            self.stacked = 0;
            self.b = Some(t as u8);
        }
        self.c &= 0x7FFFF;
    }

    fn renorm(&mut self) {
        while self.a < 0x8000 {
            self.a <<= 1;
            self.c <<= 1;
            self.ct -= 1;
            if self.ct == 0 {
                self.byte_out();
                self.ct = 8;
            }
        }
    }

    fn code(&mut self, st: &mut u8, d: u8) {
        let mps = *st >> 7;
        let (qe, nlps, nmps, switch) = QE[usize::from(*st & 0x7F)];
        self.a -= qe;
        if d == mps {
            if self.a < 0x8000 {
                if self.a < qe {
                    self.c += self.a;
                    self.a = qe;
                }
                *st = nmps | (mps << 7);
                self.renorm();
            }
        } else {
            if self.a >= qe {
                self.c += self.a;
                self.a = qe;
            }
            *st = nlps | ((mps ^ u8::from(switch)) << 7);
            self.renorm();
        }
    }

    fn code_fixed(&mut self, d: u8) {
        let mut st = 0;
        self.code(&mut st, d);
    }

    fn finish(mut self) -> Vec<u8> {
        let mut t = (self.c + self.a - 1) & 0xFFFF_0000;
        if t < self.c {
            t += 0x8000;
        }
        self.c = t << self.ct;
        self.byte_out();
        self.c <<= 8;
        self.byte_out();
        if let Some(b) = self.b {
            self.emit(b);
        }
        for _ in 0..self.stacked {
            self.emit(0xFF);
        }
        // Trailing zero bytes are dropped, as libjpeg's encoder does; the decoder supplies
        // zeros at the marker. A zero right after `FF` is stuffing and stays.
        while self.out.last() == Some(&0) && self.out.len().checked_sub(2).map(|i| self.out[i]) != Some(0xFF) {
            self.out.pop();
        }
        self.out
    }
}

/// Arithmetic conditioning of one entropy table: DC (L, U) and AC Kx (F.1.4.4).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Cond {
    l: u8,
    u: u8,
    kx: u8,
}

/// The conditioning in force without a DAC segment (B.2.4.3).
const DEFAULT_COND: Cond = Cond { l: 0, u: 1, kx: 5 };

/// Statistics per entropy table, DC prediction and context per scan component.
struct EncScan {
    dc: [[u8; 64]; 4],
    ac: [[u8; 256]; 4],
    ctx: [usize; 4],
    pred: [i32; 4],
    cond: [Cond; 4],
}

impl EncScan {
    fn new(cond: [Cond; 4]) -> Self {
        EncScan { dc: [[0; 64]; 4], ac: [[0; 256]; 4], ctx: [0; 4], pred: [0; 4], cond }
    }

    /// Encode_DC_DIFF of scan component `sc` with DC table `t`.
    fn dc(&mut self, e: &mut Enc, sc: usize, t: usize, value: i32) {
        let diff = value - self.pred[sc];
        self.pred[sc] = value;
        let st = &mut self.dc[t];
        let s0 = self.ctx[sc];
        if diff == 0 {
            e.code(&mut st[s0], 0);
            self.ctx[sc] = 0;
            return;
        }
        e.code(&mut st[s0], 1);
        let sign = u8::from(diff < 0);
        e.code(&mut st[s0 + 1], sign);
        let sp = s0 + 2 + usize::from(sign);
        let sz = diff.unsigned_abs() - 1;
        let mut m = 0u32;
        if sz == 0 {
            e.code(&mut st[sp], 0);
        } else {
            e.code(&mut st[sp], 1);
            m = 1;
            let mut s = 20;
            while m << 1 <= sz {
                e.code(&mut st[s], 1);
                m <<= 1;
                s += 1;
            }
            e.code(&mut st[s], 0);
            let mut bit = m >> 1;
            while bit != 0 {
                e.code(&mut st[s + 14], u8::from(sz & bit != 0));
                bit >>= 1;
            }
        }
        // Conditioning category as libjpeg's jcarith.c computes it, from the magnitude
        // category `m` rather than the difference.
        let Cond { l, u, .. } = self.cond[t];
        self.ctx[sc] = if m < (1 << l) >> 1 {
            0
        } else if m > (1 << u) >> 1 {
            12 + 4 * usize::from(sign)
        } else {
            4 + 4 * usize::from(sign)
        };
    }

    /// Encode_AC_coefficients with AC table `t` over `ss..=se` of the coefficients scaled down
    /// by `al`.
    fn ac(&mut self, e: &mut Enc, t: usize, blk: &[i32; 64], ss: usize, se: usize, al: u8) {
        let kx = usize::from(self.cond[t].kx);
        let st = &mut self.ac[t];
        let v = |k: usize| if blk[k] < 0 { -((-blk[k]) >> al) } else { blk[k] >> al };
        let last = (ss..=se).rev().find(|&k| v(k) != 0);
        let mut k = ss;
        while k <= se {
            if last.is_none_or(|l| k > l) {
                e.code(&mut st[3 * (k - 1)], 1);
                return;
            }
            e.code(&mut st[3 * (k - 1)], 0);
            while v(k) == 0 {
                e.code(&mut st[3 * (k - 1) + 1], 0);
                k += 1;
            }
            e.code(&mut st[3 * (k - 1) + 1], 1);
            e.code_fixed(u8::from(v(k) < 0));
            let sz = v(k).unsigned_abs() - 1;
            let sp = 3 * (k - 1) + 2;
            if sz == 0 {
                e.code(&mut st[sp], 0);
            } else {
                e.code(&mut st[sp], 1);
                if sz == 1 {
                    e.code(&mut st[sp], 0);
                } else {
                    e.code(&mut st[sp], 1);
                    let mut m = 2u32;
                    let mut s = if k <= kx { 189 } else { 217 };
                    while m << 1 <= sz {
                        e.code(&mut st[s], 1);
                        m <<= 1;
                        s += 1;
                    }
                    e.code(&mut st[s], 0);
                    let mut bit = m >> 1;
                    while bit != 0 {
                        e.code(&mut st[s + 14], u8::from(sz & bit != 0));
                        bit >>= 1;
                    }
                }
            }
            k += 1;
        }
    }

    /// Successive approximation of AC coefficients with table `t`: bit `al` of each (G.1.3.3).
    fn ac_refine(&mut self, e: &mut Enc, t: usize, blk: &[i32; 64], ss: usize, se: usize, al: u8) {
        let st = &mut self.ac[t];
        let prior = |k: usize| blk[k].unsigned_abs() >> (al + 1) != 0;
        let now = |k: usize| blk[k].unsigned_abs() >> al != 0;
        let eob_prev = (ss..=se).rev().find(|&k| prior(k)).map_or(ss, |k| k + 1);
        let eob_now = (ss..=se).rev().find(|&k| now(k)).map_or(ss, |k| k + 1);
        let mut k = ss;
        while k <= se {
            if k >= eob_prev {
                if k >= eob_now {
                    e.code(&mut st[3 * (k - 1)], 1);
                    return;
                }
                e.code(&mut st[3 * (k - 1)], 0);
            }
            loop {
                if prior(k) {
                    e.code(&mut st[3 * (k - 1) + 2], ((blk[k].unsigned_abs() >> al) & 1) as u8);
                    break;
                }
                if now(k) {
                    e.code(&mut st[3 * (k - 1) + 1], 1);
                    e.code_fixed(u8::from(blk[k] < 0));
                    break;
                }
                e.code(&mut st[3 * (k - 1) + 1], 0);
                k += 1;
            }
            k += 1;
        }
    }
}

/// Deterministic pseudo-random coefficients: mostly small, zero runs, a few large.
fn coefficients(n: usize, seed: u32, big: i32) -> Vec<[i32; 64]> {
    let mut s = seed;
    let mut next = move || {
        s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        s >> 8
    };
    (0..n)
        .map(|_| {
            let mut b = [0i32; 64];
            b[0] = (next() % (2 * big as u32 + 1)) as i32 - big;
            for v in b.iter_mut().skip(1) {
                let mag = match next() % 100 {
                    0..=59 => 0,
                    60..=89 => 1 + (next() % 4) as i32,
                    90..=97 => 1 + (next() % 60) as i32,
                    _ => 1 + (next() % big as u32) as i32,
                };
                *v = if next() % 2 == 0 { mag } else { -mag };
            }
            if next() % 3 == 0 {
                for v in &mut b[20..] {
                    *v = 0;
                }
            }
            b
        })
        .collect()
}

fn segment(out: &mut Vec<u8>, marker: u8, body: &[u8]) {
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(body);
}

/// A component of a test frame: id, sampling factors, quantization table slot, and the
/// coefficients of its block grid padded to whole MCUs, row-major.
struct TestComp {
    id: u8,
    h: usize,
    v: usize,
    tq: u8,
    blocks: Vec<[i32; 64]>,
}

/// A scan of a test frame: frame component indexes, `(ss, se, ah, al)`, and segments
/// written just before its header.
struct TestScan {
    comps: Vec<usize>,
    band: (usize, usize, u8, u8),
    before: Vec<u8>,
}

/// An arithmetic-coded frame (SOF9, or SOF10 when `progressive`). Component `ci` uses entropy
/// tables `min(ci, 1)`, as libjpeg's defaults do; a DAC segment is written for the tables
/// whose `cond` isn't the default, and quantization slot 0 holds all ones until a scan's
/// `before` replaces it.
struct TestFrame {
    precision: u8,
    progressive: bool,
    width: usize,
    height: usize,
    comps: Vec<TestComp>,
    cond: [Cond; 4],
    ri: usize,
    scans: Vec<TestScan>,
}

impl TestFrame {
    fn encode(&self) -> Vec<u8> {
        let mut s = vec![0xFF, 0xD8];
        let mut dqt = vec![0u8];
        dqt.extend([1u8; 64]);
        segment(&mut s, 0xDB, &dqt);
        let mut sof = vec![self.precision];
        sof.extend((self.height as u16).to_be_bytes());
        sof.extend((self.width as u16).to_be_bytes());
        sof.push(self.comps.len() as u8);
        for c in &self.comps {
            sof.extend([c.id, (c.h << 4 | c.v) as u8, c.tq]);
        }
        segment(&mut s, if self.progressive { 0xCA } else { 0xC9 }, &sof);
        let mut dac = Vec::new();
        for (t, c) in self.cond.iter().enumerate().filter(|(_, c)| **c != DEFAULT_COND) {
            dac.extend([t as u8, c.l | c.u << 4, 0x10 | t as u8, c.kx]);
        }
        if !dac.is_empty() {
            segment(&mut s, 0xCC, &dac);
        }
        if self.ri > 0 {
            segment(&mut s, 0xDD, &(self.ri as u16).to_be_bytes());
        }
        let hmax = self.comps.iter().map(|c| c.h).max().unwrap();
        let vmax = self.comps.iter().map(|c| c.v).max().unwrap();
        let mcux = self.width.div_ceil(8 * hmax);
        let mcuy = self.height.div_ceil(8 * vmax);
        for scan in &self.scans {
            s.extend(&scan.before);
            let (ss, se, ah, al) = scan.band;
            let mut sos = vec![scan.comps.len() as u8];
            for &ci in &scan.comps {
                let t = ci.min(1) as u8;
                sos.extend([self.comps[ci].id, t << 4 | t]);
            }
            sos.extend([ss as u8, se as u8, ah << 4 | al]);
            segment(&mut s, 0xDA, &sos);
            // Blocks in coding order (A.2): `(scan component, block)` for each MCU.
            let mut mcus: Vec<Vec<(usize, &[i32; 64])>> = Vec::new();
            if let [ci] = scan.comps[..] {
                let c = &self.comps[ci];
                let bw = mcux * c.h;
                let (cw, ch) = ((self.width * c.h).div_ceil(hmax), (self.height * c.v).div_ceil(vmax));
                for by in 0..ch.div_ceil(8) {
                    for bx in 0..cw.div_ceil(8) {
                        mcus.push(vec![(0, &c.blocks[by * bw + bx])]);
                    }
                }
            } else {
                for my in 0..mcuy {
                    for mx in 0..mcux {
                        let mut mcu = Vec::new();
                        for (sc, &ci) in scan.comps.iter().enumerate() {
                            let c = &self.comps[ci];
                            for by in my * c.v..(my + 1) * c.v {
                                for bx in mx * c.h..(mx + 1) * c.h {
                                    mcu.push((sc, &c.blocks[by * mcux * c.h + bx]));
                                }
                            }
                        }
                        mcus.push(mcu);
                    }
                }
            }
            let mut e = Enc::new();
            let mut st = EncScan::new(self.cond);
            let mut rst = 0u8;
            for (n, mcu) in mcus.iter().enumerate() {
                if self.ri > 0 && n > 0 && n % self.ri == 0 {
                    s.extend(std::mem::replace(&mut e, Enc::new()).finish());
                    s.extend([0xFF, 0xD0 + rst]);
                    rst = (rst + 1) & 7;
                    st = EncScan::new(self.cond);
                }
                for &(sc, blk) in mcu {
                    let t = scan.comps[sc].min(1);
                    match (self.progressive, ss, ah) {
                        (false, _, _) => {
                            st.dc(&mut e, sc, t, blk[0]);
                            st.ac(&mut e, t, blk, 1, 63, 0);
                        }
                        (true, 0, 0) => st.dc(&mut e, sc, t, blk[0] >> al),
                        (true, 0, _) => e.code_fixed(((blk[0] >> al) & 1) as u8),
                        (true, _, 0) => st.ac(&mut e, t, blk, ss, se, al),
                        (true, _, _) => st.ac_refine(&mut e, t, blk, ss, se, al),
                    }
                }
            }
            s.extend(e.finish());
        }
        s.extend([0xFF, 0xD9]);
        s
    }
}

/// A gray SOF9 stream (SOF10 when there are several scans) holding `blocks` (`bw` × `bh`),
/// with the scans `(ss, se, ah, al)` and restart interval `ri` (in MCUs).
fn arith_stream(precision: u8, bw: usize, bh: usize, blocks: &[[i32; 64]], scans: &[(usize, usize, u8, u8)], ri: usize) -> Vec<u8> {
    TestFrame {
        precision,
        progressive: scans.len() > 1,
        width: bw * 8,
        height: bh * 8,
        comps: vec![TestComp { id: 1, h: 1, v: 1, tq: 0, blocks: blocks.to_vec() }],
        cond: [DEFAULT_COND; 4],
        ri,
        scans: scans.iter().map(|&band| TestScan { comps: vec![0], band, before: Vec::new() }).collect(),
    }
    .encode()
}

fn decoded_blocks(stream: &[u8]) -> Vec<[i32; 64]> {
    let (mut frame, _, damaged) = decode_coefficients(stream, &Limits::default()).unwrap();
    assert!(!damaged);
    std::mem::take(&mut frame.comps[0].coefs)
}

#[test]
fn arithmetic_sequential_round_trips_coefficients() {
    for (precision, big, ri) in [(8u8, 1000, 0usize), (12, 16000, 0), (8, 300, 1), (12, 3000, 5)] {
        let blocks = coefficients(12, u32::from(precision) * 7 + ri as u32, big);
        let stream = arith_stream(precision, 4, 3, &blocks, &[(0, 63, 0, 0)], ri);
        assert_eq!(decoded_blocks(&stream), blocks, "P={precision} ri={ri}");
    }
}

#[test]
fn arithmetic_progressive_round_trips_coefficients() {
    let blocks = coefficients(6, 99, 2000);
    let scans = [(0, 0, 0, 1), (1, 5, 0, 2), (6, 63, 0, 1), (1, 5, 2, 1), (0, 0, 1, 0), (1, 5, 1, 0), (6, 63, 1, 0)];
    for ri in [0, 2] {
        let stream = arith_stream(8, 3, 2, &blocks, &scans, ri);
        assert_eq!(decoded_blocks(&stream), blocks, "ri={ri}");
    }
}

/// Every component's coefficients, and whether the stream was damaged.
fn decoded_components(stream: &[u8]) -> (Vec<Vec<[i32; 64]>>, bool) {
    let (frame, _, damaged) = decode_coefficients(stream, &Limits::default()).unwrap();
    (frame.comps.into_iter().map(|c| c.coefs).collect(), damaged)
}

/// Non-default DC conditioning (L, U) and AC Kx from a DAC segment round-trip, and the decoder
/// applies them: the same data read with the default conditioning comes out different.
#[test]
fn arithmetic_conditioning_from_dac_round_trips() {
    let blocks = coefficients(12, 21, 1000);
    for (l, u, kx) in [(2, 4, 2), (1, 1, 20), (0, 0, 1), (3, 7, 63), (5, 15, 40)] {
        let cond = Cond { l, u, kx };
        for progressive in [false, true] {
            let bands: &[(usize, usize, u8, u8)] = if progressive { &[(0, 0, 0, 0), (1, 63, 0, 0)] } else { &[(0, 63, 0, 0)] };
            let stream = TestFrame {
                precision: 8,
                progressive,
                width: 32,
                height: 24,
                comps: vec![TestComp { id: 1, h: 1, v: 1, tq: 0, blocks: blocks.clone() }],
                cond: [cond, DEFAULT_COND, DEFAULT_COND, DEFAULT_COND],
                ri: 0,
                scans: bands.iter().map(|&band| TestScan { comps: vec![0], band, before: Vec::new() }).collect(),
            }
            .encode();
            assert_eq!(decoded_blocks(&stream), blocks, "L={l} U={u} Kx={kx} progressive={progressive}");
            let dac = stream.windows(2).position(|p| p == [0xFF, 0xCC]).unwrap();
            let len = usize::from(u16::from_be_bytes([stream[dac + 2], stream[dac + 3]]));
            let mut without = stream.clone();
            without.drain(dac..dac + 2 + len);
            let differs = decode_coefficients(&without, &Limits::default()).map_or(true, |(f, _, _)| f.comps[0].coefs != blocks);
            assert!(differs, "L={l} U={u} Kx={kx} progressive={progressive}: DAC ignored");
        }
    }
}

/// Interleaved scans of a subsampled frame (2×2 luma, two 1×1 chroma components sharing
/// entropy table 1 with non-default conditioning), sequential with and without restarts, and
/// progressive.
#[test]
fn arithmetic_interleaved_subsampled_round_trips() {
    let comps = |seed: u32| {
        vec![
            TestComp { id: 1, h: 2, v: 2, tq: 0, blocks: coefficients(8, seed, 800) },
            TestComp { id: 2, h: 1, v: 1, tq: 0, blocks: coefficients(2, seed + 1, 300) },
            TestComp { id: 3, h: 1, v: 1, tq: 0, blocks: coefficients(2, seed + 2, 300) },
        ]
    };
    let cond = [DEFAULT_COND, Cond { l: 1, u: 3, kx: 8 }, DEFAULT_COND, DEFAULT_COND];
    let scan = |comps: &[usize], band| TestScan { comps: comps.to_vec(), band, before: Vec::new() };
    let progressive = vec![
        scan(&[0, 1, 2], (0, 0, 0, 1)),
        scan(&[0], (1, 63, 0, 0)),
        scan(&[2], (1, 63, 0, 0)),
        scan(&[1], (1, 63, 0, 0)),
        scan(&[0, 1, 2], (0, 0, 1, 0)),
    ];
    for (seed, ri, scans) in [(1, 0, vec![scan(&[0, 1, 2], (0, 63, 0, 0))]), (2, 1, vec![scan(&[0, 1, 2], (0, 63, 0, 0))]), (3, 0, progressive)] {
        let frame = TestFrame { precision: 8, progressive: scans.len() > 1, width: 32, height: 16, comps: comps(seed), cond, ri, scans };
        let (got, damaged) = decoded_components(&frame.encode());
        assert!(!damaged, "seed {seed}");
        for (c, want) in got.iter().zip(&frame.comps) {
            assert_eq!(c, &want.blocks, "seed {seed} component {}", want.id);
        }
    }
}

/// Each component dequantizes with the table in its slot at its first scan, not the one the
/// slot holds when the stream ends.
#[test]
fn quantization_tables_are_latched_at_the_first_scan() {
    // DC-only blocks: sample = DC × q₀ / 8 + 128.
    let mut blk = [0i32; 64];
    blk[0] = 40;
    let dqt = |q0: u8| {
        let mut body = vec![0u8, q0];
        body.extend([1u8; 63]);
        let mut seg = Vec::new();
        segment(&mut seg, 0xDB, &body);
        seg
    };
    let stream = TestFrame {
        precision: 8,
        progressive: false,
        width: 16,
        height: 16,
        comps: b"RGB".iter().map(|&id| TestComp { id, h: 1, v: 1, tq: 0, blocks: vec![blk; 4] }).collect(),
        cond: [DEFAULT_COND; 4],
        ri: 0,
        scans: [2, 4, 8].iter().enumerate().map(|(ci, &q0)| TestScan { comps: vec![ci], band: (0, 63, 0, 0), before: dqt(q0) }).collect(),
    }
    .encode();
    let img = decode_default(&stream).unwrap();
    assert!(!img.damaged);
    let Samples::U8(px) = &img.samples else { panic!("16-bit samples") };
    assert!(px.chunks(3).all(|p| p == [138, 148, 168]), "{:?}", &px[..3]);
}

#[test]
fn arithmetic_twelve_bit_flat_block_is_sixteen_bit() {
    // DC only: sample = DC / 8 + 2048 with a quantizer of 1; 1000 → 2173.
    let mut blk = [0i32; 64];
    blk[0] = 1000;
    let stream = arith_stream(12, 1, 1, &[blk], &[(0, 63, 0, 0)], 0);
    assert_eq!(frame(&stream), Some(FrameKind { marker: 0xC9, precision: 12 }));
    let img = decode_default(&stream).unwrap();
    assert_eq!((img.width, img.height, img.components), (8, 8, 1));
    let want = (2173.0f32 * 65535.0 / 4095.0).round() as u16;
    assert!(u16_samples(&img).iter().all(|&v| v.abs_diff(want) <= 1));
}

/// A 12-bit SOF1 stream built by hand: one 8×8 gray block with only a DC coefficient. The DC
/// table has one code (`0`, category 13), the AC table one (`0`, end of block).
fn twelve_bit_huffman_block() -> Vec<u8> {
    let mut s = vec![0xFF, 0xD8];
    let mut dqt = vec![0u8];
    dqt.extend([1u8; 64]);
    segment(&mut s, 0xDB, &dqt);
    segment(&mut s, 0xC1, &[12, 0, 8, 0, 8, 1, 1, 0x11, 0]);
    let mut dht = vec![0x00, 1];
    dht.extend([0u8; 15]);
    dht.push(13);
    dht.extend([0x10, 1]);
    dht.extend([0u8; 15]);
    dht.push(0x00);
    segment(&mut s, 0xC4, &dht);
    segment(&mut s, 0xDA, &[1, 1, 0x00, 0, 63, 0]);
    // DC = 7616 → sample 7616 / 8 + 2048 = 3000. Bits: `0`, 7616 in 13 bits, `0`, one pad `1`.
    let word: u16 = (7616 << 2) | 1;
    s.extend(word.to_be_bytes());
    s.extend([0xFF, 0xD9]);
    s
}

#[test]
fn twelve_bit_huffman_block_decodes_to_sixteen_bit() {
    let stream = twelve_bit_huffman_block();
    assert_eq!(frame(&stream), Some(FrameKind { marker: 0xC1, precision: 12 }));
    let img = decode_default(&stream).unwrap();
    assert_eq!((img.width, img.height), (8, 8));
    let want = (3000.0f32 * 65535.0 / 4095.0).round() as u16;
    let px = u16_samples(&img);
    assert!(px.iter().all(|&v| v.abs_diff(want) <= 1), "{:?} vs {want}", &px[..4]);
}

/// APPn segments before the frame (here an APP13 `8BIM` block right after SOI) are skipped by
/// the probe and the decoder alike; reading them is the caller's job.
#[test]
fn twelve_bit_jpeg_with_photoshop_resources_decodes() {
    let resources = b"8BIM\x04\x04\0\0\0\0\0\0".to_vec();
    let mut app13 = b"Photoshop 3.0\0".to_vec();
    app13.extend_from_slice(&resources);
    let stream = twelve_bit_huffman_block();
    let mut with = stream[..2].to_vec();
    segment(&mut with, 0xED, &app13);
    with.extend_from_slice(&stream[2..]);
    assert_eq!(frame(&with), frame(&stream));
    assert_eq!(decode_default(&with).unwrap(), decode_default(&stream).unwrap());
}

/// Where the entropy-coded data of the last scan starts.
fn last_scan_data(s: &[u8]) -> usize {
    let sos = s.windows(2).rposition(|p| p == [0xFF, 0xDA]).unwrap();
    sos + 2 + usize::from(u16::from_be_bytes([s[sos + 2], s[sos + 3]]))
}

/// A Huffman scan whose entropy data runs out is not a clean image, even with EOI present:
/// the zero padding supplied at the marker must not be decoded as real coefficients.
#[test]
fn huffman_scan_out_of_data_is_not_clean() {
    let full = twelve_bit_huffman_block();
    let data = last_scan_data(&full);
    for keep in 0..2 {
        let mut s = full[..data + keep].to_vec();
        s.extend([0xFF, 0xD9]);
        assert!(decode_default(&s).is_err(), "keep {keep}");
    }
    // A multi-MCU scan cut in half, EOI kept, decodes as damaged.
    let (w, h) = (64usize, 64usize);
    let rgb: Vec<u8> = (0..w * h).flat_map(|i| [(i % w * 4) as u8, (i / w * 4) as u8, (i * 7) as u8]).collect();
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 90).encode(&rgb, w as u16, h as u16, jpeg_encoder::ColorType::Rgb).unwrap();
    let sof = out.windows(2).position(|p| p == [0xFF, 0xC0]).unwrap();
    out[sof + 1] = 0xC1;
    let data = last_scan_data(&out);
    let end = out.len() - 2;
    let mut cut = out[..data + (end - data) / 2].to_vec();
    if cut.last() == Some(&0xFF) {
        cut.pop();
    }
    cut.extend([0xFF, 0xD9]);
    assert!(!decode(&out, &Limits::default(), None).unwrap().damaged);
    assert!(decode(&cut, &Limits::default(), None).unwrap().damaged);
}

/// Arithmetic data that runs to the end of the file, or loses a restart marker, is reported
/// damaged, and the blocks before the cut are kept. Data cut short before EOI with no restart
/// interval reads as dropped trailing zeros and can't be detected.
#[test]
fn arithmetic_scan_out_of_data_is_damaged() {
    let blocks = coefficients(64, 11, 500);
    for ri in [0, 2] {
        let full = arith_stream(8, 8, 8, &blocks, &[(0, 63, 0, 0)], ri);
        assert_eq!(decoded_blocks(&full), blocks, "ri={ri}");
        let data = last_scan_data(&full);
        let end = full.len() - 2;
        for cut in [data + (end - data) / 2, data + 1] {
            let mut s = full[..cut].to_vec();
            if s.last() == Some(&0xFF) {
                s.pop();
            }
            for eoi in [false, true] {
                if eoi && ri == 0 {
                    continue;
                }
                let mut s = s.clone();
                if eoi {
                    s.extend([0xFF, 0xD9]);
                }
                // With one byte of data the first MCU fails, which is an error, not damage.
                match decode_coefficients(&s, &Limits::default()) {
                    Ok((frame, _, damaged)) => {
                        assert!(damaged, "ri={ri} eoi={eoi} cut at {}", cut - data);
                        if cut > data + 1 {
                            assert_eq!(frame.comps[0].coefs[0], blocks[0], "ri={ri} eoi={eoi}");
                        }
                    }
                    Err(_) => assert_eq!(cut, data + 1, "ri={ri} eoi={eoi}"),
                }
            }
        }
    }
}

/// A baseline file from `jpeg-encoder`, relabeled SOF1, decodes like zune decodes the original.
#[test]
fn extended_huffman_matches_zune_on_baseline_data() {
    let (w, h) = (37u16, 29u16);
    let rgb: Vec<u8> = (0..usize::from(w) * usize::from(h))
        .flat_map(|i| {
            let (x, y) = (i % usize::from(w), i / usize::from(w));
            [(x * 7) as u8, (y * 9) as u8, ((x + y) * 3) as u8]
        })
        .collect();
    for (sampling, restart) in
        [(jpeg_encoder::SamplingFactor::R_4_2_0, 0u16), (jpeg_encoder::SamplingFactor::R_4_4_4, 3), (jpeg_encoder::SamplingFactor::R_4_2_2, 1)]
    {
        let mut out = Vec::new();
        let mut enc = jpeg_encoder::Encoder::new(&mut out, 90);
        enc.set_sampling_factor(sampling);
        if restart > 0 {
            enc.set_restart_interval(restart);
        }
        enc.encode(&rgb, w, h, jpeg_encoder::ColorType::Rgb).unwrap();
        let (zune, _) = zune_decode(&out);
        let sof = out.windows(2).position(|p| p == [0xFF, 0xC0]).unwrap();
        out[sof + 1] = 0xC1;
        let ours = decode(&out, &Limits::default(), None).unwrap();
        assert_eq!((ours.width, ours.height, ours.components), (u32::from(w), u32::from(h), 3));
        assert!(!ours.damaged);
        let Samples::U8(data) = &ours.samples else { panic!("16-bit samples") };
        let mad = data.iter().zip(&zune).map(|(&a, &b)| f32::from(a.abs_diff(b))).sum::<f32>() / data.len() as f32;
        assert!(mad < 1.5, "{sampling:?} restart {restart}: mean abs diff {mad}");
    }
}

#[test]
fn malformed_streams_return_errors_not_panics() {
    let blocks = coefficients(6, 5, 500);
    let mut streams = vec![
        arith_stream(12, 3, 2, &blocks, &[(0, 63, 0, 0)], 2),
        arith_stream(8, 3, 2, &blocks, &[(0, 0, 0, 1), (1, 63, 0, 0), (0, 0, 1, 0)], 0),
        twelve_bit_huffman_block(),
    ];
    // Header damage: zero sizes, bad component counts, sampling factors and tables, bad DAC.
    let base = arith_stream(8, 1, 1, &blocks[..1], &[(0, 63, 0, 0)], 0);
    let sof = base.windows(2).position(|p| p == [0xFF, 0xC9]).unwrap();
    for (offset, value) in [(4, 16u8), (6, 0), (8, 0), (9, 7), (10, 9), (11, 0x50), (12, 9)] {
        let mut s = base.clone();
        s[sof + offset] = value;
        assert!(decode_default(&s).is_err(), "SOF byte {offset} = {value}");
        streams.push(s);
    }
    let mut dac = base.clone();
    dac.splice(2..2, [0xFF, 0xCC, 0, 4, 0x00, 0x13]);
    assert!(decode_default(&dac).is_err());
    streams.push(dac);
    for s in &streams {
        for cut in 0..s.len() {
            let _ = decode_default(&s[..cut]);
        }
        for i in 2..s.len() {
            for v in [0x00, 0xFF, 0x5A] {
                let mut t = s.clone();
                t[i] = v;
                let _ = decode_default(&t);
            }
        }
    }
    assert!(decode_default(&streams[0][..40]).is_err());
}

/// Scans past the block budget (`SCAN_PASSES` passes over a one-block frame) aren't decoded:
/// the image keeps what the earlier scans decoded and is reported damaged.
#[test]
fn scans_past_the_block_budget_are_not_decoded() {
    let blocks = coefficients(1, 3, 500);
    for (count, damaged) in [(SCAN_PASSES, false), (SCAN_PASSES + 1, true)] {
        let stream = arith_stream(8, 1, 1, &blocks, &vec![(0, 0, 0, 0); count], 0);
        let (frame, _, d) = decode_coefficients(&stream, &Limits::default()).unwrap();
        assert_eq!(d, damaged, "{count} scans");
        assert_eq!(frame.comps[0].coefs[0][0], blocks[0][0], "{count} scans");
    }
}

#[test]
fn limits_are_checked_before_allocating() {
    let mut s = vec![0xFF, 0xD8];
    segment(&mut s, 0xC9, &[8, 0xFF, 0xFF, 0xFF, 0xFF, 3, 1, 0x44, 0, 2, 0x11, 0, 3, 0x11, 0]);
    let small = Limits { max_alloc: 1 << 20, ..Limits::default() };
    assert!(matches!(decode(&s, &small, None), Err(Error::LimitExceeded(_))));
    // 64×64 gray: 32 KiB of coefficients and plane plus 4 KiB of output, each under the limit
    // alone but not together.
    let stream = arith_stream(8, 8, 8, &coefficients(64, 1, 100), &[(0, 63, 0, 0)], 0);
    for (max_alloc, ok) in [(36 << 10, true), ((36 << 10) - 1, false), (32 << 10, false)] {
        let limits = Limits { max_alloc, ..Limits::default() };
        assert_eq!(decode(&stream, &limits, None).is_ok(), ok, "max_alloc {max_alloc}");
    }
}

#[test]
fn frame_reports_the_marker_and_precision() {
    let kind = |marker, precision| Some(FrameKind { marker, precision });
    assert_eq!(frame(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0]), kind(0xC0, 8));
    assert_eq!(frame(&[0xFF, 0xD8, 0xFF, 0xC9, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0]), kind(0xC9, 8));
    assert_eq!(frame(&[0xFF, 0xD8, 0xFF, 0xC1, 0, 11, 12, 0, 8, 0, 8, 1, 1, 0x11, 0]), kind(0xC1, 12));
    assert_eq!(frame(&[0xFF, 0xD8]), None);
}
