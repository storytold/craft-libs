use super::*;

pub(crate) struct ListWriter(pub Vec<u8>, pub u32);

impl ListWriter {
    pub fn new() -> Self {
        ListWriter(Vec::new(), 0)
    }
    pub fn op(&mut self, id: u32, params: &[u8]) -> &mut Self {
        for v in [id, 0x01030000, 0, params.len() as u32] {
            self.0.extend_from_slice(&v.to_be_bytes());
        }
        self.0.extend_from_slice(params);
        self.1 += 1;
        self
    }
    pub fn finish(&self) -> Vec<u8> {
        let mut v = self.1.to_be_bytes().to_vec();
        v.extend_from_slice(&self.0);
        v
    }
}

pub(crate) fn be(vals: &[f64]) -> Vec<u8> {
    vals.iter().flat_map(|v| v.to_be_bytes()).collect()
}
pub(crate) fn area(a: [u32; 8]) -> Vec<u8> {
    a.iter().flat_map(|v| v.to_be_bytes()).collect()
}

/// Warp and bad-pixel records each cost a full-image pass, and a hostile `OpcodeList3` repeats them
/// thousands of times ([`MAX_LIST3_PASSES`] bounds the multiplier); identity warps below the cap must
/// still apply, exactly, so the same image comes out.
#[test]
fn list3_pass_cap_skips_repeated_records() {
    let mut img = Rgb32f::new(8, 6);
    for (i, px) in img.data.iter_mut().enumerate() {
        *px = [i as f32 / 2.0, (i % 8) as f32, (i / 8) as f32];
    }
    let orig: Vec<[f32; 3]> = img.data.clone();
    let list: Vec<Opcode> =
        (0..MAX_LIST3_PASSES * 5).map(|_| Opcode::WarpRectilinear { planes: vec![[1.0, 0.0, 0.0, 0.0, 0.0, 0.0]], center: [0.5, 0.5] }).collect();
    apply_list3(&list, &mut img);
    assert_eq!(img.data, orig, "identity warps change nothing, and the cap skips the repeated rest");

    // a real warp below the cap is still applied
    let one = [Opcode::WarpRectilinear { planes: vec![[2.0, 0.0, 0.0, 0.0, 0.0, 0.0]], center: [0.5, 0.5] }];
    apply_list3(&one, &mut img);
    assert_ne!(img.data, orig, "a genuine warp record is not skipped");
}

#[test]
fn parse_and_apply_area_ops() {
    let (w, h) = (4usize, 3usize);
    let mut gm = area([0, 0, 3, 4, 0, 1, 1, 1]);
    for v in [2u32, 2] {
        gm.extend_from_slice(&v.to_be_bytes());
    }
    gm.extend(be(&[1.0, 1.0, 0.0, 0.0]));
    gm.extend_from_slice(&1u32.to_be_bytes());
    for g in [1.0f32, 2.0, 1.0, 2.0] {
        gm.extend_from_slice(&g.to_be_bytes());
    }
    let mut dpr = area([0, 0, 3, 4, 0, 1, 1, 1]);
    dpr.extend_from_slice(&3u32.to_be_bytes());
    for d in [0.0f32, 0.1, 0.2] {
        dpr.extend_from_slice(&d.to_be_bytes());
    }
    let mut poly = area([0, 0, 3, 4, 0, 1, 1, 2]);
    poly.extend_from_slice(&1u32.to_be_bytes());
    poly.extend(be(&[0.0, 2.0]));
    let blob = ListWriter::new().op(9, &gm).op(10, &dpr).op(8, &poly).op(99, &[1, 2, 3]).finish();
    let list = parse_list(&blob);
    assert_eq!(list.len(), 4);
    assert!(matches!(list[3], Opcode::Unknown { id: 99, .. }));
    assert!(list[..3].iter().all(is_applied));
    let mut buf = vec![0.25f32; w * h];
    apply_list(&list, &mut buf, w, h, 1, None, 1.0);
    // gain map: horizontal ramp 1 → 2 across relative x 0..1 (x / w)
    let expect = |x: usize, y: usize| {
        let g = 1.0 + x as f32 / w as f32;
        let v = 0.25 * g + [0.0, 0.1, 0.2][y];
        if x.is_multiple_of(2) { v * 2.0 } else { v }
    };
    for y in 0..h {
        for x in 0..w {
            assert!((buf[y * w + x] - expect(x, y)).abs() < 1e-6, "{x},{y}: {} vs {}", buf[y * w + x], expect(x, y));
        }
    }
}

