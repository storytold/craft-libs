use super::*;

// An empty image (no pixel to sample) has a fingerprint, its dimensions' (indexing its first pixel panicked)
#[test]
fn an_empty_image_has_a_fingerprint() {
    assert_eq!(fingerprint(&Rgb32f::new(0, 0)), fingerprint(&Rgb32f::new(0, 0)));
    assert_ne!(fingerprint(&Rgb32f::new(0, 3)), fingerprint(&Rgb32f::new(3, 0)));
}

/// splitmix64: deterministic pseudo-random numbers for the warp tests.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `lo..hi`.
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * ((self.next() >> 11) as f64 / (1u64 << 53) as f64)
    }
    /// `range(lo, hi)` or, one time in four, exactly 0 (to take the "no correction" branches).
    fn maybe(&mut self, lo: f64, hi: f64) -> f64 {
        if self.next().is_multiple_of(4) { 0.0 } else { self.range(lo, hi) }
    }
}

/// A pseudo-random warp: perspective, manual distortion, CA and an embedded lens warp, each sometimes off.
/// Sizes are Pythagorean (exact `hypot`), so results don't depend on the platform's libm.
fn random_warp(r: &mut Rng) -> Warp {
    const SIZES: [(f64, f64); 6] = [(4000.0, 3000.0), (3000.0, 4000.0), (1200.0, 900.0), (1200.0, 500.0), (56.0, 33.0), (6000.0, 4500.0)];
    let (w, h) = SIZES[(r.next() % SIZES.len() as u64) as usize];
    let mut wp = Warp::identity(w, h);
    if !r.next().is_multiple_of(4) {
        let m = [
            1.0 + r.range(-0.2, 0.2),
            r.range(-0.2, 0.2),
            r.range(-0.1, 0.1),
            r.range(-0.2, 0.2),
            1.0 + r.range(-0.2, 0.2),
            r.range(-0.1, 0.1),
            r.range(-0.4, 0.4),
            r.range(-0.4, 0.4),
            1.0 + r.range(-0.1, 0.1),
        ];
        wp.persp_inv = Homography(m);
        wp.persp = wp.persp_inv.inverse().unwrap_or(Homography::IDENTITY);
    }
    wp.k1 = r.maybe(-0.2, 0.2);
    wp.ca = [r.maybe(-0.003, 0.003), r.maybe(-0.003, 0.003), r.maybe(-0.003, 0.003)];
    if !r.next().is_multiple_of(4) {
        let mut plane = || {
            [
                1.0 + r.range(-0.05, 0.05),
                r.range(-0.08, 0.08),
                r.range(-0.05, 0.05),
                r.range(-0.02, 0.02),
                r.range(-0.003, 0.003),
                r.range(-0.003, 0.003),
            ]
        };
        let planes = [plane(), plane(), plane()];
        let warp = EmbeddedWarp { planes, center: Point::new(r.range(0.4, 0.6), r.range(0.4, 0.6)), radius: r.range(0.5, 0.8) };
        wp.lens = Some(EmbeddedLens { warp: Some(warp), vignette: None, ..Default::default() });
        wp.lens_dist = r.maybe(0.0, 2.0);
    }
    wp
}

