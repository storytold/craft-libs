//! Hand-built bitstreams exercising features libx264 never emits: I_PCM (CAVLC and CABAC), long-term
//! references, MMCO operations, frame_num gaps, reference list modification and field pictures.
//! Most pictures consist of I_PCM and skipped macroblocks only, so the expected output is known
//! exactly (I_PCM samples are lossless, a skipped macroblock copies or averages the selected
//! references with a zero motion vector, and deblocking cannot modify samples at these QPs). The
//! scan tests at the end code residual data and compare field pictures with progressive ones.

use crate::h264::decode::cabac::{Cabac, NEXT_STATE, RANGE_TAB_LPS};
use crate::h264::decode::{Decoder, PictureStructure};
use craft_media::bitstream::{BitWriter, escape_rbsp};

const MB_W: usize = 4;
const MB_H: usize = 3;
const NMB: usize = MB_W * MB_H;

fn nal(ref_idc: u8, t: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut v = vec![0, 0, 0, 1, (ref_idc << 5) | t];
    v.extend(escape_rbsp(rbsp));
    v
}

/// POC type used by the synthetic SPS: 0 (4-bit lsb), 1 (see below) or 2.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PocType {
    T0,
    /// offset_for_non_ref_pic = -3, one reference frame per cycle with offset 4
    T1,
    T2,
}

fn sps(profile: u8, max_refs: u32, gaps: bool, poc: PocType) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(profile as u32, 8);
    w.write_bits(0, 8);
    w.write_bits(30, 8);
    w.write_ue(0); // sps id
    w.write_ue(0); // log2_max_frame_num - 4 -> 16
    match poc {
        PocType::T0 => {
            w.write_ue(0);
            w.write_ue(0); // log2_max_pic_order_cnt_lsb - 4 -> 4-bit lsb
        }
        PocType::T1 => {
            w.write_ue(1);
            w.write_bit(false); // delta_pic_order_always_zero_flag
            w.write_se(-3); // offset_for_non_ref_pic
            w.write_se(0); // offset_for_top_to_bottom_field
            w.write_ue(1); // num_ref_frames_in_pic_order_cnt_cycle
            w.write_se(4);
        }
        PocType::T2 => w.write_ue(2), // output order = decoding order
    }
    w.write_ue(max_refs);
    w.write_bit(gaps);
    w.write_ue(MB_W as u32 - 1);
    w.write_ue(MB_H as u32 - 1);
    w.write_bit(true); // frame_mbs_only
    w.write_bit(true); // direct_8x8_inference
    w.write_bit(false); // cropping
    w.write_bit(false); // vui
    w.rbsp_trailing();
    nal(3, 7, &w.finish())
}

fn pps(cabac: bool, weighted: bool) -> Vec<u8> {
    pps_with(cabac, weighted, if weighted { 1 } else { 0 })
}

/// `pps` with weighted_bipred_idc `bipred`.
fn pps_with(cabac: bool, weighted: bool, bipred: u32) -> Vec<u8> {
    pps_full(cabac, weighted, bipred, false)
}

/// [`pps_with`], with constrained_intra_pred_flag `constrained`.
fn pps_full(cabac: bool, weighted: bool, bipred: u32, constrained: bool) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0);
    w.write_ue(0);
    w.write_bit(cabac);
    w.write_bit(false);
    w.write_ue(0); // slice groups
    w.write_ue(0); // num_ref_idx_l0_default - 1
    w.write_ue(0);
    w.write_bit(weighted); // weighted_pred
    w.write_bits(bipred, 2); // weighted_bipred_idc
    w.write_se(0); // pic_init_qp - 26
    w.write_se(0);
    w.write_se(0); // chroma_qp_index_offset
    w.write_bit(true); // deblocking_filter_control_present
    w.write_bit(constrained); // constrained_intra_pred
    w.write_bit(false);
    w.rbsp_trailing();
    nal(3, 8, &w.finish())
}

#[derive(Default, Clone)]
struct Pic {
    idr: bool,
    frame_num: u32,
    ref_idc: u8,
    long_term_ref: bool,
    /// ref_pic_list_modification entries (idc, value)
    mods: Vec<(u32, u32)>,
    /// MMCOs: (op, a, b) with a = difference_of_pic_nums_minus1 / long_term_pic_num /
    /// max_long_term_frame_idx_plus1 and b = long_term_frame_idx
    mmcos: Vec<(u32, u32, u32)>,
    /// Per MB: Some(samples) for I_PCM, None for P_Skip / B_Skip.
    mbs: Vec<Option<Vec<u8>>>,
    /// B slice (all B_Skip).
    b: bool,
    poc_lsb: Option<u32>,
    /// delta_pic_order_cnt[0] (POC type 1).
    poc_delta: Option<i32>,
    /// Explicit weights (luma, cb, cr) as (weight, offset) for refIdx 0 of list 0 and list 1;
    /// log2 denominators are 5 (luma) and 3 (chroma).
    wt: Option<[[(i32, i32); 3]; 2]>,
    /// first_mb_in_slice (`mbs` then lists the slice's macroblocks from there).
    first_mb: u32,
    /// The SPS allows field coding (frame_mbs_only_flag 0): the header carries field_pic_flag.
    interlaced: bool,
    /// A field picture: Some(bottom_field_flag).
    field: Option<bool>,
    /// disable_deblocking_filter_idc 1.
    deblock_off: bool,
    /// disable_deblocking_filter_idc when not `deblock_off` (0 or 2).
    deblock_idc: u32,
    /// slice_qp_delta.
    qp_delta: i32,
    /// P slice: num_ref_idx_l0_active overridden to the number of entries, each with explicit
    /// weights (luma, cb, cr) as (weight, offset), log2 denominators 5 and 3.
    l0_weights: Vec<[(i32, i32); 3]>,
}

impl Pic {
    fn is_p(&self) -> bool {
        !self.idr && !self.b && self.mbs.iter().any(|m| m.is_none())
    }
    fn inter(&self) -> bool {
        self.b || self.is_p()
    }
}

fn slice_header(w: &mut BitWriter, p: &Pic, cabac: bool) {
    w.write_ue(p.first_mb);
    w.write_ue(if p.b {
        6
    } else if p.is_p() {
        5
    } else {
        7
    });
    w.write_ue(0); // pps id
    w.write_bits(p.frame_num, 4);
    if p.interlaced {
        w.write_bit(p.field.is_some());
        if let Some(bottom) = p.field {
            w.write_bit(bottom);
        }
    }
    if p.idr {
        w.write_ue(0);
    }
    if let Some(lsb) = p.poc_lsb {
        w.write_bits(lsb, 4);
    }
    if let Some(d) = p.poc_delta {
        w.write_se(d);
    }
    if p.b {
        w.write_bit(true); // direct_spatial_mv_pred_flag
    }
    if p.inter() && !p.l0_weights.is_empty() {
        w.write_bit(true); // num_ref_idx_override
        w.write_ue(p.l0_weights.len() as u32 - 1);
        w.write_bit(false); // no list 0 modification
        w.write_ue(5);
        w.write_ue(3);
        for e in &p.l0_weights {
            w.write_bit(true);
            w.write_se(e[0].0);
            w.write_se(e[0].1);
            w.write_bit(true);
            for c in 1..3 {
                w.write_se(e[c].0);
                w.write_se(e[c].1);
            }
        }
    } else if p.inter() {
        w.write_bit(false); // num_ref_idx_override
        w.write_bit(!p.mods.is_empty());
        if !p.mods.is_empty() {
            for &(idc, v) in &p.mods {
                w.write_ue(idc);
                w.write_ue(v);
            }
            w.write_ue(3);
        }
        if p.b {
            w.write_bit(false); // no list 1 modification
        }
    }
    if let (Some(wt), true) = (p.wt, p.inter()) {
        w.write_ue(5);
        w.write_ue(3);
        for l in 0..if p.b { 2 } else { 1 } {
            w.write_bit(true);
            w.write_se(wt[l][0].0);
            w.write_se(wt[l][0].1);
            w.write_bit(true);
            for c in 1..3 {
                w.write_se(wt[l][c].0);
                w.write_se(wt[l][c].1);
            }
        }
    }
    if p.ref_idc != 0 {
        if p.idr {
            w.write_bit(false);
            w.write_bit(p.long_term_ref);
        } else {
            w.write_bit(!p.mmcos.is_empty());
            if !p.mmcos.is_empty() {
                for &(op, a, b) in &p.mmcos {
                    w.write_ue(op);
                    match op {
                        1 => w.write_ue(a),
                        2 => w.write_ue(a),
                        3 => {
                            w.write_ue(a);
                            w.write_ue(b);
                        }
                        4 => w.write_ue(a),
                        6 => w.write_ue(b),
                        _ => {}
                    }
                }
                w.write_ue(0);
            }
        }
    }
    if cabac && p.inter() {
        w.write_ue(0); // cabac_init_idc
    }
    w.write_se(p.qp_delta);
    if p.deblock_off {
        w.write_ue(1); // disable_deblocking_filter_idc
    } else {
        w.write_ue(p.deblock_idc);
        w.write_se(0);
        w.write_se(0);
    }
}

/// Minimal CABAC encoder (9.3.4, informative) writing into a BitWriter.
struct Enc {
    low: u32,
    range: u32,
    outstanding: u32,
    first: bool,
    ctx: [u8; 1024],
}

impl Enc {
    fn new(qp: i32, init: usize) -> Self {
        let c = Cabac::new(&[0, 0, 0, 0], 0, qp, init).unwrap();
        Enc { low: 0, range: 510, outstanding: 0, first: true, ctx: c.ctx }
    }
    fn init(&mut self) {
        self.low = 0;
        self.range = 510;
        self.outstanding = 0;
        self.first = true;
    }
    fn put(&mut self, w: &mut BitWriter, b: bool) {
        if self.first {
            self.first = false;
        } else {
            w.write_bit(b);
        }
        while self.outstanding > 0 {
            w.write_bit(!b);
            self.outstanding -= 1;
        }
    }
    fn renorm(&mut self, w: &mut BitWriter) {
        while self.range < 256 {
            if self.low < 256 {
                self.put(w, false);
            } else if self.low >= 512 {
                self.low -= 512;
                self.put(w, true);
            } else {
                self.low -= 256;
                self.outstanding += 1;
            }
            self.range <<= 1;
            self.low <<= 1;
        }
    }
    fn encode(&mut self, w: &mut BitWriter, ctx_idx: usize, bin: u32) {
        let s = self.ctx[ctx_idx] as usize;
        let lps = RANGE_TAB_LPS[s >> 1][((self.range >> 6) & 3) as usize] as u32;
        self.range -= lps;
        if bin != (s & 1) as u32 {
            self.low += self.range;
            self.range = lps;
            self.ctx[ctx_idx] = NEXT_STATE[s][1];
        } else {
            self.ctx[ctx_idx] = NEXT_STATE[s][0];
        }
        self.renorm(w);
    }
    fn bypass(&mut self, w: &mut BitWriter, bin: bool) {
        self.low <<= 1;
        if bin {
            self.low += self.range;
        }
        if self.low >= 1024 {
            self.put(w, true);
            self.low -= 1024;
        } else if self.low < 512 {
            self.put(w, false);
        } else {
            self.low -= 512;
            self.outstanding += 1;
        }
    }
    fn terminate(&mut self, w: &mut BitWriter, bin: bool) {
        self.range -= 2;
        if bin {
            self.low += self.range;
            // EncodeFlush
            self.range = 2;
            self.renorm(w);
            self.put(w, (self.low >> 9) & 1 != 0);
            w.write_bits(((self.low >> 7) & 3) | 1, 2);
        } else {
            self.renorm(w);
        }
    }
}

fn write_pcm(w: &mut BitWriter, samples: &[u8]) {
    w.align_zero();
    w.write_bytes(samples);
}

