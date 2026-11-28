//! Scope math on generated frames with known values.

use crate::*;
use craft_color::rgb_to_ycbcr;

fn rgba8(w: usize, h: usize, mut f: impl FnMut(usize, usize) -> [u8; 3]) -> Vec<u8> {
    let mut px = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = f(x, y);
            px.extend_from_slice(&[r, g, b, 255]);
        }
    }
    px
}

fn solid(w: usize, h: usize, c: [u8; 3]) -> Signal {
    Signal::from_rgba8(w, h, &rgba8(w, h, |_, _| c))
}

/// 75 % color bars (code 191): white, yellow, cyan, green, magenta, red, blue, black; 32 px each.
const BAR: u8 = 191;
const BARS: [[u8; 3]; 8] = [[BAR, BAR, BAR], [BAR, BAR, 0], [0, BAR, BAR], [0, BAR, 0], [BAR, 0, BAR], [BAR, 0, 0], [0, 0, BAR], [0, 0, 0]];

fn bars() -> Signal {
    Signal::from_rgba8(256, 16, &rgba8(256, 16, |x, _| BARS[x / 32]))
}

fn code(v: u8) -> f32 {
    v as f32 / 255.0
}

#[test]
fn single_color_lands_in_exact_histogram_bins() {
    let s = solid(64, 36, [200, 100, 50]);
    let n = 64 * 36;
    let h = histogram(&s, &Params::default());
    assert_eq!(h.samples, n);
    assert_eq!((h.r[200], h.g[100], h.b[50]), (n, n, n));
    assert_eq!(h.r.iter().sum::<u32>(), n);
    let y = rgb_to_ycbcr(code(200), code(100), code(50), Matrix::Bt709)[0];
    assert_eq!(h.y[(y * 255.0).round() as usize], n);
    assert_eq!((h.below, h.above, h.hdr), ([0; 4], [0; 4], None));
}

#[test]
fn ramp_fills_every_bin_and_waveform_row_equals_code_value() {
    let s = Signal::from_rgba8(256, 10, &rgba8(256, 10, |x, _| [x as u8; 3]));
    let p = Params::default();
    let h = histogram(&s, &p);
    for k in 0..256 {
        assert_eq!((h.r[k], h.g[k], h.b[k], h.y[k]), (10, 10, 10, 10), "bin {k}");
    }
    for kind in [WaveformType::Luma, WaveformType::YcNoChroma] {
        let w = waveform(&s, kind, &p);
        let y = w.trace("Y").unwrap();
        for x in 0..256 {
            assert_eq!(y.rows_in(x), vec![x], "{kind:?} column {x}");
            assert_eq!(y.at(x, x), 10);
        }
    }
    let w = waveform(&s, WaveformType::Rgb, &p);
    assert_eq!(w.traces.len(), 3);
    for t in &w.traces {
        assert!((0..256).all(|x| t.rows_in(x) == vec![x]), "{}", t.name);
    }
}

#[test]
fn color_bars_parade_levels_and_vectorscope_targets() {
    let s = bars();
    let p = Params::default();
    // RGB parade: every bar column holds its channel's code value exactly
    let par = parade(&s, ParadeType::Rgb, &p);
    for (ch, name) in ["R", "G", "B"].iter().enumerate() {
        let t = par.trace(name).unwrap();
        for (i, bar) in BARS.iter().enumerate() {
            for x in i * 32..(i + 1) * 32 {
                assert_eq!(t.rows_in(x), vec![bar[ch] as usize], "{name} bar {i}");
                assert_eq!(t.at(x, bar[ch] as usize), 16);
            }
        }
    }
    // RGB-White adds the luma trace; YUV has Y, Cb, Cr
    assert_eq!(parade(&s, ParadeType::RgbWhite, &p).traces.iter().map(|t| t.name).collect::<Vec<_>>(), ["R", "G", "B", "Y"]);
    assert_eq!(parade(&s, ParadeType::Yuv, &p).traces.iter().map(|t| t.name).collect::<Vec<_>>(), ["Y", "Cb", "Cr"]);
    // Vectorscope: each colored bar on its 75 % target cell, white and black in the center
    for m in [Matrix::Bt601, Matrix::Bt709, Matrix::Bt2020Ncl] {
        let p = Params { matrix: m, ..Params::default() };
        let v = vectorscope_yuv(&s, &p);
        assert_eq!(v.samples, 256 * 16);
        let bar_samples = 32 * 16;
        for t in targets(m, code(BAR)) {
            let (x, y) = v.cell(t.cb, t.cr).unwrap();
            assert_eq!(v.at(x, y), bar_samples, "{m:?} {}", t.name);
        }
        let (cx, cy) = v.cell(0.0, 0.0).unwrap();
        assert_eq!(v.at(cx, cy), 2 * bar_samples);
        // the densest spots: the center (two bars), then the six targets
        let pk = summary::peaks(&v, 10, 0.01);
        assert_eq!(pk.len(), 7);
        assert_eq!((pk[0].x, pk[0].y, pk[0].share), (cx, cy, 0.25));
        assert!(pk[1..].iter().all(|k| k.share == 0.125));
    }
}