#[test]
fn map_table_raw_scale_and_bad_pixels() {
    let mut t = area([0, 0, 2, 2, 0, 1, 1, 1]);
    t.extend_from_slice(&3u32.to_be_bytes());
    for v in [0u16, 30000, 65535] {
        t.extend_from_slice(&v.to_be_bytes());
    }
    let list = parse_list(&ListWriter::new().op(7, &t).finish());
    let mut buf = vec![0.0, 1.0 / 65535.0, 2.0 / 65535.0, 0.5];
    apply_list(&list, &mut buf, 2, 2, 1, None, 1.0);
    assert_eq!(buf[0], 0.0);
    assert!((buf[1] - 30000.0 / 65535.0).abs() < 1e-7);
    assert_eq!(buf[3], 1.0);

    let mut p = vec![];
    for v in [7u32, 0] {
        p.extend_from_slice(&v.to_be_bytes());
    }
    let list = parse_list(&ListWriter::new().op(4, &p).finish());
    let cfa = Cfa::bayer("RGGB").unwrap();
    let mut img: Vec<f32> = (0..36).map(|i| 100.0 + (i % 6) as f32).collect();
    img[2 * 6 + 2] = 7.0;
    apply_list(&list, &mut img, 6, 6, 1, Some(&cfa), 65535.0);
    assert_eq!(img[2 * 6 + 2], 102.0);
}

/// A mosaic equal to the bad-pixel constant, or one rectangle over the whole image, used to list every pixel as
/// bad: w × h entries (plus a set of them), tens of GB at the decoder's size limit. Here the rows between a good
/// first row (500) and a good last row (900) are bad: 8 rows of 64 pixels, four rows more than the cap of 256 the
/// test passes. Row 1, within the cap, is repaired from the first row; the second-last row, past the cap, must be
/// left as it is (uncapped, it was rewritten from the last row).
fn assert_bad_pixels_capped(op: Opcode) {
    let (w, h) = (64, 10);
    let mut buf = vec![0.0f32; w * h];
    buf[..w].fill(500.0);
    buf[(h - 1) * w..].fill(900.0);
    apply_list_capped(&[op], &mut buf, w, h, 1, None, 65535.0, 256);
    assert!(buf[w..2 * w].iter().all(|&v| v == 500.0), "row 1 (within the cap) not repaired: {:?}", &buf[w..w + 4]);
    let past = &buf[(h - 2) * w..(h - 1) * w];
    assert!(past.iter().all(|&v| v == 0.0), "a row past the cap was rewritten: {:?}", &past[..4]);
}

#[test]
fn bad_pixels_constant_is_capped() {
    assert_bad_pixels_capped(Opcode::FixBadPixelsConstant { constant: 0, bayer_phase: 0 });
}

#[test]
fn bad_pixels_list_is_capped_inside_a_rectangle() {
    // top, left, bottom, right: every row but the first and the last
    assert_bad_pixels_capped(Opcode::FixBadPixelsList { bayer_phase: 0, points: vec![], rects: vec![[1, 0, 9, 64]] });
}

