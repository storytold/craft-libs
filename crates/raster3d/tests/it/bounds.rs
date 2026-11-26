//! Hostile and malformed input: every entry point returns an error or a defined result before it
//! allocates the target or indexes past an array, and never panics.

use craft_raster3d::{
    EnvInfo, Image, Light, Limits, Material, RenderError, Scene, Target, TexInfo, Vertex, linear_to_srgb, sample, srgb_to_linear, try_encode,
    try_env_shadow_map, try_render, try_render_cpu, try_resolve, try_shadow_maps,
};

use crate::{bits4, limits, lit_scene, quad_scene};

const WHITE: [f32; 4] = [1.0; 4];

fn one_quad() -> Scene {
    quad_scene(8, 8, &[([1.0, 1.0, 7.0, 7.0], 100.0, WHITE)])
}

fn target(w: u32, h: u32, color: Vec<[f32; 4]>, depth: Vec<f32>) -> Target {
    Target { width: w, height: h, color, depth }
}

#[test]
fn an_empty_scene_renders_transparent_with_infinite_depth() {
    let s = Scene { width: 5, height: 3, ..Scene::default() };
    let t = try_render(&s, &Limits::default()).unwrap();
    assert_eq!((t.width, t.height, t.color.len(), t.depth.len()), (5, 3, 15, 15));
    assert!(t.color.iter().all(|c| *c == [0.0; 4]));
    assert!(t.depth.iter().all(|d| d.is_infinite()));
    let mut s = s;
    try_shadow_maps(&mut s, false, &Limits::default()).unwrap();
    assert!(s.shadows.is_empty() && s.shadow_texels.is_empty());
}

#[test]
fn zero_and_oversized_rasters_are_rejected_before_allocating() {
    for (w, h) in [(0, 8), (8, 0), (0, 0)] {
        let s = Scene { width: w, height: h, ..one_quad() };
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadSize(w, h)));
    }
    let small = limits(|l| l.max_pixels = 63);
    assert_eq!(try_render(&one_quad(), &small), Err(RenderError::TooLarge(8, 8)));
    let s = Scene { width: u32::MAX, height: u32::MAX, ..one_quad() };
    assert_eq!(try_render(&s, &limits(|l| l.max_pixels = u64::MAX)), Err(RenderError::TooLarge(u32::MAX, u32::MAX)));
    assert!(matches!(try_render_cpu(&s, &Limits::default()), Err(RenderError::TooLarge(..))));
}

#[test]
fn malformed_index_lists_are_rejected() {
    let mut s = one_quad();
    s.indices.pop();
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadIndexCount(5)));
    let mut s = one_quad();
    s.opaque_count = 4;
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadOpaqueCount { opaque: 4, len: 6 }));
    s.opaque_count = 9;
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadOpaqueCount { opaque: 9, len: 6 }));
    let mut s = one_quad();
    s.indices[4] = 99;
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadIndex { position: 4, index: 99 }));
    assert_eq!(try_shadow_maps(&mut s, false, &Limits::default()), Err(RenderError::BadIndex { position: 4, index: 99 }));
}

#[test]
fn a_shadows_only_triangle_in_the_opaque_range_is_rejected() {
    // Two opaque quads, triangles 0 and 1 then 2 and 3; the second becomes shadows-only.
    let mut s = quad_scene(8, 8, &[([1.0, 1.0, 7.0, 7.0], 100.0, WHITE), ([0.0, 0.0, 8.0, 8.0], 50.0, WHITE)]);
    s.materials[1].shadows_only = true;
    assert_eq!(s.opaque_count, 12);
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::ShadowsOnlyOpaque(2)));
    assert!(matches!(try_render_cpu(&s, &Limits::default()), Err(RenderError::ShadowsOnlyOpaque(2))));
    // A shadows-only material that is not drawn may stay in the opaque range.
    let mut hidden = s.clone();
    hidden.materials[1].visible = false;
    assert!(try_render(&hidden, &Limits::default()).is_ok());
    // In the transparent range the same triangles draw.
    s.opaque_count = 6;
    assert!(try_render(&s, &Limits::default()).is_ok());
}

#[test]
fn vertices_naming_missing_materials_are_rejected() {
    let mut s = one_quad();
    s.vertices[2].material = 7;
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadVertex(2)));
    // The shadow pass indexed the material of a caster's first vertex directly.
    let mut s = lit_scene((16, 12), 1, false);
    let first = s.indices[0] as usize;
    s.vertices[first].material = 99;
    assert_eq!(try_shadow_maps(&mut s, false, &Limits::default()), Err(RenderError::BadVertex(first)));
}

#[test]
fn values_that_are_not_finite_are_rejected() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut s = one_quad();
        s.vertices[1].pos[0] = bad;
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadVertex(1)), "{bad} position");
        let mut s = one_quad();
        s.clip[0][3] = bad;
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadTransform), "{bad} clip");
        let mut s = one_quad();
        s.materials[0].roughness = bad;
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadMaterial(0)), "{bad} roughness");
        let mut s = lit_scene((16, 12), 1, false);
        s.lights[1].color[2] = bad;
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadLight(1)), "{bad} light");
        let mut s = lit_scene((16, 12), 1, false);
        s.env.as_mut().unwrap().sun[0] = bad;
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadEnvironment), "{bad} environment");
        let mut s = lit_scene((16, 12), 1, false);
        s.texels[3][1] = bad;
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadTexel(3)), "{bad} texel");
    }
    // A finite position the clip transform takes past f32.
    let mut s = one_quad();
    s.vertices[0].pos[0] = 3e38;
    s.clip[0][0] = 10.0;
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadClipPosition(0)));
}

#[test]
fn alpha_out_of_range_in_a_material_is_rejected() {
    for (base, opacity) in [(1.5, 1.0), (-0.1, 1.0), (1.0, 2.0), (1.0, -1.0)] {
        let mut s = one_quad();
        s.materials[0].base[3] = base;
        s.materials[0].opacity = opacity;
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadMaterial(0)), "{base} {opacity}");
    }
}

#[test]
fn texture_references_and_spans_are_checked() {
    // A masked caster's base texture was indexed directly by the shadow pass.
    let mut s = lit_scene((16, 12), 1, false);
    s.materials[2].tex_base = 50;
    assert_eq!(try_shadow_maps(&mut s, false, &Limits::default()), Err(RenderError::BadMaterial(2)));
    let mut s = lit_scene((16, 12), 1, false);
    s.textures[4].width = 0;
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadTexture(4)));
    let mut s = lit_scene((16, 12), 1, false);
    s.texels.truncate(s.texels.len() - 1);
    let last = s.textures.len() - 1;
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadTexture(last)));
    let mut s = lit_scene((16, 12), 1, false);
    s.textures[0].offset = u32::MAX;
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadTexture(0)));
}

#[test]
fn environment_level_ranges_that_overflow_are_rejected() {
    let mut s = lit_scene((16, 12), 1, false);
    let e = s.env.as_mut().unwrap();
    e.radiance = u32::MAX - 1;
    e.mips = 3;
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadEnvironment));
}

#[test]
fn environment_textures_that_do_not_exist_are_rejected() {
    let n = lit_scene((16, 12), 1, false).textures.len() as u32;
    let cases: [fn(&mut EnvInfo, u32); 4] = [|e, _| e.mips = 0, |e, n| e.irradiance = n, |e, n| e.radiance = n - e.mips + 1, |e, n| e.radiance = n];
    for (i, f) in cases.into_iter().enumerate() {
        let mut s = lit_scene((16, 12), 1, false);
        f(s.env.as_mut().unwrap(), n);
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadEnvironment), "case {i}");
    }
}

#[test]
fn shadow_map_work_is_limited_and_an_error_leaves_the_scene_as_it_was() {
    // Earlier maps, so an error that cleared them would show, and the lights' request marks (0),
    // which a call that went ahead would replace with map indices.
    let mut done = lit_scene((16, 12), 1, false);
    try_shadow_maps(&mut done, true, &Limits::default()).unwrap();
    let mut s = lit_scene((16, 12), 1, false);
    (s.shadows, s.shadow_texels) = (done.shadows, done.shadow_texels);
    let state = |s: &Scene| (s.shadows.clone(), crate::bits(&s.shadow_texels), s.lights.clone(), s.env);
    let before = state(&s);
    let r = try_shadow_maps(&mut s, true, &limits(|l| l.max_work = 1000));
    assert!(matches!(r, Err(RenderError::TooMuchWork { limit: 1000, needed }) if needed > 1000), "{r:?}");
    assert_eq!(state(&s), before);
    // Casters times maps past the limit are rejected before any map is projected.
    let r = try_shadow_maps(&mut s, true, &limits(|l| l.max_work = 2));
    assert!(matches!(r, Err(RenderError::TooMuchWork { limit: 2, .. })), "{r:?}");
    assert_eq!(state(&s), before);
}

#[test]
fn shadow_marks_naming_maps_that_do_not_exist_are_rejected() {
    // Marked 0 (a request) with no maps made.
    let s = lit_scene((16, 12), 1, false);
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::MissingShadowMap(Some(0))));
    let mut s = lit_scene((16, 12), 1, false);
    try_shadow_maps(&mut s, true, &Limits::default()).unwrap();
    assert!(try_render(&s, &Limits::default()).is_ok());
    let point = s.lights.iter().position(|l| l.kind == 2).unwrap();
    let mut bad = s.clone();
    bad.lights[point].shadow = bad.shadows.len() as i32 - 5;
    assert_eq!(try_render(&bad, &Limits::default()), Err(RenderError::MissingShadowMap(Some(point))));
    let mut bad = s;
    bad.env.as_mut().unwrap().shadow = bad.shadows.len() as i32;
    assert_eq!(try_render(&bad, &Limits::default()), Err(RenderError::MissingShadowMap(None)));
}