/// FNV-1a over the bits of the f64 warp results (`Affine::apply`, `Homography::apply`, `to_corrected`,
/// `from_corrected`, `corrected_to_source`, `to_source`, `embedded_warp`) for pseudo-random warps and points.
/// The expected value was captured before these formulas became generic over `Real`: the f64 path must keep
/// the same operations in the same order, bit for bit. To recompute it after changing the inputs, run this
/// test (printing `hash`) on a scratch copy of 419261c (thumb-split, pre-generic formulas) and paste it here.
#[test]
fn warp_f64_results_are_bit_identical_to_the_pre_generic_formulas() {
    let mut hash = 0xCBF2_9CE4_8422_2325u64;
    let mut eat = |p: Point| {
        for v in [p.x, p.y] {
            for b in v.to_bits().to_le_bytes() {
                hash = (hash ^ b as u64).wrapping_mul(0x0000_0100_0000_01B3);
            }
        }
    };
    let mut r = Rng(42);
    for _ in 0..2000 {
        let wp = random_warp(&mut r);
        let a = craft_geom::Affine([
            r.range(-2.0, 2.0),
            r.range(-2.0, 2.0),
            r.range(-2.0, 2.0),
            r.range(-2.0, 2.0),
            r.range(-500.0, 500.0),
            r.range(-500.0, 500.0),
        ]);
        for _ in 0..40 {
            let p = Point::new(r.range(-0.2, 1.2) * wp.w, r.range(-0.2, 1.2) * wp.h);
            eat(a.apply(p));
            eat(wp.persp_inv.apply(Point::new(p.x / wp.w - 0.5, p.y / wp.h - 0.5)));
            eat(wp.to_corrected(p));
            eat(wp.from_corrected(p));
            for ch in 0..3 {
                eat(wp.corrected_to_source(p, ch));
                eat(wp.to_source(p, ch));
            }
            if let Some(ew) = wp.lens.and_then(|l| l.warp) {
                for ch in 0..4 {
                    eat(embedded_warp(&ew, p, ch, wp.w, wp.h));
                }
            }
        }
    }
    // `Homography::apply`'s near-zero denominator clamp, including subnormal and signed-zero denominators and
    // ±1e-300 exactly (not clamped: `<`, not `<=`)
    let hz = Homography([1.0, 0.5, 0.25, -0.5, 1.0, 0.125, 1.0, 0.0, 0.0]);
    for x in [0.0, -0.0, 1e-310, -1e-310, 1e-300, -1e-300, 1e-299, -1e-299, 0.5] {
        eat(hz.apply(Point::new(x, 0.75)));
    }
    assert_eq!(hash, 0x3FA5_7473_D795_9E32, "f64 warp results changed");
}

/// The interval evaluation of `framing` bounds the f64 evaluation of every point of the box.
#[test]
fn interval_framing_encloses_every_f64_result() {
    let mut r = Rng(7);
    let (mut decided, mut total) = (0, 0);
    for _ in 0..500 {
        let wp = random_warp(&mut r);
        let a = craft_geom::Affine([
            r.range(0.5, 2.0),
            r.range(-0.2, 0.2),
            r.range(-0.2, 0.2),
            r.range(0.5, 2.0),
            r.range(-300.0, 300.0),
            r.range(-300.0, 300.0),
        ]);
        for _ in 0..20 {
            let (x0, y0) = (r.range(-0.2, 1.1) * wp.w, r.range(-0.2, 1.1) * wp.h);
            let (bw, bh) = (r.range(0.0, 40.0), r.range(0.0, 40.0));
            let (ix, iy) = (Interval::new(x0, x0 + bw), Interval::new(y0, y0 + bh));
            let (_, (sx, sy)) = wp.framing(&a, ix, iy);
            total += 1;
            if !(sx.is_decided() && sy.is_decided()) {
                continue;
            }
            decided += 1;
            for i in 0..=6 {
                for j in 0..=6 {
                    let (x, y) = if i == 6 && j == 6 {
                        (r.range(x0, x0 + bw), r.range(y0, y0 + bh))
                    } else {
                        (x0 + bw * i as f64 / 5.0, y0 + bh * j as f64 / 5.0)
                    };
                    let (x, y) = (x.clamp(ix.lo(), ix.hi()), y.clamp(iy.lo(), iy.hi()));
                    let (_, (fx, fy)) = wp.framing(&a, x, y);
                    assert!(sx.lo() <= fx && fx <= sx.hi() && sy.lo() <= fy && fy <= sy.hi(), "({x}, {y}) → ({fx}, {fy}) outside {sx:?} × {sy:?}");
                }
            }
        }
    }
    assert!(decided * 10 > total * 9, "{decided}/{total} boxes bounded");
}

