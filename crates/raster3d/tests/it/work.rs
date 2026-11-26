//! Work limits, the transparent-pass split and projection at the ends of the `f32` range.

use craft_raster3d::{Limits, Material, RenderError, Scene, Vertex, try_render};

use crate::{limits, quad_scene};

const WHITE: [f32; 4] = [1.0; 4];

#[test]
fn a_material_with_an_opacity_below_one_is_transparent() {
    let m = |opacity: f32| Material { opacity, ..Material::default() }.transparent();
    assert!(!m(1.0));
    assert!(m(0.9995));
    assert!(m(0.5));
}

/// Two screen-filling quads; the nearer one is a masked material whose alpha fails the cutoff, so
/// every pixel is discarded and the search for a surviving fragment visits the farther quad.
fn discarding_scene() -> Scene {
    let mut s = quad_scene(4, 4, &[([0.0, 0.0, 4.0, 4.0], 100.0, WHITE), ([0.0, 0.0, 4.0, 4.0], 50.0, WHITE)]);
    s.materials[1].alpha_mode = 1;
    s.materials[1].base[3] = 0.0;
    s
}

#[test]
fn the_search_under_discarded_pixels_counts_toward_the_work_limit() {
    let s = discarding_scene();
    // Setup charges the bounding box plus one for each of the four triangles.
    let setup = 4 * (16 + 1);
    let t = try_render(&s, &limits(|l| l.max_work = u64::MAX)).unwrap();
    assert_eq!(t.color[0], WHITE);
    assert!(
        matches!(try_render(&s, &limits(|l| l.max_work = setup)), Err(RenderError::TooMuchWork { limit, needed }) if limit == setup && needed > setup)
    );
    // Without a discarded pixel the render stays within the setup charge.
    let mut kept = s;
    kept.materials[1].base[3] = 1.0;
    assert!(try_render(&kept, &limits(|l| l.max_work = setup)).is_ok());
}

#[test]
fn a_triangle_with_corners_of_very_different_w_covers_the_pixels_of_its_top_edge() {
    // The raster is 4 × 4. The top edge runs from pixel center (0.5, 0.5) to (3.5, 0.5) at clip
    // w = 2^30 and the third corner is at (2, 3) with w = 2^-130, so the ratio of the edge
    // corners' 1/w to the third corner's is 2^-160, which `f32` cannot hold.
    let render = |big: f32, tiny: f32| {
        let mut s = quad_scene(4, 4, &[]);
        s.materials.push(Material { unlit: true, double_sided: true, ..Material::default() });
        s.clip = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 0.5, 0.0], [0.0, 0.0, 1.0, 0.0]];
        for pos in [[-0.75 * big, 0.75 * big, big], [0.75 * big, 0.75 * big, big], [0.0, -0.5 * tiny, tiny]] {
            s.vertices.push(Vertex { pos, normal: [0.0, 0.0, -1.0], uv: [0.0; 2], tangent: [1.0, 0.0, 0.0, 1.0], material: 0 });
        }
        s.indices = vec![0, 1, 2];
        s.opaque_count = 3;
        try_render(&s, &Limits::default()).unwrap().color.iter().map(|c| c[3]).collect::<Vec<_>>()
    };
    let covered = render(1.0, 1.0);
    assert_eq!(covered.iter().filter(|&&a| a == 1.0).count(), 5, "{covered:?}");
    // `powi` with a negative exponent inverts a power that overflows, so 2^-130 is built from two factors.
    assert_eq!(render(2.0f32.powi(30), 2.0f32.powi(-65) * 2.0f32.powi(-65)), covered);
}
