use super::*;

/// Daylight-like multipliers: green clips first, red and blue keep rising.
const WB: [f32; 3] = [2.5, 1.0, 1.6];

fn balanced(p: [f32; 3]) -> [f32; 3] {
    [p[0] * WB[0], p[1] * WB[1], p[2] * WB[2]]
}

fn sensor(q: [f32; 3]) -> [f32; 3] {
    [q[0] / WB[0], q[1] / WB[1], q[2] / WB[2]].map(|v| v.min(1.0))
}

fn chroma(q: [f32; 3]) -> [f32; 3] {
    let s = q[0] + q[1] + q[2];
    q.map(|v| v / s)
}

/// A pale blue sky whose green channel clipped, an unclipped strip of the same sky along the top, and a dark
/// branch with purple fringes on both sides standing in the clipped part (white-balanced sky colour returned).
fn branch_scene() -> (Rgb32f, [f32; 3]) {
    let sky = [1.10, 1.25, 1.45];
    let img = Rgb32f::from_fn(192, 128, |x, y| {
        sensor(if y < 24 {
            sky.map(|v| v * 0.7)
        } else if (90..100).contains(&x) && y >= 40 {
            [0.06, 0.05, 0.03]
        } else if (x == 89 || x == 100) && y >= 40 {
            [0.75, 0.25, 0.96]
        } else {
            sky
        })
    });
    (img, sky)
}

/// How far chromaticity `got` lies off the way from `from` to neutral (largest channel difference to the
/// nearest point of that way).
fn off_the_way_to_neutral(got: [f32; 3], from: [f32; 3]) -> f32 {
    let d: [f32; 3] = std::array::from_fn(|c| 1.0 / 3.0 - from[c]);
    let len = d.iter().map(|v| v * v).sum::<f32>();
    let t = if len > 0.0 { ((0..3).map(|c| (got[c] - from[c]) * d[c]).sum::<f32>() / len).clamp(0.0, 1.0) } else { 0.0 };
    (0..3).map(|c| (got[c] - (from[c] + d[c] * t)).abs()).fold(0.0, f32::max)
}

/// Issue #523: the clipped sky beside a branch took the colour of the branch's fringes and of the branch
/// itself (magenta, green), which any darkening of the highlights then showed. Beside the branch it is now the
/// clipped sky away from it: the sky's colour, on its way to neutral.
#[test]
fn fringes_and_dark_branches_do_not_tint_a_clipped_sky() {
    let (mut img, sky) = branch_scene();
    assert!(reconstruct(&mut img, WB, 0.99) > 0);
    let mut worst = 0f32;
    for y in 40..128 {
        let away = chroma(balanced(img.get(30, y)));
        assert!(away == chroma(balanced(img.get(170, y))), "row {y}");
        let off = off_the_way_to_neutral(away, chroma(sky));
        assert!(off < 0.005, "row {y}: the clipped sky {away:?} is not the sky's colour on its way to neutral ({off})");
        for x in [84, 86, 88, 101, 103, 105] {
            let got = chroma(balanced(img.get(x, y)));
            worst = worst.max((0..3).map(|c| (got[c] - away[c]).abs()).fold(0.0, f32::max));
        }
    }
    assert!(worst < 0.01, "clipped sky beside the branch is off the sky away from it by {worst}");
}

/// A warm light with red and green clipped: left at their clip levels the pair reads red-magenta (the white
/// balance multiplies red's clip level by 2.5, green's by 1).
#[test]
fn two_clipped_channels_are_not_left_at_their_clip_levels() {
    let light = [3.5, 2.6, 0.8];
    let mut img = Rgb32f::from_fn(64, 64, |x, y| {
        let d = ((x as f32 - 32.0).powi(2) + (y as f32 - 32.0).powi(2)).sqrt();
        if d < 20.0 { sensor(light) } else { [0.02; 3] }
    });
    reconstruct(&mut img, WB, 0.99);
    let q = balanced(img.get(32, 32));
    assert!((q[2] - 0.8).abs() < 0.1, "the unclipped channel is about kept: {q:?}");
    let ratio = q[1] / q[0];
    assert!((0.6..=1.05).contains(&ratio), "green / red {ratio} ({q:?}): not a warm white");
}