#[test]
fn a_side_over_two_to_the_thirtieth_is_rejected() {
    let s = Scene { width: 1, height: (1 << 30) + 1, ..one_quad() };
    let any = limits(|l| l.max_pixels = u64::MAX);
    assert_eq!(try_render(&s, &any), Err(RenderError::TooLarge(1, (1 << 30) + 1)));
    assert_eq!(any.check(1, i32::MAX as u32), Err(RenderError::TooLarge(1, i32::MAX as u32)));
}

#[test]
fn shadow_maps_whose_views_do_not_fit_f32_are_rejected() {
    // Finite vertices 6e38 apart: the parallel light's eye sits twice the bounding radius back,
    // a light-space translation past f32.
    let mut s = lit_scene((16, 12), 1, false);
    assert!(s.lights.iter().any(|l| l.kind == 0 && l.shadow == 0));
    for (i, v) in s.vertices.iter_mut().enumerate() {
        v.pos[0] = if i % 2 == 0 { 3e38 } else { -3e38 };
    }
    let lights = s.lights.clone();
    assert_eq!(try_shadow_maps(&mut s, true, &Limits::default()), Err(RenderError::BadShadowInputs));
    assert!(s.shadows.is_empty() && s.lights == lights);
}

#[test]
fn a_shadow_casting_light_with_no_direction_is_rejected() {
    let mut s = lit_scene((16, 12), 1, false);
    let i = s.lights.iter().position(|l| l.kind < 2 && l.shadow == 0).unwrap();
    s.lights[i].dir = [0.0; 3];
    assert_eq!(try_shadow_maps(&mut s, false, &Limits::default()), Err(RenderError::BadLight(i)));
    // Shading reads the direction too, with or without a map.
    s.lights[i].shadow = -1;
    assert_eq!(try_shadow_maps(&mut s, false, &Limits::default()), Err(RenderError::BadLight(i)));
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadLight(i)));
}

#[test]
fn light_directions_that_are_not_unit_length_are_rejected() {
    let base = lit_scene((16, 12), 1, false);
    for kind in [0, 1] {
        let i = base.lights.iter().position(|l| l.kind == kind).unwrap();
        for k in [2.0, 0.5, 1.002] {
            let mut s = base.clone();
            s.lights[i].dir = s.lights[i].dir.map(|v| v * k);
            assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadLight(i)), "kind {kind} × {k}");
            assert_eq!(try_shadow_maps(&mut s, false, &Limits::default()), Err(RenderError::BadLight(i)), "kind {kind} × {k}");
        }
        // Rounding within 0.001 of unit length is accepted.
        let mut s = base.clone();
        s.lights[i].dir = s.lights[i].dir.map(|v| v * 1.0005);
        assert!(try_shadow_maps(&mut s, false, &Limits::default()).is_ok(), "kind {kind}");
    }
    // A point light does not read its direction.
    let mut s = base.clone();
    let i = s.lights.iter().position(|l| l.kind == 2).unwrap();
    s.lights[i].dir = [0.0, 0.0, 5.0];
    assert!(try_shadow_maps(&mut s, false, &Limits::default()).is_ok());
    assert!(try_render(&s, &Limits::default()).is_ok());
    // The environment light's direction is unit length or zero (no dominant direction).
    let mut s = base.clone();
    let sun = s.env.unwrap().sun;
    s.env.as_mut().unwrap().sun = sun.map(|v| v * 2.0);
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadEnvironment));
    assert_eq!(try_env_shadow_map(s.env.unwrap(), [0.0; 3], 100.0, 512), Err(RenderError::BadShadowInputs));
    s.env.as_mut().unwrap().sun = [0.0; 3];
    assert!(try_shadow_maps(&mut s, false, &Limits::default()).is_ok());
}

#[test]
fn casters_near_a_perspective_light_cast_shadows() {
    // Only the spot light asks for a map.
    let mut s = lit_scene((16, 12), 1, false);
    for l in &mut s.lights {
        if l.kind != 1 {
            l.shadow = -1;
        }
    }
    if let Some(e) = s.env.as_mut() {
        e.shadow = -1;
    }
    let spot = s.lights.iter().find(|l| l.kind == 1).copied().unwrap();
    let d = spot.dir;
    // A small opaque caster 0.5 units in front of the light, across its axis.
    let len = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let u = [d[2], 0.0, -d[0]];
    let u = u.map(|x| x / len(u));
    let v = [d[1] * u[2] - d[2] * u[1], d[2] * u[0] - d[0] * u[2], d[0] * u[1] - d[1] * u[0]];
    let c = [0, 1, 2].map(|k| spot.pos[k] + d[k] * 0.5);
    let r = 0.2;
    let corners = [(1.0f32, 0.0f32), (-0.5, 0.87), (-0.5, -0.87)].map(|(a, b)| [0, 1, 2].map(|k| c[k] + r * (a * u[k] + b * v[k])));
    let m = s.materials.iter().position(|m| m.casts_shadows && m.alpha_mode == 0).unwrap() as u32;
    let first = s.vertices.len() as u32;
    for p in corners {
        s.vertices.push(Vertex { pos: p, normal: d, uv: [0.0; 2], tangent: [1.0, 0.0, 0.0, 1.0], material: m });
    }
    s.indices.splice(0..0, [first, first + 1, first + 2]);
    s.opaque_count += 3;
    try_shadow_maps(&mut s, false, &Limits::default()).unwrap();
    let spot = s.lights.iter().find(|l| l.kind == 1).copied().unwrap();
    let map = s.shadows[spot.shadow as usize];
    let n = map.size as usize;
    let centre = s.shadow_texels[map.offset as usize + (n / 2) * n + n / 2];
    assert!((centre - 0.5).abs() < 0.01, "center texel depth {centre}");
}

#[test]
fn casters_crossing_a_perspective_light_plane_far_from_it_are_drawn() {
    // A caster with corners about 100,000 units in front of and behind the spot light: the
    // crossing with the near plane stays in front of the light.
    let base = lit_scene((16, 12), 1, false);
    let spot = base.lights.iter().find(|l| l.kind == 1).copied().unwrap();
    let d = spot.dir;
    let len = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let u = [d[2], 0.0, -d[0]];
    let u = u.map(|x| x / len(u));
    let v = [d[1] * u[2] - d[2] * u[1], d[2] * u[0] - d[0] * u[2], d[0] * u[1] - d[1] * u[0]];
    let m = base.materials.iter().position(|m| m.casts_shadows && m.alpha_mode == 0).unwrap() as u32;
    let mut without = base.clone();
    for l in &mut without.lights {
        if l.kind != 1 {
            l.shadow = -1;
        }
    }
    if let Some(e) = without.env.as_mut() {
        e.shadow = -1;
    }
    let mut before = without.clone();
    try_shadow_maps(&mut before, true, &Limits::default()).unwrap();
    for i in 0..64 {
        let mut s = without.clone();
        let (front, back) = (1.0e5 + i as f32 * 37.3, -(1.0e5 + i as f32 * 11.1));
        // In the plane 20 units to the light's side along `v`, so the map sees its face.
        let at = |t: f32, side: f32| [0, 1, 2].map(|k| spot.pos[k] + d[k] * t + u[k] * side + v[k] * 20.0);
        let first = s.vertices.len() as u32;
        for p in [at(front, 0.0), at(back, 0.0), at(front, 10.0 + i as f32)] {
            s.vertices.push(Vertex { pos: p, normal: v, uv: [0.0; 2], tangent: [1.0, 0.0, 0.0, 1.0], material: m });
        }
        s.indices.splice(0..0, [first, first + 1, first + 2]);
        s.opaque_count += 3;
        assert_eq!(try_shadow_maps(&mut s, true, &Limits::default()), Ok(()), "variant {i}");
        // The maps are the same size and view with or without the caster; the caster brings
        // some texels nearer.
        assert_eq!(s.shadows, before.shadows, "variant {i}");
        let nearer = s.shadow_texels.iter().zip(&before.shadow_texels).filter(|(a, b)| a < b).count();
        assert!(nearer > 0, "variant {i}: the caster is missing from the map");
    }
}

#[test]
fn sampling_a_texture_with_huge_sides_reads_missing_texels_as_transparent() {
    let t = TexInfo { offset: 0, width: 4_000_000_000, height: 4_000_000_000, wrap_u: 1, wrap_v: 1 };
    assert_eq!(sample(&t, &[WHITE], 0.999_999, 0.999_999), [0.0; 4]);
}

#[test]
fn unknown_light_kinds_and_alpha_modes_are_rejected() {
    let mut s = lit_scene((16, 12), 1, false);
    s.lights[2].kind = 3;
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadLight(2)));
    let mut s = lit_scene((16, 12), 1, false);
    s.materials[3].alpha_mode = 3;
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadMaterial(3)));
}