#[test]
fn vectorscope_positions_of_primaries_are_known() {
    let p = Params::default();
    // BT.709 red: Cb = −0.2126 / 1.8556 = −0.11457, Cr = 0.5
    let v = vectorscope_yuv(&solid(8, 8, [255, 0, 0]), &p);
    let pk = summary::peaks(&v, 1, 0.0)[0];
    assert_eq!((pk.x, pk.y), (103, 21));
    assert_eq!((pk.share, v.samples), (1.0, 64));
    assert!((red_angle(Matrix::Bt709) - 102.906).abs() < 0.01, "{}", red_angle(Matrix::Bt709));
    assert!((red_angle(Matrix::Bt601) - 108.645).abs() < 0.01, "{}", red_angle(Matrix::Bt601));
    // the mean chroma of a flat color is its own
    let [_, cb, cr] = rgb_to_ycbcr(1.0, 0.0, 0.0, Matrix::Bt709);
    assert!((v.mean[0] - cb).abs() < 1e-6 && (v.mean[1] - cr).abs() < 1e-6);
    // greys sit in the center for every matrix
    for m in [Matrix::Bt601, Matrix::Bt709, Matrix::Bt2020Ncl] {
        let v = vectorscope_yuv(&solid(4, 4, [77, 77, 77]), &Params { matrix: m, ..p });
        assert_eq!(v.at(127, 127), 16, "{m:?}");
    }
    // the skin-tone line sits between red and yellow
    let yl = targets(Matrix::Bt709, 1.0)[5];
    assert!(red_angle(Matrix::Bt709) < SKIN_TONE_DEG && SKIN_TONE_DEG < angle_deg(yl.cb, yl.cr));
}

#[test]
fn hls_vectorscope_puts_hue_on_the_angle_and_saturation_on_the_radius() {
    let p = Params::default();
    let red = red_angle(p.matrix);
    for (rgb, hue) in
        [([255, 0, 0], 0.0f32), ([255, 255, 0], 60.0), ([0, 255, 0], 120.0), ([0, 255, 255], 180.0), ([0, 0, 255], 240.0), ([255, 0, 255], 300.0)]
    {
        let v = vectorscope_hls(&solid(4, 4, rgb), &p);
        let a = (red + hue).to_radians();
        let want = cell_of(0.5 * a.cos(), 0.5 * a.sin(), p.vector_size, VECTOR_EXTENT).unwrap();
        let pk = summary::peaks(&v, 1, 0.0)[0];
        assert_eq!((pk.x, pk.y), want, "{rgb:?}");
        assert!((pk.magnitude - 100.0).abs() < 0.6, "{rgb:?} {}", pk.magnitude);
        // the per-sample helper agrees with the tabulated scope
        let [cb, cr] = hls_point(rgb.map(code), p.matrix);
        assert_eq!(cell_of(cb, cr, p.vector_size, VECTOR_EXTENT), Some(want), "{rgb:?}");
    }
    // half-saturated red (HSL s = 0.5) at half the radius; grey in the center
    let v = vectorscope_hls(&solid(4, 4, [191, 64, 64]), &p);
    let pk = summary::peaks(&v, 1, 0.0)[0];
    let hue_off = pk.angle_deg.min(360.0 - pk.angle_deg);
    assert!((pk.magnitude - 50.0).abs() < 1.0 && hue_off < 0.5, "{pk:?}");
    let v = vectorscope_hls(&solid(4, 4, [90, 90, 90]), &p);
    assert_eq!(v.at(127, 127), 16);
    let pk = summary::peaks(&v, 1, 0.0)[0];
    assert_eq!((pk.angle_deg, pk.magnitude), (0.0, 0.0), "{pk:?}");
    // Clamp Signal off: an over-range sample keeps its own hue
    let over = Signal::from_fn(1, 1, |_, _| [1.2, 0.5, 0.0]);
    let inside = Signal::from_fn(1, 1, |_, _| [1.0, 0.5, 0.0]);
    let off = Params { clamp: false, ..p };
    let peak = |s: &Signal, p: &Params| summary::peaks(&vectorscope_hls(s, p), 1, 0.0)[0];
    assert_ne!((peak(&over, &off).x, peak(&over, &off).y), (peak(&inside, &off).x, peak(&inside, &off).y));
    assert_eq!((peak(&over, &p).x, peak(&over, &p).y), (peak(&inside, &p).x, peak(&inside, &p).y));
}

#[test]
fn yc_waveform_draws_chroma_around_luma() {
    let p = Params::default();
    let w = waveform(&solid(4, 4, [128, 128, 128]), WaveformType::Yc, &p);
    assert_eq!((w.trace("Y").unwrap().rows_in(0), w.trace("C").unwrap().rows_in(0)), (vec![128], vec![128]));
    let w = waveform(&solid(4, 4, [255, 0, 0]), WaveformType::Yc, &p);
    let [y, cb, cr] = rgb_to_ycbcr(1.0, 0.0, 0.0, Matrix::Bt709);
    let c = (cb * cb + cr * cr).sqrt();
    let row = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as usize;
    assert_eq!(w.trace("Y").unwrap().rows_in(2), vec![row(y)]);
    assert_eq!(w.trace("C").unwrap().rows_in(2), vec![row(y - c), row(y + c)]);
}

#[test]
fn yuv_parade_offsets_chroma_to_mid_scale() {
    let w = parade(&solid(4, 4, [128, 128, 128]), ParadeType::Yuv, &Params::default());
    assert_eq!(w.trace("Y").unwrap().rows_in(0), vec![128]);
    assert_eq!(w.trace("Cb").unwrap().rows_in(0), vec![128]);
    assert_eq!(w.trace("Cr").unwrap().rows_in(0), vec![128]);
}