#[test]
fn fully_clipped_pixels_follow_the_colour_model_white() {
    let white = [1.03, 0.98, 1.06];
    let mut img = Rgb32f::filled(8, 8, [1.0; 3]);
    img.data[0] = [0.3, 0.6, 0.4];
    reconstruct_with(&mut img, WB, 0.99, white);
    let q = balanced(img.get(4, 4));
    let k = q[1] / white[1];
    assert!((0..3).all(|c| (q[c] - white[c] * k).abs() < 1e-4 * k), "{q:?} is not along {white:?}");
    assert!(k >= 2.5 * 0.99 - 1e-4, "clipped neutral at least as bright as the highest clip level: {k}");
    assert_eq!(img.data[0], [0.3, 0.6, 0.4]);
    // a white that cannot be right falls back to camera neutral, and nothing overflows
    for bad in [[f32::NAN, 1.0, 1.0], [2.5e38, 0.74, 0.74], [1.0, 0.0, 1.0], [40.0, 1.0, 1.0]] {
        let mut img = Rgb32f::filled(4, 4, [1.0; 3]);
        img.data[0] = [1.0, 1.0, 0.5];
        reconstruct_with(&mut img, WB, 0.99, bad);
        let q = balanced(img.get(1, 1));
        assert!((q[0] - q[1]).abs() < 1e-4 && (q[2] - q[1]).abs() < 1e-4, "{bad:?}: {q:?}");
        assert!(img.data.iter().flatten().all(|v| v.is_finite()), "{bad:?}");
    }
}

/// Red rises through its clip level across a sky whose green and blue clipped: the rebuilt colour changes
/// smoothly (no edge where red clips too) and arrives at the clipped neutral.
#[test]
fn rebuilt_colour_is_continuous_as_more_channels_clip() {
    let blue = [1.0, 1.3, 1.9];
    let img0 = Rgb32f::from_fn(240, 40, |x, y| {
        if y < 8 {
            return sensor(blue.map(|v| v * 0.5)); // unclipped sky: the source of colour
        }
        let level = 0.6 + 2.2 * x as f32 / 239.0; // red at 0.24 to 1.12 of its clip level
        sensor([level * WB[0] * 0.4, 1.6, 2.0])
    });
    let mut img = img0.clone();
    reconstruct(&mut img, WB, 0.99);
    let row: Vec<[f32; 3]> = (0..240).map(|x| chroma(balanced(img.get(x, 30)))).collect();
    let step = row.windows(2).map(|w| (0..3).map(|c| (w[1][c] - w[0][c]).abs()).fold(0.0, f32::max)).fold(0.0, f32::max);
    assert!(step < 0.01, "largest chromaticity step between neighbours {step}");
    let end = row[239];
    assert!(end.iter().all(|v| (v - 1.0 / 3.0).abs() < 1e-3), "fully clipped end {end:?}");
}

/// A smooth pink ramp (`[0.9k, k, 0.8k]`, k from 0.8 to 1.35) whose green clips first, then red, then blue,
/// in `rows` of a `h`-row picture that is otherwise near black: the rebuilt middle row, and where green first
/// clips on it.
fn rebuilt_ramp(rows: std::ops::Range<usize>, h: usize) -> (Vec<[f32; 3]>, usize) {
    let (w, k0, k1) = (1600usize, 0.8f32, 1.35f32);
    let k = |x: usize| k0 + (k1 - k0) * x as f32 / (w - 1) as f32;
    let img0 = Rgb32f::from_fn(w, h, |x, y| if rows.contains(&y) { [0.9 * k(x), k(x), 0.8 * k(x)].map(|v| v.min(1.0)) } else { [0.01; 3] });
    let mut img = img0.clone();
    reconstruct(&mut img, WB, 0.99);
    let y = (rows.start + rows.end) / 2;
    let row: Vec<[f32; 3]> = (0..w).map(|x| balanced(img.get(x, y))).collect();
    (row, (0..w).find(|&x| img0.get(x, y)[1] >= 0.99).unwrap_or(0))
}