#[test]
fn shading_that_overflows_is_an_error() {
    let mut s = one_quad();
    s.materials[0].base = [f32::MAX, 0.0, 0.0, 1.0];
    s.materials[0].emissive = [f32::MAX, 0.0, 0.0];
    assert!(matches!(try_render(&s, &Limits::default()), Err(RenderError::BadPixel(_))));
    // A camera depth that overflows for a drawn pixel.
    let mut s = one_quad();
    s.view[2][0] = f32::MAX;
    assert!(matches!(try_render(&s, &Limits::default()), Err(RenderError::BadPixel(_))));
}

#[test]
fn a_clip_transform_with_a_tiny_w_draws_the_same_pixels() {
    let s = one_quad();
    let mut tiny = s.clone();
    for row in &mut tiny.clip {
        for v in row.iter_mut() {
            *v *= 1e-40;
        }
    }
    let (a, b) = (try_render(&s, &Limits::default()).unwrap(), try_render(&tiny, &Limits::default()).unwrap());
    assert_eq!(b.color.iter().filter(|c| c[3] > 0.0).count(), 36);
    assert_eq!(a.color, b.color);
    // Depth within rounding: the f64 projection rounds differently.
    assert!(a.depth.iter().zip(&b.depth).all(|(x, y)| x == y || (x - y).abs() < 1e-3), "{:?}", b.depth);
}

#[test]
fn triangles_on_the_far_plane_are_drawn() {
    // Camera depth 1024 maps to ndc z exactly 0, the far end of the depth range.
    let mut s = quad_scene(8, 8, &[([1.0, 1.0, 4.0, 7.0], 1024.0, WHITE), ([4.0, 1.0, 7.0, 7.0], 1024.0, [1.0, 1.0, 1.0, 0.5])]);
    s.clip[2] = [0.0, 0.0, -1.0 / 1024.0, 1.0];
    let t = try_render(&s, &Limits::default()).unwrap();
    assert_eq!(t.color.iter().filter(|c| c[3] == 1.0).count(), 18);
    assert_eq!(t.color.iter().filter(|c| c[3] == 0.5).count(), 18);
    // Interpolated camera depth, within rounding of 1024.
    assert_eq!(t.depth.iter().filter(|d| (**d - 1024.0).abs() < 1e-3).count(), 18);
}

#[test]
fn masked_casters_use_the_material_cutoff_in_shadow_maps() {
    let maps = |s: &mut Scene| {
        try_shadow_maps(s, false, &Limits::default()).unwrap();
        s.shadow_texels.clone()
    };
    let base = lit_scene((16, 12), 1, false);
    let masked = base.materials.iter().position(|m| m.alpha_mode == 1 && m.casts_shadows).unwrap();
    let mut none = base.clone();
    none.materials[masked].casts_shadows = false;
    let none = maps(&mut none);
    // Without a base texture: alpha 0.7 casts at a cutoff of 0.5 and not at 0.8.
    let mut s = base.clone();
    s.materials[masked].tex_base = -1;
    s.materials[masked].base[3] = 0.7;
    assert_ne!(maps(&mut s), none);
    s.materials[masked].alpha_cutoff = 0.8;
    assert_eq!(maps(&mut s), none);
    // A blended caster without a base texture casts nothing at alpha 0, as shading discards it.
    s.materials[masked].alpha_mode = 2;
    s.materials[masked].alpha_cutoff = 0.5;
    assert_ne!(maps(&mut s), none);
    s.materials[masked].opacity = 0.0;
    assert_eq!(maps(&mut s), none);
    // With the checkered mask texture: a cutoff of 0 keeps the partly covered texels that 0.5
    // drops.
    let mut half = base.clone();
    let mut zero = base.clone();
    zero.materials[masked].alpha_cutoff = 0.0;
    assert_ne!(maps(&mut zero), maps(&mut half));
}

const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// A blue quad at depth 500 behind two green quads at depth 100 (left and right half), with the
/// green quads' materials (1 and 2) in mask mode.
fn masked_pair() -> Scene {
    let mut s = quad_scene(8, 4, &[([0.0, 0.0, 8.0, 4.0], 500.0, BLUE), ([0.0, 0.0, 4.0, 4.0], 100.0, GREEN), ([4.0, 0.0, 8.0, 4.0], 100.0, GREEN)]);
    for m in &mut s.materials[1..] {
        m.alpha_mode = 1;
    }
    s
}

/// One single-texel texture for each value of `alphas`, white with that alpha.
fn alpha_textures(s: &mut Scene, alphas: &[f32]) {
    s.texels = alphas.iter().map(|a| [1.0, 1.0, 1.0, *a]).collect();
    s.textures = (0..alphas.len() as u32).map(|offset| TexInfo { offset, width: 1, height: 1, wrap_u: 0, wrap_v: 0 }).collect();
}

#[test]
fn masked_surfaces_cover_fully_where_they_pass_the_cutoff() {
    // Material alpha 0.6 on the left and 0.4 on the right against the default cutoff of 0.5, first
    // as the materials' own alpha and then through their base textures.
    let mut factor = masked_pair();
    (factor.materials[1].base[3], factor.materials[2].base[3]) = (0.6, 0.4);
    let mut texel = masked_pair();
    alpha_textures(&mut texel, &[0.6, 0.4]);
    (texel.materials[1].tex_base, texel.materials[2].tex_base) = (0, 1);
    for s in [factor, texel] {
        let t = try_render(&s, &Limits::default()).unwrap();
        assert_eq!(t.color[8 + 1], GREEN, "a passing fragment draws the surface fully opaque");
        assert_eq!(t.depth[8 + 1], 100.0);
        assert_eq!(t.color[8 + 6], BLUE, "a failing fragment shows the surface behind");
        assert_eq!(t.depth[8 + 6], 500.0);
    }
}

#[test]
fn the_mask_cutoff_is_inclusive_and_ignores_layer_opacity() {
    // Material alpha equal to the cutoff passes, a zero cutoff included.
    let mut s = masked_pair();
    (s.materials[1].base[3], s.materials[1].alpha_cutoff) = (0.5, 0.5);
    (s.materials[2].base[3], s.materials[2].alpha_cutoff) = (0.0, 0.0);
    let t = try_render(&s, &Limits::default()).unwrap();
    assert_eq!(t.color[8 + 1], GREEN);
    assert_eq!(t.color[8 + 6], GREEN);
    // At half layer opacity, material alpha 0.6 still passes the 0.5 cutoff, and the surface
    // fades by the layer opacity alone.
    let mut s = quad_scene(8, 4, &[([0.0, 0.0, 8.0, 4.0], 500.0, BLUE), ([0.0, 0.0, 8.0, 4.0], 100.0, [0.0, 1.0, 0.0, 0.6])]);
    (s.materials[1].alpha_mode, s.materials[1].opacity) = (1, 0.5);
    let c = try_render(&s, &Limits::default()).unwrap().color[8 + 1];
    assert!([0.0, 0.5, 0.5, 1.0].iter().zip(c).all(|(a, b)| (a - b).abs() < 1e-6), "{c:?}");
}

#[test]
fn masked_casters_shadow_where_the_material_alpha_passes_the_cutoff() {
    let covered = |f: &dyn Fn(&mut Scene)| {
        let mut s = quad_scene(16, 16, &[([0.0, 0.0, 8.0, 8.0], 100.0, WHITE)]);
        (s.materials[0].casts_shadows, s.materials[0].alpha_mode) = (true, 1);
        f(&mut s);
        s.lights = vec![shadow_light(0)];
        try_shadow_maps(&mut s, true, &Limits::default()).unwrap();
        s.shadow_texels.iter().filter(|t| t.is_finite()).count()
    };
    let full = covered(&|_| {});
    assert!(full > 0);
    // Without a texture, the material's own alpha meets the cutoff, and the layer opacity does
    // not move it. Material alpha 0 passes a cutoff of 0.
    assert_eq!(covered(&|s| (s.materials[0].base[3], s.materials[0].alpha_cutoff) = (0.6, 0.7)), 0);
    assert_eq!(covered(&|s| (s.materials[0].base[3], s.materials[0].alpha_cutoff, s.materials[0].opacity) = (0.6, 0.6, 0.5)), full);
    assert_eq!(covered(&|s| (s.materials[0].base[3], s.materials[0].alpha_cutoff) = (0.0, 0.0)), full);
    // With a texture, the texel's alpha times the material's meets the cutoff.
    let textured = |cutoff: f32, opacity: f32| {
        covered(&|s| {
            alpha_textures(s, &[0.6]);
            (s.materials[0].tex_base, s.materials[0].alpha_cutoff, s.materials[0].opacity) = (0, cutoff, opacity);
        })
    };
    assert_eq!(textured(0.7, 1.0), 0);
    assert_eq!(textured(0.6, 0.5), full);
    // Below half opacity a masked caster casts nothing, as an opaque-mode caster does.
    assert_eq!(covered(&|s| s.materials[0].opacity = 0.4), 0);
    assert_eq!(textured(0.5, 0.4), 0);
}

#[test]
fn a_textured_blended_caster_multiplies_its_base_alpha_by_the_opacity_first() {
    // Texel alpha 9586981 / 2^24 times (0.875 × (1 − 2^-24)) rounds to 0.5 in f32, and casts;
    // (texel alpha × 0.875) × (1 − 2^-24) rounds to 0.49999997.
    let covered = |opacity: f32| {
        let mut s = quad_scene(16, 16, &[([0.0, 0.0, 8.0, 8.0], 100.0, [1.0, 1.0, 1.0, 0.875])]);
        alpha_textures(&mut s, &[9_586_981.0 / 16_777_216.0]);
        (s.materials[0].casts_shadows, s.materials[0].tex_base, s.materials[0].opacity) = (true, 0, opacity);
        s.lights = vec![shadow_light(0)];
        try_shadow_maps(&mut s, true, &Limits::default()).unwrap();
        s.shadow_texels.iter().filter(|t| t.is_finite()).count()
    };
    let full = covered(1.0);
    assert!(full > 0);
    assert_eq!(covered(1.0 - 1.0 / 16_777_216.0), full);
}