#[test]
fn vignette_and_warp_identity() {
    let mut img = Rgb32f::from_fn(9, 7, |x, y| [x as f32 / 9.0, y as f32 / 7.0, 0.5]);
    let orig = img.clone();
    let mut p = be(&[0.0; 5]);
    p.extend(be(&[0.5, 0.5]));
    let mut warp = 1u32.to_be_bytes().to_vec();
    warp.extend(be(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5]));
    let list = parse_list(&ListWriter::new().op(3, &p).op(1, &warp).finish());
    assert_eq!(list.len(), 2);
    apply_list3(&list, &mut img);
    for (a, b) in img.data.iter().zip(&orig.data) {
        for c in 0..3 {
            assert!((a[c] - b[c]).abs() < 1e-5);
        }
    }
    // positive k0 brightens the corners more than the centre
    let mut p = be(&[1.0, 0.0, 0.0, 0.0, 0.0]);
    p.extend(be(&[0.5, 0.5]));
    let list = parse_list(&ListWriter::new().op(3, &p).finish());
    let mut flat = Rgb32f::filled(9, 9, [0.5; 3]);
    apply_list3(&list, &mut flat);
    assert!(flat.get(0, 0)[0] > 0.85 && (flat.get(4, 4)[0] - 0.5).abs() < 0.01, "{:?}", flat.get(0, 0));
}

#[test]
fn warp_barrel_moves_pixels_outward() {
    let img = Rgb32f::from_fn(31, 31, |x, y| if x == 24 && y == 15 { [1.0; 3] } else { [0.0; 3] });
    // source radius = r (1 + k r²) with k < 0 → content moves outward
    let out = warp_rectilinear(&img, &[[1.0, -0.6, 0.0, 0.0, 0.0, 0.0]], [0.5, 0.5]);
    let (bx, _) = (0..31).map(|x| (x, out.get(x, 15)[0])).fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
    assert!(bx >= 25, "{bx}");
}

#[test]
fn write_parse_roundtrip() {
    let a = Area { top: 1, left: 2, bottom: 30, right: 40, plane: 0, planes: 3, row_pitch: 1, col_pitch: 2 };
    let list = vec![
        Opcode::WarpRectilinear { planes: vec![[1.0, 0.01, -0.002, 0.0, 1e-4, -2e-4]; 3], center: [0.5, 0.49] },
        Opcode::WarpFisheye { planes: vec![[1.0, 0.1, 0.0, 0.0]], center: [0.5, 0.5] },
        Opcode::FixVignetteRadial { k: [0.1, 0.2, 0.0, 0.0, 0.0], center: [0.5, 0.5] },
        Opcode::FixBadPixelsConstant { constant: 0, bayer_phase: 1 },
        Opcode::FixBadPixelsList { bayer_phase: 0, points: vec![(3, 4)], rects: vec![[1, 1, 2, 2]] },
        Opcode::TrimBounds { top: 1, left: 1, bottom: 9, right: 9 },
        Opcode::MapTable { area: a, table: vec![0, 100, 65535] },
        Opcode::MapPolynomial { area: a, coefficients: vec![0.0, 1.0, -0.1] },
        Opcode::GainMap { area: a, points_v: 2, points_h: 1, spacing: [1.0, 1.0], origin: [0.0, 0.0], map_planes: 1, gains: vec![1.0, 1.5] },
        Opcode::DeltaPerRow { area: a, deltas: vec![0.1, 0.2] },
        Opcode::DeltaPerColumn { area: a, deltas: vec![0.3] },
        Opcode::ScalePerRow { area: a, scales: vec![1.1] },
        Opcode::ScalePerColumn { area: a, scales: vec![0.9, 1.0] },
        Opcode::WarpRectilinear2 {
            planes: vec![wr2_set(&[(0, 0.5), (3, 0.1)]), wr2_set(&[(0, 0.25)])],
            center: [0.5, 0.49],
            reciprocal: true,
            optional: true,
        },
        Opcode::Unknown { id: 77, flags: 1, params: vec![1, 2, 3] },
    ];
    assert_eq!(parse_list(&write_list(&list)), list);
}

#[test]
fn truncated_lists_do_not_panic() {
    let mut gm = area([0, 0, 3, 4, 0, 1, 1, 1]);
    gm.extend_from_slice(&[0xff; 64]);
    let blob = ListWriter::new().op(9, &gm).op(5, &[0xff; 12]).op(1, &[0, 0, 0, 9]).finish();
    for n in 0..blob.len() {
        let l = parse_list(&blob[..n]);
        let mut buf = vec![0.5f32; 12];
        apply_list(&l, &mut buf, 4, 3, 1, None, 1.0);
    }
}

