//! The rasterizer, sampler and shadow-sizing tests of EffectCraft's
//! `crates/render/src/three_d/adv/tests.rs`, on this crate's API.

use craft_raster3d::{Limits, MAX_SHADOW_TEXELS, TexInfo, sample, shadow_map_size, shadow_point_face_size, shadow_sizes, try_render};

use crate::quad_scene;

#[test]
fn depth_buffer_keeps_the_nearest_surface() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.0, 1.0, 1.0];
    // Blue is listed first but is farther away.
    let s = quad_scene(40, 30, &[([0.0, 0.0, 30.0, 20.0], 500.0, blue), ([10.0, 10.0, 40.0, 30.0], 100.0, red)]);
    let t = try_render(&s, &Limits::default()).unwrap();
    let px = |x: usize, y: usize| t.color[y * 40 + x];
    assert_eq!(px(5, 5), blue);
    assert_eq!(px(15, 15), red, "overlap: the nearer quad wins");
    assert_eq!(px(35, 25), red);
    assert_eq!(px(35, 5), [0.0; 4]);
    assert_eq!(t.depth[15 * 40 + 15], 100.0);
    assert!(t.depth[5 * 40 + 35].is_infinite());
}

#[test]
fn coverage_is_exact_for_pixel_aligned_quads() {
    // Shared diagonal edges are filled once (top-left rule): exact pixel counts.
    let s = quad_scene(32, 32, &[([4.0, 4.0, 20.0, 12.0], 10.0, [1.0, 1.0, 1.0, 0.5])]);
    let t = try_render(&s, &Limits::default()).unwrap();
    let covered = t.color.iter().filter(|c| c[3] > 0.0).count();
    assert_eq!(covered, 16 * 8);
    // Blended once (no double coverage along the diagonal).
    assert!(t.color.iter().all(|c| c[3] == 0.0 || (c[3] - 0.5).abs() < 1e-6));
    // Two transparent layers composite in order.
    let s = quad_scene(8, 8, &[([0.0, 0.0, 8.0, 8.0], 100.0, [1.0, 0.0, 0.0, 0.5]), ([0.0, 0.0, 8.0, 8.0], 50.0, [0.0, 0.0, 1.0, 0.5])]);
    let c = try_render(&s, &Limits::default()).unwrap().color[0];
    assert!((c[0] - 0.25).abs() < 1e-6 && (c[2] - 0.5).abs() < 1e-6 && (c[3] - 0.75).abs() < 1e-6, "{c:?}");
}

#[test]
fn textures_sample_bilinear_with_wrap_modes() {
    let t = TexInfo { offset: 0, width: 2, height: 1, wrap_u: 0, wrap_v: 1 };
    let texels = vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]];
    assert_eq!(sample(&t, &texels, 0.25, 0.5)[0], 0.0);
    assert_eq!(sample(&t, &texels, 0.5, 0.5)[0], 0.5);
    // Repeat wraps the right edge back to texel 0.
    assert!((sample(&t, &texels, 1.0, 0.5)[0] - 0.5).abs() < 1e-6);
    let c = TexInfo { wrap_u: 1, ..t };
    assert_eq!(sample(&c, &texels, 1.0, 0.5)[0], 1.0);
}

#[test]
fn shadow_map_size_tracks_raster_resolution() {
    assert_eq!(shadow_map_size(640, 360, false), 1024);
    assert_eq!(shadow_map_size(1920, 1080, false), 2048);
    assert_eq!(shadow_map_size(3840, 2160, false), 4096);
    // Clamped to the pow2 range: small or absurd rasters stay inside 512..4096.
    assert_eq!(shadow_map_size(64, 64, false), 512);
    assert_eq!(shadow_map_size(0, 0, false), 512);
    assert_eq!(shadow_map_size(u32::MAX, u32::MAX, false), 4096);
    assert_eq!(shadow_map_size(7, 100000, false), 4096);
    // The draft budget stays at 512 regardless.
    assert_eq!(shadow_map_size(3840, 2160, true), 512);
    assert_eq!(shadow_map_size(0, 0, true), 512);
}

#[test]
fn shadow_point_faces_are_capped() {
    // Cube faces take the full map size up to 2048 (one 2048 cube ≈ 25 M f32 texels).
    assert_eq!(shadow_point_face_size(4096, false), 2048);
    assert_eq!(shadow_point_face_size(2048, false), 2048);
    assert_eq!(shadow_point_face_size(1024, false), 1024);
    assert_eq!(shadow_point_face_size(512, false), 512);
    // Draft keeps the half-size face.
    assert_eq!(shadow_point_face_size(512, true), 256);
}