fn slice_nal(p: &Pic, cabac: bool) -> Vec<u8> {
    let mut w = BitWriter::new();
    slice_header(&mut w, p, cabac);
    if !cabac && p.b {
        w.write_ue(NMB as u32);
        w.rbsp_trailing();
    } else if !cabac {
        let mut run = 0;
        for mb in &p.mbs {
            match mb {
                None => run += 1,
                Some(s) => {
                    if p.is_p() {
                        w.write_ue(run);
                        run = 0;
                        w.write_ue(30); // I_PCM in a P slice
                    } else {
                        w.write_ue(25);
                    }
                    write_pcm(&mut w, s);
                }
            }
        }
        if run > 0 {
            w.write_ue(run);
        }
        w.rbsp_trailing();
    } else {
        while !w.is_byte_aligned() {
            w.write_bit(true); // cabac_alignment_one_bit
        }
        let mut e = Enc::new(26, if p.inter() { 0 } else { 3 });
        let skip = |i: usize| p.mbs[i].is_none();
        for (i, mb) in p.mbs.iter().enumerate() {
            let (x, y) = (i % MB_W, i / MB_W);
            let a = if x > 0 { Some(i - 1) } else { None };
            let b = if y > 0 { Some(i - MB_W) } else { None };
            if p.inter() {
                let inc = a.map(|n| !skip(n) as usize).unwrap_or(0) + b.map(|n| !skip(n) as usize).unwrap_or(0);
                e.encode(&mut w, if p.b { 24 } else { 11 } + inc, skip(i) as u32);
            }
            if let Some(s) = mb {
                if p.is_p() {
                    e.encode(&mut w, 14, 1); // intra prefix
                    e.encode(&mut w, 17, 1); // not I_NxN
                } else {
                    // neighbours are I_PCM (not I_NxN) -> condTerm 1
                    let inc = a.is_some() as usize + b.is_some() as usize;
                    e.encode(&mut w, 3 + inc, 1);
                }
                e.terminate(&mut w, true); // I_PCM
                write_pcm(&mut w, s);
                e.init();
            }
            e.terminate(&mut w, i + 1 == p.mbs.len()); // end_of_slice_flag
        }
        w.align_zero();
    }
    let t = if p.idr { 5 } else { 1 };
    nal(p.ref_idc, t, &w.finish())
}

fn random_mb(seed: &mut u32) -> Vec<u8> {
    (0..384)
        .map(|_| {
            *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (*seed >> 24) as u8
        })
        .collect()
}

/// Expected cropped planes of a picture made of PCM blocks (`None` = copy from `reference`).
fn render(mbs: &[Option<Vec<u8>>], reference: Option<&(Vec<u8>, Vec<u8>, Vec<u8>)>) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (w, h) = (MB_W * 16, MB_H * 16);
    let (mut y, mut u, mut v) = match reference {
        Some(r) => r.clone(),
        None => (vec![0; w * h], vec![0; w * h / 4], vec![0; w * h / 4]),
    };
    for (i, mb) in mbs.iter().enumerate() {
        let Some(s) = mb else { continue };
        let (x0, y0) = ((i % MB_W) * 16, (i / MB_W) * 16);
        for r in 0..16 {
            y[(y0 + r) * w + x0..(y0 + r) * w + x0 + 16].copy_from_slice(&s[r * 16..r * 16 + 16]);
        }
        for r in 0..8 {
            let o = (y0 / 2 + r) * (w / 2) + x0 / 2;
            u[o..o + 8].copy_from_slice(&s[256 + r * 8..256 + r * 8 + 8]);
            v[o..o + 8].copy_from_slice(&s[320 + r * 8..320 + r * 8 + 8]);
        }
    }
    (y, u, v)
}

fn build_and_check(cabac: bool, threads: usize) {
    let mut seed = 99u32;
    let pcm_pic = |seed: &mut u32| (0..NMB).map(|_| Some(random_mb(seed))).collect::<Vec<_>>();
    let a = pcm_pic(&mut seed);
    let b = pcm_pic(&mut seed);
    let mixed: Vec<Option<Vec<u8>>> = (0..NMB).map(|i| if i % 3 == 1 { Some(random_mb(&mut seed)) } else { None }).collect();
    let skip = vec![None; NMB];
    let pics = vec![
        // 0: IDR, all PCM, marked long-term (LongTermFrameIdx 0)
        Pic { idr: true, frame_num: 0, ref_idc: 3, long_term_ref: true, mbs: a.clone(), ..Default::default() },
        // 1: all PCM, short-term
        Pic { frame_num: 1, ref_idc: 2, mbs: b.clone(), ..Default::default() },
        // 2: P_Skip from the long-term picture via modification idc 2 (default list would use pic 1)
        Pic { frame_num: 2, ref_idc: 2, mods: vec![(2, 0)], mbs: skip.clone(), ..Default::default() },
        // 3: frame_num gap (3, 4 missing); P_Skip from pic 2 (PicNum 2) via idc 0: 5 - (2 + 1) = 2
        Pic { frame_num: 5, ref_idc: 2, mods: vec![(0, 2)], mbs: skip.clone(), ..Default::default() },
        // 4: mixed PCM / P_Skip from pic 1 (PicNum 1: 6 - 5); MMCO 4 raises MaxLongTermFrameIdx to 2,
        //    MMCO 2 drops long-term 0, MMCO 3 makes pic 1 (difference 6 - 1 - 1 = 4) long-term index 1
        Pic { frame_num: 6, ref_idc: 2, mods: vec![(0, 4)], mmcos: vec![(4, 3, 0), (2, 0, 0), (3, 4, 1)], mbs: mixed.clone(), ..Default::default() },
        // 5: non-reference P_Skip from long-term index 1 (= pic 1)
        Pic { frame_num: 7, ref_idc: 0, mods: vec![(2, 1)], mbs: skip.clone(), ..Default::default() },
        // 6: P_Skip with the default list: short-term by descending PicNum -> pic 4 first
        Pic { frame_num: 7, ref_idc: 2, mmcos: vec![(1, 0, 0), (6, 0, 2)], mbs: skip.clone(), ..Default::default() },
        // 7: MMCO 5 picture made of PCM, then 8: P_Skip referencing it (only reference left)
        Pic { frame_num: 8, ref_idc: 2, mmcos: vec![(5, 0, 0)], mbs: pcm_pic(&mut seed), ..Default::default() },
    ];
    let mut stream = sps(if cabac { 77 } else { 66 }, 8, true, PocType::T2);
    stream.extend(pps(cabac, false));
    for p in &pics {
        stream.extend(slice_nal(p, cabac));
    }
    let after5 = Pic { frame_num: 1, ref_idc: 2, mbs: skip.clone(), ..Default::default() };
    stream.extend(slice_nal(&after5, cabac));

    // expected output
    let pa = render(&a, None);
    let pb = render(&b, None);
    let p2 = pa.clone();
    let p3 = p2.clone();
    let p4 = render(&mixed, Some(&pb));
    let p5 = pb.clone();
    let p6 = p4.clone();
    let p7 = render(&pics[7].mbs, None);
    let p8 = p7.clone();
    let expected = [pa, pb, p2, p3, p4, p5, p6, p7, p8];

    if threads == 1 {
        cross_check_ffmpeg(&stream, &expected, if cabac { "synth_cabac" } else { "synth_cavlc" });
    }
    let mut dec = Decoder::with_threads(threads);
    let mut out = dec.decode(&stream, 0).unwrap();
    out.extend(dec.flush());
    assert!(dec.take_error().is_none());
    assert_eq!(out.len(), expected.len(), "cabac={cabac}");
    for (i, (p, e)) in out.iter().zip(expected.iter()).enumerate() {
        assert_eq!((p.width as usize, p.height as usize), (MB_W * 16, MB_H * 16));
        assert!(p.y == e.0 && p.u == e.1 && p.v == e.2, "picture {i} differs (cabac={cabac}, threads={threads})");
    }
    let s = dec.stats();
    assert!(s.mb_pcm > 0 && s.frame_num_gaps == 1 && s.long_term_marks >= 3, "{s:?}");
}

type Planes3 = (Vec<u8>, Vec<u8>, Vec<u8>);

/// Validate the hand-built stream and its expected output with ffmpeg when available.
fn cross_check_ffmpeg(stream: &[u8], expected: &[Planes3], name: &str) {
    let Some(ff) = craft_media_testkit::ffmpeg_or_skip(name) else { return };
    let dir = craft_media_testkit::fixtures_dir("h264");
    let src = craft_media_testkit::temp_path(&dir.join(format!("{name}.h264")));
    let dst = craft_media_testkit::temp_path(&dir.join(format!("{name}.yuv")));
    std::fs::write(&src, stream).unwrap();
    let ok = std::process::Command::new(&ff)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&src)
        .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "yuv420p"])
        .arg(&dst)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "ffmpeg failed to decode {name}");
    let got = std::fs::read(&dst).unwrap();
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&dst);
    let want: Vec<u8> = expected.iter().flat_map(|(y, u, v)| y.iter().chain(u).chain(v).copied()).collect();
    assert!(got == want, "ffmpeg output differs from the expected output of {name} ({} vs {} bytes)", got.len(), want.len());
}

#[test]
fn pcm_long_term_mmco_gaps_cavlc() {
    build_and_check(false, 1);
    build_and_check(false, 4);
}

#[test]
fn pcm_long_term_mmco_gaps_cabac() {
    build_and_check(true, 1);
    build_and_check(true, 4);
}

fn weighted_check(cabac: bool, threads: usize) {
    let mut seed = 7u32;
    let a: Vec<Option<Vec<u8>>> = (0..NMB).map(|_| Some(random_mb(&mut seed))).collect();
    let b: Vec<Option<Vec<u8>>> = (0..NMB).map(|_| Some(random_mb(&mut seed))).collect();
    let skip = vec![None; NMB];
    let wp = [[(20, 3), (5, -1), (12, 2)], [(0, 0); 3]];
    let wb = [[(40, -2), (4, 1), (9, -3)], [(30, 7), (6, 0), (2, 4)]];
    // decoding order: I (POC 0), P (POC 8), B (POC 4, both refs), non-reference weighted P_Skip (POC 12)
    let pics = [
        Pic { idr: true, frame_num: 0, ref_idc: 3, poc_lsb: Some(0), mbs: a.clone(), ..Default::default() },
        Pic { frame_num: 1, ref_idc: 2, poc_lsb: Some(8), mbs: b.clone(), wt: Some(wp), ..Default::default() },
        Pic { frame_num: 2, ref_idc: 0, poc_lsb: Some(4), b: true, mbs: skip.clone(), wt: Some(wb), ..Default::default() },
        Pic { frame_num: 2, ref_idc: 0, poc_lsb: Some(12), mbs: skip.clone(), wt: Some(wp), ..Default::default() },
    ];
    let mut stream = sps(77, 2, false, PocType::T0);
    stream.extend(pps(cabac, true));
    for p in &pics {
        stream.extend(slice_nal(p, cabac));
    }
    let pa = render(&a, None);
    let pb = render(&b, None);
    let clip = |v: i32| v.clamp(0, 255) as u8;
    let mix = |x: &[u8], y: &[u8], c: usize| -> Vec<u8> {
        let (w0, o0) = wb[0][c];
        let (w1, o1) = wb[1][c];
        let d = if c == 0 { 5 } else { 3 };
        x.iter().zip(y).map(|(&p, &q)| clip(((p as i32 * w0 + q as i32 * w1 + (1 << d)) >> (d + 1)) + ((o0 + o1 + 1) >> 1))).collect()
    };
    let single = |x: &[u8], c: usize| -> Vec<u8> {
        let (w, o) = wp[0][c];
        let d = if c == 0 { 5 } else { 3 };
        x.iter().map(|&p| clip(((p as i32 * w + (1 << (d - 1))) >> d) + o)).collect()
    };
    let pbi = (mix(&pa.0, &pb.0, 0), mix(&pa.1, &pb.1, 1), mix(&pa.2, &pb.2, 2));
    let pp = (single(&pb.0, 0), single(&pb.1, 1), single(&pb.2, 2));
    let expected = [pa, pbi, pb, pp];
    if threads == 1 {
        cross_check_ffmpeg(&stream, &expected, if cabac { "synth_wp_cabac" } else { "synth_wp_cavlc" });
    }
    let mut dec = Decoder::with_threads(threads);
    let mut out = dec.decode(&stream, 0).unwrap();
    out.extend(dec.flush());
    assert!(dec.take_error().is_none());
    assert_eq!(out.len(), 4);
    for (i, (p, e)) in out.iter().zip(expected.iter()).enumerate() {
        assert!(p.y == e.0 && p.u == e.1 && p.v == e.2, "picture {i} differs (cabac={cabac}, threads={threads})");
    }
}