/// No channel changes by more than `at_clip` (relative) where green first clips, nor by 3 % anywhere.
fn assert_continuous(row: &[[f32; 3]], first: usize, at_clip: f32, case: &str) {
    // across the first clip the colour carries on as it was (the sensor's own colour, not a fade)
    let (a, b) = (row[first - 1], row[first]);
    assert!((0..3).all(|c| (b[c] / a[c] - 1.0).abs() < at_clip), "{case}, first clip: {a:?} then {b:?}");
    let worst = row.windows(2).map(|p| (0..3).map(|c| (p[1][c] / p[0][c] - 1.0).abs()).fold(0.0, f32::max)).fold(0.0, f32::max);
    // (a smooth ramp moves 0.04 % per pixel here; the rebuilt colour turns faster where it leaves the colour of
    // the unclipped part for neutral, but never by a step)
    assert!(worst < 0.03, "{case}: largest relative step between neighbours {worst}");
}

/// The rebuilt colour has no step anywhere along the ramp: not where green first clips (the neutral fade starts
/// from nothing there), nor where red does, nor where the last channel clips (the fade ends at the fully
/// clipped neutral).
#[test]
fn rebuilt_ramp_is_continuous_from_the_first_clip_to_the_last() {
    let (row, first) = rebuilt_ramp(0..32, 32);
    assert_continuous(&row, first, 0.002, "full ramp");
}

/// The same ramp as a thin line on near black has no reliable colour around (every pixel of it is on a strong
/// edge), so its clipped part falls back to the neutral guess. That guess only applies as far as the line runs
/// past its own unclipped part: no step where it first clips, however thin the line and however much dark
/// background surrounds it.
#[test]
fn thin_line_without_reliable_colour_is_continuous_where_it_clips() {
    for height in [32usize, 512, 1024] {
        for width in [1usize, 4, 16] {
            let top = (height - width) / 2;
            let (row, first) = rebuilt_ramp(top..top + width, height);
            assert_continuous(&row, first, 0.002, &format!("{width} rows of {height}"));
            // and well past the first clip the neutral guess has taken over (green as high as red and blue imply)
            let far = row[1500];
            assert!(far[1] > 0.95 * far[2], "{width} rows of {height}: {far:?}");
        }
    }
}

/// Issue #523 (a river photo darkened by Highlights -75): a pale cyan-blue sky (`[1, 1.07, 1.2]` white-balanced)
/// brightening from left to right through the point where green clips (and later blue), above bright
/// yellow-green foliage, with thin dark branches standing in its lower half. Its rebuilt colour used to lean
/// green to teal: the foliage lent its colour to the sky (green rebuilt from it), with a step where green first
/// clips, and the sky kept its own pale colour where Lightroom shows white.
#[test]
fn pale_sky_by_dark_branches_and_foliage_is_not_tinted() {
    let sky = [1.0, 1.07, 1.2];
    let (w, h) = (320usize, 96usize);
    let k = |x: usize| 0.7 * (1.2 * x as f32 / (w - 1) as f32).exp2();
    let branches = [150usize, 200, 250];
    let on_branch = |x: usize| branches.iter().any(|&b| (b.saturating_sub(1)..=b + 2).contains(&x));
    let img0 = Rgb32f::from_fn(w, h, |x, y| {
        sensor(if y >= 72 {
            [0.5, 0.6, 0.22]
        } else if y >= 40 && branches.iter().any(|&b| x == b || x == b + 1) {
            [0.1, 0.09, 0.04]
        } else {
            sky.map(|v| v * k(x))
        })
    });
    let mut img = img0.clone();
    reconstruct(&mut img, WB, 0.99);
    let at = |x: usize, y: usize| chroma(balanced(img.get(x, y)));
    let first = (0..w).find(|&x| img0.get(x, 20)[1] >= 0.99).unwrap_or(0);
    assert!(first > 0 && first < 150);
    let diff = |a: [f32; 3], b: [f32; 3]| (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f32::max);
    for y in [20, 50] {
        // no step where green clips
        let step = diff(at(first - 1, y), at(first, y));
        assert!(step < 0.001, "row {y}: step of {step} where green clips");
        for x in first..w {
            // nothing but the sky's colour on its way to neutral (no green from the foliage)...
            let off = off_the_way_to_neutral(at(x, y), chroma(sky));
            assert!(on_branch(x) || off < 0.005, "row {y}, x {x}: {:?} is off the sky's way to neutral by {off}", at(x, y));
            // ... the same beside the branches as above them ...
            let beside = diff(at(x, y), at(x, 20));
            assert!(on_branch(x) || beside < 0.005, "row {y}, x {x}: {:?} beside a branch, {:?} above", at(x, y), at(x, 20));
        }
        // ... and neutral well before the last channel clips (the sky is 0.7 stops past green's clip there)
        let end = at(w - 1, y);
        assert!(end.iter().all(|v| (v - 1.0 / 3.0).abs() < 0.002), "row {y}: {end:?} at the end");
    }
}