#[test]
fn block_coverage_agrees_with_every_pixel() {
    let lens = EmbeddedLens {
        warp: Some(EmbeddedWarp {
            planes: [[1.0, -0.03, 0.01, 0.0, 0.001, -0.002], [1.0, -0.028, 0.01, 0.0, 0.001, -0.002], [1.0, -0.026, 0.01, 0.0, 0.001, -0.002]],
            center: Point::new(0.52, 0.48),
            radius: 0.6,
        }),
        vignette: None,
        ..Default::default()
    };
    let mut cases: Vec<DevelopSettings> = Vec::new();
    let mut s = DevelopSettings::default();
    s.optics.lens_profile = true;
    s.optics.ca_red = 30.0;
    s.geometry.horizontal = -15.0;
    cases.push(s.clone());
    s.orientation = Orientation::Rotate270;
    cases.push(s.clone());
    let mut s = DevelopSettings::default();
    s.optics.distortion = -60.0;
    s.optics.ca_blue = 50.0;
    s.geometry.vertical = 25.0;
    s.geometry.rotate = 7.0;
    cases.push(s.clone());
    s.optics.distortion = 80.0;
    s.geometry.horizontal = 30.0;
    s.orientation = Orientation::Rotate90;
    cases.push(s);
    for (i, s) in cases.iter().enumerate() {
        let f = crate::geometry::Frame::with_lens(900, 600, s, true, Some(&lens));
        let wp = f.warp.as_ref().expect("warp");
        let (w, h) = f.fit(700, 700);
        let o2t = f.out_to_oriented(w, h);
        let (mut decided, mut tiles, mut blank) = (0, 0, 0);
        for (tw, th) in [(32, 8), (7, 5), (64, 64)] {
            for y0 in (0..h).step_by(th) {
                for x0 in (0..w).step_by(tw) {
                    let (x1, y1) = ((x0 + tw).min(w), (y0 + th).min(h));
                    tiles += 1;
                    let Some(all) = wp.block_coverage(&o2t, x0, x1, y0, y1) else { continue };
                    decided += 1;
                    for y in y0..y1 {
                        for x in x0..x1 {
                            assert_eq!(wp.covers(&o2t, x, y), all, "case {i}: pixel {x},{y} in block {x0}..{x1} × {y0}..{y1}");
                            blank += !all as usize;
                        }
                    }
                }
            }
        }
        // most blocks are decided wholesale, including blank ones away from the edge
        assert!(decided * 10 > tiles * 8, "case {i}: {decided}/{tiles} blocks decided");
        // strong distortion leaves blank margins wide enough for whole blocks
        assert!(blank > 0 || i < 2, "case {i}: no blank block decided");
        assert_eq!(wp.block_coverage(&o2t, 5, 5, 0, 1), None, "empty block");
    }
}

/// A grid of dark lines on white with lateral CA: red magnified by `ar`, blue by `ab` (about the centre).
fn ca_grid(w: usize, h: usize, ar: f64, ab: f64) -> Rgb32f {
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let val = |x: f64, y: f64| -> f32 {
        // smooth-edged discs on a grid (edges in every direction)
        let cell = w as f64 / 10.0;
        let (fx, fy) = ((x / cell).fract() - 0.5, (y / cell).fract() - 0.5);
        let d = fx.hypot(fy) * cell - cell * 0.28;
        (0.05 + 0.85 * (d / 1.2).clamp(-0.5, 0.5) + 0.425) as f32
    };
    Rgb32f::from_fn(w, h, |x, y| {
        let (x, y) = (x as f64 + 0.5, y as f64 + 0.5);
        let at = |a: f64| val(cx + (x - cx) / (1.0 + a), cy + (y - cy) / (1.0 + a));
        [at(ar), at(0.0), at(ab)]
    })
}

#[test]
fn ca_estimate_recovers_scale() {
    let img = ca_grid(600, 400, 0.004, -0.003);
    let [r, b] = estimate_lateral_ca_uncached(&img);
    assert!((r - 0.004).abs() < 0.0008, "red {r}");
    assert!((b + 0.003).abs() < 0.0008, "blue {b}");
    let clean = ca_grid(600, 400, 0.0, 0.0);
    let [r, b] = estimate_lateral_ca_uncached(&clean);
    assert!(r.abs() < 0.0005 && b.abs() < 0.0005, "{r} {b}");
}

#[test]
fn warp_identity_and_distortion_direction() {
    let s = DevelopSettings::default();
    let wp = Warp::from_settings(300.0, 200.0, &s, None);
    assert!(wp.is_identity());
    let mut s = s;
    s.optics.distortion = 50.0; // corrects barrel: corners sample from further in
    let wp = Warp::from_settings(300.0, 200.0, &s, None);
    let q = wp.to_source(Point::new(0.0, 0.0), 1);
    assert!(q.x > 0.0 && q.y > 0.0);
    let c = wp.to_source(Point::new(150.0, 100.0), 1);
    assert!(c.dist(Point::new(150.0, 100.0)) < 1e-9);
}