#[test]
fn a_caster_texture_coordinate_that_overflows_f32_at_a_spot_map_depth_is_wrapped() {
    // A masked caster 1/64 in front of a spot light, with a repeated texture of three texels whose
    // alpha is 0, 1, 0. U = 0 samples between the end texels, which fail the cutoff, and so does
    // U = f32::MAX, a whole number of periods away; U × 64 overflows f32.
    let covered = |u: f32, alphas: [f32; 3]| {
        let mut s = quad_scene(8, 8, &[]);
        s.materials.push(Material { casts_shadows: true, alpha_mode: 1, tex_base: 0, ..Material::default() });
        s.texels = alphas.map(|a| [1.0, 1.0, 1.0, a]).to_vec();
        s.textures = vec![TexInfo { offset: 0, width: 3, height: 1, wrap_u: 0, wrap_v: 0 }];
        let z = 1.0 / 64.0;
        for p in [[-0.002, -0.002, z], [0.002, -0.002, z], [0.0, 0.002, z]] {
            s.vertices.push(Vertex { pos: p, normal: [0.0, 0.0, -1.0], uv: [u, 0.5], tangent: [1.0, 0.0, 0.0, 1.0], material: 0 });
        }
        (s.indices, s.opaque_count) = (vec![0, 1, 2], 3);
        s.lights = vec![spot([0.0; 3], [0.0, 0.0, 1.0], 0.95, 0.85)];
        try_shadow_maps(&mut s, false, &Limits::default()).unwrap();
        s.shadow_texels.iter().filter(|t| t.is_finite()).count()
    };
    assert!(covered(f32::MAX, [1.0; 3]) > 0, "the caster covers texels of the map");
    assert_eq!(covered(0.0, [0.0, 1.0, 0.0]), 0);
    assert_eq!(covered(f32::MAX, [0.0, 1.0, 0.0]), 0);
}

#[test]
fn a_caster_whose_spot_map_corners_overflow_f32_is_projected_in_f64() {
    // A spot light at the origin along +z. The first corner's 1e37 times the map's focal length
    // overflows f32; divided by its depth of 1e38 it lies a few dozen texels from the center.
    let mut s = quad_scene(8, 8, &[]);
    s.materials.push(Material { casts_shadows: true, ..Material::default() });
    for p in [[1e37, 0.0, 1e38], [0.0, 1.0, 10.0], [-1.0, 0.0, 10.0]] {
        s.vertices.push(Vertex { pos: p, normal: [0.0, 0.0, -1.0], uv: [0.0; 2], tangent: [1.0, 0.0, 0.0, 1.0], material: 0 });
    }
    (s.indices, s.opaque_count) = (vec![0, 1, 2], 3);
    s.lights = vec![spot([0.0; 3], [0.0, 0.0, 1.0], 0.95, 0.85)];
    assert_eq!(try_shadow_maps(&mut s, false, &Limits::default()), Ok(()));
    assert_eq!(s.shadows.len(), 1);
    assert!(s.shadow_texels.iter().any(|d| d.is_finite()), "the caster draws into the map");
}

#[test]
fn sampling_between_large_texels_of_opposite_signs_is_finite() {
    let t = TexInfo { offset: 0, width: 2, height: 2, wrap_u: 1, wrap_v: 1 };
    let texels = [[3e38, 1.0, 0.0, 1.0], [-3e38, 1.0, 0.0, 1.0], [3e38, 1.0, 0.0, 1.0], [-3e38, 1.0, 0.0, 1.0]];
    // Halfway between the texel centers in x.
    let o = sample(&t, &texels, 0.5, 0.25);
    assert!(o.iter().all(|c| c.is_finite()), "{o:?}");
    assert_eq!(o, [0.0, 1.0, 0.0, 1.0]);
    // A quarter of the way: the mean of 3e38 and -3e38 weighted 3:1 is 1.5e38.
    let o = sample(&t, &texels, 0.375, 0.25);
    assert_eq!(o[0], 1.5e38);
}

#[test]
fn cone_cosines_outside_minus_1_to_1_are_rejected() {
    for (inner, outer) in [(2.0, 0.0), (0.9, -1.5)] {
        let mut s = lit_scene((16, 12), 1, false);
        s.lights[1].cos_inner = inner;
        s.lights[1].cos_outer = outer;
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadLight(1)), "{inner} {outer}");
    }
}

#[test]
fn texel_alpha_outside_0_to_1_is_rejected() {
    for alpha in [1.5, -0.25] {
        let mut s = lit_scene((16, 12), 1, false);
        s.texels[2][3] = alpha;
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadTexel(2)), "{alpha}");
    }
}

#[test]
fn supplied_shadow_maps_are_checked() {
    let mut s = lit_scene((16, 12), 1, false);
    try_shadow_maps(&mut s, true, &Limits::default()).unwrap();
    let mut bad = s.clone();
    bad.shadow_texels.truncate(bad.shadow_texels.len() - 1);
    let last = bad.shadows.len() - 1;
    assert_eq!(try_render(&bad, &Limits::default()), Err(RenderError::BadShadowMap(last)));
    let mut bad = s.clone();
    bad.shadows[0].focal = 0.0;
    assert_eq!(try_render(&bad, &Limits::default()), Err(RenderError::BadShadowMap(0)));
    let mut bad = s.clone();
    bad.shadows[0].radius = -1.0;
    assert_eq!(try_render(&bad, &Limits::default()), Err(RenderError::BadShadowMap(0)));
    bad.shadows[0].radius = 0.0;
    assert!(try_render(&bad, &Limits::default()).is_ok(), "a radius of 0 is valid");
    let mut bad = s;
    bad.shadow_texels[5] = f32::NAN;
    assert_eq!(try_render(&bad, &Limits::default()), Err(RenderError::BadShadowTexel(5)));
}

#[test]
fn texture_coordinates_past_the_integer_range_are_sampled() {
    for wrap in 0..3 {
        for uv in [[f32::MAX, f32::MAX], [-f32::MAX, f32::MAX], [1e30, -1e30]] {
            let mut s = one_quad();
            let tex = crate::texture(&mut s, 2, 2, (wrap, wrap), |x, y| [x as f32, y as f32, 1.0, 1.0]);
            s.materials[0].tex_base = tex;
            for v in &mut s.vertices {
                v.uv = uv;
            }
            let t = try_render(&s, &Limits::default()).unwrap();
            assert!(t.color.iter().flatten().all(|c| c.is_finite()), "wrap {wrap}, {uv:?}");
            assert_eq!(t.color.iter().filter(|c| c[3] > 0.0).count(), 36, "wrap {wrap}, {uv:?}");
        }
    }
}

#[test]
fn degenerate_triangles_draw_nothing() {
    // Zero area on the screen, and a corner at w = 0.
    let mut s = one_quad();
    for v in &mut s.vertices {
        v.pos[1] = 4.0;
    }
    let t = try_render(&s, &Limits::default()).unwrap();
    assert!(t.color.iter().all(|c| c[3] == 0.0));
    let mut s = one_quad();
    s.clip[3] = [0.0, 0.0, 0.0, 0.0];
    let t = try_render(&s, &Limits::default()).unwrap();
    assert!(t.color.iter().all(|c| c[3] == 0.0) && t.depth.iter().all(|d| d.is_infinite()));
}

#[test]
fn triangle_light_and_work_counts_are_limited() {
    let s = one_quad();
    assert_eq!(try_render(&s, &limits(|l| l.max_triangles = 1)), Err(RenderError::TooManyTriangles { count: 2, limit: 1 }));
    assert_eq!(try_render(&s, &limits(|l| l.max_vertices = 3)), Err(RenderError::TooManyVertices { count: 4, limit: 3 }));
    // Unused vertices count too.
    let mut empty = s.clone();
    empty.indices.clear();
    empty.opaque_count = 0;
    assert_eq!(try_render(&empty, &limits(|l| l.max_vertices = 3)), Err(RenderError::TooManyVertices { count: 4, limit: 3 }));
    // Two triangles covering 6 × 6 pixels: two 6 × 6 boxes plus one each.
    assert_eq!(try_render(&s, &limits(|l| l.max_work = 73)), Err(RenderError::TooMuchWork { needed: 74, limit: 73 }));
    assert!(try_render(&s, &limits(|l| l.max_work = 74)).is_ok());
    // The count stops at the first triangle past the limit: 37 after the first.
    assert_eq!(try_render(&s, &limits(|l| l.max_work = 36)), Err(RenderError::TooMuchWork { needed: 37, limit: 36 }));
    let mut s = lit_scene((16, 12), 1, false);
    assert_eq!(try_render(&s, &limits(|l| l.max_lights = 2)), Err(RenderError::TooManyLights { count: 3, limit: 2 }));
    assert_eq!(try_shadow_maps(&mut s, false, &limits(|l| l.max_lights = 2)), Err(RenderError::TooManyLights { count: 3, limit: 2 }));
}

