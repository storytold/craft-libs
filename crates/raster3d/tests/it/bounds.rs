//! Hostile and malformed input: every entry point returns an error or a defined result before it
//! allocates the target or indexes past an array, and never panics.

use craft_raster3d::{
    EnvInfo, Image, Limits, Material, RenderError, Scene, Target, TexInfo, Vertex, linear_to_srgb, sample, try_encode, try_env_shadow_map,
    try_render, try_render_cpu, try_resolve, try_shadow_maps,
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
    assert_eq!(try_shadow_maps(&mut s, false, &Limits::default()), Err(RenderError::BadShadowInputs));
    // Without a request, the direction is not used for a map.
    s.lights[i].shadow = -1;
    assert!(try_shadow_maps(&mut s, false, &Limits::default()).is_ok());
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

#[test]
fn a_caster_whose_spot_map_corners_overflow_is_rejected() {
    // Only the spot light asks for a map, so the map's view comes from the light alone.
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
    // One caster corner far along the light's direction and to its side: finite in the view,
    // past f32 once scaled by the map's focal length.
    let i = s.vertices.iter().position(|v| s.materials[v.material as usize].casts_shadows).unwrap();
    let side = [spot.dir[1], -spot.dir[0], 0.0];
    s.vertices[i].pos = [0, 1, 2].map(|k| spot.pos[k] + spot.dir[k] * 1e38 + side[k] * 1e37);
    assert_eq!(try_shadow_maps(&mut s, false, &Limits::default()), Err(RenderError::BadShadowInputs));
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
        // Finite before and after the division; the curve's linear segment overflows.
        [-1e38, 0.0, 0.0, 1.0],
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
