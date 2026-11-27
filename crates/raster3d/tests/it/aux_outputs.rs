//! The auxiliary outputs: camera depth, view-space normal and part ID of each pixel's surface.

use craft_raster3d::{
    AuxImage, AuxPlanes, AuxTarget, Limits, Material, NO_PART, RenderError, Scene, TexInfo, Vertex, try_render, try_render_cpu,
    try_render_cpu_with_aux, try_render_with_aux, try_resolve_aux, try_shadow_maps,
};

use crate::{bits, bits4, limits, lit_scene, quad_scene};

/// `s` with part ID `i` for triangle `i`.
fn with_ids(mut s: Scene) -> Scene {
    s.part_ids = (0..s.indices.len() as u32 / 3).collect();
    s
}

/// The quad scenes of `differential.rs`.
fn quad_scenes() -> Vec<(&'static str, Scene)> {
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.0, 1.0, 1.0];
    vec![
        ("overlap", quad_scene(40, 30, &[([0.0, 0.0, 30.0, 20.0], 500.0, blue), ([10.0, 10.0, 40.0, 30.0], 100.0, red)])),
        ("half-alpha quad", quad_scene(32, 32, &[([4.0, 4.0, 20.0, 12.0], 10.0, [1.0, 1.0, 1.0, 0.5])])),
        (
            "two transparent layers",
            quad_scene(8, 8, &[([0.0, 0.0, 8.0, 8.0], 100.0, [1.0, 0.0, 0.0, 0.5]), ([0.0, 0.0, 8.0, 8.0], 50.0, [0.0, 0.0, 1.0, 0.5])]),
        ),
        (
            "past the edges",
            quad_scene(
                24,
                16,
                &[
                    ([-30.0, -5.0, 10.0, 40.0], 300.0, red),
                    ([5.0, 3.0, 90.0, 12.0], 200.0, [0.0, 1.0, 0.0, 0.5]),
                    ([30.0, 0.0, 40.0, 8.0], 1.0, blue),
                ],
            ),
        ),
    ]
}

/// The lit scenes of `differential.rs`, with their shadow maps.
fn lit_scenes() -> Vec<(&'static str, Scene)> {
    let mut out = vec![];
    for (name, out_size, ssaa, ortho, draft) in [
        ("lit perspective", (48, 36), 1, false, false),
        ("lit perspective 2x2", (32, 24), 2, false, false),
        ("lit orthographic draft", (40, 30), 1, true, true),
    ] {
        let mut s = lit_scene(out_size, ssaa, ortho);
        try_shadow_maps(&mut s, draft, &Limits::default()).unwrap();
        out.push((name, s));
    }
    out
}

/// Every plane marks the same pixels as having a surface, and those are the pixels with alpha
/// above 0 in `color`.
fn assert_same_coverage(name: &str, color: &[[f32; 4]], aux: &AuxPlanes) {
    assert_eq!(aux.depth.len(), color.len(), "{name}: depth length");
    assert_eq!(aux.normal.len(), color.len(), "{name}: normal length");
    assert_eq!(aux.part_id.len(), color.len(), "{name}: part length");
    for (i, c) in color.iter().enumerate() {
        let covered = c[3] > 0.0;
        assert_eq!(aux.depth[i].is_finite(), covered, "{name}: depth at {i}");
        assert_eq!(aux.part_id[i] != NO_PART, covered, "{name}: part at {i}");
        let n = aux.normal[i];
        if covered {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-5, "{name}: normal at {i} has length {len}");
        } else {
            assert_eq!(n, [0.0; 3], "{name}: normal at {i}");
        }
    }
}

/// With and without auxiliary outputs, the target, the image and its depth are the same bit for
/// bit, and the auxiliary planes cover the pixels the color covers.
fn check_scene(name: &str, s: &Scene) {
    let s = with_ids(s.clone());
    let t = try_render(&s, &Limits::default()).unwrap();
    let a = try_render_with_aux(&s, &Limits::default()).unwrap();
    assert_eq!(bits4(&a.target.color), bits4(&t.color), "{name}: color");
    assert_eq!(bits(&a.target.depth), bits(&t.depth), "{name}: depth");
    assert!(a.aux.part_id.iter().any(|&p| p != NO_PART), "{name}: nothing drawn");
    assert_same_coverage(name, &a.target.color, &a.aux);
    let (img, _) = try_render_cpu(&s, &Limits::default()).unwrap();
    let r = try_render_cpu_with_aux(&s, &Limits::default()).unwrap();
    assert_eq!((r.image.width, r.image.height), (img.width, img.height), "{name}: image size");
    assert_eq!(bits4(&r.image.data), bits4(&img.data), "{name}: resolved color");
    assert_same_coverage(name, &r.image.data, &r.aux);
}