#[test]
fn a_triangle_far_past_the_viewport_and_across_the_depth_range_is_clipped_and_drawn() {
    // Orthographic depth 0..1000 as in `quad_scene`, with clip x equal to world x: corners at
    // x = ±3e38 overflow f32 when clipping at the depth range interpolates between them.
    let mut s = Scene { width: 16, height: 16, ssaa: 1, ortho: true, cam_fwd: [0.0, 0.0, 1.0], ..Scene::default() };
    s.clip = [[1.0, 0.0, 0.0, 0.0], [0.0, -0.1, 0.0, 0.0], [0.0, 0.0, -0.001, 1.0], [0.0, 0.0, 0.0, 1.0]];
    s.view = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    s.materials = vec![Material { unlit: true, double_sided: true, ..Material::default() }];
    let v = |x: f32, y: f32, z: f32| Vertex { pos: [x, y, z], normal: [0.0, 0.0, -1.0], tangent: [1.0, 0.0, 0.0, 1.0], ..Vertex::default() };
    s.vertices = vec![v(-3e38, 30.0, 500.0), v(3e38, 30.0, -200.0), v(0.0, -30.0, 500.0)];
    s.indices = vec![0, 1, 2];
    s.opaque_count = 3;
    let t = try_render(&s, &Limits::default()).unwrap();
    assert!(t.color.iter().chain(&[[0.0; 4]]).flatten().all(|c| c.is_finite()));
    assert_eq!(t.color[8 * 16 + 8], WHITE, "the center pixel is inside the visible part");
}

#[test]
fn resolve_factors_and_buffer_lengths_are_checked() {
    let t = target(4, 6, vec![[0.0; 4]; 24], vec![f32::INFINITY; 24]);
    for k in [0, 4, 5, 7] {
        assert_eq!(try_resolve(&t, k), Err(RenderError::BadFactor { k, width: 4, height: 6 }));
    }
    let t = target(4, 4, vec![[0.0; 4]; 15], vec![f32::INFINITY; 16]);
    assert_eq!(try_resolve(&t, 2), Err(RenderError::BadDataLength { expected: 16, found: 15 }));
    let t = target(4, 4, vec![[0.0; 4]; 16], vec![f32::INFINITY; 17]);
    assert_eq!(try_resolve(&t, 2), Err(RenderError::BadDataLength { expected: 16, found: 17 }));
    let t = target(u32::MAX, u32::MAX, vec![], vec![]);
    assert!(try_resolve(&t, 1).is_err());
    // An empty target and a factor whose square passes u32.
    let t = target(0, 65536, vec![], vec![]);
    let (img, depth) = try_resolve(&t, 65536).unwrap();
    assert_eq!((img.width, img.height, img.data.len(), depth.len()), (0, 1, 0, 0));
}

#[test]
fn render_cpu_rejects_a_supersampling_factor_before_rendering() {
    // A raster too large for the limits: the factor error comes first, so nothing is rendered.
    let s = Scene { width: 9, height: 8, ssaa: 2, ..one_quad() };
    assert_eq!(try_render_cpu(&s, &limits(|l| l.max_pixels = 1)), Err(RenderError::BadFactor { k: 2, width: 9, height: 8 }));
}

#[test]
fn resolve_is_a_box_filter_with_the_mean_covered_depth() {
    let c = |v: f32| [v, v * 0.5, 0.0, v];
    let t = target(2, 2, vec![c(1.0), c(0.0), c(0.5), c(0.5)], vec![10.0, f32::INFINITY, 20.0, f32::INFINITY]);
    let (img, depth) = try_resolve(&t, 2).unwrap();
    assert_eq!((img.width, img.height), (1, 1));
    assert_eq!(img.data[0], [0.5, 0.25, 0.0, 0.5]);
    assert_eq!(depth, vec![15.0]);
    let (img, depth) = try_resolve(&t, 1).unwrap();
    assert_eq!(img.data, t.color);
    assert_eq!(depth[1], f32::INFINITY);
    let t = target(2, 2, vec![[0.0; 4]; 4], vec![f32::INFINITY; 4]);
    assert_eq!(try_resolve(&t, 2).unwrap().1, vec![f32::INFINITY]);
    // Large finite samples whose f32 sum overflows still average to their mean.
    let t = target(2, 1, vec![[3e38, -3e38, 0.0, 1.0]; 2], vec![3e38; 2]);
    let (img, depth) = try_resolve(&t, 1).unwrap();
    assert_eq!((img.data[0], depth[0]), ([3e38, -3e38, 0.0, 1.0], 3e38));
    let t = target(2, 2, vec![[3e38, -3e38, 0.0, 1.0]; 4], vec![3e38; 4]);
    let (img, depth) = try_resolve(&t, 2).unwrap();
    assert_eq!((img.data[0], depth[0]), ([3e38, -3e38, 0.0, 1.0], 3e38));
}

#[test]
fn encode_rejects_pixels_it_cannot_encode() {
    for (i, p) in [
        [0.5, 0.5, 0.5, f32::NAN],
        [0.5, 0.5, 0.5, f32::INFINITY],
        [0.5, 0.5, 0.5, -0.25],
        [0.5, 0.5, 0.5, 1.5],
        [f32::NAN, 0.5, 0.5, 0.5],
        [0.5, f32::NEG_INFINITY, 0.5, 0.5],
        [3e38, 0.5, 0.5, 1e-5],
    ]
    .into_iter()
    .enumerate()
    {
        let mut img = Image { width: 2, height: 1, data: vec![[0.25, 0.25, 0.25, 0.5], p] };
        let before = bits4(&img.data);
        assert_eq!(try_encode(&mut img), Err(RenderError::BadPixel(1)), "case {i}: {p:?}");
        assert_eq!(bits4(&img.data), before, "case {i}: nothing changes when a pixel is rejected");
    }
    let mut img = Image { width: 2, height: 2, data: vec![[0.0; 4]; 3] };
    assert_eq!(try_encode(&mut img), Err(RenderError::BadDataLength { expected: 4, found: 3 }));
}

#[test]
fn encode_follows_the_srgb_curve_on_unpremultiplied_color() {
    // The two segments of the sRGB encoding: linear below 0.0031308, a 1/2.4 power above.
    let curve = |c: f64| if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    let mut img =
        Image { width: 4, height: 1, data: vec![[0.1, 0.001, 0.4, 0.5], [0.0, 0.0, 0.0, 0.0], [0.2, 0.5, 1.0, 1.0], [1e-7, 1e-7, 1e-7, 1e-7]] };
    try_encode(&mut img).unwrap();
    let p = img.data[0];
    for (c, linear) in [0.2f64, 0.002, 0.8].into_iter().enumerate() {
        assert!((f64::from(p[c]) - curve(linear) * 0.5).abs() < 1e-6, "channel {c}: {}", p[c]);
    }
    assert_eq!(p[3], 0.5, "alpha stays linear");
    assert_eq!(img.data[1], [0.0; 4]);
    assert_eq!(img.data[2], [linear_to_srgb(0.2), linear_to_srgb(0.5), linear_to_srgb(1.0), 1.0]);
    assert_eq!(img.data[3], [1e-7; 4], "a pixel with alpha at or under 1e-6 is left as it is");
}

#[test]
fn encoding_mirrors_the_srgb_curve_for_negative_color() {
    // The segment is chosen by magnitude and the sign is kept: red −0.25 encodes to about −0.537
    // (on the linear segment it would be −3.23), −0.001 stays on the linear segment, and the
    // finite extremes encode to finite values.
    let curve = |c: f64| if c.abs() <= 0.0031308 { 12.92 * c } else { (1.055 * c.abs().powf(1.0 / 2.4) - 0.055).copysign(c) };
    let mut img = Image { width: 2, height: 1, data: vec![[-0.25, -0.001, 0.25, 1.0], [-f32::MAX, f32::MAX, -1e38, 1.0]] };
    try_encode(&mut img).unwrap();
    for (c, linear) in [-0.25f64, -0.001, 0.25].into_iter().enumerate() {
        assert!((f64::from(img.data[0][c]) - curve(linear)).abs() < 1e-6, "channel {c}: {}", img.data[0][c]);
    }
    assert!((img.data[0][0] + 0.537).abs() < 1e-3, "{}", img.data[0][0]);
    assert_eq!(img.data[0][0], -img.data[0][2]);
    assert!(img.data[1].iter().all(|c| c.is_finite()), "{:?}", img.data[1]);
    assert_eq!(img.data[1][0], -img.data[1][1]);
    // Decoding takes a negative value back the same way.
    assert_eq!(srgb_to_linear(-linear_to_srgb(0.25)), -srgb_to_linear(linear_to_srgb(0.25)));
    assert!((srgb_to_linear(-0.5) + 0.214).abs() < 1e-3, "{}", srgb_to_linear(-0.5));
}

#[test]
fn shadow_maps_twice_give_the_same_maps() {
    let mut s = lit_scene((16, 12), 1, false);
    try_shadow_maps(&mut s, true, &Limits::default()).unwrap();
    let (maps, texels, lights, env) = (s.shadows.clone(), crate::bits(&s.shadow_texels), s.lights.clone(), s.env);
    assert!(lights.iter().filter(|l| l.shadow > 0).count() >= 2, "lights after the first get maps");
    // The second call reads the indices the first one wrote as requests.
    try_shadow_maps(&mut s, true, &Limits::default()).unwrap();
    assert_eq!(s.shadows, maps);
    assert_eq!(crate::bits(&s.shadow_texels), texels);
    assert_eq!(s.lights, lights);
    assert_eq!(s.env, env);
}