#[test]
fn decimation_keeps_whole_samples() {
    let px = rgba8(1920, 1080, |x, y| [(x % 256) as u8, (y % 256) as u8, 7]);
    let s = Signal::from_rgba8(1920, 1080, &px);
    assert_eq!((s.w, s.h), (480, 270));
    assert_eq!(s.rgb[1], [code(4), 0.0, code(7)]);
    assert_eq!(s.rgb[480], [0.0, code(4), code(7)]);
    let s = Signal::from_rgba8(1921, 9, &rgba8(1921, 9, |_, _| [1, 2, 3]));
    assert_eq!((s.w, s.h), (385, 9));
    assert!(Signal::from_rgba8(10, 10, &[0; 12]).is_empty());
}

#[test]
fn clamp_signal_controls_out_of_range_values() {
    let s = Signal::from_fn(4, 4, |x, _| if x < 2 { [1.2, 0.5, -0.05] } else { [0.5, 0.5, 0.5] });
    let on = Params::default();
    let off = Params { clamp: false, ..on };
    let h = histogram(&s, &on);
    assert_eq!((h.r[255], h.above[0], h.b[0], h.below[2]), (8, 8, 8, 8));
    let h = histogram(&s, &off);
    assert_eq!((h.r[255], h.above[0], h.b[0], h.below[2]), (0, 8, 0, 8));
    // with clamping, the histogram's luma comes from the clamped R'G'B', as the waveform's does
    let hot = Signal::from_fn(1, 1, |_, _| [2.0, -1.0, 0.0]);
    let rows = waveform(&hot, WaveformType::Luma, &on).trace("Y").unwrap().rows_in(0);
    let hh = histogram(&hot, &on);
    assert_eq!(hh.y.iter().position(|&c| c > 0), rows.first().copied());
    // and the luma of the sample as it is still counts as out of range
    assert_eq!((hh.below[3], hh.above[3]), (1, 0));
    let white = histogram(&Signal::from_fn(1, 1, |_, _| [2.0; 3]), &on);
    assert_eq!((white.y[255], white.above[3]), (1, 1));
    // waveform: clamped onto the top row, or plotted above 100 % within −10..110
    let w = waveform(&s, WaveformType::Rgb, &on);
    assert_eq!(w.trace("R").unwrap().rows_in(0), vec![255]);
    let w = waveform(&s, WaveformType::Rgb, &off);
    assert_eq!((w.lo, w.hi), (-0.1, 1.1));
    assert!(w.trace("R").unwrap().rows_in(0).is_empty(), "1.2 is outside −10..110");
    let r = w.trace("B").unwrap().rows_in(0);
    assert_eq!(r, vec![row_of(-0.05, -0.1, 1.1, 256).unwrap()]);
}

#[test]
fn hdr_axis_helpers() {
    assert!(nits_to_pq(0.0) < 1e-5);
    assert!((nits_to_pq(10_000.0) - 1.0).abs() < 1e-6);
    // reference white (203 cd/m²) ≈ 58 % of the PQ range (BT.2408)
    let w = linear_to_pq([1.0, 1.0, 1.0, 1.0])[0];
    assert!((w - nits_to_pq(203.0)).abs() < 1e-6 && (w - 0.58).abs() < 0.01, "{w}");
    // premultiplied: half-covered white is still white
    assert!((linear_to_pq([0.5, 0.5, 0.5, 0.5])[0] - w).abs() < 1e-6);
    assert!((sdr_to_pq(1.0) - nits_to_pq(100.0)).abs() < 1e-6);
}

#[test]
fn nan_and_empty_signals_do_not_panic() {
    let s = Signal::from_fn(3, 3, |x, _| if x == 1 { [f32::NAN; 3] } else { [0.5; 3] });
    for clamp in [true, false] {
        let p = Params { clamp, ..Params::default() };
        for k in ScopeKind::ALL {
            let _ = compute(k, &s, &Params { waveform: WaveformType::Yc, parade: ParadeType::RgbWhite, ..p });
            let _ = compute(k, &Signal::default(), &p);
        }
    }
    assert!(summary::channel_stats(&Signal::default(), Matrix::Bt709).is_none());
    // NaN and infinite samples are left out of the levels, so every number stays finite
    let st = summary::channel_stats(&s, Matrix::Bt709).unwrap();
    assert_eq!((st.r.min, st.r.max, st.r.mean, st.y.mean), (50.0, 50.0, 50.0, 50.0));
    assert!(st.chroma_mean.iter().all(|v| v.is_finite()) && st.saturation.is_finite());
    let inf = Signal::from_fn(2, 1, |x, _| if x == 0 { [f32::INFINITY, 0.25, 0.25] } else { [0.25; 3] });
    let st = summary::channel_stats(&inf, Matrix::Bt709).unwrap();
    assert_eq!((st.r.max, st.g.mean), (25.0, 25.0));
    assert!(summary::channel_stats(&Signal::from_fn(1, 1, |_, _| [f32::NAN; 3]), Matrix::Bt709).is_none());
}