fn budget_texels(sizes: &[Option<u32>], kinds: &[u32]) -> u64 {
    sizes.iter().zip(kinds).map(|(s, &k)| s.map(|z| (z as u64) * (z as u64) * if k == 2 { 6 } else { 1 }).unwrap_or(0)).sum()
}

#[test]
fn shadow_sizes_stay_within_the_scene_budget() {
    // Within budget a light gets its full size: one 4K point cube keeps 2048 faces.
    assert_eq!(shadow_sizes(4096, 2160, false, &[2]), vec![Some(2048)]);
    // Several cubes overflow: deterministically halved (largest first, earliest light
    // on ties) until inside: three or four settle at 1024, five push two to 512.
    assert_eq!(shadow_sizes(4096, 2160, false, &[2; 3]), vec![Some(1024); 3]);
    assert_eq!(shadow_sizes(4096, 2160, false, &[2; 4]), vec![Some(1024); 4]);
    let sizes = shadow_sizes(4096, 2160, false, &[2; 5]);
    assert_eq!(sizes, vec![Some(512), Some(512), Some(1024), Some(1024), Some(1024)]);
    assert!(budget_texels(&sizes, &[2; 5]) <= MAX_SHADOW_TEXELS);
    // A hostile light count can never break the cap; sizes only shrink or drop to None.
    let many = vec![2u32; 1000];
    let sizes = shadow_sizes(3840, 2160, false, &many);
    assert_eq!(sizes.len(), 1000);
    assert!(budget_texels(&sizes, &many) <= MAX_SHADOW_TEXELS);
    assert!(sizes.iter().flatten().all(|&z| (128..=2048).contains(&z)));
    // Deterministic: same inputs, same plan.
    assert_eq!(sizes, shadow_sizes(3840, 2160, false, &many));
    // Non-point lights and a modest scene keep full quality sizes.
    assert_eq!(shadow_sizes(1920, 1080, false, &[1; 2]), vec![Some(2048), Some(2048)]);
    // A light that casts nothing costs nothing: it stays None and leaves its
    // full-size budget to the lights that do cast.
    assert_eq!(shadow_sizes(640, 360, false, &[0, 1]), vec![None, Some(1024)]);
    assert!(shadow_sizes(640, 360, false, &[0; 500]).iter().all(Option::is_none));
}

#[test]
fn shadow_sizes_match_the_halve_largest_rule_and_scale_to_many_lights() {
    // The specification `shadow_sizes` implements, step by step: halve the largest map
    // above its floor (earliest light on ties) until the scene fits, then drop maps
    // from the last light.
    let spec = |w: u32, h: u32, draft: bool, kinds: &[u32]| -> Vec<Option<u32>> {
        let base = shadow_map_size(w, h, draft);
        let mut sizes: Vec<u32> = kinds
            .iter()
            .map(|&k| {
                if k == 0 {
                    0
                } else if k == 2 {
                    shadow_point_face_size(base, draft)
                } else {
                    base
                }
            })
            .collect();
        let total = |s: &[u32]| -> u64 { s.iter().zip(kinds).map(|(&z, &k)| (z as u64) * (z as u64) * if k == 2 { 6 } else { 1 }).sum() };
        while total(&sizes) > MAX_SHADOW_TEXELS {
            let pick =
                (0..sizes.len()).filter(|&i| sizes[i] > if kinds[i] == 2 { 128 } else { 512 }).max_by_key(|&i| (sizes[i], std::cmp::Reverse(i)));
            match pick {
                Some(i) => sizes[i] /= 2,
                None => match sizes.iter().rposition(|&z| z > 0) {
                    Some(i) => sizes[i] = 0,
                    None => break,
                },
            }
        }
        sizes.into_iter().map(|z| (z > 0).then_some(z)).collect()
    };
    let mut seed = 0x2545_F491u32;
    for case in 0..300 {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let n = (seed >> 8) as usize % 40;
        let kinds: Vec<u32> = (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 16) % 3
            })
            .collect();
        let (w, h, draft) = ([640, 1920, 3840, 8000][case % 4], [360, 1080, 2160, 100][case % 4], case % 7 == 0);
        assert_eq!(shadow_sizes(w, h, draft, &kinds), spec(w, h, draft, &kinds), "case {case}: {kinds:?}");
    }
    // A hostile light count stays fast (n log n) and inside the budget.
    let many = vec![2u32; 100_000];
    let sizes = shadow_sizes(3840, 2160, false, &many);
    assert!(budget_texels(&sizes, &many) <= MAX_SHADOW_TEXELS);
}