#[test]
fn shadow_maps_without_casters_clear_every_mark() {
    let mut s = lit_scene((16, 12), 1, false);
    for m in &mut s.materials {
        m.casts_shadows = false;
    }
    try_shadow_maps(&mut s, false, &Limits::default()).unwrap();
    assert!(s.shadows.is_empty() && s.shadow_texels.is_empty());
    assert!(s.lights.iter().all(|l| l.shadow == -1));
    assert_eq!(s.env.unwrap().shadow, -1);
}

#[test]
fn environment_shadow_map_inputs_are_checked() {
    let e = EnvInfo {
        radiance: 0,
        mips: 1,
        irradiance: 0,
        intensity: 1.0,
        rotation: 0.0,
        sun: [0.0, -1.0, 0.0],
        key: 0.5,
        shadow: 0,
        shadow_darkness: 1.0,
        shadow_diffusion: 0.0,
    };
    let ok = try_env_shadow_map(e, [0.0; 3], 100.0, 512).unwrap();
    assert!(ok.ortho && ok.size == 512 && ok.focal.is_finite() && ok.view.iter().flatten().all(|v| v.is_finite()));
    assert_eq!(try_env_shadow_map(e, [0.0; 3], 100.0, 0), Err(RenderError::BadShadowInputs));
    for radius in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(try_env_shadow_map(e, [0.0; 3], radius, 512), Err(RenderError::BadShadowInputs), "radius {radius}");
    }
    assert_eq!(try_env_shadow_map(e, [f64::NAN, 0.0, 0.0], 100.0, 512), Err(RenderError::BadShadowInputs));
    for sun in [[0.0; 3], [f32::NAN, 0.0, 0.0], [f32::INFINITY, 1.0, 0.0]] {
        assert_eq!(try_env_shadow_map(EnvInfo { sun, ..e }, [0.0; 3], 100.0, 512), Err(RenderError::BadShadowInputs), "{sun:?}");
    }
    // Finite inputs whose map does not fit f32: the focal length and the eye position.
    assert_eq!(try_env_shadow_map(e, [0.0; 3], 1e-300, 512), Err(RenderError::BadShadowInputs));
    assert_eq!(try_env_shadow_map(e, [0.0, 1e300, 0.0], 100.0, 512), Err(RenderError::BadShadowInputs));
}

/// An unlit, double-sided triangle with corners `c` in that order on an 8 × 8 raster, whose clip
/// coordinates are its world coordinates with w = 1.
fn clip_space_triangle(c: [[f32; 3]; 3]) -> Scene {
    let id = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let mut s = Scene { width: 8, height: 8, ssaa: 1, ortho: true, cam_fwd: [0.0, 0.0, 1.0], clip: id, view: id, ..Scene::default() };
    s.materials = vec![Material { unlit: true, double_sided: true, ..Material::default() }];
    s.vertices = c.iter().map(|&pos| Vertex { pos, normal: [0.0, 0.0, -1.0], tangent: [1.0, 0.0, 0.0, 1.0], ..Vertex::default() }).collect();
    s.indices = vec![0, 1, 2];
    s.opaque_count = 3;
    s
}

#[test]
fn guard_band_crossings_next_to_a_corner_stay_on_the_band() {
    // Corner a lies 1e30 to the left, so both edges at a cross the band's left side, x = -1024,
    // next to b and c. The lower edge runs at y = 5.5004 on the screen at b and changes by less
    // than 1e-29 across the raster, so the center of pixel (0, 5) is inside.
    let (a, b, c) = ([-1e30, 0.0, 0.5], [0.0, -0.3751, 0.5], [0.0, 0.625, 0.5]);
    // a → b leaves the band and b → a enters it.
    for corners in [[a, b, c], [c, b, a]] {
        let t = try_render(&clip_space_triangle(corners), &Limits::default()).unwrap();
        assert_eq!(t.color[5 * 8], WHITE, "{corners:?}");
    }
}

#[test]
fn depth_range_crossings_next_to_a_corner_stay_on_the_plane() {
    // Corner a lies 1e10 past the far end of the depth range and 1e10 to the left, so both edges
    // at a cross z = 0 at x = -0.5, half a viewport left of b and c. Pixel (2, 5) has its center
    // at x = -0.375 and just above the lower edge.
    let (a, b, c) = ([-1e10, 0.0, -1e10], [0.0, -0.3751, 0.5], [0.0, 0.625, 0.5]);
    for corners in [[a, b, c], [c, b, a]] {
        let t = try_render(&clip_space_triangle(corners), &Limits::default()).unwrap();
        assert_eq!(t.color[5 * 8 + 2], WHITE, "{corners:?}");
    }
}

#[test]
fn a_depth_distance_that_overflows_is_clipped_in_f64() {
    // w = 1 − 2z: corner a has w = −f32::MAX and z = f32::MAX / 2, so its distance to the far
    // plane, w − z, overflows f32. In f64 the edges at a cross that plane at x = −0.147,
    // w = 0.333, and the triangle covers screen x from 2.24 to 4.
    let mut s = clip_space_triangle([[-3e38, 0.0, f32::MAX / 2.0], [0.0, -0.3751, 0.25], [0.0, 0.3, 0.25]]);
    s.clip[3] = [0.0, 0.0, -2.0, 1.0];
    let t = try_render(&s, &Limits::default()).unwrap();
    assert_eq!(t.color[4 * 8 + 3], WHITE);
}

#[test]
fn a_discarded_pixel_gives_way_only_to_fragments_inside_the_depth_range() {
    // A white triangle on the near plane (ndc z 1) over a blue background. At pixel (6, 5) the
    // white triangle's interpolated depth rounds to 1 + 2^-23, outside the depth range, and the
    // pixel shows the background. A masked, fully transparent triangle at z 0.9 is visible there
    // and discarded, and the pixel still shows the background.
    let mut plain = clip_space_triangle([[0.795, -0.932, 1.0], [0.436, 0.251, 1.0], [-0.323, 0.723, 1.0]]);
    let blue = Material { base: [0.0, 0.0, 1.0, 1.0], unlit: true, double_sided: true, ..Material::default() };
    let hole = Material { alpha_mode: 1, base: [1.0, 1.0, 1.0, 0.0], double_sided: true, ..Material::default() };
    plain.materials.extend([blue, hole]);
    let cover_all = |z: f32, material: u32| {
        [[-1.0, -1.0, z], [3.0, -1.0, z], [-1.0, 3.0, z]].map(|pos| Vertex {
            pos,
            normal: [0.0, 0.0, -1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
            material,
            ..Vertex::default()
        })
    };
    plain.vertices.extend(cover_all(0.5, 1));
    plain.indices.extend([3, 4, 5]);
    let mut covered = plain.clone();
    covered.vertices.extend(cover_all(0.9, 2));
    covered.indices.extend([6, 7, 8]);
    for s in [&mut plain, &mut covered] {
        s.opaque_count = s.indices.len() as u32;
    }
    let (a, b) = (try_render(&plain, &Limits::default()).unwrap(), try_render(&covered, &Limits::default()).unwrap());
    assert_eq!(a.color[5 * 8 + 6], [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(bits4(&b.color), bits4(&a.color));
}

#[test]
fn negative_shadow_diffusion_is_rejected() {
    for v in [-0.5, -100.0] {
        let mut s = lit_scene((16, 12), 1, false);
        s.lights[1].shadow_diffusion = v;
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadLight(1)), "{v}");
        assert_eq!(try_shadow_maps(&mut s, false, &Limits::default()), Err(RenderError::BadLight(1)), "{v}");
        let mut s = lit_scene((16, 12), 1, false);
        s.env.as_mut().unwrap().shadow_diffusion = v;
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadEnvironment), "{v}");
        let e = EnvInfo { sun: [0.0, -1.0, 0.0], ..s.env.unwrap() };
        assert_eq!(try_env_shadow_map(e, [0.0; 3], 100.0, 512), Err(RenderError::BadShadowInputs), "{v}");
    }
    // Zero keeps the minimum penumbra of eight output pixels.
    let mut s = lit_scene((16, 12), 1, false);
    s.lights[1].shadow_diffusion = 0.0;
    s.env.as_mut().unwrap().shadow_diffusion = 0.0;
    try_shadow_maps(&mut s, false, &Limits::default()).unwrap();
    assert!(try_render(&s, &Limits::default()).is_ok());
    let e = EnvInfo { sun: [0.0, -1.0, 0.0], ..s.env.unwrap() };
    assert_eq!(try_env_shadow_map(e, [0.0; 3], 100.0, 512).map(|m| m.radius), Ok(8.0));
}

#[test]
fn shadow_darkness_and_environment_key_outside_0_to_1_are_rejected() {
    for v in [1.5, -0.25] {
        let mut s = lit_scene((16, 12), 1, false);
        s.lights[1].shadow_darkness = v;
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadLight(1)), "{v}");
        let mut s = lit_scene((16, 12), 1, false);
        s.env.as_mut().unwrap().shadow_darkness = v;
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadEnvironment), "{v}");
        let mut s = lit_scene((16, 12), 1, false);
        s.env.as_mut().unwrap().key = v;
        assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadEnvironment), "{v}");
    }
    // At the top of the range, the shadows-only card's alpha stays within 0 to 1 and encodes.
    let mut s = lit_scene((16, 12), 1, false);
    for l in &mut s.lights {
        l.shadow_darkness = 1.0;
    }
    let e = s.env.as_mut().unwrap();
    (e.shadow_darkness, e.key) = (1.0, 1.0);
    try_shadow_maps(&mut s, false, &Limits::default()).unwrap();
    let (mut img, _) = try_render_cpu(&s, &Limits::default()).unwrap();
    assert!(try_encode(&mut img).is_ok());
}