#[test]
fn every_output_covers_the_pixels_the_color_covers() {
    for (name, s) in quad_scenes().into_iter().chain(lit_scenes()) {
        check_scene(name, &s);
    }
}

#[test]
fn an_empty_scene_has_no_surface() {
    let s = Scene { width: 5, height: 3, ..Scene::default() };
    let a = try_render_with_aux(&s, &Limits::default()).unwrap();
    assert_eq!(a.target, try_render(&s, &Limits::default()).unwrap());
    assert_eq!(a.aux, AuxPlanes { depth: vec![f32::INFINITY; 15], normal: vec![[0.0; 3]; 15], part_id: vec![NO_PART; 15] });
}

/// A perspective camera at (0, 0, −500) looking along +z (+y down), as in `lit_scene`, with one
/// unlit, single-sided material.
fn perspective(w: u32, h: u32) -> Scene {
    let base = lit_scene((w, h), 1, false);
    let mut s = Scene {
        width: w,
        height: h,
        ssaa: 1,
        clip: base.clip,
        view: base.view,
        eye: base.eye,
        cam_fwd: base.cam_fwd,
        linear_io: true,
        ..Scene::default()
    };
    s.materials.push(Material { unlit: true, ..Material::default() });
    s
}

/// Appends a front-facing quad through `c` (clockwise in world x/y with y down, so
/// counterclockwise on the screen) with `normal` and material `m` as two opaque triangles.
fn push_quad(s: &mut Scene, c: [[f32; 3]; 4], normal: [f32; 3], m: u32) {
    let b = s.vertices.len() as u32;
    for p in c {
        s.vertices.push(Vertex { pos: p, normal, uv: [0.0; 2], tangent: [1.0, 0.0, 0.0, 1.0], material: m });
    }
    s.indices.extend([b, b + 2, b + 1, b, b + 3, b + 2]);
}

#[test]
fn depth_rises_along_a_tilted_plane() {
    let mut s = perspective(64, 48);
    // A plane that recedes from z = 0 on the left to z = 600 on the right.
    push_quad(&mut s, [[-300.0, -200.0, 0.0], [300.0, -200.0, 600.0], [300.0, 200.0, 600.0], [-300.0, 200.0, 0.0]], [-0.707, 0.0, -0.707], 0);
    s.opaque_count = s.indices.len() as u32;
    let a = try_render_with_aux(&with_ids(s), &Limits::default()).unwrap();
    let row: Vec<f32> = a.aux.depth[24 * 64..25 * 64].iter().copied().filter(|d| d.is_finite()).collect();
    assert!(row.len() > 40, "{} pixels drawn", row.len());
    assert!(row.windows(2).all(|p| p[1] > p[0]), "{row:?}");
    // Camera depth is the view's z: from 500 (z = 0) toward 1100 (z = 600).
    assert!(row[0] >= 500.0 && row[row.len() - 1] <= 1100.0, "{row:?}");
}