#[test]
fn explicit_weighted_prediction_p_and_b_skip() {
    for cabac in [false, true] {
        weighted_check(cabac, 1);
        weighted_check(cabac, 3);
    }
}

/// A picture with MMCO 5 decodes with its own order count; it becomes 0 only when the picture is
/// stored (8.2.1, tempPicOrderCnt). A reference B picture with MMCO 5 at POC 12 (above every
/// earlier picture, as 8.2.1 requires) predicts with implicit weights from POC 8 (list 0, weight 96)
/// and POC 0 (list 1, weight -32); with POC 0 for itself it would copy the POC 0 picture.
#[test]
fn mmco5_picture_decodes_with_its_own_order_count() {
    for cabac in [false, true] {
        let mut seed = 11u32;
        let a: Vec<Option<Vec<u8>>> = (0..NMB).map(|_| Some(random_mb(&mut seed))).collect();
        let b: Vec<Option<Vec<u8>>> = (0..NMB).map(|_| Some(random_mb(&mut seed))).collect();
        let pics = [
            Pic { idr: true, frame_num: 0, ref_idc: 3, poc_lsb: Some(0), mbs: a.clone(), ..Default::default() },
            Pic { frame_num: 1, ref_idc: 2, poc_lsb: Some(8), mbs: b.clone(), ..Default::default() },
            Pic { frame_num: 2, ref_idc: 1, poc_lsb: Some(12), b: true, mmcos: vec![(5, 0, 0)], mbs: vec![None; NMB], ..Default::default() },
        ];
        let mut stream = sps(77, 2, false, PocType::T0);
        stream.extend(pps_with(cabac, false, 2));
        for p in &pics {
            stream.extend(slice_nal(p, cabac));
        }
        let (pa, pb) = (render(&a, None), render(&b, None));
        let mix = |x: &[u8], y: &[u8]| -> Vec<u8> {
            x.iter().zip(y).map(|(&p, &q)| ((q as i32 * 96 - p as i32 * 32 + 32) >> 6).clamp(0, 255) as u8).collect()
        };
        let bi = (mix(&pa.0, &pb.0), mix(&pa.1, &pb.1), mix(&pa.2, &pb.2));
        let expected = [pa, pb, bi];
        cross_check_ffmpeg(&stream, &expected, if cabac { "synth_mmco5_poc_cabac" } else { "synth_mmco5_poc_cavlc" });
        for threads in [1, 3] {
            let mut dec = Decoder::with_threads(threads);
            let mut out = dec.decode(&stream, 0).unwrap();
            out.extend(dec.flush());
            assert!(dec.take_error().is_none());
            assert_eq!(out.len(), 3);
            for (i, (p, e)) in out.iter().zip(expected.iter()).enumerate() {
                assert!(p.y == e.0 && p.u == e.1 && p.v == e.2, "picture {i} differs (cabac={cabac}, threads={threads})");
            }
        }
    }
}

/// POC type 1: non-reference pictures are displayed before the preceding reference picture.
#[test]
fn poc_type1_reordering() {
    for cabac in [false, true] {
        let mut seed = 5u32;
        let contents: Vec<Vec<Option<Vec<u8>>>> = (0..5).map(|_| (0..NMB).map(|_| Some(random_mb(&mut seed))).collect()).collect();
        // (frame_num, ref_idc): POCs 0, 4, 1, 8, 5
        let spec = [(0, 3), (1, 2), (2, 0), (2, 2), (3, 0)];
        let mut stream = sps(77, 2, false, PocType::T1);
        stream.extend(pps(cabac, false));
        for (i, &(frame_num, ref_idc)) in spec.iter().enumerate() {
            let p = Pic { idr: i == 0, frame_num, ref_idc, poc_delta: Some(0), mbs: contents[i].clone(), ..Default::default() };
            stream.extend(slice_nal(&p, cabac));
        }
        let expected_order = [0usize, 2, 1, 4, 3];
        let expected_poc = [0, 1, 4, 5, 8];
        for threads in [1, 2] {
            let mut dec = Decoder::with_threads(threads);
            let mut out = dec.decode(&stream, 0).unwrap();
            out.extend(dec.flush());
            assert_eq!(out.len(), 5);
            for (k, p) in out.iter().enumerate() {
                let e = render(&contents[expected_order[k]], None);
                assert_eq!(p.poc, expected_poc[k]);
                assert!(p.y == e.0 && p.u == e.1 && p.v == e.2, "output {k} (cabac={cabac})");
            }
        }
    }
}

/// A stream this decoder does not implement fails as unsupported whatever its slice headers hold
/// (#626). A 10-bit slice may code a QP below 0 (down to -12); checked against the 8-bit range
/// first, it failed as invalid, so the media pool took it for a damaged frame and rendered black
/// instead of the unreadable slate. 4:4:4 here, which stays unsupported after High 10 and 4:2:2.
#[test]
fn unsupported_stream_fails_as_unsupported_at_any_slice_qp() {
    let mut w = BitWriter::new();
    w.write_bits(244, 8); // High 4:4:4 Predictive
    w.write_bits(0, 8);
    w.write_bits(30, 8);
    w.write_ue(0); // sps id
    w.write_ue(3); // chroma_format_idc: 4:4:4
    w.write_bit(false); // separate_colour_plane
    w.write_ue(2); // bit_depth_luma - 8
    w.write_ue(2); // bit_depth_chroma - 8
    w.write_bit(false); // qpprime_y_zero_transform_bypass
    w.write_bit(false); // seq_scaling_matrix_present
    w.write_ue(0); // log2_max_frame_num - 4
    w.write_ue(2); // pic_order_cnt_type
    w.write_ue(0); // max_num_ref_frames
    w.write_bit(false); // gaps
    w.write_ue(MB_W as u32 - 1);
    w.write_ue(MB_H as u32 - 1);
    w.write_bit(true); // frame_mbs_only
    w.write_bit(true); // direct_8x8_inference
    w.write_bit(false); // cropping
    w.write_bit(false); // vui
    w.rbsp_trailing();
    let sps = nal(3, 7, &w.finish());
    // QP 26, and QP -4 (pic_init_qp 26 + slice_qp_delta -30)
    for qp_delta in [0, -30] {
        let mut w = BitWriter::new();
        w.write_ue(0); // first_mb
        w.write_ue(7); // I slice
        w.write_ue(0); // pps id
        w.write_bits(0, 4); // frame_num
        w.write_ue(0); // idr_pic_id
        w.write_bit(false); // no_output_of_prior_pics
        w.write_bit(false); // long_term_reference
        w.write_se(qp_delta);
        w.write_ue(1); // disable_deblocking_filter_idc
        w.rbsp_trailing();
        let mut stream = sps.clone();
        stream.extend(pps(false, false));
        stream.extend(nal(3, 5, &w.finish()));
        for threads in [1, 2] {
            let r = Decoder::with_threads(threads).decode(&stream, 0).map(|pics| pics.len());
            assert!(
                matches!(&r, Err(crate::h264::decode::Error::Unsupported(m)) if m.contains("chroma_format_idc 3")),
                "slice_qp_delta {qp_delta}: {r:?}"
            );
        }
    }
}

#[test]
fn avcc_accepts_generated_sps_and_pps() {
    let sps = sps(66, 4, false, PocType::T2);
    let pps = pps(false, false);
    let (sps, pps) = (&sps[4..], &pps[4..]);
    let mut avcc = vec![1, 66, 0, 30, 0xff, 0xe1];
    avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    avcc.extend_from_slice(sps);
    avcc.push(1);
    avcc.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    avcc.extend_from_slice(pps);
    assert!(Decoder::from_avcc(&avcc).is_ok());
}

#[test]
fn avcc_rejects_a_valid_picture_slice_without_starting_a_picture() {
    let sequence = sps(66, 4, false, PocType::T0);
    let picture = pps(false, false);
    let slice = slice_nal(
        &Pic {
            idr: true,
            frame_num: 0,
            ref_idc: 3,
            long_term_ref: false,
            mods: vec![],
            mmcos: vec![],
            mbs: vec![Some(vec![128; 384]); NMB],
            b: false,
            poc_lsb: Some(0),
            poc_delta: None,
            wt: None,
            first_mb: 0,
            interlaced: false,
            field: None,
            deblock_off: false,
            deblock_idc: 0,
            qp_delta: 0,
            l0_weights: vec![],
        },
        false,
    );
    let mut avcc = vec![1, 66, 0, 30, 0xff, 0xe1];
    let nal = &sequence[4..];
    avcc.extend_from_slice(&(nal.len() as u16).to_be_bytes());
    avcc.extend_from_slice(nal);
    avcc.push(2);
    avcc.extend_from_slice(&((picture.len() - 4) as u16).to_be_bytes());
    avcc.extend_from_slice(&picture[4..]);
    avcc.extend_from_slice(&((slice.len() - 4) as u16).to_be_bytes());
    avcc.extend_from_slice(&slice[4..]);

    let mut decoder = Decoder::with_threads(1);
    assert!(decoder.configure_avcc(&avcc).is_err());
    assert!(decoder.flush().is_empty());
    assert_eq!(decoder.stats().pictures, 0);
}

/// One IDR picture of PCM macroblocks.
fn idr_stream(max_refs: u32) -> Vec<u8> {
    let mut seed = 5u32;
    let mbs: Vec<Option<Vec<u8>>> = (0..NMB).map(|_| Some(random_mb(&mut seed))).collect();
    let mut stream = sps(66, max_refs, false, PocType::T2);
    stream.extend(pps(false, false));
    stream.extend(slice_nal(&Pic { idr: true, frame_num: 0, ref_idc: 3, mbs, ..Default::default() }, false));
    stream
}

fn decodes_within(stream: &[u8], budget: u64) -> bool {
    let mut dec = Decoder::with_threads(1);
    dec.set_memory_budget(budget);
    let Ok(mut out) = dec.decode(stream, 0) else { return false };
    out.extend(dec.flush());
    dec.take_error().is_none() && out.len() == 1
}

#[test]
fn picture_store_is_checked_against_the_memory_budget() {
    let stream = idr_stream(4);
    let sps_nal = sps(66, 4, false, PocType::T2);
    let parsed = crate::h264::decode::params::Sps::parse(&craft_media::bitstream::unescape_rbsp(&sps_nal[5..])).unwrap();
    // 4:2:0, 8-bit: luma plus two quarter-size chroma planes per picture; the DPB, the current
    // picture and threads + 2 in flight.
    let one = (NMB * 256 + 2 * NMB * 64) as u64;
    let pictures = (parsed.max_dpb_frames() + 1 + 3) as u64;
    assert!(decodes_within(&stream, pictures * one));
    assert!(!decodes_within(&stream, pictures * one - 1));
    assert!(decodes_within(&stream, crate::budget::DECODE_MEMORY_BUDGET));
}

#[test]
fn largest_accepted_h264_size_is_over_the_default_budget() {
    // 1024 x 1024 macroblocks (16384 x 16384) with 16 references and 3 in flight.
    let shape = crate::budget::PictureShape { width: 16384, height: 16384, shift_x: 1, shift_y: 1, chroma_planes: 2, bytes_per_sample: 1 };
    assert!(!shape.fits(16 + 1 + 3, crate::budget::DECODE_MEMORY_BUDGET));
}

fn gap_stream(gaps: bool) -> Vec<u8> {
    let mut seed = 7u32;
    let a: Vec<Option<Vec<u8>>> = (0..NMB).map(|_| Some(random_mb(&mut seed))).collect();
    let mut stream = sps(66, 2, gaps, PocType::T2);
    stream.extend(pps(false, false));
    stream.extend(slice_nal(&Pic { idr: true, frame_num: 0, ref_idc: 3, mbs: a, ..Default::default() }, false));
    // frame_num 1..=9 are missing.
    stream.extend(slice_nal(&Pic { frame_num: 10, ref_idc: 2, mbs: vec![None; NMB], ..Default::default() }, false));
    stream
}