#[test]
fn unclipped_pixels_are_left_exactly() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 40) as f32 / (1u64 << 24) as f32
    };
    for (w, h) in [(1, 1), (3, 1), (7, 5), (64, 48), (129, 77)] {
        let noise: Vec<f32> = (0..w * h).map(|_| rnd()).collect();
        let img = Rgb32f::from_fn(w, h, |x, y| {
            let t = ((x as f32 * 0.21).sin() + (y as f32 * 0.17).cos()) * 0.5;
            let base = [0.5 + 0.4 * t, 0.45 - 0.2 * t, 0.3 + 0.1 * t];
            match noise[y * w + x] {
                r if r < 0.1 => [1.0; 3],
                r if r < 0.4 => [1.0, base[1] * 1.6, base[2]],
                r if r > 0.995 => [0.0, 0.0, 0.00001],
                _ => base,
            }
        });
        let mut out = img.clone();
        let n = reconstruct_with(&mut out, [2.1, 1.0, 1.6], 0.99, [1.02, 0.99, 1.05]);
        let clipped = img.data.iter().filter(|p| p.iter().any(|&v| v >= 0.99)).count();
        assert_eq!(n, clipped);
        for (a, b) in img.data.iter().zip(&out.data) {
            if a.iter().all(|&v| v < 0.99) {
                assert_eq!(a, b, "{w}×{h}");
            } else {
                assert!(b.iter().all(|v| v.is_finite() && *v >= 0.0), "{w}×{h}: {b:?}");
            }
        }
    }
}

#[test]
fn clip_neutral_makes_clipped_white_neutral() {
    let wb = [2.0, 1.0, 1.5];
    let mut img = Rgb32f::filled(2, 1, [1.0, 1.0, 1.0]);
    img.data[1] = [0.2, 0.3, 0.4];
    clip_neutral(&mut img, wb, 1.0);
    let p = img.data[0];
    let q = [p[0] * wb[0], p[1] * wb[1], p[2] * wb[2]];
    assert!((q[0] - q[1]).abs() < 1e-6 && (q[1] - q[2]).abs() < 1e-6);
    assert_eq!(img.data[1], [0.2, 0.3, 0.4]);
}

