use super::*;

#[test]
fn reject_opcode_counts_whose_bytes_exceed_the_remaining_data() {
    let mut gain_map = Vec::new();
    for value in [u32::MAX, 1] {
        gain_map.extend_from_slice(&value.to_be_bytes());
    }
    gain_map.extend(be(&[1.0, 1.0, 0.0, 0.0]));
    gain_map.extend_from_slice(&1u32.to_be_bytes());
    assert!(parse_gain_map(Area::default(), &mut Rd { d: &gain_map, p: 0 }).is_none());

    let delta_count = u32::MAX.to_be_bytes();
    assert!(parse_delta_scale(10, Area::default(), &mut Rd { d: &delta_count, p: 0 }).is_none());
}

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

#[test]
fn area_ops_parse_and_unknown_ones_are_kept() {
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
    assert!(matches!(list[0], Opcode::GainMap { points_v: 2, points_h: 2, .. }));
    assert!(matches!(&list[1], Opcode::DeltaPerRow { deltas, .. } if deltas.len() == 3));
    assert!(matches!(&list[2], Opcode::MapPolynomial { coefficients, .. } if coefficients == &[0.0, 2.0]));
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
        assert!(l.len() <= 3);
        // whatever was parsed writes back to something that parses the same
        assert_eq!(parse_list(&write_list(&l)), l);
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
    assert!(list[0].is_lens_correction());
    // written back as opcode 14, version 1.6.0.0, optional flag kept
    let again = vec![Opcode::WarpRectilinear2 { planes: vec![k], center: [0.5, 0.5], reciprocal: false, optional: true }];
    let bytes = write_list(&again);
    assert_eq!(bytes.get(4..8), Some(&14u32.to_be_bytes()[..]));
    assert_eq!(bytes.get(8..12), Some(&0x0106_0000u32.to_be_bytes()[..]));
    assert_eq!(parse_list(&bytes), again);
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
    // refused ones stay Unknown
    let list = parse_list(&ListWriter::new().op(14, &good[..10]).finish());
    assert!(matches!(list.as_slice(), [Opcode::Unknown { id: 14, .. }]));
    // a hostile but accepted set (huge radial terms) still gives a finite factor, or none
    let k = wr2_set(&[(0, 1.0), (14, 1e9), (13, -1e9)]);
    for reciprocal in [false, true] {
        for r in [0.0, 0.3, 1.0, 5.0] {
            assert!(wr2_factor(&k, reciprocal, r).is_none_or(f64::is_finite));
        }
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

#[test]
fn cropped_refuses_what_it_cannot_reframe() {
    let (w, h) = (100usize, 80usize);
    let warp = Opcode::WarpRectilinear { planes: vec![[1.0, 0.02, 0.0, 0.0, 0.001, 0.0]], center: [0.5, 0.5] };
    assert_eq!(warp.cropped(w, h, crate::Rect::new(0, 0, w, h)), Some(warp.clone()), "the whole image is itself");
    assert!(warp.cropped(1, h, crate::Rect::new(0, 0, 1, h)).is_none());
    assert!(warp.cropped(w, h, crate::Rect::new(40, 0, 70, 50)).is_none());
    assert!(warp.cropped(w, h, crate::Rect::new(usize::MAX, 0, 70, 50)).is_none());
    assert!(Opcode::TrimBounds { top: 0, left: 0, bottom: 1, right: 1 }.cropped(w, h, crate::Rect::new(0, 0, w, h)).is_none());
    // a crop moves the centre into the crop's frame
    let Some(Opcode::WarpRectilinear { center, .. }) = warp.cropped(w, h, crate::Rect::new(0, 0, 50, 40)) else { panic!("a warp") };
    assert!(center[0] > 1.0 && center[1] > 1.0, "{center:?}");
}