#[test]
fn summaries_report_levels_in_percent() {
    let s = solid(8, 8, [200, 100, 50]);
    let st = summary::channel_stats(&s, Matrix::Bt709).unwrap();
    assert_eq!((st.r.mean, st.g.mean, st.b.mean), (78.43, 39.22, 19.61));
    assert_eq!((st.r.min, st.r.max), (78.43, 78.43));
    // a two-level frame: left half black, right half white
    let s = Signal::from_rgba8(100, 4, &rgba8(100, 4, |x, _| if x < 50 { [0; 3] } else { [255; 3] }));
    let w = waveform(&s, WaveformType::Luma, &Params::default());
    let cols = summary::trace_columns(&w, w.trace("Y").unwrap(), 4);
    assert_eq!(cols.len(), 4);
    assert_eq!(cols[0].unwrap(), summary::Level { min: 0.0, max: 0.0, mean: 0.0 });
    assert_eq!(cols[3].unwrap(), summary::Level { min: 100.0, max: 100.0, mean: 100.0 });
    assert_eq!((cols[1].unwrap().max, cols[2].unwrap().min), (0.0, 100.0));
    let c = summary::coarse(&vectorscope_yuv(&bars(), &Params::default()), 16);
    assert_eq!(c.iter().map(|e| e[2]).sum::<u32>(), 256 * 16);
}

#[test]
fn paint_adds_traces_and_leaves_empty_cells_transparent() {
    let s = solid(4, 8, [128, 128, 128]);
    let w = waveform(&s, WaveformType::Rgb, &Params::default());
    let layers: Vec<(&Grid, [f32; 3])> = w.traces.iter().zip([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]).collect();
    let (pw, ph, px) = paint::grids(&layers, paint::waveform_reference(w.per_column), 1.0);
    assert_eq!((pw, ph, px.len()), (4, 256, 4 * 256 * 4));
    // row 128 from the bottom = image row 127; R+G+B coincide → white
    let i = (127 * 4) * 4;
    assert_eq!(&px[i..i + 4], &[255, 255, 255, 255]);
    assert_eq!(&px[0..4], &[0, 0, 0, 0]);
    // dimmed is darker than bright
    assert!(paint::intensity(3, 100.0, Brightness::Dimmed.gain()) < paint::intensity(3, 100.0, Brightness::Bright.gain()));
    let v = vectorscope_yuv(&bars(), &Params::default());
    let (n, _, vp) = paint::vectorscope(&v, [1.0; 3], 1.0, true);
    assert_eq!(vp.len(), n * n * 4);
    // seven spots of 3 × 3 texels each (none touch)
    assert_eq!(vp.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count(), 7 * 9);
}

#[test]
fn names_round_trip() {
    for k in ScopeKind::ALL {
        assert_eq!(ScopeKind::from_name(k.name()), Some(k));
        assert_eq!(ScopeKind::from_name(k.label()), Some(k));
        assert_eq!(serde_json::to_value(k).unwrap(), serde_json::Value::from(k.name()));
    }
    assert_eq!(ScopeKind::from_name("vectorscope"), Some(ScopeKind::VectorscopeYuv));
    assert_eq!(WaveformType::from_name("Luma and chroma, chroma hidden"), Some(WaveformType::YcNoChroma));
    assert_eq!(WaveformType::from_name("ycNoChroma"), Some(WaveformType::YcNoChroma));
    assert_eq!(ParadeType::from_name("rgb and luma"), Some(ParadeType::RgbWhite));
    assert_eq!(ColorSpace::from_name("709"), Some(ColorSpace::Rec709));
    assert_eq!(ColorSpace::from_name("Rec. 2100"), Some(ColorSpace::Rec2100));
    assert_eq!(Scale::from_name("8 bit"), Some(Scale::Bits8));
    assert_eq!(serde_json::to_value(Targets::Percent100).unwrap(), serde_json::Value::from("100"));
    assert_eq!(ColorSpace::Auto.resolve(true), ColorSpace::Rec2100);
    assert_eq!(ColorSpace::Auto.resolve(false).matrix(), Matrix::Bt709);
}

// ------------------------------------------------------------------------------------ frames

/// An NV12 frame of `w × h` with every pixel at one (Y', Cb, Cr) code value, `pad` spare bytes at
/// the end of each row of both planes.
fn nv12(w: usize, h: usize, pad: usize, [y, cb, cr]: [u8; 3]) -> (Vec<u8>, usize, Vec<u8>, usize) {
    let ys = w + pad;
    let cs = w.div_ceil(2) * 2 + pad;
    let mut yp = vec![0xEE; ys * h];
    let mut uv = vec![0xEE; cs * h.div_ceil(2)];
    for row in yp.chunks_mut(ys) {
        row[..w].fill(y);
    }
    for row in uv.chunks_mut(cs) {
        for pair in row[..w.div_ceil(2) * 2].chunks_mut(2) {
            pair.copy_from_slice(&[cb, cr]);
        }
    }
    (yp, ys, uv, cs)
}

fn read_nv12(w: usize, h: usize, pad: usize, ycc: [u8; 3], matrix: Matrix, range: Range) -> Signal {
    let (y, y_stride, uv, uv_stride) = nv12(w, h, pad, ycc);
    let mut s = Signal::default();
    s.fill(&Frame::Nv12 { width: w, height: h, y: &y, y_stride, uv: &uv, uv_stride, matrix, range }).unwrap();
    s
}

/// 8-bit limited-range BT.709 code values of the 100 % color bars (SMPTE RP 219-1, table 1;
/// the Y'CbCr equations of ITU-R BT.709-6 item 3), decoded back to R'G'B'.
#[test]
fn nv12_bt709_limited_bars_decode_to_their_rgb() {
    let bars: [([u8; 3], [f32; 3]); 8] = [
        ([235, 128, 128], [1.0, 1.0, 1.0]),
        ([219, 16, 138], [1.0, 1.0, 0.0]),
        ([188, 154, 16], [0.0, 1.0, 1.0]),
        ([173, 42, 26], [0.0, 1.0, 0.0]),
        ([78, 214, 230], [1.0, 0.0, 1.0]),
        ([63, 102, 240], [1.0, 0.0, 0.0]),
        ([32, 240, 118], [0.0, 0.0, 1.0]),
        ([16, 128, 128], [0.0, 0.0, 0.0]),
    ];
    for (ycc, want) in bars {
        let s = read_nv12(6, 4, 0, ycc, Matrix::Bt709, Range::Limited);
        assert_eq!((s.w, s.h, s.len()), (6, 4, 24));
        for got in &s.rgb {
            for k in 0..3 {
                // the published code values are rounded to whole codes: within 1.5 codes of 255
                assert!((got[k] - want[k]).abs() < 1.5 / 255.0, "{ycc:?}: {got:?} vs {want:?}");
            }
        }
    }
}

