use super::*;

/// The draws [`hold_for_waiters`] holds, by cache: how many renders wait for each, and how many
/// draws are left to hold.
static HOLDS: Mutex<Vec<(usize, usize, usize)>> = Mutex::new(Vec::new());

/// Holds the draw of `d` until the renders [`HOLDS`] names for its cache are waiting for it (each
/// holds a clone of its slot, as do the drawing and its in-flight entry), so a test can make
/// renders miss a tile together.
pub(super) fn hold_for_waiters(d: &Drawing) {
    let waiters = {
        let mut holds = HOLDS.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(h) = holds.iter_mut().find(|h| h.0 == d.id.0 && h.2 > 0) else { return };
        h.2 -= 1;
        h.1
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while Arc::strong_count(&d.done) < 2 + waiters {
        assert!(std::time::Instant::now() < deadline, "the renders never waited");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// The caches [`evict_before_waking`] empties.
static EVICTS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// Empties `cache` if [`EVICTS`] names it, after its drawer `d` has cached its tile and before it
/// wakes the renders waiting for it, as other renders' tiles can push it out in that gap.
pub(super) fn evict_before_waking(cache: &Cache, d: &Drawing) {
    if EVICTS.lock().unwrap_or_else(PoisonError::into_inner).contains(&d.id.0) {
        cache.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }
}

/// Waits until `n` renders wait for the draw of `d`.
fn until_waiting(d: &Drawing, n: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while Arc::strong_count(&d.done) < 2 + n {
        assert!(std::time::Instant::now() < deadline, "the renders never waited");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// The field rendered at `long` px per long edge on an `n × n` patch starting at pixel `origin`.
fn patch(tile: &CrystalTile, long: f64, n: usize, origin: usize) -> Vec<f32> {
    let w = (TILE_PER_LONG / long) as f32;
    (0..n * n).map(|k| tile.field(((origin + k % n) as f32 + 0.5) * w, ((origin + k / n) as f32 + 0.5) * w, w)).collect()
}

fn std(v: &[f32]) -> f64 {
    let m = v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64;
    (v.iter().map(|&x| (x as f64 - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
}

/// Lag (px) at which the autocorrelation (mean of horizontal and vertical) falls to ½.
fn half_width(v: &[f32], n: usize) -> f64 {
    let m = v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64;
    let d: Vec<f64> = v.iter().map(|&x| x as f64 - m).collect();
    let var = d.iter().map(|x| x * x).sum::<f64>() / d.len() as f64;
    let corr = |k: usize, horizontal: bool| {
        let mut s = 0.0;
        let mut c = 0usize;
        for y in 0..n - if horizontal { 0 } else { k } {
            for x in 0..n - if horizontal { k } else { 0 } {
                let b = if horizontal { y * n + x + k } else { (y + k) * n + x };
                s += d[y * n + x] * d[b];
                c += 1;
            }
        }
        s / c as f64 / var
    };
    let mut out = 0.0;
    for horizontal in [true, false] {
        let mut prev = 1.0;
        let mut w = 16.0;
        for k in 1..16 {
            let c = corr(k, horizontal);
            if c <= 0.5 {
                w = k as f64 - 1.0 + (prev - 0.5) / (prev - c).max(1e-9);
                break;
            }
            prev = c;
        }
        out += w / 2.0;
    }
    out
}

fn tile_of(size: f64, roughness: f64) -> Arc<CrystalTile> {
    tile(CrystalLook::new(size, roughness), 7, true).expect("a tile")
}

#[test]
fn reference_render_has_unit_variance_and_no_mean() {
    let t = tile_of(25.0, 50.0);
    let v = patch(&t, REF_LONG, 600, 37);
    let m = v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64;
    assert!((std(&v) - 1.0).abs() < 0.05, "std {}", std(&v));
    assert!(m.abs() < 0.03, "mean {m}");
}

#[test]
fn small_render_is_the_downscaled_reference() {
    for (size, rough) in [(0.0, 50.0), (25.0, 50.0), (100.0, 50.0), (25.0, 0.0), (25.0, 100.0)] {
        let t = tile_of(size, rough);
        for k in [2usize, 8] {
            let n = 32;
            let full = patch(&t, REF_LONG, n * k, 0);
            let down: Vec<f32> = (0..n * n)
                .map(|q| {
                    let (x, y) = (q % n, q / n);
                    let mut s = 0.0;
                    for dy in 0..k {
                        for dx in 0..k {
                            s += full[(y * k + dy) * n * k + x * k + dx];
                        }
                    }
                    s / (k * k) as f32
                })
                .collect();
            let small = patch(&t, REF_LONG / k as f64, n, 0);
            let worst = down.iter().zip(&small).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            eprintln!("DIAG downscale size {size} roughness {rough} 1/{k}: worst {worst} (limit 2e-3)");
            assert!(worst < 2e-3, "size {size} roughness {rough} 1/{k}: differs by {worst}");
        }
    }
}

#[test]
fn strength_matches_the_target_at_mid_grey() {
    for (a, s, r) in [(50.0, 25.0, 50.0), (25.0, 25.0, 50.0), (50.0, 0.0, 50.0), (50.0, 100.0, 50.0), (50.0, 25.0, 0.0)] {
        let g = CrystalGrain::new(&Grain { amount: a, size: s, roughness: r, seed: 3 }, REF_LONG).expect("grain");
        let n = 200;
        let lum: Vec<f32> = (0..n * n)
            .map(|k| {
                let (x, y) = ((k % n) as f32 + 0.5, (k / n) as f32 + 0.5);
                let e = g.apply([MID_GREY; 3], x / REF_LONG as f32, y / REF_LONG as f32);
                (0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2]) * 255.0
            })
            .collect();
        let want = strength_levels(a, s, r);
        eprintln!("DIAG strength {a}/{s}/{r}: deviation {:.4} (limit 0.06)", std(&lum) / want - 1.0);
        assert!((std(&lum) / want - 1.0).abs() < 0.06, "{a}/{s}/{r}: {} levels, want {want}", std(&lum));
    }
}

#[test]
fn tones_keep_their_mean() {
    // the strongest settings in the shadows; near white, strong grain clips at 255 like any grain
    let cases = [
        (100.0, 0.0, &[5.0f32, 20.0, 40.0, 60.0, 128.0, 255.0][..]),
        (50.0, 0.0, &[200.0, 245.0][..]),
        (50.0, 25.0, &[20.0, 128.0, 245.0, 255.0][..]),
    ];
    for (amount, size, levels) in cases {
        let g = CrystalGrain::new(&Grain { amount, size, roughness: 50.0, seed: 3 }, REF_LONG).expect("grain");
        for &level in levels {
            let n = 256;
            let mut sum = 0.0f64;
            for k in 0..n * n {
                let (x, y) = ((k % n) as f32 + 0.5, (k / n) as f32 + 0.5);
                let e = g.apply([level / 255.0; 3], x / REF_LONG as f32, y / REF_LONG as f32);
                sum += (e[1].clamp(0.0, 1.0) * 255.0) as f64;
            }
            let shift = sum / (n * n) as f64 - level as f64;
            assert!(shift.abs() < 0.5, "{amount}/{size}: level {level} moved {shift:+.2}");
        }
    }
}

#[test]
fn hostile_settings_render_finite_or_no_grain() {
    let bad = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e9, 1e9, 0.0];
    for &a in &bad {
        for &s in &bad {
            for &ppl in &[f64::NAN, 0.0, -5.0, 1.0, 24.0, 1e12, 6000.0] {
                let g = Grain { amount: if a == 0.0 { 50.0 } else { a }, size: s, roughness: s, seed: u32::MAX };
                if let Some(c) = CrystalGrain::new(&g, ppl) {
                    for (x, y) in [(0.0, 0.0), (-3.5, 1e7), (f32::NAN, 0.2), (0.5, f32::INFINITY), (1.0, 0.6667)] {
                        let e = c.apply([0.5, 0.2, 0.9], x, y);
                        assert!(e.iter().all(|v| v.is_finite()), "{g:?} at {ppl}: {e:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn seed_picks_the_pattern() {
    let look = CrystalLook::new(25.0, 50.0);
    let (Some(a), Some(c)) = (tile(look, 1, true), tile(look, 2, true)) else { panic!("tiles") };
    assert_ne!(patch(&a, REF_LONG, 64, 0), patch(&c, REF_LONG, 64, 0));
}

/// A tile drawn again (its cache entry evicted, another process, an export after a preview) is the
/// same to the bit, however many threads draw it. These draw directly: through [`tile`], the second
/// would be the first one out of the cache.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_tile_draws_the_same_every_time() {
    let look = CrystalLook::new(25.0, 50.0);
    let one_thread = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("a pool");
    let (Some(a), Some(b)) = (CrystalTile::new(look, 1), one_thread.install(|| CrystalTile::new(look, 1))) else { panic!("tiles") };
    assert!(a.sats == b.sats, "the same seed drew different tiles");
}

/// Every seed has the same mix of crystal sizes, dealt to the layers in its own order.
#[test]
fn seeds_share_the_mix_of_crystal_sizes() {
    let look = CrystalLook::new(50.0, 80.0);
    let radii = |seed| emulsion(seed, &look).iter().map(|l| l.r).collect::<Vec<f64>>();
    let sorted = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v
    };
    let (a, b) = (radii(1), radii(2));
    assert_eq!(a.len(), look.layers);
    assert_eq!(a, radii(1), "the same seed deals the same order");
    assert_ne!(a, b, "another seed deals another order");
    assert_eq!(sorted(a), sorted(b));
    assert_eq!(emulsion(1, &CrystalLook { layers: 0, ..look }).len(), 1, "an emulsion has a layer");
}

#[test]
fn sliders_clamp_and_fall_back() {
    let look = CrystalLook::new;
    assert_eq!(look(f64::NAN, f64::NAN), look(25.0, 50.0), "NaN is the default");
    assert_eq!(look(-1e9, f64::NEG_INFINITY), look(0.0, 0.0));
    assert_eq!(look(1e9, f64::INFINITY), look(100.0, 100.0));
    // the ends of the tables
    assert_eq!((look(0.0, 50.0).width, look(0.0, 50.0).layers), (0.55, 16));
    assert_eq!((look(100.0, 50.0).width, look(100.0, 50.0).layers), (12.38, 70));
    assert_eq!(look(100.0, 0.0).width, 12.38 * 0.734);
    assert_eq!(look(100.0, 100.0).width, 12.38 * 1.422);
    assert!(look(25.0, 100.0).spread > look(25.0, 0.0).spread);

    let default = strength_levels(50.0, 25.0, 50.0);
    assert_eq!(strength_levels(f64::NAN, 25.0, 50.0), 0.0, "a NaN Amount is no grain");
    assert_eq!(strength_levels(-5.0, 25.0, 50.0), 0.0);
    assert_eq!(strength_levels(50.0, f64::NAN, f64::NAN), default);
    assert_eq!(strength_levels(1e9, 25.0, 50.0), 2.0 * default);
    assert_eq!(strength_levels(50.0, -1e9, 50.0), strength_levels(50.0, 0.0, 50.0));
    assert_eq!(strength_levels(50.0, 25.0, f64::INFINITY), strength_levels(50.0, 25.0, 100.0));
    // Lightroom Classic's at the ends of each slider (`docs/grain.md`)
    for (want, (a, s, r)) in [(23.08, (50.0, 0.0, 50.0)), (5.83, (50.0, 100.0, 50.0)), (12.42, (50.0, 25.0, 0.0)), (11.68, (50.0, 25.0, 100.0))] {
        let got = strength_levels(a, s, r);
        assert!((got / want - 1.0).abs() < 0.01, "{a}/{s}/{r}: {got}, want {want}");
    }
}

#[test]
fn interp_is_linear_between_knots_and_constant_outside() {
    let knots = [(0.0, 1.0), (10.0, 3.0), (20.0, 2.0)];
    for (x, want) in [(-5.0, 1.0), (0.0, 1.0), (5.0, 2.0), (10.0, 3.0), (15.0, 2.5), (20.0, 2.0), (99.0, 2.0)] {
        assert_eq!(interp(&knots, x), want, "at {x}");
    }
    assert_eq!(interp(&[], 5.0), 0.0);
    assert_eq!(interp(&[(3.0, 7.0)], 5.0), 7.0);
}

#[test]
fn normal_quantile_matches_the_table() {
    for (p, z) in [(0.5, 0.0), (0.975, 1.959_963_985), (0.841_344_746, 1.0), (0.001, -3.090_232_306), (1e-6, -4.753_424_309)] {
        assert!((normal_quantile(p) - z).abs() < 1e-7, "at {p}: {}, want {z}", normal_quantile(p));
        assert!((normal_quantile(1.0 - p) + z).abs() < 1e-7, "at 1 - {p}");
    }
    // increasing across the approximation's joins (0.02425 and its mirror), finite at the ends
    let sweep: Vec<f64> = (0..=1000).map(|k| normal_quantile(k as f64 / 1000.0)).collect();
    assert!(sweep.iter().all(|z| z.is_finite()));
    assert!(sweep.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn poisson_counts_have_the_mean_and_variance_of_mu() {
    let n = 40_000u32;
    for mu in [0.5, 4.0, 40.0, 150.0] {
        let cdf = poisson_cdf(mu);
        assert!(cdf.windows(2).all(|w| w[0] <= w[1]), "mu {mu}: the distribution must not fall");
        assert!(cdf.last().is_some_and(|&c| c >= 1.0 - 1e-12), "mu {mu}: the distribution must reach 1");
        let draws: Vec<f64> = (0..n).map(|i| poisson(&cdf, mix(i)) as f64).collect();
        let mean = draws.iter().sum::<f64>() / n as f64;
        let var = draws.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / n as f64;
        assert!((mean - mu).abs() < 4.0 * (mu / n as f64).sqrt(), "mu {mu}: mean {mean}");
        assert!((var / mu - 1.0).abs() < 0.05, "mu {mu}: variance {var}");
    }
}

#[test]
fn poisson_stays_in_range_at_the_ends() {
    for h in [0, u32::MAX] {
        assert_eq!(poisson(&poisson_cdf(0.0), h), 0, "mean 0 is never a crystal");
        // past what the model uses, e^-mu underflows: a capped count, not a panic
        assert!(poisson(&poisson_cdf(1e6), h) <= 512);
        assert_eq!(poisson(&[], h), 0);
    }
}

#[test]
fn summed_area_matches_brute_force() {
    let t = 7;
    let count: Vec<u8> = (0..t * t).map(|i| (mix(i as u32) % 71) as u8).collect();
    let sat = summed_area(&count, t);
    assert_eq!(sat.len(), (t + 1) * (t + 1));
    for y in 0..=t {
        for x in 0..=t {
            let want: u32 = (0..y).flat_map(|j| (0..x).map(move |i| (i, j))).map(|(i, j)| count[j * t + i] as u32).sum();
            assert_eq!(sat[y * (t + 1) + x], want, "[0, {x}) × [0, {y})");
        }
    }
}

/// Boxes against the emulsion's own pixels (each table's counts, less their mean, scaled to unit
/// variance as [`CrystalTile::new`] does), integrated by hand. Whole and fractional boxes, across
/// the torus's seams, at negative coordinates and periods away.
#[test]
fn box_sum_is_the_integral_of_the_emulsion() {
    let (look, seed) = (CrystalLook::new(25.0, 50.0), 7);
    let tile = CrystalTile::new(look, seed).expect("a tile");
    let counts = [0, 1].map(|which| draw_torus(which, TORI[which], seed, &look));
    let means: [f64; 2] = [0, 1].map(|w| counts[w].iter().map(|&c| c as f64).sum::<f64>() / (TORI[w] * TORI[w]) as f64);
    let var: f64 = [0, 1]
        .map(|w| {
            let (sq, n) = reference_square_sum(&counts[w], TORI[w], means[w]);
            sq / n as f64
        })
        .iter()
        .sum();
    let k = 1.0 / var.sqrt();
    // The overlap of the cell at `i` with `[a, a + w]`.
    let overlap = |i: i64, a: f64, w: f64| ((i + 1) as f64).min(a + w) - (i as f64).max(a);
    let integral_by_hand = |which: usize, x0: f32, y0: f32, w: f32| {
        let (x0, y0, w, t) = (x0 as f64, y0 as f64, w as f64, TORI[which] as i64);
        let mut sum = 0.0;
        for j in y0.floor() as i64..(y0 + w).ceil() as i64 {
            for i in x0.floor() as i64..(x0 + w).ceil() as i64 {
                let c = counts[which][(j.rem_euclid(t) * t + i.rem_euclid(t)) as usize] as f64;
                sum += (c - means[which]) * k * overlap(i, x0, w) * overlap(j, y0, w);
            }
        }
        sum
    };
    for which in [0, 1] {
        let t = TORI[which] as f32;
        for (x0, y0, w) in [
            (0.0, 0.0, 2.0),
            (100.0, 200.0, 37.0),
            (3.25, 7.5, 2.0),
            (10.6, 11.3, 0.7),
            (50.5, 60.25, 400.0),
            (t - 1.5, 20.0, 4.0),
            (20.0, t - 1.5, 4.0),
            (t - 0.5, t - 0.25, 31.0),
            (-1.5, -2.25, 8.0),
            (3.0 * t + 12.5, -2.0 * t + 40.5, 8.0),
        ] {
            let (got, want) = (tile.box_sum(which, x0, y0, w) as f64, integral_by_hand(which, x0, y0, w));
            let area = (w * w) as f64;
            assert!((got - want).abs() / area < 2e-3, "torus {which}, ({x0}, {y0}) side {w}: {got} vs {want}");
        }
    }
    // the field is the two tori's mean, centred on its position
    let (x, y, w) = (640.25, 1133.5, 3.0);
    let want = (integral_by_hand(0, x - 0.5 * w, y - 0.5 * w, w) + integral_by_hand(1, x - 0.5 * w, y - 0.5 * w, w)) / (w * w) as f64;
    assert!((tile.field(x, y, w) as f64 - want).abs() < 2e-3, "field {} vs {want}", tile.field(x, y, w));
}

/// A render under the footprint limit gets grain, one at or past it none, whatever else it asks.
#[test]
fn only_a_render_past_the_footprint_limit_loses_its_grain() {
    let g = Grain { amount: 50.0, size: 25.0, roughness: 50.0, seed: 3 };
    let limit = TILE_PER_LONG / MAX_FOOTPRINT as f64;
    assert_eq!(limit, 24.0);
    for none in [limit, limit - 1.0, 1.0, 0.0, -5.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(CrystalGrain::new(&g, none).is_none(), "{none} px per long edge");
    }
    assert_eq!(CrystalGrain::new(&g, limit + 1.0).expect("grain").foot, 480.0);
    assert_eq!(CrystalGrain::new(&g, REF_LONG).expect("grain").foot, 2.0);
    assert!(CrystalGrain::new(&Grain { amount: 0.0, ..g }, REF_LONG).is_none(), "no grain at Amount 0");
}

/// A draft (a Size or Roughness drag) doesn't wait for a new tile: the last tile of the seed stands
/// in while the new one is drawn in the background, and then the drag gets it. A render that
/// waits always gets the exact tile, as does a draft for a seed with no tile yet.
#[test]
fn a_drag_takes_the_last_tile_while_the_next_is_drawn() {
    // a cache of its own: other tests' tiles (through any V7 render) would evict from the shared one
    static OWN: Cache = Mutex::new(Vec::new());
    let tile = |look, seed, wait| tile_in(&OWN, look, seed, wait);
    let (seed, fresh) = (0x0d4a_0001, 0x0d4a_0002);
    let (a, b) = (CrystalLook::new(30.0, 50.0), CrystalLook::new(31.0, 50.0));
    let exact = |look: CrystalLook, seed: u32| patch(&CrystalTile::new(look, seed).expect("tile"), REF_LONG, 32, 0);
    let drawing = std::time::Instant::now();
    let first = tile(a, seed, true).expect("tile");
    eprintln!("DIAG crystal tile at size 30: {:?}", drawing.elapsed());
    assert_eq!(first.look, a);
    let stand_in = tile(b, seed, false).expect("tile");
    assert_eq!(stand_in.look, a, "the draft waited for its tile");
    // the drag's next renders get the new tile once it is drawn
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let drawn = loop {
        let t = tile(b, seed, false).expect("tile");
        if t.look == b {
            break t;
        }
        assert!(std::time::Instant::now() < deadline, "the background tile never arrived");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let exact_b = exact(b, seed);
    assert_eq!(patch(&drawn, REF_LONG, 32, 0), exact_b);
    assert_eq!(patch(&tile(b, seed, true).expect("tile"), REF_LONG, 32, 0), exact_b);
    // nothing drawn for this seed: nothing to stand in, the draft draws it
    assert_eq!(patch(&tile(a, fresh, false).expect("tile"), REF_LONG, 32, 0), exact(a, fresh));
}

/// A render that misses a tile another thread is drawing waits for that tile rather than drawing
/// it a second time.
#[test]
fn a_render_waits_for_a_tile_being_drawn() {
    static OWN: Cache = Mutex::new(Vec::new());
    let (look, seed) = (CrystalLook::new(40.0, 50.0), 0x0d4a_0003);
    let key = key_of(look, seed);
    let marker = start_drawing((std::ptr::from_ref(&OWN) as usize, key)).expect("not drawn yet");
    let render = std::thread::spawn(move || tile_in(&OWN, look, seed, true));
    until_waiting(&marker, 1);
    let drawn = Arc::new(CrystalTile::new(look, seed).expect("tile"));
    keep(&OWN, key, drawn.clone());
    drop(marker);
    let got = render.join().expect("render thread").expect("tile");
    assert!(Arc::ptr_eq(&got, &drawn), "the render drew the tile again");
}

/// A render waiting for a tile gets it from its drawer even when the cache evicted it before the
/// render woke.
#[test]
fn a_waiting_render_gets_the_tile_the_cache_evicted() {
    static OWN: Cache = Mutex::new(Vec::new());
    let (look, seed) = (CrystalLook::new(40.0, 50.0), 0x0d4a_0004);
    let key = key_of(look, seed);
    let marker = start_drawing((std::ptr::from_ref(&OWN) as usize, key)).expect("not drawn yet");
    let render = std::thread::spawn(move || tile_in(&OWN, look, seed, true));
    until_waiting(&marker, 1);
    let drawn = Arc::new(CrystalTile::new(look, seed).expect("tile"));
    marker.finish(&drawn);
    keep(&OWN, key, drawn.clone());
    // other renders' tiles push it out of the cache before the waiter wakes
    // (each its own tile: a waiter that took another key's would fail the check below)
    // (one tile of another seed under every key: a different `Arc` from the drawn one, drawn once)
    let other_tile = Arc::new(CrystalTile::new(look, seed + 1).expect("tile"));
    for other in 1..=CACHE as u32 {
        keep(&OWN, key_of(look, seed + other), other_tile.clone());
    }
    assert!(cached(&OWN, key).is_none(), "the tile is still cached");
    drop(marker);
    let got = render.join().expect("render thread").expect("tile");
    assert!(Arc::ptr_eq(&got, &drawn), "the render drew the tile again");
}

/// A render waiting for a tile another render is drawing gets the drawer's tile even when the
/// cache drops it before the waiter wakes.
#[test]
fn a_render_drawing_a_tile_hands_it_to_its_waiter() {
    static OWN: Cache = Mutex::new(Vec::new());
    let id = std::ptr::from_ref(&OWN) as usize;
    // the first render's draw (only: a second would be the waiter drawing it again) waits for the
    // second render to wait for it, and the cache empties before the second wakes
    HOLDS.lock().unwrap_or_else(PoisonError::into_inner).push((id, 1, 1));
    EVICTS.lock().unwrap_or_else(PoisonError::into_inner).push(id);
    let (look, seed) = (CrystalLook::new(0.0, 50.0), 0x0d4a_0005);
    let renders: Vec<_> = (0..2).map(|_| std::thread::spawn(move || tile_in(&OWN, look, seed, true))).collect();
    let got: Vec<_> = renders.into_iter().map(|r| r.join().expect("render thread").expect("tile")).collect();
    assert!(cached(&OWN, key_of(look, seed)).is_none(), "the cache kept the tile");
    assert!(Arc::ptr_eq(&got[0], &got[1]), "the waiter drew the tile again");
}

/// Renders that miss the same tile together draw it once: the first registers the draw and the
/// others wait for it, as no gap lets a second render find it neither cached nor being drawn.
/// The first draw is held until the other renders all wait for it, so the misses overlap.
#[test]
fn renders_that_miss_a_tile_together_draw_it_once() {
    static OWN: Cache = Mutex::new(Vec::new());
    const N: usize = 8;
    // the first render's draw waits for the other seven to miss the tile too
    // (each seed is a full trial, as the draw is held until the others wait, so a few are enough)
    const SEEDS: u32 = 2;
    HOLDS.lock().unwrap_or_else(PoisonError::into_inner).push((std::ptr::from_ref(&OWN) as usize, N - 1, SEEDS as usize));
    let look = CrystalLook::new(0.0, 50.0);
    for seed in 0x0d4a_1000..0x0d4a_1000 + SEEDS {
        let start = Arc::new(std::sync::Barrier::new(N));
        let renders: Vec<_> = (0..N)
            .map(|_| {
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    tile_in(&OWN, look, seed, true)
                })
            })
            .collect();
        let got: Vec<_> = renders.into_iter().map(|r| r.join().expect("render thread").expect("tile")).collect();
        assert!(got.iter().all(|t| Arc::ptr_eq(t, &got[0])), "seed {seed:#x}: a render drew the tile again");
    }
}

/// Grain width (autocorrelation half-width at mid grey, 6000 px) at Lightroom Classic's
/// measured settings (open PR #1060's table); Size 0 is as fine as uncorrelated pixels get (0.5).
#[test]
fn width_matches_lightroom_classic() {
    let lrc = [(0.0, 50.0, 0.5), (25.0, 50.0, 1.13), (100.0, 50.0, 3.38), (25.0, 0.0, 0.86), (25.0, 100.0, 1.54)];
    for (s, r, want) in lrc {
        let got = half_width(&patch(&tile_of(s, r), REF_LONG, 256, 0), 256);
        eprintln!("DIAG width size {s} roughness {r}: {got:.3} want {want} (limit 10 percent)");
        assert!((got / want - 1.0).abs() < 0.1, "size {s} roughness {r}: width {got:.3}, want {want}");
    }
}

/// Prints how long drawing a tile takes per setting (`--ignored --nocapture`, alone).
#[test]
#[ignore]
fn tile_timing_report() {
    for (s, r) in [(0.0, 50.0), (10.0, 50.0), (25.0, 50.0), (50.0, 50.0), (100.0, 50.0), (25.0, 0.0), (25.0, 100.0), (100.0, 100.0)] {
        let look = CrystalLook::new(s, r);
        let mut best = f64::INFINITY;
        for seed in 0..5 {
            let start = std::time::Instant::now();
            let t = CrystalTile::new(look, 100 + seed);
            best = best.min(start.elapsed().as_secs_f64() * 1e3);
            assert!(t.is_some());
        }
        eprintln!("size {s:>5} roughness {r:>5}: {best:.1} ms ({:?})", look);
    }
}

/// Fits the median crystal width per Size (Roughness 50) and the Roughness factors at Size 25 to
/// Lightroom Classic's grain widths (`--ignored --nocapture`; paste into the look tables).
#[test]
#[ignore]
fn fit_widths() {
    let width_of = |look: CrystalLook| {
        let mut w = 0.0;
        for seed in [11, 12] {
            let t = CrystalTile::new(look, seed).expect("tile");
            w += half_width(&patch(&t, REF_LONG, 360, 0), 360) / 2.0;
        }
        w
    };
    let fit = |s: f64, r: f64, want: f64| {
        let base = CrystalLook::new(s, r);
        let (mut lo, mut hi) = (0.05f64, 40.0f64);
        for _ in 0..18 {
            let mid = (lo * hi).sqrt();
            if width_of(CrystalLook { width: mid, ..base }) < want { lo = mid } else { hi = mid }
        }
        let d = (lo * hi).sqrt();
        (d, width_of(CrystalLook { width: d, ..base }))
    };
    for (s, want) in [(0.0, 0.43), (10.0, 0.71), (25.0, 1.13), (50.0, 1.70), (75.0, 2.54), (100.0, 3.38)] {
        let (d, got) = fit(s, 50.0, want);
        eprintln!("size {s}: width {d:.3} gives {got:.3} (want {want})");
    }
    let (d50, _) = fit(25.0, 50.0, 1.13);
    for (r, want) in [(0.0, 0.86), (100.0, 1.54)] {
        let (d, got) = fit(25.0, r, want);
        eprintln!("roughness {r}: factor {:.3} gives {got:.3} (want {want})", d / d50);
    }
}

/// Autocorrelation at lags 1–4 and the strength kept by box downscales of 2–16 (relative), of
/// candidate looks (`--ignored --nocapture`): compares a look's texture with a reference's.
#[test]
#[ignore]
fn texture_report() {
    let profile = |look: CrystalLook| {
        let n = 512;
        let (mut acf, mut down) = ([0.0; 4], [0.0; 4]);
        for seed in [21, 22] {
            let t = CrystalTile::new(look, seed).expect("tile");
            let v = patch(&t, REF_LONG, n, 0);
            let s0 = std(&v);
            for (k, a) in acf.iter_mut().enumerate() {
                let lag = k + 1;
                let mut s = 0.0f64;
                for y in 0..n {
                    for x in 0..n - lag {
                        s += (v[y * n + x] * v[y * n + x + lag] + v[x * n + y] * v[(x + lag) * n + y]) as f64;
                    }
                }
                *a += s / (2 * n * (n - lag)) as f64 / (s0 * s0) / 2.0;
            }
            for (k, d) in down.iter_mut().enumerate() {
                let f = 2usize << k;
                let m = n / f;
                let b: Vec<f32> = (0..m * m)
                    .map(|q| {
                        let (x, y) = (q % m * f, q / m * f);
                        (0..f * f).map(|i| v[(y + i / f) * n + x + i % f]).sum::<f32>() / (f * f) as f32
                    })
                    .collect();
                *d += std(&b) / s0 / 2.0;
            }
        }
        (acf, down)
    };
    for (s, r) in [(25.0, 50.0), (50.0, 50.0), (100.0, 50.0), (25.0, 0.0), (25.0, 100.0)] {
        let (a, d) = profile(CrystalLook::new(s, r));
        eprintln!(
            "size {s} roughness {r}: acf {:.2} {:.2} {:.2} {:.2}  down {:.2} {:.2} {:.2} {:.3}",
            a[0], a[1], a[2], a[3], d[0], d[1], d[2], d[3]
        );
    }
}

/// A row computed with shared box edges equals the per-pixel boxes: any size, mirrored rows, rows
/// across the tori's seams and far from the origin.
#[test]
fn row_field_matches_per_pixel_field() {
    let t = tile_of(25.0, 50.0);
    for (w, sign, x, y) in [
        (2.0f32, 1.0f32, 3.3f32, 7.9f32),
        (2.0, -1.0, 9000.2, 1275.0),
        (8.0, 1.0, 11990.0, 2260.5),
        (0.7, 1.0, 1279.6, 0.1),
        (31.0, -1.0, 400.0, 1133.9),
    ] {
        for along_x in [true, false] {
            let mut row = vec![0.0f32; 700];
            t.row_field(x, y, w, sign * w, along_x, &mut row);
            for (k, &v) in row.iter().enumerate() {
                let d = sign * w * k as f32;
                let want = if along_x { t.field(x + d, y, w) } else { t.field(x, y + d, w) };
                assert!((v - want).abs() < 2e-3, "w {w} sign {sign} x {along_x} from ({x}, {y}), pixel {k}: {v} vs {want}");
            }
        }
    }
}

/// Per-pixel cost of the grain on one thread (`--ignored --nocapture`).
#[test]
#[ignore]
fn pixel_cost_report() {
    let g = CrystalGrain::new(&Grain { amount: 50.0, size: 25.0, roughness: 50.0, seed: 1 }, REF_LONG).expect("grain");
    let (w, h) = (4000usize, 600usize);
    let mut sink = 0.0f32;
    let t0 = std::time::Instant::now();
    for y in 0..h {
        for x in 0..w {
            sink += g.tile.field((x as f32 + 0.5) * 2.0, (y as f32 + 0.5) * 2.0, 2.0);
        }
    }
    let per_pixel = t0.elapsed().as_secs_f64() / (w * h) as f64 * 1e9;
    let mut row = vec![0.0; w];
    let t1 = std::time::Instant::now();
    for y in 0..h {
        g.tile.row_field(1.0, (y as f32 + 0.5) * 2.0, 2.0, 2.0, true, &mut row);
        sink += row[7];
    }
    let per_row = t1.elapsed().as_secs_f64() / (w * h) as f64 * 1e9;
    let t3 = std::time::Instant::now();
    for y in 0..h {
        g.tile.row_field((y as f32 + 0.5) * 2.0, 1.0, 2.0, 2.0, false, &mut row);
        sink += row[7];
    }
    eprintln!("column {:.1} ns/px", t3.elapsed().as_secs_f64() / (w * h) as f64 * 1e9);
    let t2 = std::time::Instant::now();
    for y in 0..h {
        for x in 0..w {
            sink += g.apply_field([0.4, 0.5, 0.6], (x + y) as f32 * 1e-4)[1];
        }
    }
    let tone = t2.elapsed().as_secs_f64() / (w * h) as f64 * 1e9;
    eprintln!("field {per_pixel:.1} ns/px, row {per_row:.1} ns/px, tone {tone:.1} ns/px ({sink})");
}