/// A sphere of radius `r` around `c`, single-sided and wound to face outward, with unit
/// outward vertex normals.
fn push_sphere(s: &mut Scene, c: [f32; 3], r: f32, m: u32) {
    let (stacks, slices) = (32u32, 64u32);
    let b = s.vertices.len() as u32;
    for i in 0..=stacks {
        let th = std::f32::consts::PI * i as f32 / stacks as f32;
        for j in 0..=slices {
            let ph = std::f32::consts::TAU * j as f32 / slices as f32;
            let n = [th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin()];
            s.vertices.push(Vertex {
                pos: [c[0] + r * n[0], c[1] + r * n[1], c[2] + r * n[2]],
                normal: n,
                uv: [0.0; 2],
                tangent: [1.0, 0.0, 0.0, 1.0],
                material: m,
            });
        }
    }
    let at = |i: u32, j: u32| b + i * (slices + 1) + j;
    for i in 0..stacks {
        for j in 0..slices {
            for t in [[at(i, j), at(i + 1, j), at(i + 1, j + 1)], [at(i, j), at(i + 1, j + 1), at(i, j + 1)]] {
                let p = t.map(|k| s.vertices[k as usize].pos);
                let (u, v) = ([p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]], [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]]);
                let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                let mid = [0, 1, 2].map(|k| (p[0][k] + p[1][k] + p[2][k]) / 3.0 - c[k]);
                if n == [0.0; 3] {
                    continue;
                }
                let outward = n[0] * mid[0] + n[1] * mid[1] + n[2] * mid[2] > 0.0;
                s.indices.extend(if outward { t } else { [t[0], t[2], t[1]] });
            }
        }
    }
}