#[test]
fn frame_num_gap_is_concealed_even_when_the_sps_forbids_gaps() {
    // A cut or lossy stream: the gap is filled (bounded) instead of dropping the picture.
    let mut dec = Decoder::with_threads(1);
    let mut out = dec.decode(&gap_stream(false), 0).unwrap();
    out.extend(dec.flush());
    assert_eq!(out.len(), 2);
    assert_eq!(dec.stats().frame_num_gaps, 1);
}

#[test]
fn frame_num_gap_is_filled_when_the_sps_allows_gaps() {
    let mut dec = Decoder::with_threads(1);
    let mut out = dec.decode(&gap_stream(true), 0).unwrap();
    out.extend(dec.flush());
    assert!(dec.take_error().is_none());
    assert_eq!(out.len(), 2);
    assert_eq!(dec.stats().frame_num_gaps, 1);
}

/// An IDR picture followed by `pictures` reference P pictures of P_Skip macroblocks.
fn skip_stream(pictures: u32) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut seed = 11u32;
    let a: Vec<Option<Vec<u8>>> = (0..NMB).map(|_| Some(random_mb(&mut seed))).collect();
    let mut head = sps(66, 2, false, PocType::T2);
    head.extend(pps(false, false));
    head.extend(slice_nal(&Pic { idr: true, frame_num: 0, ref_idc: 3, mbs: a, ..Default::default() }, false));
    let ps = (1..=pictures).map(|k| slice_nal(&Pic { frame_num: k % 16, ref_idc: 2, mbs: vec![None; NMB], ..Default::default() }, false)).collect();
    (head, ps)
}

#[test]
fn overlapping_slices_are_not_reconstructed_twice() {
    let (head, ps) = skip_stream(1);
    let mut once = Decoder::with_threads(1);
    let mut out = once.decode(&[head.clone(), ps[0].clone()].concat(), 0).unwrap();
    out.extend(once.flush());
    assert!(once.take_error().is_none());
    assert_eq!(out.len(), 2);
    // A second slice of the same picture starting at macroblock 1 covers those macroblocks again.
    let again = slice_nal(&Pic { frame_num: 1, ref_idc: 2, mbs: vec![None; NMB - 1], first_mb: 1, ..Default::default() }, false);
    let mut twice = Decoder::with_threads(1);
    let result = twice.decode(&[head, ps[0].clone(), again].concat(), 0);
    twice.flush();
    let err = result.err().or_else(|| twice.take_error()).expect("overlapping slice refused");
    assert!(err.to_string().contains("overlaps"), "{err}");
}

#[test]
fn output_queue_counts_against_the_memory_budget() {
    let (head, ps) = skip_stream(40);
    let sps_nal = sps(66, 2, false, PocType::T2);
    let parsed = crate::h264::decode::params::Sps::parse(&craft_media::bitstream::unescape_rbsp(&sps_nal[5..])).unwrap();
    let one = (NMB * 256 + 2 * NMB * 64) as u64;
    let budget = (parsed.max_dpb_frames() + 1 + 3) as u64 * one;
    // One access unit per call: output drains as it goes, and the stream fits.
    let mut dec = Decoder::with_threads(1);
    dec.set_memory_budget(budget);
    let mut out = dec.decode(&head, 0).unwrap();
    for (i, p) in ps.iter().enumerate() {
        out.extend(dec.decode(p, i as i64 + 1).unwrap());
    }
    out.extend(dec.flush());
    assert!(dec.take_error().is_none());
    assert_eq!(out.len(), 41);
    // Every access unit in one call: the pictures held for output stop fitting the budget.
    let mut dec = Decoder::with_threads(1);
    dec.set_memory_budget(budget);
    let err = dec.decode(&[head, ps.concat()].concat(), 0).expect_err("output queue is over budget");
    assert!(err.to_string().contains("memory budget"), "{err}");
}

/// A held picture is charged at its own size (luma and both 4:2:0 chroma planes), the same
/// count the budget uses for the incoming SPS's pictures.
#[test]
fn a_held_frame_is_charged_at_its_own_size() {
    let f = crate::h264::decode::picture::Frame::new(0, 0, 2, 3);
    assert_eq!(f.bytes(), (2 * 3 * 256 + 2 * 2 * 3 * 64) as u64);
}

/// A slice_qp_delta at the end of the Exp-Golomb range is an error, not an overflow of
/// `pic_init_qp + slice_qp_delta`.
#[test]
fn slice_qp_addition_does_not_overflow() {
    for qp_delta in [26, i32::MAX, -i32::MAX] {
        let mut w = BitWriter::new();
        w.write_ue(0); // first_mb
        w.write_ue(7); // I slice
        w.write_ue(0); // pps id
        w.write_bits(0, 4); // frame_num
        w.write_ue(0); // idr_pic_id
        w.write_bit(false); // no_output_of_prior_pics
        w.write_bit(false); // long_term_reference
        w.write_se(qp_delta);
        w.write_ue(1); // disable_deblocking_filter_idc
        w.rbsp_trailing();
        let mut stream = sps(66, 1, false, PocType::T2);
        stream.extend(pps(false, false));
        stream.extend(nal(3, 5, &w.finish()));
        let r = Decoder::with_threads(1).decode(&stream, 0).map(|pics| pics.len());
        assert!(matches!(&r, Err(crate::h264::decode::Error::Invalid(m)) if m.contains("slice QP")), "slice_qp_delta {qp_delta}: {r:?}");
    }
}

// ------------------------------------------------------------------------------------------------
// Field pictures (PAFF)
// ------------------------------------------------------------------------------------------------

/// Frame height in macroblocks of the interlaced streams; each field is half of it.
const FRAME_MB_H: usize = 4;
const FIELD_NMB: usize = MB_W * FRAME_MB_H / 2;

/// An SPS with frame_mbs_only_flag 0 (field pictures allowed, no MBAFF), POC type 0 with a 4-bit
/// lsb, 4 reference frames.
fn interlaced_sps(profile: u8) -> Vec<u8> {
    interlaced_sps_with(profile, false)
}

/// [`interlaced_sps`], with mb_adaptive_frame_field_flag set when `mbaff`.
fn interlaced_sps_with(profile: u8, mbaff: bool) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(profile as u32, 8);
    w.write_bits(0, 8);
    w.write_bits(30, 8);
    w.write_ue(0); // sps id
    w.write_ue(0); // log2_max_frame_num - 4
    w.write_ue(0); // pic_order_cnt_type
    w.write_ue(0); // log2_max_pic_order_cnt_lsb - 4
    w.write_ue(4); // max_num_ref_frames
    w.write_bit(false); // gaps
    w.write_ue(MB_W as u32 - 1);
    w.write_ue(FRAME_MB_H as u32 / 2 - 1); // pic_height_in_map_units_minus1 (field rows)
    w.write_bit(false); // frame_mbs_only_flag
    w.write_bit(mbaff); // mb_adaptive_frame_field_flag
    w.write_bit(true); // direct_8x8_inference
    w.write_bit(false); // cropping
    w.write_bit(false); // vui
    w.rbsp_trailing();
    nal(3, 7, &w.finish())
}

/// Planes of a picture `mb_w` x `mb_h` macroblocks made of PCM blocks.
fn render_pcm(mbs: &[Option<Vec<u8>>], mb_w: usize, mb_h: usize) -> Planes3 {
    let (w, h) = (mb_w * 16, mb_h * 16);
    let (mut y, mut u, mut v) = (vec![0; w * h], vec![0; w * h / 4], vec![0; w * h / 4]);
    for (i, mb) in mbs.iter().enumerate() {
        let Some(s) = mb else { continue };
        let (x0, y0) = ((i % mb_w) * 16, (i / mb_w) * 16);
        for r in 0..16 {
            y[(y0 + r) * w + x0..(y0 + r) * w + x0 + 16].copy_from_slice(&s[r * 16..r * 16 + 16]);
        }
        for r in 0..8 {
            let o = (y0 / 2 + r) * (w / 2) + x0 / 2;
            u[o..o + 8].copy_from_slice(&s[256 + r * 8..256 + r * 8 + 8]);
            v[o..o + 8].copy_from_slice(&s[320 + r * 8..320 + r * 8 + 8]);
        }
    }
    (y, u, v)
}

/// A field predicted with a zero motion vector from a field of the other parity: luma is copied,
/// chroma moves by `dy` eighth chroma samples (Table 8-10) and is interpolated (8.4.2.2.2).
fn chroma_moved(field: &Planes3, dy: i32) -> Planes3 {
    let (cw, ch) = (MB_W * 8, FRAME_MB_H * 4);
    let (step, frac) = (dy.div_euclid(8), dy.rem_euclid(8));
    let shift = |c: &[u8]| -> Vec<u8> {
        let at = |x: usize, r: i32| i32::from(c[r.clamp(0, ch as i32 - 1) as usize * cw + x]);
        (0..ch as i32)
            .flat_map(|r| (0..cw).map(move |x| ((8 - frac) * at(x, r + step) + frac * at(x, r + step + 1) + 4) >> 3))
            .map(|v| v as u8)
            .collect()
    };
    (field.0.clone(), shift(&field.1), shift(&field.2))
}

/// The frame whose even lines are `top` and odd lines `bottom`.
fn weave(top: &Planes3, bottom: &Planes3) -> Planes3 {
    let lines = |t: &[u8], b: &[u8], w: usize| -> Vec<u8> { t.chunks(w).zip(b.chunks(w)).flat_map(|(a, b)| a.iter().chain(b).copied()).collect() };
    (lines(&top.0, &bottom.0, MB_W * 16), lines(&top.1, &bottom.1, MB_W * 8), lines(&top.2, &bottom.2, MB_W * 8))
}

/// Decode `pics` as one interlaced stream at 1 and 3 threads and compare with `expected`; with a
/// name, also check that ffmpeg agrees.
/// `structures` is the expected [`PictureStructure`] of each output picture.
fn check_interlaced(cabac: bool, pics: &[Pic], expected: &[Planes3], structures: &[PictureStructure], ffmpeg_name: Option<&str>) {
    let mut stream = interlaced_sps(77);
    stream.extend(pps(cabac, false));
    for p in pics {
        stream.extend(slice_nal(&Pic { interlaced: true, ..p.clone() }, cabac));
    }
    if let Some(name) = ffmpeg_name {
        cross_check_ffmpeg(&stream, expected, &format!("{name}_{}", if cabac { "cabac" } else { "cavlc" }));
    }
    for threads in [1, 3] {
        let mut dec = Decoder::with_threads(threads);
        let mut out = dec.decode(&stream, 0).unwrap();
        out.extend(dec.flush());
        assert!(dec.take_error().is_none(), "cabac={cabac} threads={threads}");
        assert_eq!(out.len(), expected.len(), "cabac={cabac} threads={threads}");
        let got: Vec<PictureStructure> = out.iter().map(|p| p.structure).collect();
        assert_eq!(got, structures, "cabac={cabac} threads={threads}");
        for (i, (p, e)) in out.iter().zip(expected).enumerate() {
            assert_eq!((p.width as usize, p.height as usize), (MB_W * 16, FRAME_MB_H * 16));
            assert!(p.y == e.0 && p.u == e.1 && p.v == e.2, "picture {i} differs (cabac={cabac}, threads={threads})");
        }
    }
}

fn pcm_field(seed: &mut u32) -> Vec<Option<Vec<u8>>> {
    (0..FIELD_NMB).map(|_| Some(random_mb(seed))).collect()
}