#[test]
fn nv12_full_range_and_odd_sizes() {
    // full range: Y' 255 with neutral chroma is white, Y' 0 is black
    let s = read_nv12(5, 3, 3, [255, 128, 128], Matrix::Bt601, Range::Full);
    assert_eq!((s.w, s.h), (5, 3));
    assert!(s.rgb.iter().all(|c| c.iter().all(|v| (v - 1.0).abs() < 0.01)), "{:?}", s.rgb[0]);
    let s = read_nv12(1, 1, 0, [0, 128, 128], Matrix::Bt601, Range::Full);
    assert!(s.rgb[0].iter().all(|v| v.abs() < 0.01), "{:?}", s.rgb[0]);
    // a grey NV12 frame and the same grey as RGBA8 give the same histogram luma bin
    let s = read_nv12(7, 5, 1, [126, 128, 128], Matrix::Bt709, Range::Limited);
    let h = histogram(&s, &Params::default());
    let grey = ((126.0 - 16.0) / 219.0 * 255.0f32).round() as usize;
    assert_eq!(h.y[grey], 35);
}

#[test]
fn rgba_with_a_stride_reads_the_same_samples_as_packed_rgba() {
    let (w, h) = (9, 5);
    let packed = rgba8(w, h, |x, y| [x as u8 * 20, y as u8 * 40, 3]);
    let stride = w * 4 + 12;
    let mut strided = vec![0xAB; stride * h];
    for y in 0..h {
        strided[y * stride..y * stride + w * 4].copy_from_slice(&packed[y * w * 4..(y + 1) * w * 4]);
    }
    let mut s = Signal::default();
    s.fill(&Frame::Rgba8 { width: w, height: h, stride, data: &strided }).unwrap();
    assert_eq!(s, Signal::from_rgba8(w, h, &packed));
    // the last row needs only its own pixels, not a whole stride
    s.fill(&Frame::Rgba8 { width: w, height: h, stride, data: &strided[..stride * (h - 1) + w * 4] }).unwrap();
    assert_eq!(s.len(), w * h);
}

#[test]
fn bad_frames_give_errors_and_an_empty_signal() {
    let px = [0u8; 64];
    let mut s = solid(2, 2, [1, 2, 3]);
    assert_eq!(s.fill(&Frame::Rgba8 { width: 4, height: 4, stride: 8, data: &px }), Err(FrameError::Stride { plane: "RGBA", stride: 8, needed: 16 }));
    assert!(s.is_empty());
    assert_eq!(s.fill(&Frame::Rgba8 { width: 4, height: 5, stride: 16, data: &px }), Err(FrameError::Short { plane: "RGBA", len: 64, needed: 80 }));
    assert_eq!(s.fill(&Frame::Rgba8 { width: usize::MAX, height: 2, stride: 16, data: &px }), Err(FrameError::TooLarge));
    assert_eq!(s.fill(&Frame::Rgba8 { width: 4, height: usize::MAX, stride: usize::MAX / 2, data: &px }), Err(FrameError::TooLarge));
    let nv = |width, height, y: &[u8], y_stride, uv: &[u8], uv_stride| {
        let mut s = Signal::default();
        s.fill(&Frame::Nv12 { width, height, y, y_stride, uv, uv_stride, matrix: Matrix::Bt709, range: Range::Limited })
    };
    assert_eq!(nv(4, 4, &px[..16], 4, &px[..7], 4), Err(FrameError::Short { plane: "UV", len: 7, needed: 8 }));
    assert_eq!(nv(5, 2, &px[..10], 5, &px[..6], 5), Err(FrameError::Stride { plane: "UV", stride: 5, needed: 6 }));
    assert_eq!(nv(4, 4, &px[..15], 4, &px[..8], 4), Err(FrameError::Short { plane: "Y", len: 15, needed: 16 }));
    // a frame with no area is valid and empty
    assert_eq!(nv(0, 4, &[], 0, &[], 0), Ok(()));
    assert_eq!(s.fill(&Frame::Rgba8 { width: 3, height: 0, stride: 0, data: &[] }), Ok(()));
    assert_eq!(s.fill(&Frame::Rgba8 { width: usize::MAX, height: 0, stride: 0, data: &[] }), Ok(()));
    assert_eq!(nv(usize::MAX, 0, &[], 0, &[], 0), Ok(()));
    assert!(s.is_empty());
    assert!(FrameError::TooLarge.to_string().contains("too large"));
}