#[test]
fn manual_vignette_brightens_corners() {
    let mut s = DevelopSettings::default();
    s.optics.vignetting = 100.0;
    let wp = Warp::from_settings(300.0, 200.0, &s, None);
    assert!((wp.gain(Point::new(150.0, 100.0)) - 1.0).abs() < 1e-6);
    assert!(wp.gain(Point::new(0.0, 0.0)) > 2.0);
}

#[test]
fn reorient_keeps_centre_mapping() {
    let lens = EmbeddedLens {
        warp: Some(EmbeddedWarp { planes: [[1.0, 0.01, 0.0, 0.0, 0.001, 0.002]; 3], center: Point::new(0.4, 0.5), radius: 0.6 }),
        vignette: Some(EmbeddedVignette { k: [0.2, 0.0, 0.0, 0.0, 0.0], center: Point::new(0.4, 0.5), radius: 0.6 }),
        ..Default::default()
    };
    let r = reorient_lens(&lens, Orientation::Rotate90, 300.0, 200.0);
    // rotate 90° cw: (x, y) → (h − y, x)
    let c = r.warp.unwrap().center;
    assert!((c.x - 0.5).abs() < 1e-9 && (c.y - 0.4).abs() < 1e-9, "{c:?}");
    // a point's warped position transforms like the point itself
    let p = Point::new(50.0, 30.0);
    let s0 = embedded_warp(&lens.warp.unwrap(), p, 0, 300.0, 200.0);
    let m = |q: Point| {
        let (x, y) = Orientation::Rotate90.map(q.x, q.y, 300.0, 200.0);
        Point::new(x, y)
    };
    let s1 = embedded_warp(&r.warp.unwrap(), m(p), 0, 200.0, 300.0);
    assert!(s1.dist(m(s0)) < 1e-6, "{s1:?} vs {:?}", m(s0));
}

/// A `WarpRectilinear2` stage with the same coefficients on every plane.
fn stage(kr: &[(usize, f64)], center: Point, radius: f64, reciprocal: bool) -> EmbeddedWarpStage {
    let mut k = [0.0; 19];
    for &(i, v) in kr {
        k[i] = v;
    }
    k[18] = 1.0;
    EmbeddedWarpStage { planes: [k; 3], center, radius, reciprocal }
}

fn chain(stages: &[EmbeddedWarpStage]) -> Warp {
    let mut wp = Warp::identity(300.0, 200.0);
    let mut lens = EmbeddedLens::default();
    for (slot, st) in lens.stages.iter_mut().zip(stages) {
        *slot = Some(*st);
    }
    wp.lens = Some(lens);
    wp.lens_dist = 1.0;
    wp
}

#[test]
fn warp_rectilinear2_stages_identity_chain_order_and_reciprocal() {
    let c = Point::new(0.5, 0.5);
    let p = Point::new(40.0, 25.0);
    // identity coefficients (kr0 = 1), plain and reciprocal, leave positions alone and don't move pixels
    for reciprocal in [false, true] {
        let wp = chain(&[stage(&[(0, 1.0)], c, 0.6, reciprocal)]);
        assert!(wp.moves_pixels() && !wp.per_channel());
        assert!(wp.corrected_to_source(p, 1).dist(p) < 1e-9);
    }
    // the radial factor at r is f(r); with kr0 = 1, kr1 = 0.5: s = d·(1 + 0.5 r) (and d / (1 + 0.5 r) reciprocal)
    let m = 0.6 * 300.0;
    let d = Point::new((p.x - 150.0) / m, (p.y - 100.0) / m);
    let r = d.x.hypot(d.y);
    let fwd = chain(&[stage(&[(0, 1.0), (1, 0.5)], c, 0.6, false)]).corrected_to_source(p, 0);
    assert!((fwd.x - (150.0 + m * d.x * (1.0 + 0.5 * r))).abs() < 1e-9);
    let rec = chain(&[stage(&[(0, 1.0), (1, 0.5)], c, 0.6, true)]).corrected_to_source(p, 0);
    assert!((rec.y - (100.0 + m * d.y / (1.0 + 0.5 * r))).abs() < 1e-9);
    // list order: [A, B] maps an output position through B first, then A
    let a = stage(&[(0, 1.0), (2, -0.3)], c, 0.6, false);
    let b = stage(&[(0, 1.1), (1, 0.2)], Point::new(0.45, 0.5), 0.6, false);
    let ab = chain(&[a, b]);
    let by_hand = chain(&[a]).corrected_to_source(chain(&[b]).corrected_to_source(p, 1), 1);
    assert!(ab.corrected_to_source(p, 1).dist(by_hand) < 1e-9);
    assert!(ab.corrected_to_source(p, 1).dist(chain(&[b, a]).corrected_to_source(p, 1)) > 1e-3, "order matters");
    // strength scales the correction; a lens without a chain or warp does not move pixels
    let mut half = ab.clone();
    half.lens_dist = 0.5;
    let (full, mid) = (ab.corrected_to_source(p, 1), half.corrected_to_source(p, 1));
    assert!(mid.dist(Point::new((p.x + full.x) / 2.0, (p.y + full.y) / 2.0)) < 1e-9);
    assert!(ab.needs_f64() && !chain(&[]).moves_pixels() && !Warp::identity(3.0, 3.0).needs_f64());
    // planes that differ (lateral CA) are sampled separately
    let mut st = stage(&[(0, 1.0)], c, 0.6, false);
    st.planes[0][0] = 1.01;
    assert!(chain(&[st]).per_channel());
}