/// `WarpRectilinear2` parameter bytes (spec 1.6.0.0: N, then per set kr0..kr14, kt0, kt1, min/max valid radius,
/// then the centre and `reciprocalRadial`).
fn wr2_params(sets: &[[f64; WR2_LEN]], center: [f64; 2], reciprocal: u32) -> Vec<u8> {
    let mut p = (sets.len() as u32).to_be_bytes().to_vec();
    for s in sets {
        p.extend(be(s));
    }
    p.extend(be(&center));
    p.extend_from_slice(&reciprocal.to_be_bytes());
    p
}

fn wr2_set(kr: &[(usize, f64)]) -> [f64; WR2_LEN] {
    let mut k = [0.0; WR2_LEN];
    for &(i, v) in kr {
        k[i] = v;
    }
    k[18] = 1.0;
    k
}

#[test]
fn warp_rectilinear2_parses_the_spec_layout() {
    let mut k = wr2_set(&[(0, 1.0), (1, 0.25), (14, -2.0)]);
    k[15] = 0.001;
    k[16] = -0.002;
    k[17] = 0.1;
    k[18] = 0.9;
    let blob = ListWriter::new().op(14, &wr2_params(&[k, wr2_set(&[(0, 1.0)])], [0.5, 0.25], 1)).finish();
    let list = parse_list(&blob);
    assert_eq!(
        list,
        vec![Opcode::WarpRectilinear2 { planes: vec![k, wr2_set(&[(0, 1.0)])], center: [0.5, 0.25], reciprocal: true, optional: false }]
    );
    assert!(is_applied(&list[0]) && list[0].is_lens_correction());
    // written back as opcode 14, version 1.6.0.0, optional flag kept
    let again = vec![Opcode::WarpRectilinear2 { planes: vec![k], center: [0.5, 0.5], reciprocal: false, optional: true }];
    let bytes = write_list(&again);
    assert_eq!(bytes.get(4..8), Some(&14u32.to_be_bytes()[..]));
    assert_eq!(bytes.get(8..12), Some(&0x0106_0000u32.to_be_bytes()[..]));
    assert_eq!(parse_list(&bytes), again);
}

#[test]
fn warp_rectilinear2_identity_leaves_the_image() {
    let img = Rgb32f::from_fn(21, 15, |x, y| [x as f32 / 21.0, y as f32 / 15.0, 0.25]);
    for reciprocal in [false, true] {
        let mut i = img.clone();
        let p = wr2_params(&[wr2_set(&[(0, 1.0)])], [0.5, 0.5], reciprocal as u32);
        apply_list3(&parse_list(&ListWriter::new().op(14, &p).finish()), &mut i);
        for (a, b) in i.data.iter().zip(&img.data) {
            for c in 0..3 {
                assert!((a[c] - b[c]).abs() < 1e-5, "{a:?} {b:?}");
            }
        }
    }
}

#[test]
fn warp_rectilinear2_matches_warp_rectilinear_on_even_terms() {
    // kr0 + kr2 r^2 + kr4 r^4 is WarpRectilinear's kr0 + kr1 r^2 + kr2 r^4
    let img = Rgb32f::from_fn(31, 31, |x, y| [(x * 7 % 13) as f32 / 13.0, (y * 5 % 11) as f32 / 11.0, ((x + y) % 4) as f32 / 4.0]);
    let a = warp_rectilinear(&img, &[[1.0, -0.3, 0.1, 0.0, 0.01, -0.02]], [0.5, 0.5]);
    let mut k = wr2_set(&[(0, 1.0), (2, -0.3), (4, 0.1)]);
    k[15] = 0.01;
    k[16] = -0.02;
    let b = warp_rectilinear2(&img, &[k], [0.5, 0.5], false);
    for (p, q) in a.data.iter().zip(&b.data) {
        for c in 0..3 {
            assert!((p[c] - q[c]).abs() < 1e-5);
        }
    }
    // odd terms, the reciprocal and the valid-radius clamp give the factor the spec defines
    let k = wr2_set(&[(0, 1.0), (1, 0.5)]);
    assert_eq!(wr2_factor(&k, false, 0.5), Some(1.25));
    assert_eq!(wr2_factor(&k, true, 0.5), Some(0.8));
    let mut k = k;
    k[18] = 0.5; // max_valid_radius
    assert_eq!(wr2_factor(&k, false, 0.9), Some(1.25));
    k[17] = 0.25; // min_valid_radius
    assert_eq!(wr2_factor(&k, false, 0.0), Some(1.125));
    assert_eq!(wr2_factor(&wr2_set(&[]), true, 0.3), None, "1 / 0");
}