#[test]
fn colorized_vectorscope_points_use_the_scope_matrix() {
    let s = Signal::from_fn(1, 1, |_, _| [0.0, 1.0, 0.0]);
    let p = Params { matrix: Matrix::Bt2020Ncl, ..Params::default() };
    let v = vectorscope_yuv(&s, &p);
    assert_eq!(v.matrix, Matrix::Bt2020Ncl);
    let [_, cb, cr] = rgb_to_ycbcr(0.0, 1.0, 0.0, Matrix::Bt2020Ncl);
    let (x, y) = v.cell(cb, cr).unwrap();
    let (n, _, img) = paint::vectorscope(&v, [1.0; 3], 1.0, true);
    let px = &img[(y * n + x) * 4..(y * n + x) * 4 + 4];
    let [ccb, ccr] = v.point(x, y);
    let tint = |m| craft_color::ycbcr_to_rgb(0.6, ccb * 0.8, ccr * 0.8, m).map(|c: f32| 0.55 + 0.45 * c.clamp(0.0, 1.0));
    let l = f32::from(px[3]) / 255.0;
    let got = [0, 1, 2].map(|i| f32::from(px[i]) / 255.0 / l);
    let (want, other) = (tint(Matrix::Bt2020Ncl), tint(Matrix::Bt709));
    let err = |t: [f32; 3]| (0..3).map(|i| (got[i] - t[i]).abs()).fold(0.0f32, f32::max);
    assert!(err(want) < 0.005, "{got:?} vs {want:?}");
    assert!(err(other) > 0.01, "the BT.709 tint {other:?} is not told apart from {want:?}");
    // HLS points are colored by their hue: saturated yellow paints yellow, not orange
    let yellow = Signal::from_fn(1, 1, |_, _| [1.0, 1.0, 0.0]);
    let v = vectorscope_hls(&yellow, &Params::default());
    assert!(v.hls);
    let [hx, hy] = hls_point([1.0, 1.0, 0.0], Matrix::Bt709);
    let (x, y) = v.cell(hx, hy).unwrap();
    let (n, _, img) = paint::vectorscope(&v, [1.0; 3], 1.0, true);
    let px = &img[(y * n + x) * 4..(y * n + x) * 4 + 3];
    assert!(px[0].abs_diff(px[1]) <= 6 && px[2] + 40 < px[1], "{px:?}");
    // HLS peaks report the hue: yellow at 60°, green at 120°
    assert!((summary::peaks(&v, 1, 0.0)[0].angle_deg - 60.0).abs() < 1.0, "{:?}", summary::peaks(&v, 1, 0.0));
    let green = vectorscope_hls(&Signal::from_fn(1, 1, |_, _| [0.0, 1.0, 0.0]), &Params { matrix: Matrix::Bt601, ..Params::default() });
    assert!((summary::peaks(&green, 1, 0.0)[0].angle_deg - 120.0).abs() < 1.0, "{:?}", summary::peaks(&green, 1, 0.0));
}

// ------------------------------------------------------------------------------------ reuse