#[test]
fn coordinates_from_two_to_the_twenty_fourth_keep_the_wrap_period() {
    let texels = [[0.0, 0.0, 0.0, 1.0], [0.25, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 1.0]];
    // Whole, even coordinates: a repeated or a mirrored axis samples as at 0.
    for wrap in [0, 2] {
        let across = TexInfo { offset: 0, width: 3, height: 1, wrap_u: wrap, wrap_v: wrap };
        let down = TexInfo { offset: 0, width: 1, height: 3, wrap_u: wrap, wrap_v: wrap };
        for c in [2f32.powi(24), 2f32.powi(64), -(2f32.powi(64)), 3e38] {
            assert_eq!(sample(&across, &texels, c, 0.5), sample(&across, &texels, 0.0, 0.5), "wrap {wrap}, u {c}");
            assert_eq!(sample(&down, &texels, 0.5, c), sample(&down, &texels, 0.5, 0.0), "wrap {wrap}, v {c}");
        }
    }
}

#[test]
fn a_shadows_only_material_that_receives_no_shadows_draws_nothing() {
    // A parallel light along the view direction, a hidden caster in front and a shadows-only
    // surface behind it that fills the raster.
    let mut s = quad_scene(16, 16, &[([4.0, 4.0, 12.0, 12.0], 100.0, WHITE), ([0.0, 0.0, 16.0, 16.0], 500.0, WHITE)]);
    s.materials[0] = Material { casts_shadows: true, visible: false, ..Material::default() };
    s.materials[1] = Material { shadows_only: true, double_sided: true, ..Material::default() };
    s.opaque_count = 0;
    s.lights = vec![shadow_light(0)];
    try_shadow_maps(&mut s, false, &Limits::default()).unwrap();
    let m = 1;
    let render = |f: fn(&mut Material)| {
        let mut s = s.clone();
        f(&mut s.materials[m]);
        bits4(&try_render(&s, &Limits::default()).unwrap().color)
    };
    let hidden = render(|mat| mat.visible = false);
    assert_ne!(render(|_| {}), hidden, "the surface draws the shadow it receives");
    assert_eq!(render(|mat| mat.receives_shadows = false), hidden);
}

#[test]
fn receivers_far_off_a_shadow_map_with_a_huge_focal_are_unshadowed() {
    // A parallel light along the view direction, a hidden caster in front and a lit surface
    // behind it, with the map's view moved about 100 units off the surface and a focal length
    // that puts the surface's map coordinates near `f32::MAX` (finite at `f32::MAX / 400`,
    // infinite at `f32::MAX`).
    let mut s = quad_scene(16, 16, &[([4.0, 4.0, 12.0, 12.0], 100.0, WHITE), ([0.0, 0.0, 16.0, 16.0], 500.0, WHITE)]);
    s.materials[0] = Material { casts_shadows: true, visible: false, ..Material::default() };
    s.materials[1] = Material { double_sided: true, ..Material::default() };
    s.opaque_count = 0;
    s.lights = vec![shadow_light(0)];
    try_shadow_maps(&mut s, false, &Limits::default()).unwrap();
    let mut unshadowed = s.clone();
    unshadowed.materials[1].receives_shadows = false;
    let unshadowed = bits4(&try_render(&unshadowed, &Limits::default()).unwrap().color);
    assert_ne!(bits4(&try_render(&s, &Limits::default()).unwrap().color), unshadowed, "the caster shadows the surface");
    for focal in [f32::MAX / 400.0, f32::MAX] {
        let mut s = s.clone();
        s.shadows[0].view[0][3] += 100.0;
        s.shadows[0].focal = focal;
        let r = try_render(&s, &Limits::default());
        assert_eq!(r.map(|img| bits4(&img.color)), Ok(unshadowed.clone()), "focal {focal}");
    }
}

/// A light of `kind` at the origin along +z that asks for a shadow map.
fn shadow_light(kind: u32) -> Light {
    Light {
        kind,
        pos: [0.0; 3],
        dir: [0.0, 0.0, 1.0],
        color: [1.0; 3],
        cos_inner: 0.6,
        cos_outer: 0.5,
        falloff: 0,
        radius: 0.0,
        falloff_distance: 0.0,
        shadow: 0,
        shadow_darkness: 1.0,
        shadow_diffusion: 0.0,
    }
}

#[test]
fn shadow_map_work_stops_counting_once_past_the_limit() {
    // 64 small casters in one parallel light's map: each triangle's box holds about 600 texels,
    // and the whole scene about 74,000.
    let quads: Vec<_> = (0..64)
        .map(|i| {
            let (x, y) = ((i % 8) as f32 * 10.0, (i / 8) as f32 * 10.0);
            ([x, y, x + 5.0, y + 5.0], 100.0, WHITE)
        })
        .collect();
    let mut s = quad_scene(16, 16, &quads);
    for m in &mut s.materials {
        m.casts_shadows = true;
    }
    s.lights = vec![shadow_light(0)];
    let needed = |limit: u64| match try_shadow_maps(&mut s.clone(), true, &limits(|l| l.max_work = limit)) {
        Err(RenderError::TooMuchWork { needed, .. }) => needed,
        r => panic!("limit {limit}: {r:?}"),
    };
    let (low, high) = (needed(500), needed(20_000));
    assert!(low < 2_000 && (20_000..22_000).contains(&high), "{low} {high}");
    assert!(try_shadow_maps(&mut s, true, &Limits::default()).is_ok());
}

#[test]
fn shadow_map_work_does_not_count_the_boxes_of_casters_that_cast_nothing() {
    // One large quad in a parallel light's map: its two triangles' boxes hold thousands of texels.
    // As an untextured masked caster with alpha 0 under a cutoff of 0.5, or an opaque-mode caster
    // at an opacity of 0.4, it casts nothing, and the work is one test per caster and map.
    let fits = |f: &dyn Fn(&mut Material)| {
        let mut s = quad_scene(16, 16, &[([0.0, 0.0, 16.0, 16.0], 100.0, WHITE)]);
        alpha_textures(&mut s, &[1.0]);
        s.materials[0].casts_shadows = true;
        f(&mut s.materials[0]);
        s.lights = vec![shadow_light(0)];
        try_shadow_maps(&mut s, true, &limits(|l| l.max_work = 2))
    };
    assert!(matches!(fits(&|_| {}), Err(RenderError::TooMuchWork { needed, .. }) if needed > 1_000));
    assert_eq!(fits(&|m| (m.alpha_mode, m.base[3]) = (1, 0.0)), Ok(()));
    assert_eq!(fits(&|m| m.opacity = 0.4), Ok(()));
    // With a base texture: a blended caster at an opacity of 0, and a masked caster with base
    // alpha 0 under a cutoff of 0.5. A masked caster under a cutoff of 0 still casts.
    let textured = |f: &dyn Fn(&mut Material)| {
        fits(&|m| {
            m.tex_base = 0;
            f(m);
        })
    };
    assert_eq!(textured(&|m| (m.alpha_mode, m.opacity) = (2, 0.0)), Ok(()));
    assert_eq!(textured(&|m| (m.alpha_mode, m.base[3]) = (1, 0.0)), Ok(()));
    assert!(matches!(textured(&|m| (m.alpha_mode, m.base[3], m.alpha_cutoff) = (1, 0.0, 0.0)), Err(RenderError::TooMuchWork { .. })));
}

#[test]
fn spot_map_near_plane_crossings_next_to_a_corner_stay_on_the_plane() {
    // A spot light at the origin along +z (light space is world space) and a caster with corner a
    // 1e10 behind the light and to its left: both edges at a cross the near plane, z = 0.001, two
    // units left of b and c. On the map the caster's edges run from b and c at a slope of ±0.1 to
    // the left edge, about 12,800 texels.
    let covered = |c: [[f32; 3]; 3]| {
        let mut s = Scene { width: 16, height: 16, ssaa: 1, ..Scene::default() };
        s.materials = vec![Material { casts_shadows: true, ..Material::default() }];
        s.vertices = c.iter().map(|&pos| Vertex { pos, ..Vertex::default() }).collect();
        s.indices = vec![0, 1, 2];
        s.opaque_count = 3;
        s.lights = vec![shadow_light(1)];
        try_shadow_maps(&mut s, true, &Limits::default()).unwrap();
        s.shadow_texels.iter().filter(|t| t.is_finite()).count()
    };
    let (a, b, c) = ([-1e10, 0.0, -1e10], [0.0, -0.2, 2.0], [0.0, 0.2, 2.0]);
    for corners in [[a, b, c], [a, c, b]] {
        let n = covered(corners);
        assert!((12_000..13_500).contains(&n), "{corners:?}: {n}");
    }
}

