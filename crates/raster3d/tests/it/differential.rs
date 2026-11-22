//! This crate against the app's previous code (`crate::reference`), on the same scenes. For the
//! scenes in this module, every shadow texel, raster pixel, depth, resolved pixel and encoded
//! pixel matches bit for bit; the README's Origin section lists the scenes that differ. Each test
//! prints the 64-bit FNV-1a hash of each output from both sides.

use craft_raster3d::{Limits, Scene, try_encode, try_render, try_render_cpu, try_resolve, try_shadow_maps};

use crate::reference::effectcraft as ec;
use crate::{bits, bits4, fnv, lit_scene, quad_scene};

/// Runs both sides on `scene`: shadow maps (when `shadows`), the raster, the resolve and the
/// encoding, and asserts that every output matches bit for bit.
fn compare(name: &str, scene: &Scene, shadows: Option<bool>) {
    let (mut ours, mut theirs) = (scene.clone(), scene.clone());
    if let Some(draft) = shadows {
        try_shadow_maps(&mut ours, draft, &Limits::default()).unwrap();
        ec::raster::shadow_maps(&mut theirs, draft);
        assert_eq!(ours.shadows, theirs.shadows, "{name}: shadow maps");
        assert_eq!(bits(&ours.shadow_texels), bits(&theirs.shadow_texels), "{name}: shadow texels");
        assert_eq!(ours.lights, theirs.lights, "{name}: light marks");
        assert_eq!(ours.env, theirs.env, "{name}: environment mark");
        println!("{name} shadow texels: ours {:016x} theirs {:016x}", fnv(&[], &ours.shadow_texels), fnv(&[], &theirs.shadow_texels));
    }
    let t = try_render(&ours, &Limits::default()).unwrap();
    let r = ec::raster::render(&theirs);
    assert_eq!((t.width, t.height), (r.width, r.height));
    assert_eq!(bits4(&t.color), bits4(&r.color), "{name}: raster color");
    assert_eq!(bits(&t.depth), bits(&r.depth), "{name}: raster depth");
    println!("{name} raster: ours {:016x} theirs {:016x}", fnv(&t.color, &t.depth), fnv(&r.color, &r.depth));
    let k = ours.ssaa.max(1);
    let (img, depth) = try_resolve(&t, k).unwrap();
    let (rimg, rdepth) = ec::resolve(&r, k);
    assert_eq!((img.width, img.height), (rimg.width, rimg.height));
    assert_eq!(bits4(&img.data), bits4(&rimg.data), "{name}: resolved color");
    assert_eq!(bits(&depth), bits(&rdepth), "{name}: resolved depth");
    let (cpu, cpu_depth) = try_render_cpu(&ours, &Limits::default()).unwrap();
    let (rcpu, rcpu_depth) = ec::render_cpu(&theirs);
    assert_eq!(bits4(&cpu.data), bits4(&rcpu.data), "{name}: render_cpu color");
    assert_eq!(bits(&cpu_depth), bits(&rcpu_depth), "{name}: render_cpu depth");
    println!("{name} resolved: ours {:016x} theirs {:016x}", fnv(&img.data, &depth), fnv(&rimg.data, &rdepth));
    let (mut enc, mut renc) = (img, rimg);
    try_encode(&mut enc).unwrap();
    ec::encode(&mut renc);
    assert_eq!(bits4(&enc.data), bits4(&renc.data), "{name}: encoded");
    println!("{name} encoded: ours {:016x} theirs {:016x}", fnv(&enc.data, &[]), fnv(&renc.data, &[]));
}

#[test]
fn quad_scenes_match_the_previous_renderer() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.0, 1.0, 1.0];
    compare("overlap", &quad_scene(40, 30, &[([0.0, 0.0, 30.0, 20.0], 500.0, blue), ([10.0, 10.0, 40.0, 30.0], 100.0, red)]), None);
    compare("half-alpha quad", &quad_scene(32, 32, &[([4.0, 4.0, 20.0, 12.0], 10.0, [1.0, 1.0, 1.0, 0.5])]), None);
    compare(
        "two transparent layers",
        &quad_scene(8, 8, &[([0.0, 0.0, 8.0, 8.0], 100.0, [1.0, 0.0, 0.0, 0.5]), ([0.0, 0.0, 8.0, 8.0], 50.0, [0.0, 0.0, 1.0, 0.5])]),
        None,
    );
    // Quads reaching past the viewport (inside the guard band) and one off it.
    compare(
        "past the edges",
        &quad_scene(
            24,
            16,
            &[([-30.0, -5.0, 10.0, 40.0], 300.0, red), ([5.0, 3.0, 90.0, 12.0], 200.0, [0.0, 1.0, 0.0, 0.5]), ([30.0, 0.0, 40.0, 8.0], 1.0, blue)],
        ),
        None,
    );
}

/// The lit scene covers over a quarter of the pixels in more than 100 colors, with shadow-map texels covered by
/// casters, so a match is not a match of empty outputs.
fn assert_drawn(s: &Scene, draft: bool) {
    let mut s = s.clone();
    try_shadow_maps(&mut s, draft, &Limits::default()).unwrap();
    assert!(s.shadow_texels.iter().filter(|t| t.is_finite()).count() > 1000);
    let t = try_render(&s, &Limits::default()).unwrap();
    let covered = t.color.iter().filter(|c| c[3] > 0.0).count();
    assert!(covered * 4 > t.color.len(), "covered {covered} of {}", t.color.len());
    let mut colors: Vec<[u32; 4]> = bits4(&t.color);
    colors.sort_unstable();
    colors.dedup();
    assert!(colors.len() > 100, "{} colors", colors.len());
}

#[test]
fn lit_perspective_scene_matches_the_previous_renderer() {
    assert_drawn(&lit_scene((48, 36), 1, false), false);
    compare("lit perspective", &lit_scene((48, 36), 1, false), Some(false));
}

#[test]
fn lit_supersampled_scene_matches_the_previous_renderer() {
    compare("lit perspective 2x2", &lit_scene((32, 24), 2, false), Some(false));
}

#[test]
fn lit_orthographic_draft_scene_matches_the_previous_renderer() {
    assert_drawn(&lit_scene((40, 30), 1, true), true);
    compare("lit orthographic draft", &lit_scene((40, 30), 1, true), Some(true));
}

#[test]
fn lit_scene_without_shadow_maps_matches_the_previous_renderer() {
    // No maps: every mark is −1, as the app sets them when shadows are off.
    let mut s = lit_scene((40, 30), 1, false);
    s.env.as_mut().unwrap().shadow = -1;
    for l in &mut s.lights {
        l.shadow = -1;
    }
    compare("lit without maps", &s, None);
    for m in &mut s.materials {
        m.casts_shadows = false;
    }
    // No casters, with every light and the environment light marked 0: every mark is cleared.
    let s = Scene { materials: s.materials, ..lit_scene((40, 30), 1, false) };
    compare("lit without casters", &s, Some(false));
}