#[test]
fn into_functions_match_the_allocating_ones_and_keep_their_buffers() {
    let frame_px = rgba8(1920, 1080, |x, y| [(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8]);
    let frame = Frame::Rgba8 { width: 1920, height: 1080, stride: 1920 * 4, data: &frame_px };
    let p = Params::default();
    let mut s = Signal::default();
    let mut w = Waveform::default();
    let mut h = Histogram::default();
    let mut v = Vectorscope::default();
    let mut scratch = paint::Scratch::default();
    let mut img = Vec::new();
    let mut c = Computed::default();
    let mut run = |s: &mut Signal| {
        s.fill(&frame).unwrap();
        waveform_into(s, WaveformType::Yc, &p, &mut w);
        histogram_into(s, &p, &mut h);
        vectorscope_hls_into(s, &p, &mut v);
        let layers: Vec<(&Grid, [f32; 3])> = w.traces.iter().map(|t| (t, [1.0; 3])).collect();
        paint::grids_into(&layers, 10.0, 1.0, &mut scratch, &mut img);
        compute_into(ScopeKind::Parade, s, &Params { parade: ParadeType::RgbWhite, ..p }, &mut c);
        (s.rgb.as_ptr(), w.traces[1].data.as_ptr(), h.y.as_ptr(), v.data.as_ptr(), img.as_ptr(), w.clone(), h.clone(), v.clone(), c.clone())
    };
    let first = run(&mut s);
    let second = run(&mut s);
    assert_eq!((first.0, first.1, first.2, first.3, first.4), (second.0, second.1, second.2, second.3, second.4), "buffers were reallocated");
    assert_eq!(second.5, waveform(&s, WaveformType::Yc, &p));
    assert_eq!(second.6, histogram(&s, &p));
    assert_eq!(second.7, vectorscope_hls(&s, &p));
    assert_eq!(second.8, compute(ScopeKind::Parade, &s, &Params { parade: ParadeType::RgbWhite, ..p }));
    // switching kinds keeps working and gives the same result as a fresh compute
    let mut c = Computed::default();
    let lp = Params { waveform: WaveformType::Luma, parade: ParadeType::Yuv, ..p };
    for k in [ScopeKind::Parade, ScopeKind::Histogram, ScopeKind::VectorscopeYuv, ScopeKind::VectorscopeHls, ScopeKind::Waveform, ScopeKind::Parade] {
        compute_into(k, &s, &lp, &mut c);
        assert_eq!(c, compute(k, &s, &lp), "{k:?}");
    }
}

// ------------------------------------------------------------------------------------ hostile input

#[test]
fn hostile_params_and_signals_stay_bounded() {
    let s = bars();
    // rows and grid sizes are clamped
    let w = waveform(&s, WaveformType::Rgb, &Params { rows: usize::MAX, ..Params::default() });
    assert_eq!(w.traces[0].rows, MAX_ROWS);
    let w = waveform(&s, WaveformType::Rgb, &Params { rows: 0, ..Params::default() });
    assert_eq!(w.traces[0].rows, 2);
    let v = vectorscope_yuv(&s, &Params { vector_size: usize::MAX, ..Params::default() });
    assert_eq!(v.size, MAX_VECTOR_SIZE);
    let v = vectorscope_hls(&s, &Params { vector_size: 0, ..Params::default() });
    assert_eq!((v.size, v.samples), (3, 256 * 16));
    // an even size becomes odd, so a gray sample lands on the center cell with no magnitude
    let gray = Signal::from_fn(1, 1, |_, _| [0.5; 3]);
    let v = vectorscope_yuv(&gray, &Params { vector_size: 256, ..Params::default() });
    assert_eq!(v.size, 257);
    assert_eq!(v.cell(0.0, 0.0), Some((128, 128)));
    assert_eq!(v.point(128, 128), [0.0, 0.0]);
    // a signal whose public fields disagree reads only whole rows and never indexes past them
    let odd = Signal { w: 1000, h: 1000, rgb: vec![[0.5; 3]; 10] };
    let w = waveform(&odd, WaveformType::Rgb, &Params::default());
    assert_eq!((w.traces[0].cols, w.per_column, w.traces[0].total()), (10, 0, 0));
    // sizes asked for past the caps are clamped, and samples past a clamped grid are dropped
    let px = rgba8(2000, 4, |_, _| [9, 9, 9]);
    let mut big = Signal::default();
    big.fill_max(&Frame::Rgba8 { width: 2000, height: 4, stride: 8000, data: &px }, usize::MAX, usize::MAX).unwrap();
    assert_eq!((big.w, big.h), (1000, 4), "2000 columns decimated by 2 to fit 1920");
    let f = Signal::from_rgba_f32_with(4000, 1, &vec![0.5; 16_000], usize::MAX, usize::MAX, |p| [p[0], p[1], p[2]]);
    assert_eq!(f.w, 1334, "4000 columns decimated by 3");
    let g = Grid::new("Y", usize::MAX, usize::MAX);
    assert_eq!((g.cols, g.rows, g.data.len()), (MAX_SIGNAL_W, MAX_ROWS, MAX_SIGNAL_W * MAX_ROWS));
    let wide = Signal::from_fn(3000, 2, |_, _| [0.5; 3]);
    let w = waveform(&wide, WaveformType::Luma, &Params::default());
    assert_eq!((w.traces[0].cols, w.traces[0].total()), (MAX_SIGNAL_W, 2 * MAX_SIGNAL_W as u64));
    let wide = Signal { w: usize::MAX, h: 0, rgb: Vec::new() };
    assert_eq!(waveform(&wide, WaveformType::Luma, &Params::default()).traces[0].data.len(), 0);
    // helpers outside their range answer None or 0 instead of panicking
    assert_eq!(row_of(0.5, 0.0, 1.0, 1), None);
    assert_eq!(row_of(0.5, 1.0, 1.0, 256), None);
    assert_eq!(cell_of(0.0, 0.0, 1, VECTOR_EXTENT), None);
    assert_eq!(cell_of(0.0, 0.0, 255, f32::NAN), None);
    let g = Grid::new("Y", 3, 3);
    assert_eq!((g.at(3, 0), g.at(0, 3), g.at(0, usize::MAX), g.at(usize::MAX, usize::MAX)), (0, 0, 0, 0));
    let v = vectorscope_yuv(&s, &Params::default());
    assert_eq!((v.at(0, usize::MAX), v.at(usize::MAX, 0)), (0, 0));
    let empty = Vectorscope::default();
    assert!(summary::coarse(&empty, 0).is_empty());
    assert!(summary::peaks(&empty, 5, 0.0).is_empty());
    assert_eq!(empty.at(0, 0), 0);
}

#[test]
fn hand_built_values_past_the_caps_paint_nothing() {
    let huge = Signal::from_fn(usize::MAX, 2, |_, _| [0.5; 3]);
    assert_eq!((huge.w, huge.h, huge.len()), (MAX_SIGNAL_W, 2, 2 * MAX_SIGNAL_W));
    let mut scratch = paint::Scratch::default();
    let mut out = vec![1u8; 4];
    let v = Vectorscope { size: usize::MAX, ..Vectorscope::default() };
    assert_eq!(paint::vectorscope_into(&v, [1.0; 3], 1.0, true, &mut scratch, &mut out), (0, 0));
    assert!(out.is_empty());
    assert!(summary::coarse(&v, usize::MAX).is_empty());
    let big = Vectorscope { size: MAX_VECTOR_SIZE + 1, data: vec![1; 4], samples: 4, ..Vectorscope::default() };
    assert!(summary::peaks(&big, 5, 0.0).is_empty());
    assert!(summary::coarse(&big, 4).is_empty());
    // entries past the size² plane are not cells
    let mut data = vec![0; 10];
    data[9] = 5;
    let trailing = Vectorscope { size: 3, extent: VECTOR_EXTENT, data, samples: 5, ..Vectorscope::default() };
    assert!(summary::peaks(&trailing, 5, 0.0).is_empty());
    assert!(summary::coarse(&trailing, 3).is_empty());
    let g = Grid { name: "Y", cols: usize::MAX, rows: usize::MAX, data: Vec::new() };
    out.push(1);
    assert_eq!(paint::grids_into(&[(&g, [1.0; 3])], 1.0, 1.0, &mut scratch, &mut out), (0, 0));
    assert!(out.is_empty());
    assert!(summary::trace_columns(&Waveform::default(), &g, usize::MAX).is_empty());
    let tall = Grid { name: "Y", cols: 4, rows: MAX_ROWS + 1, data: Vec::new() };
    assert_eq!(paint::grids_into(&[(&tall, [1.0; 3])], 1.0, 1.0, &mut scratch, &mut out), (0, 0));
}

// ------------------------------------------------------------------------------------ HDR

#[test]
fn hdr_bins_split_sdr_and_stops() {
    let px = Signal { w: 5, h: 1, rgb: vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [4.0, 4.0, 4.0], [16.0, 16.0, 16.0], [f32::NAN, -1.0, f32::INFINITY]] };
    let mut h = Histogram::default();
    histogram_hdr_into(&px, Matrix::Bt709, HdrBins::new(4.0), &mut h);
    let b = h.hdr.unwrap();
    assert_eq!(h.samples, 5);
    assert_eq!(b.bin(1.0), b.from - 1, "SDR white is the last SDR bin");
    assert_eq!(b.bin(16.0), 255);
    assert_eq!(b.bin(1e9), 255);
    let two = b.bin(4.0);
    assert!(two > b.from && two < 255, "{two}");
    assert_eq!(h.r[0], 2, "black and NaN");
    assert_eq!(h.b[255], 2, "16 and +inf");
    assert_eq!((h.below[1], h.above[2]), (1, 1), "-1 below, +inf above 2^4");
    assert!((above_sdr(&h) - 0.6).abs() < 1e-6, "{}", above_sdr(&h));
    // over-white samples in different channels each count once
    let split = Signal { w: 2, h: 1, rgb: vec![[2.0, 0.5, 0.5], [0.5, 0.5, 2.0]] };
    histogram_hdr_into(&split, Matrix::Bt709, HdrBins::new(4.0), &mut h);
    assert_eq!((h.above_sdr_samples, above_sdr(&h)), (2, 1.0));
    histogram_hdr_into(&Signal::default(), Matrix::Bt709, HdrBins::new(4.0), &mut h);
    assert_eq!(above_sdr(&h), 0.0);
    assert_eq!(above_sdr(&histogram(&bars(), &Params::default())), 0.0);
    // the SDR half follows the sRGB curve: linear 0.5 is about 73.5 % of the way to SDR white
    let half = b.bin(0.5);
    assert_eq!(half, (craft_color::linear_to_srgb(0.5) * 127.0 + 0.5) as usize);
}