#[test]
fn warp_rectilinear2_valid_radius_clamps_the_polynomial() {
    let c = Point::new(0.5, 0.5);
    let mut st = stage(&[(0, 1.0), (1, 1.0)], c, 0.5, false);
    st.planes.iter_mut().for_each(|k| (k[17], k[18]) = (0.2, 0.4));
    let wp = chain(&[st]);
    let m = 0.5 * 300.0;
    // far out (r > 0.4): f stays f(0.4) = 1.4
    let p = Point::new(10.0, 100.0);
    let d = (p.x - 150.0) / m;
    assert!(d.abs() > 0.4);
    assert!((wp.corrected_to_source(p, 0).x - (150.0 + m * d * 1.4)).abs() < 1e-9);
    // near the centre (r < 0.2): f stays f(0.2) = 1.2
    let p = Point::new(150.0 + 0.1 * m, 100.0);
    assert!((wp.corrected_to_source(p, 0).x - (150.0 + 0.1 * m * 1.2)).abs() < 1e-9);
}

#[test]
fn warp_rectilinear2_interval_framing_encloses_every_f64_result() {
    let mut r = Rng(11);
    for _ in 0..200 {
        let mut stages = Vec::new();
        for _ in 0..2 {
            let mut st = stage(&[], Point::new(r.range(0.4, 0.6), r.range(0.4, 0.6)), r.range(0.5, 0.8), r.next().is_multiple_of(3));
            for k in st.planes.iter_mut() {
                k[0] = 1.0 + r.range(-0.05, 0.05);
                for v in k[1..15].iter_mut() {
                    *v = r.maybe(-0.5, 0.5);
                }
                (k[15], k[16]) = (r.range(-0.003, 0.003), r.range(-0.003, 0.003));
                (k[17], k[18]) = (r.maybe(0.0, 0.3), r.range(0.5, 1.0));
            }
            stages.push(st);
        }
        let wp = chain(&stages);
        let a = craft_geom::Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        for _ in 0..20 {
            let (x0, y0) = (r.range(0.0, 1.0) * wp.w, r.range(0.0, 1.0) * wp.h);
            let (bw, bh) = (r.range(0.0, 20.0), r.range(0.0, 20.0));
            let (ix, iy) = (Interval::new(x0, x0 + bw), Interval::new(y0, y0 + bh));
            let (_, (sx, sy)) = wp.framing(&a, ix, iy);
            if !(sx.is_decided() && sy.is_decided()) {
                continue;
            }
            for i in 0..=4 {
                for j in 0..=4 {
                    let (x, y) = ((x0 + bw * i as f64 / 4.0).clamp(ix.lo(), ix.hi()), (y0 + bh * j as f64 / 4.0).clamp(iy.lo(), iy.hi()));
                    let (_, (fx, fy)) = wp.framing(&a, x, y);
                    assert!(sx.lo() <= fx && fx <= sx.hi() && sy.lo() <= fy && fy <= sy.hi(), "({x}, {y}) → ({fx}, {fy}) outside {sx:?} × {sy:?}");
                }
            }
        }
    }
}