/// An orthographic 32 × 32 scene (world x/y = pixels, camera depth = z) holding a sphere of
/// radius 12 around (16, 16, 500), and its expected outward unit normal at a pixel center well
/// inside the outline.
fn sphere_scene() -> (Scene, impl Fn(usize, usize) -> Option<[f32; 3]>) {
    let mut s = quad_scene(32, 32, &[]);
    s.materials.push(Material { unlit: true, ..Material::default() });
    push_sphere(&mut s, [16.0, 16.0, 500.0], 12.0, 0);
    s.opaque_count = s.indices.len() as u32;
    let expected = |x: usize, y: usize| {
        let (dx, dy) = ((x as f32 + 0.5 - 16.0) / 12.0, (y as f32 + 0.5 - 16.0) / 12.0);
        let rr = dx * dx + dy * dy;
        (rr < 0.8 * 0.8).then(|| [dx, dy, -(1.0 - rr).sqrt()])
    };
    (with_ids(s), expected)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[test]
fn sphere_normals_point_outward() {
    let (s, expected) = sphere_scene();
    let a = try_render_with_aux(&s, &Limits::default()).unwrap();
    let mut checked = 0;
    for y in 0..32 {
        for x in 0..32 {
            if let Some(e) = expected(x, y) {
                let n = a.aux.normal[y * 32 + x];
                assert!(dot(n, e) > 0.995, "({x}, {y}): {n:?} against {e:?}");
                checked += 1;
            }
        }
    }
    assert!(checked > 200, "{checked} pixels checked");
}

#[test]
fn normals_are_turned_by_the_view() {
    // The view turns a quarter turn about z: x → y, y → −x. Depth (the third row) is unchanged.
    let (mut s, expected) = sphere_scene();
    s.view = [[0.0, -1.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let a = try_render_with_aux(&s, &Limits::default()).unwrap();
    let e = expected(22, 16).unwrap();
    let n = a.aux.normal[16 * 32 + 22];
    assert!(dot(n, [-e[1], e[0], e[2]]) > 0.995, "{n:?} against {e:?}");
    // A uniform scale in the view does not change the unit normal.
    s.view = [[0.0, -3.0, 0.0, 0.0], [3.0, 0.0, 0.0, 0.0], [0.0, 0.0, 3.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let b = try_render_with_aux(&s, &Limits::default()).unwrap();
    assert!((dot(b.aux.normal[16 * 32 + 22], n) - 1.0).abs() < 1e-6);
}

#[test]
fn a_view_that_stretches_and_mirrors_keeps_normals_perpendicular_to_the_surface() {
    // The view doubles x and mirrors it. A surface's tangents turn by that 3 × 3, so its normal
    // turns by the inverse transpose, diag(−1/2, 1, 1), and keeps facing outward.
    let (mut s, expected) = sphere_scene();
    s.view = [[-2.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let a = try_render_with_aux(&s, &Limits::default()).unwrap();
    let mut checked = 0;
    for y in 0..32 {
        for x in 0..32 {
            if let Some(e) = expected(x, y) {
                let t = [-e[0] / 2.0, e[1], e[2]];
                let len = dot(t, t).sqrt();
                let t = t.map(|v| v / len);
                let n = a.aux.normal[y * 32 + x];
                assert!(dot(n, t) > 0.995, "({x}, {y}): {n:?} against {t:?}");
                checked += 1;
            }
        }
    }
    assert!(checked > 200, "{checked} pixels checked");
}

#[test]
fn a_double_sided_back_face_reports_the_reversed_normal_and_a_zero_normal_reads_as_toward_the_camera() {
    let mut s = quad_scene(8, 8, &[([0.0, 0.0, 8.0, 8.0], 100.0, [1.0; 4])]);
    // quad_scene's quads face the camera; reversing the winding shows their back.
    for t in s.indices.chunks_mut(3) {
        t.swap(1, 2);
    }
    s.vertices.iter_mut().for_each(|v| v.normal = [0.3, 0.0, -0.4]);
    let a = try_render_with_aux(&with_ids(s.clone()), &Limits::default()).unwrap();
    assert!(a.aux.normal.iter().all(|n| dot(*n, [-0.6, 0.0, 0.8]) > 0.99999), "{:?}", a.aux.normal[0]);
    s.vertices.iter_mut().for_each(|v| v.normal = [0.0; 3]);
    let a = try_render_with_aux(&with_ids(s), &Limits::default()).unwrap();
    assert!(a.aux.normal.iter().all(|n| *n == [0.0, 0.0, 1.0]), "{:?}", a.aux.normal[0]);
}

#[test]
fn part_ids_separate_two_meshes_whatever_their_order() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.0, 1.0, 1.0];
    // Blue (far, left) and red (near, right) overlap in x 10 to 30.
    let mut s = quad_scene(40, 30, &[([0.0, 0.0, 30.0, 20.0], 500.0, blue), ([10.0, 10.0, 40.0, 30.0], 100.0, red)]);
    s.part_ids = vec![7, 7, 0, 0];
    let a = try_render_with_aux(&s, &Limits::default()).unwrap();
    let at = |a: &AuxTarget, x: usize, y: usize| (a.aux.part_id[y * 40 + x], a.aux.depth[y * 40 + x]);
    assert_eq!(at(&a, 5, 5), (7, 500.0));
    assert_eq!(at(&a, 20, 15), (0, 100.0));
    assert_eq!(at(&a, 35, 25), (0, 100.0));
    assert_eq!(at(&a, 35, 5), (NO_PART, f32::INFINITY));
    // The same quads in the other order.
    let mut swapped = s.clone();
    swapped.indices = s.indices[6..].iter().chain(&s.indices[..6]).copied().collect();
    swapped.part_ids = vec![0, 0, 7, 7];
    let b = try_render_with_aux(&swapped, &Limits::default()).unwrap();
    assert_eq!(b.aux.part_id, a.aux.part_id);
}

#[test]
fn a_clipped_triangle_keeps_its_part_id() {
    // The lit scene's floor (triangles 0 and 1) crosses the near plane and reaches past the
    // viewport, so it is clipped into several triangles.
    let s = with_ids(lit_scene((48, 36), 1, false));
    let a = try_render_with_aux(&Scene { lights: vec![], env: None, ..s }, &Limits::default()).unwrap();
    for x in 0..48 {
        assert!(a.aux.part_id[35 * 48 + x] <= 1, "bottom row at {x}: {}", a.aux.part_id[35 * 48 + x]);
    }
}

#[test]
fn a_transparent_fragment_that_adds_alpha_is_the_surface() {
    let grey = [0.5, 0.5, 0.5, 1.0];
    let glass = [0.0, 0.0, 1.0, 0.25];
    // Glass in front of an opaque quad on the left half; glass over nothing on the right half.
    let mut s = quad_scene(8, 4, &[([0.0, 0.0, 4.0, 4.0], 300.0, grey), ([0.0, 0.0, 8.0, 4.0], 100.0, glass)]);
    s.part_ids = vec![3, 3, 9, 9];
    let a = try_render_with_aux(&s, &Limits::default()).unwrap();
    assert_eq!((a.aux.part_id[1], a.aux.depth[1]), (9, 100.0));
    assert_eq!((a.aux.part_id[6], a.aux.depth[6]), (9, 100.0));
    // The opaque depth of the target is unchanged.
    assert_eq!((a.target.depth[1], a.target.depth[6]), (300.0, f32::INFINITY));
    // Glass behind the opaque quad is hidden: the opaque quad stays the surface.
    let mut s = quad_scene(8, 4, &[([0.0, 0.0, 4.0, 4.0], 100.0, grey), ([0.0, 0.0, 8.0, 4.0], 300.0, glass)]);
    s.part_ids = vec![3, 3, 9, 9];
    let a = try_render_with_aux(&s, &Limits::default()).unwrap();
    assert_eq!((a.aux.part_id[1], a.aux.part_id[6]), (3, 9));
}

#[test]
fn of_several_transparent_layers_the_nearest_is_the_surface() {
    // Drawn back to front, and also in the wrong order: the nearer layer is the surface either way.
    for (first, second) in [(100.0, 50.0), (50.0, 100.0)] {
        let mut s = quad_scene(8, 8, &[([0.0, 0.0, 8.0, 8.0], first, [1.0, 0.0, 0.0, 0.5]), ([0.0, 0.0, 8.0, 8.0], second, [0.0, 0.0, 1.0, 0.5])]);
        s.part_ids = vec![1, 1, 2, 2];
        let a = try_render_with_aux(&s, &Limits::default()).unwrap();
        let id = if second < first { 2 } else { 1 };
        assert!(a.aux.part_id.iter().all(|&p| p == id), "depths {first} then {second}: {:?}", a.aux.part_id);
        assert!(a.aux.depth.iter().all(|&d| d == 50.0));
    }
}

#[test]
fn a_fragment_that_adds_no_alpha_is_not_the_surface() {
    // A shadows-only card with no light draws nothing (alpha 0) over an opaque quad and over
    // nothing.
    let mut s = quad_scene(8, 4, &[([0.0, 0.0, 4.0, 4.0], 300.0, [0.5, 0.5, 0.5, 1.0])]);
    let m = s.materials.len() as u32;
    s.materials.push(Material { shadows_only: true, double_sided: true, ..Material::default() });
    s.materials[0].unlit = true;
    s.ambient = [0.0; 3];
    let b = s.vertices.len() as u32;
    for (x, y) in [(0.0, 0.0), (8.0, 0.0), (8.0, 4.0), (0.0, 4.0)] {
        s.vertices.push(Vertex { pos: [x, y, 100.0], normal: [0.0, 0.0, -1.0], uv: [0.0; 2], tangent: [1.0, 0.0, 0.0, 1.0], material: m });
    }
    s.indices.extend([b, b + 2, b + 1, b, b + 3, b + 2]);
    s.part_ids = vec![3, 3, 9, 9];
    let a = try_render_with_aux(&s, &Limits::default()).unwrap();
    assert_eq!(bits4(&a.target.color), bits4(&try_render(&s, &Limits::default()).unwrap().color));
    assert_eq!((a.aux.part_id[1], a.aux.depth[1]), (3, 300.0));
    assert_eq!((a.aux.part_id[6], a.aux.depth[6]), (NO_PART, f32::INFINITY));
}

#[test]
fn a_masked_out_fragment_hands_the_surface_to_the_one_behind() {
    // A near opaque quad whose mask texture is clear on its left half, over a far quad.
    let mut s = quad_scene(8, 4, &[([0.0, 0.0, 8.0, 4.0], 100.0, [1.0; 4]), ([0.0, 0.0, 8.0, 4.0], 300.0, [0.5, 0.5, 0.5, 1.0])]);
    s.texels = vec![[1.0, 1.0, 1.0, 0.0], [1.0, 1.0, 1.0, 1.0]];
    s.textures = vec![TexInfo { offset: 0, width: 2, height: 1, wrap_u: 1, wrap_v: 1 }];
    s.materials[0].alpha_mode = 1;
    s.materials[0].tex_base = 0;
    // u runs 0 to 1 across the near quad.
    for (k, v) in s.vertices[..4].iter_mut().enumerate() {
        v.uv = [if k == 1 || k == 2 { 1.0 } else { 0.0 }, 0.0];
        v.normal = [0.0, 0.6, -0.8];
    }
    s.part_ids = vec![5, 5, 6, 6];
    let a = try_render_with_aux(&s, &Limits::default()).unwrap();
    assert_eq!((a.aux.part_id[0], a.aux.depth[0], a.aux.normal[0]), (6, 300.0, [0.0, 0.0, -1.0]));
    assert_eq!((a.aux.part_id[7], a.aux.depth[7]), (5, 100.0));
    assert!(dot(a.aux.normal[7], [0.0, 0.6, -0.8]) > 0.99999);
    assert_eq!(a.target.depth[0], 300.0);
}

#[test]
fn a_zero_area_triangle_has_no_surface() {
    let mut s = quad_scene(8, 8, &[]);
    s.materials.push(Material { unlit: true, double_sided: true, ..Material::default() });
    for p in [[1.0, 1.0, 10.0], [4.0, 4.0, 10.0], [7.0, 7.0, 10.0]] {
        s.vertices.push(Vertex { pos: p, normal: [0.0, 0.0, -1.0], uv: [0.0; 2], tangent: [1.0, 0.0, 0.0, 1.0], material: 0 });
    }
    s.indices = vec![0, 1, 2];
    s.opaque_count = 3;
    s.part_ids = vec![4];
    let a = try_render_with_aux(&s, &Limits::default()).unwrap();
    assert!(a.aux.part_id.iter().all(|&p| p == NO_PART));
    assert!(a.target.color.iter().all(|c| c[3] == 0.0));
}

#[test]
fn a_resolve_copies_the_nearest_subsample_of_each_block_whole() {
    // At 2 × 2, a near quad's edge at x = 5 splits output pixel 2 between it and a far quad.
    let mut s = quad_scene(16, 8, &[([0.0, 0.0, 16.0, 8.0], 500.0, [0.0, 0.0, 1.0, 1.0]), ([5.0, 0.0, 16.0, 8.0], 100.0, [1.0, 0.0, 0.0, 1.0])]);
    s.ssaa = 2;
    s.part_ids = vec![1, 1, 2, 2];
    let t = try_render_with_aux(&s, &Limits::default()).unwrap();
    let r = try_resolve_aux(&t, 2).unwrap();
    assert_eq!(r, try_render_cpu_with_aux(&s, &Limits::default()).unwrap());
    for y in 0..4 {
        for x in 0..8 {
            let block: Vec<usize> = (0..2).flat_map(|j| (0..2).map(move |i| (2 * y + j) * 16 + 2 * x + i)).collect();
            let best = block.iter().copied().min_by(|&a, &b| t.aux.depth[a].total_cmp(&t.aux.depth[b])).unwrap();
            let o = y * 8 + x;
            assert_eq!(
                (r.aux.depth[o], r.aux.normal[o], r.aux.part_id[o]),
                (t.aux.depth[best], t.aux.normal[best], t.aux.part_id[best]),
                "({x}, {y})"
            );
        }
    }
    // Pixel 2 mixes both colors but reports one surface, the near quad, with its depth.
    assert_eq!((r.aux.part_id[2], r.aux.depth[2]), (2, 100.0));
    assert!(r.image.data[2][0] > 0.0 && r.image.data[2][2] > 0.0);
}

#[test]
fn a_resolve_breaks_depth_ties_by_the_first_subsample() {
    let planes = |ids: [u32; 4]| AuxPlanes { depth: vec![7.0; 4], normal: vec![[0.0, 0.0, -1.0]; 4], part_id: ids.to_vec() };
    let t = AuxTarget {
        target: craft_raster3d::Target { width: 2, height: 2, color: vec![[1.0; 4]; 4], depth: vec![7.0; 4] },
        aux: planes([4, 3, 2, 1]),
    };
    let AuxImage { aux, .. } = try_resolve_aux(&t, 2).unwrap();
    assert_eq!(aux.part_id, vec![4]);
}

#[test]
fn a_resolve_rejects_planes_of_the_wrong_length_and_a_bad_factor() {
    let s = with_ids(quad_scene(4, 4, &[([0.0, 0.0, 4.0, 4.0], 10.0, [1.0; 4])]));
    let t = try_render_with_aux(&s, &Limits::default()).unwrap();
    for f in [
        |t: &mut AuxTarget| t.aux.depth.pop().map(drop),
        |t: &mut AuxTarget| t.aux.normal.pop().map(drop),
        |t: &mut AuxTarget| t.aux.part_id.pop().map(drop),
    ] {
        let mut bad = t.clone();
        f(&mut bad);
        assert_eq!(try_resolve_aux(&bad, 2), Err(RenderError::BadDataLength { expected: 16, found: 15 }));
    }
    assert_eq!(try_resolve_aux(&t, 3), Err(RenderError::BadFactor { k: 3, width: 4, height: 4 }));
    assert_eq!(try_resolve_aux(&t, 0), Err(RenderError::BadFactor { k: 0, width: 4, height: 4 }));
}

#[test]
fn part_ids_must_name_every_triangle_with_an_unreserved_id() {
    let s = quad_scene(4, 4, &[([0.0, 0.0, 4.0, 4.0], 10.0, [1.0; 4])]);
    assert_eq!(try_render_with_aux(&s, &Limits::default()), Err(RenderError::BadPartIdCount { expected: 2, found: 0 }));
    let s = Scene { part_ids: vec![1, 2, 3], ..s };
    assert_eq!(try_render_with_aux(&s, &Limits::default()), Err(RenderError::BadPartIdCount { expected: 2, found: 3 }));
    let s = Scene { part_ids: vec![0, NO_PART], ..s };
    assert_eq!(try_render_with_aux(&s, &Limits::default()), Err(RenderError::ReservedPartId(1)));
    assert_eq!(try_render_cpu_with_aux(&s, &Limits::default()), Err(RenderError::ReservedPartId(1)));
    // The renderers without auxiliary outputs do not read them.
    assert!(try_render(&s, &Limits::default()).is_ok());
}

#[test]
fn a_view_that_cannot_turn_normals_is_rejected() {
    let mut s = with_ids(quad_scene(4, 4, &[([0.0, 0.0, 4.0, 4.0], 10.0, [1.0; 4])]));
    s.view = [[1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    assert_eq!(try_render_with_aux(&s, &Limits::default()), Err(RenderError::BadTransform));
    assert!(try_render(&s, &Limits::default()).is_ok());
}

#[test]
fn the_byte_check_counts_the_auxiliary_planes() {
    // The base target's 40 bytes per pixel fit `isize::MAX`, and 60 do not.
    let l = limits(|l| l.max_pixels = u64::MAX);
    #[cfg(target_pointer_width = "64")]
    let (w, h, resolve_h) = (1 << 30, 150_000_000, 100_000_000);
    #[cfg(target_pointer_width = "32")]
    let (w, h, resolve_h) = (1 << 20, 40, 24);
    assert!(l.check(w, h).is_ok());
    let s = Scene { width: w, height: h, ..Scene::default() };
    assert_eq!(try_render_with_aux(&s, &l), Err(RenderError::TooLarge(w, h)));
    assert_eq!(try_render_cpu_with_aux(&s, &l), Err(RenderError::TooLarge(w, h)));
    // The smaller raster fits, and its resolve at 1 × 1 adds 40 bytes per pixel more.
    let s = Scene { width: w, height: resolve_h, ..Scene::default() };
    assert_eq!(try_render_cpu_with_aux(&s, &l), Err(RenderError::TooLarge(w, resolve_h)));
}

#[test]
fn the_auxiliary_pass_obeys_the_fallback_work_limit() {
    let white = [1.0; 4];
    let mut s = with_ids(quad_scene(4, 32, &[([0.0, 0.0, 4.0, 32.0], 100.0, white), ([0.0, 0.0, 4.0, 32.0], 50.0, white)]));
    s.materials[1].alpha_mode = 1;
    s.materials[1].base[3] = 0.0;
    let setup = 4 * (4 * 32 + 1);
    let limited = limits(|l| l.max_work = setup);
    assert!(try_render_with_aux(&s, &limits(|l| l.max_work = u64::MAX)).is_ok());
    assert!(matches!(try_render_with_aux(&s, &limited), Err(RenderError::TooMuchWork { limit, needed }) if limit == setup && needed > setup));
    assert!(matches!(try_render_cpu_with_aux(&s, &limited), Err(RenderError::TooMuchWork { limit, needed }) if limit == setup && needed > setup));
    s.materials[1].base[3] = 1.0;
    assert!(try_render_with_aux(&s, &limited).is_ok());
    assert!(try_render_cpu_with_aux(&s, &limited).is_ok());
}