#[test]
fn malformed_warp_rectilinear2_is_unknown_not_a_panic() {
    let good = wr2_params(&[wr2_set(&[(0, 1.0)])], [0.5, 0.5], 0);
    let is_wr2 = |p: &[u8]| matches!(parse_list(&ListWriter::new().op(14, p).finish()).as_slice(), [Opcode::WarpRectilinear2 { .. }]);
    assert!(is_wr2(&good));
    // every truncation and any trailing byte
    for n in 0..good.len() {
        assert!(!is_wr2(&good[..n]), "{n}");
    }
    assert!(!is_wr2(&[&good[..], &[0u8][..]].concat()));
    // N = 0, N too big for the data, N beyond the plane limit
    for n in [0u32, 2, 5, u32::MAX] {
        let mut p = good.clone();
        p[..4].copy_from_slice(&n.to_be_bytes());
        assert!(!is_wr2(&p), "N = {n}");
    }
    // non-finite or absurd values, in a coefficient, the valid range and the centre
    let patch = |at: usize, v: f64| {
        let mut p = good.clone();
        p[at..at + 8].copy_from_slice(&v.to_be_bytes());
        p
    };
    for (at, v) in [
        (4, f64::NAN),
        (4 + 8, f64::INFINITY),
        (4 + 8 * 3, 1e300),
        (4 + 8 * 17, -0.5),
        (4 + 8 * 18, 1.5),
        (4 + 8 * 17, 1.0),
        (4 + 19 * 8, f64::NAN),
        (4 + 19 * 8 + 8, 1e30),
    ] {
        assert!(!is_wr2(&patch(at, v)), "{at} {v}");
    }
    // refused ones stay Unknown and are not applied
    let list = parse_list(&ListWriter::new().op(14, &good[..10]).finish());
    assert!(matches!(list.as_slice(), [Opcode::Unknown { id: 14, .. }]));
    let mut img = Rgb32f::filled(5, 5, [0.5; 3]);
    apply_list3(&list, &mut img);
    assert_eq!(img.get(2, 2), [0.5; 3]);
    // a hostile but accepted set (huge radial terms) still gives a finite image
    let k = wr2_set(&[(0, 1.0), (14, 1e9), (13, -1e9)]);
    let img = Rgb32f::from_fn(9, 9, |x, y| [x as f32 / 9.0, y as f32 / 9.0, 0.5]);
    for reciprocal in [false, true] {
        let out = warp_rectilinear2(&img, &[k], [0.5, 0.5], reciprocal);
        assert!(out.data.iter().all(|p| p.iter().all(|v| v.is_finite())));
    }
}