#[test]
fn a_blended_caster_casts_as_it_would_with_a_white_texture() {
    // A blended caster in a parallel light's map, at each alpha, with and without an all-white
    // base texture.
    let covered = |alpha: f32, textured: bool| {
        let mut s = quad_scene(16, 16, &[([0.0, 0.0, 8.0, 8.0], 100.0, [1.0, 1.0, 1.0, alpha])]);
        s.materials[0].casts_shadows = true;
        if textured {
            s.textures = vec![TexInfo { offset: 0, width: 1, height: 1, wrap_u: 0, wrap_v: 0 }];
            s.texels = vec![[1.0; 4]];
            s.materials[0].tex_base = 0;
        }
        s.lights = vec![shadow_light(0)];
        try_shadow_maps(&mut s, true, &Limits::default()).unwrap();
        s.shadow_texels.iter().filter(|t| t.is_finite()).count()
    };
    for alpha in [0.3, 0.5, 0.8] {
        assert_eq!(covered(alpha, false), covered(alpha, true), "{alpha}");
    }
    assert_eq!(covered(0.3, false), 0);
    assert!(covered(0.8, false) > 0);
}

#[test]
fn an_opaque_mode_caster_below_half_opacity_casts_no_shadow() {
    let covered = |opacity: f32| {
        let mut s = quad_scene(16, 16, &[([0.0, 0.0, 8.0, 8.0], 100.0, WHITE)]);
        (s.materials[0].casts_shadows, s.materials[0].opacity) = (true, opacity);
        s.lights = vec![shadow_light(0)];
        try_shadow_maps(&mut s, true, &Limits::default()).unwrap();
        s.shadow_texels.iter().filter(|t| t.is_finite()).count()
    };
    assert_eq!(covered(0.0), 0);
    assert_eq!(covered(0.3), 0);
    assert_eq!(covered(0.5), covered(1.0));
    assert!(covered(1.0) > 0);
}

#[test]
fn metallic_roughness_and_occlusion_strength_outside_0_to_1_are_rejected() {
    let set: [fn(&mut Material, f32); 3] = [|m, v| m.metallic = v, |m, v| m.roughness = v, |m, v| m.occlusion_strength = v];
    for (k, f) in set.iter().enumerate() {
        for v in [-0.25, 1.5] {
            let mut s = lit_scene((16, 12), 1, false);
            f(&mut s.materials[1], v);
            assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadMaterial(1)), "{k} {v}");
        }
        for v in [0.0, 1.0] {
            let mut s = lit_scene((16, 12), 1, false);
            f(&mut s.materials[1], v);
            try_shadow_maps(&mut s, false, &Limits::default()).unwrap();
            assert!(try_render(&s, &Limits::default()).is_ok(), "{k} {v}");
        }
    }
}

#[test]
fn a_spot_light_with_its_cone_cosines_out_of_order_is_rejected() {
    let mut s = lit_scene((16, 12), 1, false);
    (s.lights[1].cos_inner, s.lights[1].cos_outer) = (-1.0, 0.85);
    assert_eq!(try_render(&s, &Limits::default()), Err(RenderError::BadLight(1)));
    // Equal cosines are a cone with no feather, and a parallel or point light does not read them.
    let mut s = lit_scene((16, 12), 1, false);
    s.lights[1].cos_inner = s.lights[1].cos_outer;
    (s.lights[0].cos_inner, s.lights[0].cos_outer, s.lights[2].cos_inner, s.lights[2].cos_outer) = (-1.0, 0.85, -1.0, 0.85);
    try_shadow_maps(&mut s, false, &Limits::default()).unwrap();
    assert!(try_render(&s, &Limits::default()).is_ok());
}

/// A shadow-casting spot light at `pos` along `dir` with the given cone cosines.
fn spot(pos: [f32; 3], dir: [f32; 3], cos_inner: f32, cos_outer: f32) -> Light {
    Light {
        kind: 1,
        pos,
        dir,
        color: [1.0; 3],
        cos_inner,
        cos_outer,
        falloff: 0,
        radius: 0.0,
        falloff_distance: 0.0,
        shadow: 0,
        shadow_darkness: 1.0,
        shadow_diffusion: 0.0,
    }
}

#[test]
fn a_spot_cone_wider_than_one_map_shadows_its_whole_cone() {
    // A receiver at 26..34 and a caster at 16..24 across, the caster halfway along the rays from
    // a spot light at (10, 16, 500) whose cone opens past a hemisphere toward −z. The receiver's
    // center lies 109° off the light's direction, inside the inner cone.
    let mut s = quad_scene(40, 32, &[([26.0, 12.0, 34.0, 20.0], 507.0, WHITE), ([16.0, 10.0, 24.0, 22.0], 503.5, [0.5, 0.5, 0.5, 1.0])]);
    for m in &mut s.materials {
        (m.unlit, m.casts_shadows) = (false, true);
    }
    s.lights = vec![spot([10.0, 16.0, 500.0], [0.0, 0.0, -1.0], -0.5, -0.75)];
    let mut lit = s.clone();
    lit.lights[0].shadow = -1;
    let (lit, _) = try_render_cpu(&lit, &Limits::default()).unwrap();
    try_shadow_maps(&mut s, false, &Limits::default()).unwrap();
    assert_eq!((s.lights[0].shadow, s.shadows.len()), (0, 6));
    let (shadowed, _) = try_render_cpu(&s, &Limits::default()).unwrap();
    let px = 16 * 40 + 30;
    assert!(lit.data[px][0] > 0.1, "{:?}", lit.data[px]);
    assert!(shadowed.data[px][0] < 0.5 * lit.data[px][0], "{:?} {:?}", shadowed.data[px], lit.data[px]);
}

#[test]
fn large_finite_normals_shade_as_their_unit_normals() {
    let mut s = lit_scene((40, 30), 1, false);
    for l in &mut s.lights {
        l.shadow = -1;
    }
    if let Some(e) = &mut s.env {
        e.shadow = -1;
    }
    let (want, _) = try_render_cpu(&s, &Limits::default()).unwrap();
    // The axis-aligned normals scaled by 1e20: their squared lengths overflow f32.
    let mut scaled = 0;
    for v in &mut s.vertices {
        if v.normal.iter().filter(|c| **c != 0.0).count() == 1 {
            v.normal = v.normal.map(|c| c * 1e20);
            scaled += 1;
        }
    }
    assert!(scaled > 0);
    let (got, _) = try_render_cpu(&s, &Limits::default()).unwrap();
    assert_eq!(bits4(&got.data), bits4(&want.data));
}

#[test]
fn a_point_light_whose_squared_distance_overflows_f32_still_lights() {
    // A point light without falloff lights a surface the same from 1e20 units as from 1,000 units
    // in about the same direction; 1e20 squared overflows f32.
    let mut s = quad_scene(8, 8, &[([0.0, 0.0, 8.0, 8.0], 500.0, WHITE)]);
    s.materials[0].unlit = false;
    let mut point = spot([4.0, 4.0, -1000.0], [0.0, 0.0, 1.0], 1.0, 1.0);
    (point.kind, point.shadow) = (2, -1);
    s.lights = vec![point];
    let (near, _) = try_render_cpu(&s, &Limits::default()).unwrap();
    s.lights[0].pos[2] = -1e20;
    let (far, _) = try_render_cpu(&s, &Limits::default()).unwrap();
    let px = 4 * 8 + 4;
    assert!(near.data[px][0] > 0.1, "{:?}", near.data[px]);
    assert!((far.data[px][0] - near.data[px][0]).abs() < 1e-3 * near.data[px][0], "{:?} {:?}", far.data[px], near.data[px]);
}

#[test]
fn coordinates_from_two_to_the_twenty_third_texels_keep_the_half_texel_shift() {
    let texels = [[0.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 1.0]];
    // Repeated, 2^23 across two texels is a whole period, so it samples as at 0: halfway between
    // the last texel and the first.
    let repeat = TexInfo { offset: 0, width: 2, height: 1, wrap_u: 0, wrap_v: 0 };
    assert_eq!(sample(&repeat, &texels, 0.0, 0.5)[0], 0.5);
    assert_eq!(sample(&repeat, &texels, 8_388_608.0, 0.5)[0], 0.5);
    // Mirrored, 2^22 + 0.5 is 0.5 past a whole period of 2, halfway between the two texels.
    let mirror = TexInfo { wrap_u: 2, wrap_v: 2, ..repeat };
    assert_eq!(sample(&mirror, &texels, 0.5, 0.5)[0], 0.5);
    assert_eq!(sample(&mirror, &texels, 4_194_304.5, 0.5)[0], 0.5);
}

#[test]
fn a_block_of_more_than_two_to_the_twenty_fourth_subsamples_counts_each_one() {
    // 4097² = 16,785,409 subsamples, the first half at depth 0 and the rest at depth 1. An f32
    // count stops at 2^24 = 16,777,216.
    let k = 4097u32;
    let n = (k * k) as usize;
    let depth: Vec<f32> = (0..n).map(|i| if i < n / 2 { 0.0 } else { 1.0 }).collect();
    let (_, d) = try_resolve(&target(k, k, vec![[0.0; 4]; n], depth), k).unwrap();
    assert_eq!(d, vec![((n - n / 2) as f64 / n as f64) as f32]);
}

#[test]
fn encoding_names_the_first_pixel_that_fails() {
    // Pixel 0 is finite but its color divided by its alpha overflows; pixel 1 has a negative alpha.
    let mut img = Image { width: 2, height: 1, data: vec![[3e38, 0.0, 0.0, 1e-5], [0.0, 0.0, 0.0, -1.0]] };
    let before = img.clone();
    assert_eq!(try_encode(&mut img), Err(RenderError::BadPixel(0)));
    assert_eq!(img, before);
}