/// A hand-built split outside the bins (any `from`, any `stops`) never panics and stays in range.
#[test]
fn hdr_bins_with_any_split_stay_in_range() {
    for from in [0, 1, 2, 128, 255, 256, usize::MAX] {
        for stops in [0.0, 3.0, f32::NAN, f32::INFINITY] {
            let b = HdrBins { from, stops };
            for v in [f32::NAN, -1.0, 0.0, 0.5, 1.0, 2.0, 1e9] {
                assert!(b.bin(v) < BINS, "{from} {stops} {v}");
            }
            let mut h = Histogram::default();
            histogram_hdr_into(&Signal { w: 1, h: 1, rgb: vec![[2.0; 3]] }, Matrix::Bt2020Ncl, b, &mut h);
            assert!((0.0..=1.0).contains(&above_sdr(&h)));
        }
    }
}

/// `cargo test --release -p craft-scopes perf -- --ignored --nocapture`
#[test]
#[ignore]
fn perf_scopes_at_1080p() {
    let mut seed = 0x1234_5678u32;
    let px = rgba8(1920, 1080, |_, _| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        [seed as u8, (seed >> 8) as u8, (seed >> 16) as u8]
    });
    let reps = 20;
    let mut s = Signal::default();
    let frame = Frame::Rgba8 { width: 1920, height: 1080, stride: 1920 * 4, data: &px };
    let t = std::time::Instant::now();
    for _ in 0..reps {
        s.fill(&frame).unwrap();
    }
    println!("decimate RGBA 1920x1080: {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / reps as f64);
    let (y, y_stride, uv, uv_stride) = nv12(1920, 1080, 0, [100, 120, 140]);
    let nv = Frame::Nv12 { width: 1920, height: 1080, y: &y, y_stride, uv: &uv, uv_stride, matrix: Matrix::Bt709, range: Range::Limited };
    let t = std::time::Instant::now();
    for _ in 0..reps {
        s.fill(&nv).unwrap();
    }
    println!("decimate NV12 1920x1080: {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / reps as f64);
    s.fill(&frame).unwrap();
    let p = Params::default();
    let mut c = Computed::default();
    for (k, wt, pt) in [
        (ScopeKind::Waveform, WaveformType::Rgb, ParadeType::Rgb),
        (ScopeKind::Waveform, WaveformType::Yc, ParadeType::Rgb),
        (ScopeKind::Parade, WaveformType::Rgb, ParadeType::RgbWhite),
        (ScopeKind::Histogram, WaveformType::Rgb, ParadeType::Rgb),
        (ScopeKind::VectorscopeYuv, WaveformType::Rgb, ParadeType::Rgb),
        (ScopeKind::VectorscopeHls, WaveformType::Rgb, ParadeType::Rgb),
    ] {
        let t = std::time::Instant::now();
        for _ in 0..reps {
            compute_into(k, &s, &Params { waveform: wt, parade: pt, ..p }, &mut c);
            std::hint::black_box(&c);
        }
        println!("{k:?} {wt:?}/{pt:?}: {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / reps as f64);
    }
}