#[test]
fn warp_list_order_and_the_optional_skip_rule() {
    let wr2 = |optional| Opcode::WarpRectilinear2 { planes: vec![wr2_set(&[(0, 1.0)])], center: [0.5, 0.5], reciprocal: false, optional };
    let wr = Opcode::WarpRectilinear { planes: vec![[1.0, 0.0, 0.0, 0.0, 0.0, 0.0]], center: [0.5, 0.5] };
    let fish = Opcode::WarpFisheye { planes: vec![[1.0, 0.0, 0.0, 0.0]], center: [0.5, 0.5] };
    let vig = Opcode::FixVignetteRadial { k: [0.0; 5], center: [0.5, 0.5] };
    let d = std::mem::discriminant::<Opcode>;
    let ids = |l: &[Opcode]| applied_ops(l).map(d).collect::<Vec<_>>();
    // WarpRectilinear2 (optional) then WarpRectilinear / WarpFisheye: the second is skipped
    assert_eq!(ids(&[wr2(true), wr.clone()]), vec![d(&wr2(true))]);
    assert_eq!(ids(&[wr2(true), fish.clone()]), vec![d(&wr2(true))]);
    // not optional: both run, in list order
    assert_eq!(ids(&[wr2(false), wr.clone()]), vec![d(&wr2(false)), d(&wr)]);
    // only the opcode immediately after is skipped
    assert_eq!(ids(&[wr2(true), vig.clone(), wr.clone()]), vec![d(&wr2(true)), d(&vig), d(&wr)]);
    // two in a row: both run, and the WarpRectilinear after the optional second one is skipped
    assert_eq!(ids(&[wr2(true), wr2(true), wr.clone()]), vec![d(&wr2(true)); 2]);
    // a WarpRectilinear before the WarpRectilinear2 runs, in list order
    assert_eq!(ids(&[wr.clone(), wr2(true)]), vec![d(&wr), d(&wr2(true))]);
    // the same from bytes: an optional (flag 1) opcode 14 followed by opcode 1
    let w2 = wr2_params(&[wr2_set(&[(0, 1.0)])], [0.5, 0.5], 0);
    let mut one = 1u32.to_be_bytes().to_vec();
    one.extend(be(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5]));
    let mut blob = 2u32.to_be_bytes().to_vec();
    for (id, flags, p) in [(14u32, 1u32, &w2), (1, 1, &one)] {
        for v in [id, 0x0106_0000, flags, p.len() as u32] {
            blob.extend_from_slice(&v.to_be_bytes());
        }
        blob.extend_from_slice(p);
    }
    let list = parse_list(&blob);
    assert_eq!(list.len(), 2);
    assert_eq!(applied_ops(&list).count(), 1);
    // a refused (Unknown) opcode 14 triggers no skip
    let bad = Opcode::Unknown { id: 14, flags: 1, params: vec![] };
    assert!(applied_ops(&[bad, wr]).any(|o| matches!(o, Opcode::WarpRectilinear { .. })));
}