#[test]
fn reconstruct_restores_clipped_channel() {
    // a warm gradient whose red channel clips at its bright end
    let truth = Rgb32f::from_fn(256, 16, |x, _| {
        let v = 0.3 + 1.0 * x as f32 / 255.0;
        [v * 1.0, v * 0.6, v * 0.35]
    });
    let mut img = truth.map(|p| p.map(|v| v.min(1.0)));
    let first = (0..256).find(|&x| img.get(x, 8)[0] >= 0.999).unwrap();
    // near the unclipped gradient (within the reach of its colour) red is rebuilt from it
    let err = |img: &Rgb32f| -> f32 { (first..first + 12).map(|x| (img.get(x, 8)[0] - truth.get(x, 8)[0]).abs()).sum() };
    let before = err(&img);
    let n = reconstruct(&mut img, [1.0, 1.0, 1.0], 0.999);
    assert!(n > 0);
    let after = err(&img);
    assert!(after < before * 0.2, "before {before} after {after}");
    // unclipped pixels untouched
    assert_eq!(img.get(0, 0), truth.get(0, 0));
    assert_eq!(img.get(first - 1, 8), truth.get(first - 1, 8));
}

#[test]
fn fully_clipped_and_no_clipping() {
    let mut img = Rgb32f::filled(4, 4, [1.0; 3]);
    assert_eq!(reconstruct(&mut img, [2.0, 1.0, 1.5], 0.99), 16);
    let p = img.data[0];
    assert!((p[0] * 2.0 - p[1]).abs() < 1e-5 && (p[2] * 1.5 - p[1]).abs() < 1e-5);
    let mut img = Rgb32f::filled(4, 4, [0.5; 3]);
    assert_eq!(reconstruct(&mut img, [2.0, 1.0, 1.5], 0.99), 0);
    let mut empty = Rgb32f::new(0, 0);
    assert_eq!(reconstruct(&mut empty, [1.0; 3], 0.99), 0);
}

/// `cargo test --release -p lightcraft-raw --lib -- --ignored bench_reconstruct --nocapture`
#[test]
#[ignore]
fn bench_reconstruct() {
    let img = Rgb32f::from_fn(6000, 4000, |x, y| {
        let v = 0.4 + 0.8 * ((x as f32 * 0.003).sin() * (y as f32 * 0.002).cos()).abs();
        [v.min(1.0), (v * 0.7).min(1.0), (v * 0.5).min(1.0)]
    });
    let ms = (0..3)
        .map(|_| {
            let mut i = img.clone();
            let t = std::time::Instant::now();
            reconstruct(&mut i, [2.0, 1.0, 1.5], 0.99);
            t.elapsed().as_secs_f64() * 1e3
        })
        .fold(f64::MAX, f64::min);
    eprintln!("reconstruct 24 MP: {ms:.0} ms");
}

/// A mask decides which channels are clipped (their values are lower bounds), whatever their values; the
/// unmarked pixels are left as they were; a mask of another size leaves clipping to the values.
#[test]
fn a_mask_marks_the_clipped_channels() {
    let base = Rgb32f::from_fn(32, 32, |x, _| if x >= 16 { [0.2, 0.5, 0.25] } else { [0.5, 0.5, 0.5] });
    let mask: Vec<u8> = (0..32 * 32).map(|i| if i % 32 >= 16 { CLIPPED_R } else { 0 }).collect();
    let mut img = base.clone();
    assert_eq!(reconstruct_masked(&mut img, [1.0; 3], 0.99, [1.0; 3], &mask), 512);
    for (p, b) in img.data.iter().zip(&base.data) {
        assert!(p.iter().all(|v| v.is_finite()), "{p:?}");
        if b[0] == 0.5 {
            assert_eq!(p, b);
        } else {
            // red rebuilt from at least its mean, the others kept
            assert!(p[0] >= 0.2 && p[1] == 0.5 && p[2] == 0.25, "{p:?}");
        }
    }
    // red is rebuilt above its mean, towards the neighbours' neutral (as far as the pixel's own blue agrees)
    assert!(img.get(24, 16)[0] > 0.3, "{:?}", img.get(24, 16));
    // the values alone see nothing clipped here, nor does a mask of another size
    let mut plain = base.clone();
    assert_eq!(reconstruct(&mut plain, [1.0; 3], 0.99), 0);
    assert_eq!(reconstruct_masked(&mut plain, [1.0; 3], 0.99, [1.0; 3], &mask[1..]), 0);
    assert!(plain.data == base.data);
}