/// A field pair whose second field is predicted from the first (chroma moved by a quarter chroma
/// sample, Table 8-10), a frame predicted from the pair (the pair woven) and a field pair
/// predicted from that frame (every other line of it, 8.2.4.2.5).
fn field_pair_then_frame_then_fields(cabac: bool, bottom_first: bool) {
    let mut seed = if bottom_first { 7 } else { 5 };
    let first = pcm_field(&mut seed);
    let skip_field = vec![None; FIELD_NMB];
    let field = |frame_num, bottom, poc_lsb, mbs: &Vec<Option<Vec<u8>>>| Pic {
        idr: frame_num == 0 && bottom == bottom_first,
        frame_num,
        ref_idc: 2,
        field: Some(bottom),
        poc_lsb: Some(poc_lsb),
        mbs: mbs.clone(),
        ..Default::default()
    };
    let (b, t) = (bottom_first, !bottom_first);
    let pics = [
        field(0, b, 0, &first),
        field(0, t, 1, &skip_field),
        Pic { frame_num: 1, ref_idc: 2, poc_lsb: Some(4), mbs: vec![None; MB_W * FRAME_MB_H], ..Default::default() },
        field(2, b, 8, &skip_field),
        field(2, t, 9, &skip_field),
    ];
    let p0 = render_pcm(&first, MB_W, FRAME_MB_H / 2);
    // a bottom field referencing a top field moves chroma down (+2), a top field up (-2)
    let p1 = chroma_moved(&p0, if bottom_first { -2 } else { 2 });
    let frame = if bottom_first { weave(&p1, &p0) } else { weave(&p0, &p1) };
    let name = if bottom_first { "synth_fields_bottom_first" } else { "synth_fields_top_first" };
    use PictureStructure::{FieldPair, Frame};
    check_interlaced(cabac, &pics, &[frame.clone(), frame.clone(), frame], &[FieldPair, Frame, FieldPair], Some(name));
}

#[test]
fn field_pictures_reference_fields_frames_and_pairs() {
    for cabac in [false, true] {
        for bottom_first in [false, true] {
            field_pair_then_frame_then_fields(cabac, bottom_first);
        }
    }
}

/// A reference field followed by a non-reference field of the same frame_num does not form a
/// complementary field pair (3.30, 3.37): each is a non-paired field in a frame buffer of its own
/// and is output on its own, its lines repeated.
#[test]
fn reference_and_non_reference_fields_are_output_separately() {
    for cabac in [false, true] {
        let mut seed = 3;
        let (top, bottom) = (pcm_field(&mut seed), pcm_field(&mut seed));
        let pics = [
            Pic { idr: true, ref_idc: 2, field: Some(false), poc_lsb: Some(0), mbs: top.clone(), ..Default::default() },
            Pic { ref_idc: 0, field: Some(true), poc_lsb: Some(1), mbs: bottom.clone(), ..Default::default() },
        ];
        let (t, b) = (render_pcm(&top, MB_W, FRAME_MB_H / 2), render_pcm(&bottom, MB_W, FRAME_MB_H / 2));
        check_interlaced(cabac, &pics, &[weave(&t, &t), weave(&b, &b)], &[PictureStructure::TopField, PictureStructure::BottomField], None);
    }
}

/// A field without a second field is output once the next picture shows none follows.
#[test]
fn a_lone_field_is_output_with_its_lines_repeated() {
    for cabac in [false, true] {
        let mut seed = 11;
        let top = pcm_field(&mut seed);
        let frame: Vec<Option<Vec<u8>>> = (0..MB_W * FRAME_MB_H).map(|_| Some(random_mb(&mut seed))).collect();
        let pics = [
            Pic { idr: true, ref_idc: 2, field: Some(false), poc_lsb: Some(0), mbs: top.clone(), ..Default::default() },
            Pic { frame_num: 1, ref_idc: 2, poc_lsb: Some(2), mbs: frame.clone(), ..Default::default() },
        ];
        let t = render_pcm(&top, MB_W, FRAME_MB_H / 2);
        check_interlaced(
            cabac,
            &pics,
            &[weave(&t, &t), render_pcm(&frame, MB_W, FRAME_MB_H)],
            &[PictureStructure::TopField, PictureStructure::Frame],
            None,
        );
    }
}

// ------------------------------------------------------------------------------------------------
// Coefficient scans and significance contexts of field pictures
// ------------------------------------------------------------------------------------------------
//
// A field picture is coded with field scans (8.5.6, 8.5.7) and, in CABAC, with the contexts of
// field coded blocks (Tables 9-34, 9-43). Each test codes a field pair whose transform blocks carry
// one coefficient each, at every scan position across the pair, and compares it with two
// progressive pictures of the same size carrying the same coefficient at the zig-zag position that
// lands on the same matrix entry. The tables below are transcribed from the specification, not taken
// from the decoder, so a wrong decoder table makes the two differ.

/// Table 8-13: matrix entry c_ij ("ij") of 4x4 scan position idx.
const ZIGZAG4: [&str; 16] = ["00", "01", "10", "20", "11", "02", "03", "12", "21", "30", "31", "22", "13", "23", "32", "33"];
const FIELD4: [&str; 16] = ["00", "10", "01", "20", "30", "11", "21", "31", "02", "12", "22", "32", "03", "13", "23", "33"];
/// Table 8-14: the same for 8x8 blocks.
const ZIGZAG8: [&str; 64] = [
    "00", "01", "10", "20", "11", "02", "03", "12", "21", "30", "40", "31", "22", "13", "04", "05", "14", "23", "32", "41", "50", "60", "51", "42",
    "33", "24", "15", "06", "07", "16", "25", "34", "43", "52", "61", "70", "71", "62", "53", "44", "35", "26", "17", "27", "36", "45", "54", "63",
    "72", "73", "64", "55", "46", "37", "47", "56", "65", "74", "75", "66", "57", "67", "76", "77",
];
const FIELD8: [&str; 64] = [
    "00", "10", "20", "01", "11", "30", "40", "21", "02", "31", "50", "60", "70", "41", "12", "03", "22", "51", "61", "71", "32", "13", "04", "23",
    "42", "52", "62", "72", "33", "14", "05", "24", "43", "53", "63", "73", "34", "15", "06", "25", "44", "54", "64", "74", "35", "16", "26", "45",
    "55", "65", "75", "36", "07", "17", "46", "56", "66", "76", "27", "37", "47", "57", "67", "77",
];
/// Table 9-43: ctxIdxInc of significant_coeff_flag (frame and field coded) and
/// last_significant_coeff_flag of 8x8 blocks by scan position.
const SIG8_FRAME_INC: [usize; 63] = [
    0, 1, 2, 3, 4, 5, 5, 4, 4, 3, 3, 4, 4, 4, 5, 5, 4, 4, 4, 4, 3, 3, 6, 7, 7, 7, 8, 9, 10, 9, 8, 7, 7, 6, 11, 12, 13, 11, 6, 7, 8, 9, 14, 10, 9, 8,
    6, 11, 12, 13, 11, 6, 9, 14, 10, 9, 11, 12, 13, 11, 14, 10, 12,
];
const SIG8_FIELD_INC: [usize; 63] = [
    0, 1, 1, 2, 2, 3, 3, 4, 5, 6, 7, 7, 7, 8, 4, 5, 6, 9, 10, 10, 8, 11, 12, 11, 9, 9, 10, 10, 8, 11, 12, 11, 9, 9, 10, 10, 8, 11, 12, 11, 9, 9, 10,
    10, 8, 13, 13, 9, 9, 10, 10, 8, 13, 13, 9, 9, 10, 10, 14, 14, 14, 14, 14,
];
const LAST8_INC: [usize; 63] = [
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4,
    5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8,
];
/// Table 9-7, tzVlcIndex 1: total_zeros codes (length, value) for one coefficient in 16.
const TOTAL_ZEROS1: [(u32, u32); 16] =
    [(1, 1), (3, 3), (3, 2), (4, 3), (4, 2), (5, 3), (5, 2), (6, 3), (6, 2), (7, 3), (7, 2), (8, 3), (8, 2), (9, 3), (9, 2), (9, 1)];
/// The level each test coefficient carries.
const SCAN_LEVEL: u32 = 8;
/// Macroblocks per picture of the scan tests (MB_W x 2, a field of the interlaced SPS).
const SCAN_NMB: usize = FIELD_NMB;

/// SPS of the scan tests: MB_W x 2 macroblocks per picture (a field of a 4-row frame when
/// `interlaced`), High profile when `high`.
fn scan_sps(high: bool, interlaced: bool) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(if high { 100 } else { 77 }, 8);
    w.write_bits(0, 8);
    w.write_bits(30, 8);
    w.write_ue(0); // sps id
    if high {
        w.write_ue(1); // chroma_format_idc 4:2:0
        w.write_ue(0); // bit_depth_luma_minus8
        w.write_ue(0); // bit_depth_chroma_minus8
        w.write_bit(false); // qpprime_y_zero_transform_bypass_flag
        w.write_bit(false); // seq_scaling_matrix_present_flag
    }
    w.write_ue(0); // log2_max_frame_num - 4
    w.write_ue(0); // pic_order_cnt_type
    w.write_ue(0); // log2_max_pic_order_cnt_lsb - 4
    w.write_ue(2); // max_num_ref_frames
    w.write_bit(false); // gaps
    w.write_ue(MB_W as u32 - 1);
    w.write_ue(1); // pic_height_in_map_units_minus1: two MB rows per field or frame
    w.write_bit(!interlaced); // frame_mbs_only_flag
    if interlaced {
        w.write_bit(false); // mb_adaptive_frame_field_flag
    }
    w.write_bit(true); // direct_8x8_inference
    w.write_bit(false); // cropping
    w.write_bit(false); // vui
    w.rbsp_trailing();
    nal(3, 7, &w.finish())
}

/// PPS of the scan tests, with transform_8x8_mode_flag when `t8`.
fn scan_pps(cabac: bool, t8: bool) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0);
    w.write_ue(0);
    w.write_bit(cabac);
    w.write_bit(false);
    w.write_ue(0); // slice groups
    w.write_ue(0);
    w.write_ue(0);
    w.write_bit(false);
    w.write_bits(0, 2);
    w.write_se(0); // pic_init_qp - 26
    w.write_se(0);
    w.write_se(0);
    w.write_bit(true); // deblocking_filter_control_present
    w.write_bit(false);
    w.write_bit(false);
    if t8 {
        w.write_bit(true); // transform_8x8_mode_flag
        w.write_bit(false); // pic_scaling_matrix_present_flag
        w.write_se(0); // second_chroma_qp_index_offset
    }
    w.rbsp_trailing();
    nal(3, 8, &w.finish())
}

/// A residual block coded with CABAC holding SCAN_LEVEL at scan position `pos` of `n`
/// (significance map and level, 9.3.2.3; the coded_block_flag is the caller's).
fn cabac_one_coeff(
    e: &mut Enc,
    w: &mut BitWriter,
    pos: usize,
    n: usize,
    sig: impl Fn(usize) -> usize,
    last: impl Fn(usize) -> usize,
    level_ctx: usize,
) {
    for i in 0..n - 1 {
        e.encode(w, sig(i), u32::from(i == pos));
        if i == pos {
            e.encode(w, last(i), 1);
            break;
        }
    }
    // coeff_abs_level_minus1 = SCAN_LEVEL - 1, unary below cMax 14 (ctxIdxInc 1, then 5)
    e.encode(w, level_ctx + 1, 1);
    for _ in 1..SCAN_LEVEL - 1 {
        e.encode(w, level_ctx + 5, 1);
    }
    e.encode(w, level_ctx + 5, 0);
    e.bypass(w, false); // coeff_sign_flag: positive
}