#[test]
fn reorient_rotates_a_warp_rectilinear2_stage_like_the_point() {
    let mut st = stage(&[(0, 1.0), (1, 0.05), (2, -0.1)], Point::new(0.4, 0.5), 0.6, false);
    st.planes.iter_mut().for_each(|k| (k[15], k[16]) = (0.001, 0.002));
    let mut lens = EmbeddedLens::default();
    lens.stages[0] = Some(st);
    let p = Point::new(50.0, 30.0);
    let m = |q: Point| {
        let (x, y) = Orientation::Rotate90.map(q.x, q.y, 300.0, 200.0);
        Point::new(x, y)
    };
    let r = reorient_lens(&lens, Orientation::Rotate90, 300.0, 200.0);
    let s0 = embedded_stage(&st, p, 0, 300.0, 200.0);
    let s1 = embedded_stage(&r.stages[0].unwrap(), m(p), 0, 200.0, 300.0);
    assert!(s1.dist(m(s0)) < 1e-6, "{s1:?} vs {:?}", m(s0));
    assert_eq!(reorient_lens(&lens, Orientation::Normal, 300.0, 200.0), lens);
}

#[test]
fn defringe_removes_purple_at_edges_only() {
    let mut s = DevelopSettings::default();
    s.optics.defringe_purple_amount = 10.0;
    // white | black edge with a purple column next to it, and a purple patch far from edges
    let purple = [0.25f32, 0.05, 0.45];
    let mut img = Rgb32f::from_fn(40, 20, |x, _| if x < 20 { [0.9; 3] } else { [0.02; 3] });
    for y in 0..20 {
        img.set(20, y, purple);
    }
    let flat = Rgb32f::filled(40, 20, purple);
    let mut flat2 = flat.clone();
    defringe(&mut img, &s, 1.0);
    defringe(&mut flat2, &s, 1.0);
    let chroma = |p: [f32; 3]| {
        let l = oklab_from_2020(p);
        l[1].hypot(l[2])
    };
    assert!(chroma(img.get(20, 10)) < chroma(purple) * 0.3);
    assert!((chroma(flat2.get(20, 10)) - chroma(purple)).abs() < 1e-4);
}

/// Found by fuzzing catalog snapshots: a defringe amount far past the sliders' 20 (from a
/// damaged catalog or sidecar) made the filter's radius overflow (`x + r`), which panics.
/// It acts as the sliders' end.
#[test]
fn defringe_amounts_past_the_sliders_act_as_their_end() {
    let purple = [0.25f32, 0.05, 0.45];
    let scene = Rgb32f::from_fn(40, 20, |x, _| {
        if x < 20 {
            [0.9; 3]
        } else if x == 20 {
            purple
        } else {
            [0.02; 3]
        }
    });
    let at = |amount: f64, scale: f64| {
        let mut s = DevelopSettings::default();
        s.optics.defringe_purple_amount = amount;
        s.optics.defringe_green_amount = amount / 2.0;
        let mut img = scene.clone();
        defringe(&mut img, &s, scale);
        img
    };
    assert_eq!(at(1e300, 1.0).data, at(20.0, 1.0).data);
    assert_eq!(at(f64::MAX, 1e300).data.len(), 800, "a huge scale too");
}

#[test]
fn the_min_max_filter_matches_a_brute_force_scan_and_is_linear_in_its_radius() {
    let (w, h) = (37, 23);
    let mut p = Plane::new(w, h);
    for (i, v) in p.data.iter_mut().enumerate() {
        *v = ((i * 7919) % 101) as f32 / 101.0;
    }
    for r in [0, 1, 2, 5, 22, 40] {
        for max in [false, true] {
            let got = min_filter(&p, r, max);
            for y in 0..h {
                for x in 0..w {
                    let mut m = p.data[y * w + x];
                    for yy in y.saturating_sub(r)..=(y + r).min(h - 1) {
                        for xx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                            let v = p.data[yy * w + xx];
                            m = if max { m.max(v) } else { m.min(v) };
                        }
                    }
                    assert_eq!(got.data[y * w + x], m, "r {r} max {max} at {x},{y}");
                }
            }
        }
    }
    // a radius as wide as a large output: was ~4 L³ comparisons
    let big = Plane::new(2000, 2000);
    let t = std::time::Instant::now();
    let _ = min_filter(&big, 2000, true);
    assert!(t.elapsed().as_secs() < 20, "took {:?}", t.elapsed());
}