/// `cropped` re-expresses a lens correction for a crop: the corrected crop is the crop of the corrected whole. A
/// linear ramp makes bilinear sampling exact, so only the borders (where the warp reaches past the crop) differ.
#[test]
fn cropped_lens_corrections_correct_the_crop_like_the_whole() {
    let (w, h) = (90usize, 64usize);
    let crop = super::super::Rect::new(7, 5, 70, 50);
    let ramp = Rgb32f::from_fn(w, h, |x, y| [0.002 * x as f32 + 0.001 * y as f32 + 0.1, 0.3 + 0.0015 * y as f32, 0.2]);
    let mut wr2 = [0.0; WR2_LEN];
    wr2[..5].copy_from_slice(&[1.0, 0.01, -0.03, 0.0, 0.01]);
    wr2[15] = 0.002;
    wr2[16] = -0.001;
    wr2[17] = 0.05;
    wr2[18] = 0.9;
    let warps = [
        Opcode::WarpRectilinear { planes: vec![[0.97, 0.04, -0.01, 0.002, 0.003, -0.002]], center: [0.47, 0.53] },
        Opcode::WarpRectilinear2 { planes: vec![wr2], center: [0.52, 0.46], reciprocal: false, optional: true },
        Opcode::WarpRectilinear2 { planes: vec![wr2], center: [0.52, 0.46], reciprocal: true, optional: false },
    ];
    for op in &warps {
        let mut whole = ramp.clone();
        apply_list3(std::slice::from_ref(op), &mut whole);
        let re = op.cropped(w, h, crop).unwrap();
        let mut part = ramp.clone().into_crop(crop.x, crop.y, crop.width, crop.height);
        apply_list3(std::slice::from_ref(&re), &mut part);
        let mut worst = 0f32;
        for y in 6..crop.height - 6 {
            for x in 6..crop.width - 6 {
                let (a, b) = (whole.data[(y + crop.y) * w + x + crop.x], part.data[y * crop.width + x]);
                worst = (0..3).map(|c| (a[c] - b[c]).abs()).fold(worst, f32::max);
            }
        }
        assert!(worst < 1e-4, "{op:?} → {re:?}: {worst}");
    }
    // the vignette, in the warps' centre convention the develop pipeline reads it with
    let gain = |k: &[f64; 5], c: [f64; 2], w: usize, h: usize, x: f64, y: f64| {
        let (aw, ah) = (w as f64 - 1.0, h as f64 - 1.0);
        let (px, py) = (c[0] * aw, c[1] * ah);
        let r2 = ((x - px).powi(2) + (y - py).powi(2)) / max_radius(px, py, aw, ah).powi(2);
        1.0 + (0..5).map(|i| k[i] * r2.powi(i as i32 + 1)).sum::<f64>()
    };
    let (k, center) = ([-0.3, 0.1, -0.05, 0.02, -0.01], [0.45, 0.55]);
    let Some(Opcode::FixVignetteRadial { k: k2, center: c2 }) = (Opcode::FixVignetteRadial { k, center }).cropped(w, h, crop) else {
        panic!("vignette")
    };
    for (x, y) in [(0.0, 0.0), (69.0, 49.0), (33.5, 20.0), (69.0, 0.0), (12.0, 40.0)] {
        let (a, b) = (gain(&k, center, w, h, x + crop.x as f64, y + crop.y as f64), gain(&k2, c2, crop.width, crop.height, x, y));
        assert!((a - b).abs() < 1e-12, "({x}, {y}): {a} vs {b}");
    }
    // no change for a full-frame "crop"; nothing for other opcodes, tiny images or a crop past the edge
    assert_eq!(warps[0].cropped(w, h, super::super::Rect::new(0, 0, w, h)).unwrap(), warps[0]);
    assert!(Opcode::TrimBounds { top: 0, left: 0, bottom: 2, right: 2 }.cropped(w, h, crop).is_none());
    assert!(warps[0].cropped(1, h, super::super::Rect::new(0, 0, 1, h)).is_none());
    assert!(warps[0].cropped(w, h, super::super::Rect::new(30, 0, 70, 50)).is_none());
    assert!(warps[0].cropped(w, h, super::super::Rect::new(usize::MAX, 0, 70, 50)).is_none());
}

/// Ten thousand full-image polynomials (a few hundred KB of opcode data): the list is held to
/// `MAX_LIST_PASSES` passes of work, the first ones applied and the rest left out.
#[test]
fn a_list_repeating_full_image_work_stops_at_its_budget() {
    let (w, h) = (64usize, 48usize);
    let a = Area { top: 0, left: 0, bottom: h as u32, right: w as u32, plane: 0, planes: 1, row_pitch: 1, col_pitch: 1 };
    // v + 0.001 each, two coefficients: each costs two passes
    let list = vec![Opcode::MapPolynomial { area: a, coefficients: vec![0.001, 1.0] }; 10_000];
    let mut buf = vec![0.0f32; w * h];
    apply_list(&list, &mut buf, w, h, 1, None, 1.0);
    let applied = (buf[0] / 0.001).round() as usize;
    assert_eq!(applied, MAX_LIST_PASSES / 2, "{}", buf[0]);
    assert!(buf.iter().all(|v| *v == buf[0]));
}

/// Opcodes `apply_list` does nothing for spend none of the budget: a later real one still runs.
#[test]
fn ops_that_apply_nothing_cost_no_budget() {
    let (w, h) = (8usize, 8usize);
    let a = Area { top: 0, left: 0, bottom: h as u32, right: w as u32, plane: 0, planes: 1, row_pitch: 1, col_pitch: 1 };
    let mut list = vec![Opcode::Unknown { id: 9999, flags: 0, params: Vec::new() }; 4 * MAX_LIST_PASSES];
    list.push(Opcode::MapPolynomial { area: a, coefficients: vec![0.5, 1.0] });
    let mut buf = vec![0.0f32; w * h];
    apply_list(&list, &mut buf, w, h, 1, None, 1.0);
    assert_eq!(buf[0], 0.5);
}