/// One I picture of the scan tests (deblocking off). Without `t8` every macroblock is
/// I_16x16_2_0_0 (DC prediction, no AC, no chroma residual) whose Intra16x16DCLevel holds one
/// coefficient at scan position `pos[mb]`; with `t8` (CABAC only) every macroblock is I_NxN with
/// the 8x8 transform, all four 8x8 luma blocks coded, block `b` holding one coefficient at
/// `pos[4 * mb + b]`. `field` is the parity of a field picture, None for a progressive frame.
fn scan_picture(cabac: bool, t8: bool, field: Option<bool>, idr: bool, pos: &[usize]) -> Vec<u8> {
    let second = field == Some(true);
    let header = Pic {
        idr,
        frame_num: u32::from(!idr && field.is_none()),
        ref_idc: 2,
        poc_lsb: Some(if field.is_some() {
            u32::from(second)
        } else if idr {
            0
        } else {
            2
        }),
        mbs: vec![Some(Vec::new()); SCAN_NMB],
        interlaced: field.is_some(),
        field,
        deblock_off: true,
        ..Default::default()
    };
    let mut w = BitWriter::new();
    slice_header(&mut w, &header, cabac);
    let fld = field.is_some();
    if !cabac {
        for &p in pos {
            w.write_ue(3); // I_16x16_2_0_0
            w.write_ue(0); // intra_chroma_pred_mode DC
            w.write_se(0); // mb_qp_delta
            // Intra16x16DCLevel: nC 0 (the neighbours' AC blocks are empty), TotalCoeff 1 without
            // trailing ones, level_prefix 12 (levelCode 12 + 2 = 14, level 8), total_zeros = pos
            w.write_bits(0b000101, 6);
            w.write_bits(1, 13);
            let (len, code) = TOTAL_ZEROS1[p];
            w.write_bits(code, len);
        }
        w.rbsp_trailing();
    } else {
        while !w.is_byte_aligned() {
            w.write_bit(true);
        }
        let mut e = Enc::new(26, 3);
        for mb in 0..SCAN_NMB {
            let avail = usize::from(mb % MB_W > 0) + usize::from(mb >= MB_W);
            if t8 {
                e.encode(&mut w, 3, 0); // I_NxN (neighbours are I_NxN: condTerm 0)
                e.encode(&mut w, 399 + avail, 1); // transform_size_8x8_flag
                for _ in 0..4 {
                    e.encode(&mut w, 68, 1); // prev_intra8x8_pred_mode_flag
                }
                e.encode(&mut w, 64, 0); // intra_chroma_pred_mode DC
                for _ in 0..4 {
                    e.encode(&mut w, 73, 1); // coded_block_pattern luma: every neighbour coded
                }
                e.encode(&mut w, 77, 0); // no chroma
                e.encode(&mut w, 60, 0); // mb_qp_delta
                for b in 0..4 {
                    // 4:2:0: no coded_block_flag for 8x8 blocks
                    let (sig, last) = if fld { (436, 451) } else { (402, 417) };
                    let inc = if fld { &SIG8_FIELD_INC } else { &SIG8_FRAME_INC };
                    cabac_one_coeff(&mut e, &mut w, pos[4 * mb + b], 64, |i| sig + inc[i], |i| last + LAST8_INC[i], 426);
                }
            } else {
                e.encode(&mut w, 3 + avail, 1); // not I_NxN (neighbours are I_16x16)
                e.terminate(&mut w, false); // not I_PCM
                e.encode(&mut w, 6, 0); // no AC
                e.encode(&mut w, 7, 0); // no chroma
                e.encode(&mut w, 9, 1); // prediction mode 2
                e.encode(&mut w, 10, 0);
                e.encode(&mut w, 64, 0); // intra_chroma_pred_mode DC
                e.encode(&mut w, 60, 0); // mb_qp_delta
                // coded_block_flag of the DC block: neighbours coded or unavailable (intra) -> 3
                e.encode(&mut w, 85 + 3, 1);
                let (sig, last) = if fld { (277, 338) } else { (105, 166) };
                cabac_one_coeff(&mut e, &mut w, pos[mb], 16, |i| sig + i, |i| last + i, 227);
            }
            e.terminate(&mut w, mb + 1 == SCAN_NMB);
        }
        w.align_zero();
    }
    nal(2, if idr { 5 } else { 1 }, &w.finish())
}

fn decode_all(stream: &[u8], threads: usize) -> Vec<Planes3> {
    let mut dec = Decoder::with_threads(threads);
    let mut out = dec.decode(stream, 0).unwrap();
    out.extend(dec.flush());
    assert!(dec.take_error().is_none());
    out.into_iter().map(|p| (p.y, p.u, p.v)).collect()
}

/// A field pair coding every scan position once against two progressive pictures coding the
/// matching zig-zag positions.
fn check_field_scan(cabac: bool, t8: bool, name: &str) {
    let (field_scan, zigzag): (&[&str], &[&str]) = if t8 { (&FIELD8, &ZIGZAG8) } else { (&FIELD4, &ZIGZAG4) };
    let n = field_scan.len();
    let per_pic = if t8 { 4 * SCAN_NMB } else { SCAN_NMB };
    assert_eq!(2 * per_pic, n);
    // field positions 0..n over the pair; the progressive pictures code the same entries
    let field_pos: Vec<usize> = (0..n).collect();
    let frame_pos: Vec<usize> = field_pos.iter().map(|&p| zigzag.iter().position(|z| *z == field_scan[p]).unwrap()).collect();
    let mut fields = scan_sps(t8, true);
    fields.extend(scan_pps(cabac, t8));
    fields.extend(scan_picture(cabac, t8, Some(false), true, &field_pos[..per_pic]));
    fields.extend(scan_picture(cabac, t8, Some(true), false, &field_pos[per_pic..]));
    let mut frames = scan_sps(t8, false);
    frames.extend(scan_pps(cabac, t8));
    frames.extend(scan_picture(cabac, t8, None, true, &frame_pos[..per_pic]));
    frames.extend(scan_picture(cabac, t8, None, false, &frame_pos[per_pic..]));
    let progressive = decode_all(&frames, 1);
    assert_eq!(progressive.len(), 2);
    // the coefficients do reach the samples
    assert!(progressive[0].0.iter().any(|&v| v != progressive[0].0[0]));
    let expected = [weave(&progressive[0], &progressive[1])];
    cross_check_ffmpeg(&fields, &expected, name);
    for threads in [1, 3] {
        assert!(decode_all(&fields, threads) == expected, "{name}: field pair differs (threads={threads})");
    }
}

/// Intra16x16DCLevel of field pictures uses the field scan (CAVLC).
#[test]
fn field_pictures_use_the_field_scan_cavlc() {
    check_field_scan(false, false, "synth_field_scan4_cavlc");
}

/// Intra16x16DCLevel of field pictures uses the field scan and the significance contexts of field
/// coded blocks (ctxIdx 277 and 338 instead of 105 and 166).
#[test]
fn field_pictures_use_the_field_scan_and_contexts_cabac() {
    check_field_scan(true, false, "synth_field_scan4_cabac");
}

/// 8x8 blocks of field pictures use the 8x8 field scan and the field contexts of Table 9-43.
#[test]
fn field_pictures_use_the_8x8_field_scan_and_contexts() {
    check_field_scan(true, true, "synth_field_scan8_cabac");
}

/// Draft mode switched on between the two fields of a non-reference pair: the output pair says
/// draft, since its second field skipped deblocking.
#[test]
fn draft_mode_switched_between_two_fields_marks_the_pair_draft() {
    for (d0, d1) in [(false, true), (true, false)] {
        let mut seed = 3u32;
        let first = pcm_field(&mut seed);
        let skip = vec![None; FIELD_NMB];
        let field = |frame_num: u32, ref_idc: u8, bottom: bool, poc_lsb: u32, mbs: &Vec<Option<Vec<u8>>>| Pic {
            idr: frame_num == 0 && !bottom,
            frame_num,
            ref_idc,
            field: Some(bottom),
            poc_lsb: Some(poc_lsb),
            mbs: mbs.clone(),
            interlaced: true,
            ..Default::default()
        };
        let mut head = interlaced_sps(77);
        head.extend(pps(false, false));
        head.extend(slice_nal(&field(0, 2, false, 0, &first), false));
        head.extend(slice_nal(&field(0, 2, true, 1, &skip), false));
        let top = slice_nal(&field(1, 0, false, 4, &skip), false);
        let bottom = slice_nal(&field(1, 0, true, 5, &skip), false);
        let mut dec = Decoder::with_threads(1);
        let mut out = dec.decode(&head, 0).unwrap();
        dec.set_draft(d0);
        out.extend(dec.decode(&top, 1).unwrap());
        dec.set_draft(d1);
        out.extend(dec.decode(&bottom, 2).unwrap());
        out.extend(dec.flush());
        assert!(dec.take_error().is_none());
        let drafts: Vec<bool> = out.iter().map(|p| p.draft).collect();
        assert_eq!(drafts, [false, true], "draft {d0} then {d1}");
    }
}

/// A macroblock pair of an MBAFF frame: a field or frame macroblock pair of two I_PCM macroblocks
/// (top, bottom), or two skipped macroblocks (`mbs` None) whose field flag is inferred (7.4.4).
#[derive(Clone)]
struct Pair {
    field: bool,
    mbs: Option<[Vec<u8>; 2]>,
    /// The top macroblock is skipped, the bottom one coded (`mbs` Some; its first entry unused):
    /// mb_field_decoding_flag then comes with the bottom macroblock (7.3.4). Only as the last pair
    /// of a slice (the writer takes the neighbours of later pairs as whole pairs).
    top_skip: bool,
}

const PAIRS: usize = MB_W * FRAME_MB_H / 2;

/// mb_field_decoding_flag of each pair: as coded, or inferred for a skipped pair from the left
/// pair, else the pair above, else frame (7.4.4; one slice from pair 0).
fn pair_fields(pairs: &[Pair]) -> Vec<bool> {
    pair_fields_from(0, pairs)
}

/// [`pair_fields`] for a slice whose first pair is pair `first` of the picture (neighbours in
/// earlier slices are not available).
fn pair_fields_from(first: usize, pairs: &[Pair]) -> Vec<bool> {
    let mut f = vec![false; pairs.len()];
    for (i, p) in pairs.iter().enumerate() {
        let a = first + i;
        f[i] = if p.mbs.is_some() {
            p.field
        } else if !a.is_multiple_of(MB_W) && i >= 1 {
            f[i - 1]
        } else if i >= MB_W {
            f[i - MB_W]
        } else {
            false
        };
    }
    f
}

/// One slice of an MBAFF frame (I if every pair is I_PCM, else P), macroblocks in pair order,
/// from pair `p.first_mb`.
fn mbaff_slice_nal(p: &Pic, pairs: &[Pair], cabac: bool) -> Vec<u8> {
    let first = p.first_mb as usize;
    let intra = pairs.iter().all(|q| q.mbs.is_some());
    // a dummy macroblock list that makes the header writer pick the slice type
    let head = Pic { interlaced: true, field: None, mbs: if intra { vec![] } else { vec![None] }, ..p.clone() };
    let mut w = BitWriter::new();
    slice_header(&mut w, &head, cabac);
    let fields = pair_fields_from(first, pairs);
    let coded = |i: usize| pairs[i].mbs.is_some();
    if !cabac {
        let mut run = 0;
        for (i, q) in pairs.iter().enumerate() {
            let Some(mbs) = &q.mbs else {
                run += 2;
                continue;
            };
            assert!(!q.top_skip || (!intra && i + 1 == pairs.len()));
            for (k, s) in mbs.iter().enumerate() {
                if k == 0 && q.top_skip {
                    run += 1;
                    continue;
                }
                if !intra {
                    w.write_ue(run);
                    run = 0;
                }
                // with the top macroblock, or with the bottom one after a skipped top one
                if k == 0 || q.top_skip {
                    w.write_bit(q.field);
                }
                w.write_ue(if intra { 25 } else { 30 }); // I_PCM
                write_pcm(&mut w, s);
            }
        }
        if run > 0 {
            w.write_ue(run);
        }
        w.rbsp_trailing();
    } else {
        while !w.is_byte_aligned() {
            w.write_bit(true); // cabac_alignment_one_bit
        }
        let mut e = Enc::new(26, if intra { 3 } else { 0 });
        // a macroblock (pair i, bottom k) is coded
        let coded_mb = |i: usize, k: usize| pairs[i].mbs.is_some() && !(k == 0 && pairs[i].top_skip);
        for (i, q) in pairs.iter().enumerate() {
            assert!(!q.top_skip || (!intra && i + 1 == pairs.len()));
            // (indices into `pairs`; pairs of earlier slices are not available)
            let (left, above) = ((!(first + i).is_multiple_of(MB_W) && i >= 1).then(|| i - 1), (i >= MB_W).then(|| i - MB_W));
            // the field flag the skip flags' neighbours follow: for a pair whose flag comes with
            // the bottom macroblock, the one inferred from the neighbours, as the flag is not
            // decoded yet (7.4.4)
            let ctx_field = if q.top_skip { left.map(|l| fields[l]).or(above.map(|a| fields[a])).unwrap_or(false) } else { fields[i] };
            for k in 0..2 {
                let bottom = k == 1;
                // neighbours A and B (6.4.12.2): A is in the left pair; B is the top macroblock of
                // this pair for a bottom frame macroblock, else in the pair above
                let b_own = bottom && !ctx_field;
                if !intra {
                    let term = |n: Option<usize>| n.is_some_and(coded) as usize;
                    let inc = term(left) + if b_own { coded_mb(i, 0) as usize } else { term(above) };
                    e.encode(&mut w, 11 + inc, !coded_mb(i, k) as u32);
                }
                if let (Some(mbs), true) = (&q.mbs, coded_mb(i, k)) {
                    if k == 0 || q.top_skip {
                        let term = |n: Option<usize>| n.is_some_and(|n| fields[n]) as usize;
                        e.encode(&mut w, 70 + term(left) + term(above), q.field as u32);
                    }
                    if intra {
                        // the neighbours are I_PCM (not I_NxN): condTerm 1 when available
                        let inc = left.is_some() as usize + (b_own || above.is_some()) as usize;
                        e.encode(&mut w, 3 + inc, 1);
                    } else {
                        e.encode(&mut w, 14, 1); // intra prefix
                        e.encode(&mut w, 17, 1); // not I_NxN
                    }
                    e.terminate(&mut w, true); // I_PCM
                    write_pcm(&mut w, &mbs[k]);
                    e.init();
                }
                if bottom {
                    // end_of_slice_flag follows each pair, not its top macroblock (7.3.4)
                    e.terminate(&mut w, i + 1 == pairs.len());
                }
            }
        }
        w.align_zero();
    }
    nal(p.ref_idc, if p.idr { 5 } else { 1 }, &w.finish())
}

/// The frame an MBAFF picture of `pairs` decodes to: a frame macroblock covers 16 lines of its
/// pair, a field macroblock every other line (the top macroblock the even ones); skipped pairs
/// copy `reference`.
fn render_mbaff(pairs: &[Pair], reference: Option<&Planes3>) -> Planes3 {
    let (w, h) = (MB_W * 16, FRAME_MB_H * 16);
    let (mut y, mut u, mut v) = reference.cloned().unwrap_or_else(|| (vec![0; w * h], vec![0; w * h / 4], vec![0; w * h / 4]));
    for (i, q) in pairs.iter().enumerate() {
        let Some(mbs) = &q.mbs else { continue };
        let (x0, y0) = ((i % MB_W) * 16, (i / MB_W) * 32);
        for (k, s) in mbs.iter().enumerate().skip(q.top_skip as usize) {
            let line = |r: usize, n: usize| if q.field { 2 * r + k } else { n * k + r };
            for r in 0..16 {
                let o = (y0 + line(r, 16)) * w + x0;
                y[o..o + 16].copy_from_slice(&s[r * 16..r * 16 + 16]);
            }
            for r in 0..8 {
                let o = (y0 / 2 + line(r, 8)) * (w / 2) + x0 / 2;
                u[o..o + 8].copy_from_slice(&s[256 + r * 8..256 + r * 8 + 8]);
                v[o..o + 8].copy_from_slice(&s[320 + r * 8..320 + r * 8 + 8]);
            }
        }
    }
    (y, u, v)
}

/// MBAFF frames of I_PCM field and frame macroblock pairs, then P frames whose skipped pairs
/// (field or frame, inferred from the left pair or the pair above) copy the reference: field
/// macroblocks with refIdx 0 predict from the field of their own parity. 1 and 3 threads, CAVLC
/// and CABAC, checked against ffmpeg.
#[test]
fn mbaff_frames_of_field_and_frame_pairs() {
    for cabac in [false, true] {
        let mut seed = 21u32;
        let mut pcm = |field: bool| Pair { field, mbs: Some([random_mb(&mut seed), random_mb(&mut seed)]), top_skip: false };
        let skip = Pair { field: false, mbs: None, top_skip: false };
        // row 0: field, frame, field, field; row 1: frame, field, frame, frame
        let i_pairs: Vec<Pair> = [true, false, true, true, false, true, false, false].map(&mut pcm).to_vec();
        // skipped pairs infer: (1, 0) and (3, 0) field from the left, (0, 1) field from above,
        // (2, 1) frame from the left although the pair above is field; in the last frame the
        // skipped pairs are frame (the first has no neighbour, the others follow it). The
        // inferred flags select CABAC contexts of later pairs.
        // The last pair of each P frame has a skipped top and an I_PCM bottom macroblock, which
        // carries the field flag (7.3.4): a field pair although the neighbours infer frame for
        // the skip flags' contexts, then a frame pair (a bottom frame macroblock whose neighbour B
        // is the skipped top one).
        let top_skip = |mut q: Pair| {
            q.top_skip = true;
            q
        };
        let p_pairs = vec![pcm(true), skip.clone(), pcm(true), skip.clone(), skip.clone(), pcm(false), skip.clone(), top_skip(pcm(true))];
        assert_eq!(pair_fields(&p_pairs), [true, true, true, true, true, false, false, true]);
        let mut p2_pairs = vec![skip.clone(); PAIRS];
        p2_pairs[1] = pcm(false);
        p2_pairs[PAIRS - 1] = top_skip(pcm(false));
        assert_eq!(pair_fields(&p2_pairs), [false; PAIRS]);
        let pics = [
            (Pic { idr: true, frame_num: 0, ref_idc: 3, poc_lsb: Some(0), ..Default::default() }, &i_pairs),
            (Pic { frame_num: 1, ref_idc: 2, poc_lsb: Some(4), ..Default::default() }, &p_pairs),
            (Pic { frame_num: 2, ref_idc: 2, poc_lsb: Some(8), ..Default::default() }, &p2_pairs),
        ];
        let mut stream = interlaced_sps_with(77, true);
        stream.extend(pps(cabac, false));
        for (p, pairs) in &pics {
            stream.extend(mbaff_slice_nal(p, pairs, cabac));
        }
        let f0 = render_mbaff(&i_pairs, None);
        let f1 = render_mbaff(&p_pairs, Some(&f0));
        let f2 = render_mbaff(&p2_pairs, Some(&f1));
        let expected = [f0, f1, f2];
        cross_check_ffmpeg(&stream, &expected, if cabac { "synth_mbaff_cabac" } else { "synth_mbaff_cavlc" });
        for threads in [1, 3] {
            let mut dec = Decoder::with_threads(threads);
            let mut out = dec.decode(&stream, 0).unwrap();
            out.extend(dec.flush());
            assert!(dec.take_error().is_none(), "cabac={cabac} threads={threads}");
            assert_eq!(out.len(), expected.len());
            for (i, (p, e)) in out.iter().zip(&expected).enumerate() {
                assert_eq!(p.structure, PictureStructure::Frame);
                assert!(p.y == e.0 && p.u == e.1 && p.v == e.2, "picture {i} differs (cabac={cabac}, threads={threads})");
            }
        }
    }
}

/// A slice of an MBAFF P frame cut off at every length, the next slice decoding the rest of the
/// frame with deblocking across the slice boundary. A cut between the two macroblocks of a pair
/// leaves the bottom one undecoded; the frame macroblocks of the next slice beside and under that
/// pair must neither panic nor filter against it (that indexed the slice table with the
/// undecoded marker, caught as an internal error). With one thread the P frame reuses the I
/// frame's picture state, so this also needs `PicState::reset` to clear the kind of a
/// macroblock that stays undecoded.
#[test]
fn mbaff_slice_cut_between_the_macroblocks_of_a_pair() {
    for cabac in [false, true] {
        let mut seed = 5u32;
        let mut pcm = |field: bool| Pair { field, mbs: Some([random_mb(&mut seed), random_mb(&mut seed)]), top_skip: false };
        let i_pairs: Vec<Pair> = [false; PAIRS].map(&mut pcm).to_vec();
        let skip = Pair { field: false, mbs: None, top_skip: false };
        // slice A: pairs 0 and 1 (a frame I_PCM pair, the one to be cut); slice B: pairs 2.. all
        // skipped frame pairs (pair 2 beside pair 1, pair 5 under it)
        let a_pairs = vec![skip.clone(), pcm(false)];
        let b_pairs = vec![skip.clone(); PAIRS - 2];
        let mut head = interlaced_sps_with(77, true);
        head.extend(pps(cabac, false));
        head.extend(mbaff_slice_nal(&Pic { idr: true, frame_num: 0, ref_idc: 3, poc_lsb: Some(0), ..Default::default() }, &i_pairs, cabac));
        let p = Pic { frame_num: 1, ref_idc: 2, poc_lsb: Some(4), ..Default::default() };
        let slice_a = mbaff_slice_nal(&p, &a_pairs, cabac);
        let slice_b = mbaff_slice_nal(&Pic { first_mb: 2, ..p.clone() }, &b_pairs, cabac);
        // the uncut frame decodes cleanly (the stream itself is valid)
        let mut whole = head.clone();
        whole.extend(&slice_a);
        whole.extend(&slice_b);
        assert_eq!(decode_all(&whole, 1).len(), 2);
        // a cut slice: start code, NAL header and some of the slice header kept at least
        for len in 8..slice_a.len() {
            let mut stream = head.clone();
            stream.extend(&slice_a[..len]);
            stream.extend(&slice_b);
            for threads in [1, 3] {
                // (a panic inside a picture job is caught and reported as an internal error, by
                // `decode` itself or later by `take_error`)
                let no_panic = |e: &dyn std::fmt::Display| {
                    assert!(!e.to_string().contains("internal error"), "cabac={cabac} len={len} threads={threads}: {e}");
                };
                let mut dec = Decoder::with_threads(threads);
                let mut out = match dec.decode(&stream, 0) {
                    Ok(out) => out,
                    Err(e) => {
                        no_panic(&e);
                        Vec::new()
                    }
                };
                out.extend(dec.flush());
                if let Some(e) = dec.take_error() {
                    no_panic(&e);
                }
            }
        }
    }
}

/// A macroblock of [`mbaff_inter_slice_nal`].
#[derive(Clone, Copy)]
enum Mb {
    /// P_Skip.
    Skip,
    /// P_L0_16x16 with refIdx 0 and this mvd, no residual.
    Mvd(i32, i32),
    /// P_L0_16x16 with this refIdx and mvd, no residual.
    Ref(u32, i32, i32),
    /// P_L0_L0_16x8 with refIdx 0 and these mvds for the upper and the lower partition, no
    /// residual.
    Split16x8([i32; 2], [i32; 2]),
    /// I_16x16 with DC prediction (luma and chroma) and no residual.
    Intra,
    /// [`Mb::Intra`] whose neighbour A or B of luma block 0 is I_PCM (nC 8 or 16 for the
    /// Intra16x16DCLevel coeff_token, 9.2.1), the other not.
    IntraByPcm,
    /// I_PCM with the samples of `random_mb` from this seed.
    Pcm(u32),
}

/// A CAVLC P slice of an MBAFF frame from pair `p.first_mb`: per pair its field flag and its two
/// macroblocks (a pair is either skipped as a whole or has a coded top macroblock, so the flag is
/// sent with the top one).
fn mbaff_inter_slice_nal(p: &Pic, pairs: &[(bool, [Mb; 2])]) -> Vec<u8> {
    let head = Pic { interlaced: true, field: None, mbs: vec![None], ..p.clone() };
    let mut w = BitWriter::new();
    slice_header(&mut w, &head, false);
    // reference indices of frame macroblocks range over the list, of field macroblocks over twice
    // as many fields
    let n_ref = p.l0_weights.len().max(1) as u32;
    let mut run = 0;
    for &(field, mbs) in pairs {
        assert!(!matches!(mbs[0], Mb::Skip) || matches!(mbs[1], Mb::Skip), "flag would go with the bottom macroblock");
        let ref0 = |w: &mut BitWriter| match if field { 2 * n_ref } else { n_ref } {
            1 => {}
            2 => w.write_bit(true),
            _ => w.write_ue(0),
        };
        for (k, mb) in mbs.into_iter().enumerate() {
            if let Mb::Skip = mb {
                run += 1;
                continue;
            }
            w.write_ue(run);
            run = 0;
            if k == 0 {
                w.write_bit(field);
            }
            match mb {
                Mb::Skip => {}
                Mb::Mvd(x, y) | Mb::Ref(_, x, y) => {
                    w.write_ue(0); // P_L0_16x16
                    let ri = if let Mb::Ref(r, ..) = mb { r } else { 0 };
                    // ref_idx_l0 te(v) with range n - 1 (7.4.5.1, 9.1.2)
                    match if field { 2 * n_ref } else { n_ref } {
                        1 => assert_eq!(ri, 0),
                        2 => w.write_bit(ri == 0),
                        _ => w.write_ue(ri),
                    }
                    w.write_se(x);
                    w.write_se(y);
                    w.write_ue(0); // coded_block_pattern 0
                }
                Mb::Split16x8(m0, m1) => {
                    w.write_ue(1); // P_L0_L0_16x8
                    ref0(&mut w);
                    ref0(&mut w);
                    for m in [m0, m1] {
                        w.write_se(m[0]);
                        w.write_se(m[1]);
                    }
                    w.write_ue(0); // coded_block_pattern 0
                }
                Mb::Intra | Mb::IntraByPcm => {
                    w.write_ue(5 + 3); // I_16x16_2_0_0 (DC prediction, no AC, no chroma)
                    w.write_ue(0); // intra_chroma_pred_mode DC
                    w.write_se(0); // mb_qp_delta
                    // Intra16x16DCLevel: coeff_token TotalCoeff 0, for nC 0..1 or 8 and over
                    if let Mb::Intra = mb {
                        w.write_bit(true);
                    } else {
                        w.write_bits(0b000011, 6);
                    }
                }
                Mb::Pcm(mut seed) => {
                    w.write_ue(5 + 25); // I_PCM
                    write_pcm(&mut w, &random_mb(&mut seed));
                }
            }
        }
    }
    if run > 0 {
        w.write_ue(run);
    }
    w.rbsp_trailing();
    nal(p.ref_idc, 1, &w.finish())
}

/// FNV-1a over the planes of the output pictures.
fn planes_hash(pics: &[Planes3]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for (y, u, v) in pics {
        for &b in y.iter().chain(u).chain(v) {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

/// An I frame of I_PCM frame pairs holding a smooth picture (luma rising 1 per column and 5 per
/// line, chroma likewise), the reference of [`mbaff_deblocking_frame`].
fn smooth_mbaff_i_frame() -> (Vec<u8>, Planes3) {
    let (w, h) = (MB_W * 16, FRAME_MB_H * 16);
    let y: Vec<u8> = (0..w * h).map(|i| (20 + i % w + 5 * (i / w) / 2) as u8).collect();
    let c: Vec<u8> = (0..w * h / 4).map(|i| (60 + i % (w / 2) + 3 * (i / (w / 2))) as u8).collect();
    let pairs: Vec<Pair> = (0..PAIRS)
        .map(|i| {
            let mb = |k: usize| -> Vec<u8> {
                let (x0, y0) = ((i % MB_W) * 16, (i / MB_W) * 32 + 16 * k);
                let mut s = Vec::with_capacity(384);
                (0..16).for_each(|r| s.extend_from_slice(&y[(y0 + r) * w + x0..][..16]));
                for _ in 0..2 {
                    (0..8).for_each(|r| s.extend_from_slice(&c[(y0 / 2 + r) * (w / 2) + x0 / 2..][..8]));
                }
                s
            };
            Pair { field: false, mbs: Some([mb(0), mb(1)]), top_skip: false }
        })
        .collect();
    let pic = Pic { idr: true, frame_num: 0, ref_idc: 3, poc_lsb: Some(0), ..Default::default() };
    (mbaff_slice_nal(&pic, &pairs, false), render_mbaff(&pairs, None))
}

/// The P frame of [`mbaff_deblocking_frame`]: every MBAFF edge case of 8.7 at QP 38 over
/// smooth content. Row 0: field, field, field (intra top), frame; row 1: frame, frame (intra
/// top), field (intra top), field (intra bottom). That gives same-kind field edges with small
/// vertical motion differences (pairs 0 and 1; the top macroblock of pair 1 has two 16x8
/// partitions, predicted (4, 0) from the left and (4, 3), so its internal edge compares a
/// vertical difference of 3 against the field limit of 2), mixed left edges whose bS alternates per line
/// between an intra and an inter macroblock for a frame macroblock (pair 3 beside field pair 2)
/// and for a field macroblock (pair 6 beside frame pair 5), frame pairs under field pairs (pairs 4
/// and 5, two field passes) and an intra field macroblock over a frame pair (pair 7, mixed
/// horizontal edge).
const DEBLOCK_PAIRS: [(bool, [Mb; 2]); PAIRS] = [
    (true, [Mb::Mvd(4, 0), Mb::Mvd(0, 6)]),
    (true, [Mb::Split16x8([0, 0], [0, 3]), Mb::Mvd(-4, 3)]),
    (true, [Mb::Intra, Mb::Mvd(2, -2)]),
    (false, [Mb::Mvd(-2, 0), Mb::Mvd(0, -4)]),
    (false, [Mb::Mvd(3, 1), Mb::Skip]),
    (false, [Mb::Intra, Mb::Mvd(1, 0)]),
    (true, [Mb::Intra, Mb::Mvd(-3, 2)]),
    (true, [Mb::Mvd(2, 2), Mb::Intra]),
];

/// Decode `stream` (the smooth I frame and MBAFF P slices) at 1 and 3 threads: the same pictures
/// both times, equal to ffmpeg's when it is installed, and hashing to `want` (ffmpeg's output,
/// recorded).
fn check_mbaff_deblocking(stream: &[u8], want: u64, name: &str) {
    let one = decode_all(stream, 1);
    assert_eq!(one.len(), 2, "{name}");
    assert!(decode_all(stream, 3) == one, "{name}: threads differ");
    cross_check_ffmpeg(stream, &one, name);
    assert_eq!(planes_hash(&one), want, "{name}: output differs from ffmpeg's recorded output");
}

/// Deblocking of an MBAFF frame (8.7 with MbaffFrameFlag 1): mixed frame and field edges with bS
/// per line, frame macroblocks under field pairs filtered once per field, field macroblock motion
/// limits and intra bS on mixed horizontal edges, against ffmpeg.
#[test]
fn mbaff_deblocking_frame() {
    let (i_frame, _) = smooth_mbaff_i_frame();
    let mut stream = interlaced_sps_with(77, true);
    stream.extend(pps(false, false));
    stream.extend(i_frame);
    let p = Pic { frame_num: 1, ref_idc: 2, poc_lsb: Some(4), qp_delta: 12, ..Default::default() };
    stream.extend(mbaff_inter_slice_nal(&p, &DEBLOCK_PAIRS));
    check_mbaff_deblocking(&stream, 10407637496959991110, "synth_mbaff_deblock");
}

/// disable_deblocking_filter_idc 2 in an MBAFF frame of two slices: edges between the slices
/// stay unfiltered (the second slice starts at pair 2, so pair 2 has no left neighbour and pairs
/// 4 to 6 none above for the filter), against ffmpeg.
#[test]
fn mbaff_deblocking_idc2_stops_at_slices() {
    let (i_frame, _) = smooth_mbaff_i_frame();
    for (idc, want) in [(0u32, 15451903455263300175u64), (2, 14323264706295858958)] {
        let mut stream = interlaced_sps_with(77, true);
        stream.extend(pps(false, false));
        stream.extend(i_frame.clone());
        let p = Pic { frame_num: 1, ref_idc: 2, poc_lsb: Some(4), qp_delta: 12, deblock_idc: idc, ..Default::default() };
        stream.extend(mbaff_inter_slice_nal(&p, &DEBLOCK_PAIRS[..2]));
        stream.extend(mbaff_inter_slice_nal(&Pic { first_mb: 2, ..p }, &DEBLOCK_PAIRS[2..]));
        check_mbaff_deblocking(&stream, want, &format!("synth_mbaff_deblock_idc{idc}"));
    }
}

/// An MBAFF P frame with explicit weights (8.4.2.3) for two reference frames whose entries
/// differ: field macroblocks index fields, refIdx 0 to 3, and take the weights of frame entry
/// refIdx >> 1 (8-284); frame macroblocks index frames. The references are the smooth I frame and
/// a frame of random I_PCM pairs. 1 and 3 threads, against ffmpeg.
#[test]
fn mbaff_explicit_weights_of_field_macroblocks() {
    let (i_frame, _) = smooth_mbaff_i_frame();
    let mut seed = 9u32;
    let random: Vec<Pair> =
        (0..PAIRS).map(|_| Pair { field: false, mbs: Some([random_mb(&mut seed), random_mb(&mut seed)]), top_skip: false }).collect();
    let pairs = [
        (true, [Mb::Ref(2, 0, 0), Mb::Ref(3, 4, 0)]),
        (false, [Mb::Ref(1, 0, 0), Mb::Ref(0, 0, 0)]),
        (true, [Mb::Ref(1, 0, 4), Mb::Ref(0, 0, 0)]),
        (true, [Mb::Ref(3, 0, 0), Mb::Ref(2, 2, 2)]),
        (false, [Mb::Ref(0, 2, 0), Mb::Ref(1, 0, 2)]),
        (true, [Mb::Ref(2, 0, 0), Mb::Ref(1, 0, 0)]),
        (false, [Mb::Ref(1, 0, 0), Mb::Skip]),
        (true, [Mb::Ref(3, 0, 0), Mb::Ref(0, 0, 0)]),
    ];
    let mut stream = interlaced_sps_with(77, true);
    stream.extend(pps(false, true));
    stream.extend(i_frame);
    stream.extend(mbaff_slice_nal(&Pic { frame_num: 1, ref_idc: 2, poc_lsb: Some(4), ..Default::default() }, &random, false));
    let p = Pic {
        frame_num: 2,
        ref_idc: 2,
        poc_lsb: Some(8),
        deblock_off: true,
        l0_weights: vec![[(40, 3), (20, -2), (12, 5)], [(24, -6), (6, 4), (10, -3)]],
        ..Default::default()
    };
    stream.extend(mbaff_inter_slice_nal(&p, &pairs));
    let one = decode_all(&stream, 1);
    assert_eq!(one.len(), 3);
    assert!(decode_all(&stream, 3) == one, "threads differ");
    cross_check_ffmpeg(&stream, &one, "synth_mbaff_weights");
    assert_eq!(planes_hash(&one), 12712244450079857279, "output differs from ffmpeg's recorded output");
}

/// Constrained intra prediction in an MBAFF frame: a field macroblock beside a frame pair of one
/// intra (I_PCM) and one inter macroblock sees half of its left column as available, the half of
/// its lines in the intra macroblock (8.3.1.2 with 6.4.12.2): Intra16x16 needs the whole column
/// (it predicts from the top only), chroma DC uses the left samples for the chroma blocks of the
/// available half. Pair 1 has the intra macroblock above, pair 5 below. Against ffmpeg.
#[test]
fn mbaff_constrained_intra_with_half_the_left_column() {
    let (i_frame, _) = smooth_mbaff_i_frame();
    let pairs = [
        (false, [Mb::Pcm(1), Mb::Mvd(2, 0)]),
        (true, [Mb::IntraByPcm, Mb::IntraByPcm]),
        (false, [Mb::Mvd(0, 2), Mb::Mvd(1, 0)]),
        (false, [Mb::Skip, Mb::Skip]),
        (false, [Mb::Mvd(1, 1), Mb::Pcm(3)]),
        (true, [Mb::Intra, Mb::Intra]),
        (false, [Mb::Skip, Mb::Skip]),
        (false, [Mb::Skip, Mb::Skip]),
    ];
    let mut stream = interlaced_sps_with(77, true);
    stream.extend(pps_full(false, false, 0, true));
    stream.extend(i_frame);
    let p = Pic { frame_num: 1, ref_idc: 2, poc_lsb: Some(4), deblock_off: true, ..Default::default() };
    stream.extend(mbaff_inter_slice_nal(&p, &pairs));
    let one = decode_all(&stream, 1);
    assert_eq!(one.len(), 2);
    assert!(decode_all(&stream, 3) == one, "threads differ");
    cross_check_ffmpeg(&stream, &one, "synth_mbaff_constrained_intra");
    assert_eq!(planes_hash(&one), 10295339011532637482, "output differs from ffmpeg's recorded output");
}
