//! Camera fitting and the render ceilings: fitted cameras frame what they fit, degenerate input
//! is an error or a documented fallback, the default limits sit at their ceilings, and the fit
//! matches the app's previous code (`crate::reference::pdfcraft`) on the same inputs.

use craft_raster3d::{
    Bounds, Camera, CameraError, Limits, MAX_RENDER_PIXELS, MAX_RENDER_TRIANGLES, MAX_RENDER_VERTICES, MAX_RENDER_WORK, Material, Projection,
    RenderError, Scene, Vertex, try_render,
};

use crate::reference::pdfcraft as pc;
use crate::{limits, quad_scene};

const UP: [f64; 3] = [0.0, 1.0, 0.0];

/// A `w × h` scene holding the twelve triangles of box `b`, unlit and double-sided.
fn box_scene(b: &Bounds, w: u32, h: u32) -> Scene {
    let mut s = Scene { width: w, height: h, ssaa: 1, linear_io: true, ..Scene::default() };
    s.materials.push(Material { base: [1.0, 0.5, 0.25, 1.0], unlit: true, double_sided: true, ..Material::default() });
    for k in 0..8u32 {
        let pos = [0, 1, 2].map(|i| (if k >> i & 1 == 0 { b.min[i] } else { b.max[i] }) as f32);
        s.vertices.push(Vertex { pos, normal: [0.0, 0.0, -1.0], tangent: [1.0, 0.0, 0.0, 1.0], ..Vertex::default() });
    }
    // Two triangles per face, by the corner bits x = 1, y = 2, z = 4.
    for [a, b, c, d] in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
        s.indices.extend([a, b, c, a, c, d]);
    }
    s.opaque_count = s.indices.len() as u32;
    s
}

/// The normalized device coordinates of world point `p` under the scene's clip transform.
fn ndc(s: &Scene, p: [f64; 3]) -> [f64; 3] {
    let c: [f64; 4] = std::array::from_fn(|r| (0..3).map(|k| f64::from(s.clip[r][k]) * p[k]).sum::<f64>() + f64::from(s.clip[r][3]));
    [c[0] / c[3], c[1] / c[3], c[2] / c[3]]
}

fn corners(b: &Bounds) -> impl Iterator<Item = [f64; 3]> + '_ {
    (0..8).map(|k| [0, 1, 2].map(|i| if k >> i & 1 == 0 { b.min[i] } else { b.max[i] }))
}

fn bad(r: Result<impl std::fmt::Debug, RenderError>) -> CameraError {
    match r {
        Err(RenderError::BadCamera(e)) => e,
        other => panic!("expected a camera error, got {other:?}"),
    }
}

#[test]
fn a_fitted_camera_keeps_every_box_corner_inside_the_view() {
    let b = Bounds { min: [-3.0, -1.0, 2.0], max: [5.0, 4.0, 3.5] };
    let directions = [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0], [1.0, -0.7, 0.4], [-0.3, 0.2, -1.0]];
    for perspective in [true, false] {
        for (w, h) in [(40, 40), (80, 40), (30, 60)] {
            for dir in directions {
                let mut s = box_scene(&b, w, h);
                let camera = Camera::fit(&b, dir, UP, perspective, f64::from(w) / f64::from(h)).unwrap();
                camera.apply(&mut s).unwrap();
                assert_eq!(s.ortho, !perspective);
                let mut widest = 0.0f64;
                for p in corners(&b) {
                    let [x, y, z] = ndc(&s, p);
                    // 5% clear on each side of the tighter axis, within f32 rounding.
                    assert!(x.abs() <= 1.0 / 1.1 + 1e-5 && y.abs() <= 1.0 / 1.1 + 1e-5, "{perspective} {w}x{h} {dir:?}: {x} {y}");
                    assert!((0.0..=1.0).contains(&z), "{perspective} {w}x{h} {dir:?}: depth {z}");
                    widest = widest.max(x.abs()).max(y.abs());
                }
                // The tighter axis is filled to the margin.
                assert!((widest - 1.0 / 1.1).abs() < 1e-4, "{perspective} {w}x{h} {dir:?}: {widest}");
                // Drawn: the center is covered, and nothing reaches the border.
                let t = try_render(&s, &Limits::default()).unwrap();
                assert_ne!(t.color[(h / 2 * w + w / 2) as usize][3], 0.0);
                for x in 0..w {
                    assert_eq!(t.color[x as usize][3], 0.0);
                    assert_eq!(t.color[((h - 1) * w + x) as usize][3], 0.0);
                }
                for y in 0..h {
                    assert_eq!(t.color[(y * w) as usize][3], 0.0);
                    assert_eq!(t.color[(y * w + w - 1) as usize][3], 0.0);
                }
            }
        }
    }
}

#[test]
fn perspective_makes_the_nearer_face_larger_and_orthographic_does_not() {
    // Seen along +z, the face at z = 0 is nearer than the face at z = 4.
    let b = Bounds { min: [-1.0, -1.0, 0.0], max: [1.0, 1.0, 4.0] };
    // Right is forward × up, so world +x is on the image's left here.
    let width = |s: &Scene, z: f64| (ndc(s, [1.0, 0.0, z])[0] - ndc(s, [-1.0, 0.0, z])[0]).abs();
    let mut s = box_scene(&b, 40, 40);
    Camera::fit(&b, [0.0, 0.0, 1.0], UP, true, 1.0).unwrap().apply(&mut s).unwrap();
    assert!(width(&s, 0.0) > 1.5 * width(&s, 4.0), "{} {}", width(&s, 0.0), width(&s, 4.0));
    // Larger normalized device z is nearer.
    assert!(ndc(&s, [0.0, 0.0, 0.0])[2] > ndc(&s, [0.0, 0.0, 4.0])[2]);
    Camera::fit(&b, [0.0, 0.0, 1.0], UP, false, 1.0).unwrap().apply(&mut s).unwrap();
    assert!((width(&s, 0.0) - width(&s, 4.0)).abs() < 1e-6);
    assert!(ndc(&s, [0.0, 0.0, 0.0])[2] > ndc(&s, [0.0, 0.0, 4.0])[2]);
}

#[test]
fn a_perspective_camera_keeps_the_nearest_point_past_its_near_plane() {
    let b = Bounds { min: [0.0, 0.0, 0.0], max: [10.0, 1.0, 10.0] };
    let mut s = box_scene(&b, 64, 32);
    let camera = Camera::fit(&b, [0.3, -0.5, 1.0], UP, true, 2.0).unwrap();
    camera.apply(&mut s).unwrap();
    let depth = |p: [f64; 3]| (0..3).map(|k| f64::from(s.view[2][k]) * p[k]).sum::<f64>() + f64::from(s.view[2][3]);
    let near = f64::from(s.clip[2][3]);
    assert!(near > 0.0);
    for p in corners(&b) {
        assert!(depth(p) > near, "{p:?}: {} <= {near}", depth(p));
    }
}

#[test]
fn a_scene_fit_frames_drawn_vertices_and_ignores_empty_box_corners() {
    // A thin strip along the box's diagonal: seen across the diagonal, the box is as tall as it
    // is wide, the strip is not.
    let mut s = box_scene(&Bounds { min: [0.0; 3], max: [0.0; 3] }, 40, 20);
    s.vertices.truncate(3);
    for (v, pos) in s.vertices.iter_mut().zip([[0.0, 0.0, 0.0], [10.0, 10.0, 0.0], [10.0, 10.01, 0.0]]) {
        v.pos = pos;
    }
    s.indices = vec![0, 1, 2];
    s.opaque_count = 3;
    let (dir, up) = ([0.0, 0.0, -1.0], [1.0, -1.0, 0.0]);
    let height = |c: Camera| match c.projection {
        Projection::Orthographic { height } => height,
        Projection::Perspective { .. } => panic!("orthographic expected"),
    };
    let boxed = height(Camera::fit(&Bounds::of_scene(&s).unwrap(), dir, up, false, 2.0).unwrap());
    let tight = height(Camera::fit_scene(&s, dir, up, false, 2.0).unwrap());
    assert!((boxed / tight - 2.0).abs() < 0.01, "{boxed} vs {tight}");
    // A hidden triangle, or one with a vertex that does not exist, does not widen the box.
    s.materials.push(Material { visible: false, ..Material::default() });
    s.vertices.push(Vertex { pos: [1e6, 0.0, 0.0], material: 1, ..Vertex::default() });
    s.indices.extend([3, 0, 1, 0, 1, 99]);
    assert_eq!(Bounds::of_scene(&s).unwrap().max, [10.0, 10.01, 0.0].map(|c: f32| f64::from(c)));
    assert_eq!(height(Camera::fit_scene(&s, dir, up, false, 2.0).unwrap()), tight);
}

const VIEWS: [([f64; 3], [f64; 3]); 4] =
    [([0.0, 0.0, -1.0], UP), ([1.0, 1.0, 1.0], UP), ([0.0, 0.3, -1.0], [1.0, 1.0, 0.0]), ([2.0, -5.0, 0.5], [0.0, 0.0, 3.0])];

fn theirs(c: pc::Camera) -> Camera {
    let projection = match c.projection {
        pc::Projection::Perspective { fov_y } => Projection::Perspective { fov_y },
        pc::Projection::Orthographic { height } => Projection::Orthographic { height },
    };
    Camera { eye: c.eye, target: c.target, up: c.up, projection }
}

/// Every field of a camera as bits, so `+0.0` and `-0.0` differ.
fn bits(c: &Camera) -> [u64; 11] {
    let (kind, p) = match c.projection {
        Projection::Perspective { fov_y } => (0, fov_y),
        Projection::Orthographic { height } => (1, height),
    };
    let mut out = [kind, p.to_bits(), 0, 0, 0, 0, 0, 0, 0, 0, 0];
    for (o, v) in out[2..].iter_mut().zip(c.eye.iter().chain(&c.target).chain(&c.up)) {
        *o = v.to_bits();
    }
    out
}

/// Whether every point is inside the orthographic camera's view, measured in its own basis.
fn ortho_holds(c: &Camera, aspect: f64, points: &[[f64; 3]]) -> bool {
    let Projection::Orthographic { height } = c.projection else { panic!("orthographic expected") };
    let [r, u, _] = c.basis().unwrap();
    points.iter().all(|p| {
        let d = [0, 1, 2].map(|i| p[i] - c.target[i]);
        let along = |a: [f64; 3]| a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
        along(u).abs() <= height / 2.0 && along(r).abs() <= height / 2.0 * aspect
    })
}

#[test]
fn camera_fits_match_the_previous_code() {
    // Each case fits on both sides, and every field matches bit for bit.
    let boxes = [
        Bounds { min: [-3.0, -1.0, 2.0], max: [5.0, 4.0, 3.5] },
        Bounds { min: [0.0; 3], max: [0.0; 3] },
        Bounds { min: [-1e150, 2.0, -7.0], max: [1e150, 3.0, 9.0] },
    ];
    for b in boxes {
        let pb = pc::Bounds { min: b.min, max: b.max };
        let points: Vec<[f64; 3]> = corners(&b).take(5).collect();
        for (dir, up) in VIEWS {
            for perspective in [true, false] {
                for aspect in [0.5, 1.0, 2.0] {
                    let case = format!("{b:?} {dir:?} {perspective} {aspect}");
                    let ours = Camera::fit(&b, dir, up, perspective, aspect).unwrap_or_else(|e| panic!("{case}: {e:?}"));
                    let prev = pc::Camera::fit(&pb, dir, up, perspective, aspect).unwrap_or_else(|e| panic!("{case}: {e:?}"));
                    assert_eq!(bits(&ours), bits(&theirs(prev)), "{case}: {ours:?}");
                    // The scene's vertices are f32; these points are exact in f32 only for the
                    // first two boxes.
                    if b.min.iter().chain(&b.max).all(|&c| f64::from(c as f32) == c) {
                        let prev = pc::Camera::fit_points(&points, dir, up, perspective, aspect).unwrap();
                        let mut s = box_scene(&b, 8, 8);
                        s.indices = vec![0, 1, 2, 3, 4, 2];
                        s.opaque_count = 6;
                        let ours = Camera::fit_scene(&s, dir, up, perspective, aspect).unwrap_or_else(|e| panic!("{case}: {e:?}"));
                        assert_eq!(bits(&ours), bits(&theirs(prev)), "{case} (scene): {ours:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn an_orthographic_fit_whose_eye_rounds_is_widened_to_hold_its_points() {
    // Far from the origin, rounding the eye to f64 turns the view: the previous code's camera
    // misses two corners in its own basis, and ours is widened around the same target.
    let b = Bounds { min: [1e16, -1.0, 0.0], max: [1e16, 1.0, 0.0] };
    let (dir, up) = ([1.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
    let points: Vec<[f64; 3]> = corners(&b).collect();
    let prev = theirs(pc::Camera::fit(&pc::Bounds { min: b.min, max: b.max }, dir, up, false, 1.0).unwrap());
    assert!(!ortho_holds(&prev, 1.0, &points), "{prev:?}");
    let ours = Camera::fit(&b, dir, up, false, 1.0).unwrap();
    assert!(ortho_holds(&ours, 1.0, &points), "{ours:?}");
    assert_eq!((ours.eye, ours.target, ours.up), (prev.eye, prev.target, prev.up));
    // A box near 1e16 in every view: ours holds its points, and is the previous camera where
    // that holds them too, or it is an error because its f32 transform would clip a point.
    let b = Bounds { min: [1e16, 1e16, 1e16], max: [1e16 + 4.0, 1e16 + 4.0, 1e16] };
    let points: Vec<[f64; 3]> = corners(&b).collect();
    for (dir, up) in VIEWS {
        for aspect in [0.5, 1.0, 2.0] {
            let prev = theirs(pc::Camera::fit(&pc::Bounds { min: b.min, max: b.max }, dir, up, false, aspect).unwrap());
            match Camera::fit(&b, dir, up, false, aspect) {
                Ok(ours) => {
                    assert!(ortho_holds(&ours, aspect, &points), "{dir:?} {aspect}: {ours:?}");
                    if ortho_holds(&prev, aspect, &points) {
                        assert_eq!(bits(&ours), bits(&prev), "{dir:?} {aspect}");
                    }
                }
                Err(e) => assert_eq!(e, RenderError::BadCamera(CameraError::OutOfRange), "{dir:?} {aspect}"),
            }
        }
    }
}

#[test]
fn a_fit_that_the_f32_transform_would_clip_is_an_error() {
    let z = [0.0, 0.0, 1.0];
    let cases = [
        // Seen along +z, the face at z = 1e8 is 2 units past the eye in f64, but in f32 the eye
        // rounds onto that face and the rasterizer would clip it.
        (Bounds { min: [-0.5, -0.5, 1e8], max: [0.5, 0.5, 1e8 + 16.0] }, z, UP, true),
        // The f32 x row's offset rounds by about one image width.
        (Bounds { min: [10_000_006.0, -1.0, 0.0], max: [10_000_008.0, 1.0, 0.0] }, z, UP, false),
    ];
    for (b, dir, up, perspective) in cases {
        let case = format!("{b:?} {dir:?} {perspective}");
        assert!(pc::Camera::fit(&pc::Bounds { min: b.min, max: b.max }, dir, up, perspective, 1.0).is_ok(), "{case}");
        assert_eq!(bad(Camera::fit(&b, dir, up, perspective, 1.0)), CameraError::OutOfRange, "{case}");
        let s = box_scene(&b, 8, 8);
        assert_eq!(bad(Camera::fit_scene(&s, dir, up, perspective, 1.0)), CameraError::OutOfRange, "{case}");
    }
    // The same boxes near the origin fit, and every corner is inside the f32 transform.
    let near_origin =
        [(Bounds { min: [-0.5, -0.5, 0.0], max: [0.5, 0.5, 16.0] }, true), (Bounds { min: [6.0, -1.0, 0.0], max: [8.0, 1.0, 0.0] }, false)];
    for (b, perspective) in near_origin {
        let mut s = box_scene(&b, 8, 8);
        Camera::fit(&b, z, UP, perspective, 1.0).unwrap().apply(&mut s).unwrap();
        for p in corners(&b) {
            let q = p.map(|c| c as f32);
            let c: [f32; 4] = std::array::from_fn(|r| s.clip[r][0] * q[0] + s.clip[r][1] * q[1] + s.clip[r][2] * q[2] + s.clip[r][3]);
            assert!(c[3] > 0.0 && c[0].abs() <= c[3] && c[1].abs() <= c[3] && (0.0..=c[3]).contains(&c[2]), "{p:?}: {c:?}");
        }
    }
}

#[test]
fn invalid_bounds_are_rejected_and_a_point_has_a_finite_camera() {
    for (min, max) in [
        ([f64::NAN, 0.0, 0.0], [1.0; 3]),
        ([0.0; 3], [f64::INFINITY, 1.0, 1.0]),
        ([f64::NEG_INFINITY, 0.0, 0.0], [1.0; 3]),
        ([0.0, 2.0, 0.0], [1.0; 3]),
    ] {
        assert_eq!(bad(Camera::fit(&Bounds { min, max }, [0.0, 0.0, 1.0], UP, true, 1.0)), CameraError::Bounds, "{min:?} {max:?}");
    }
    // A point, or a box flat in the view direction, is framed as a box of radius 1 or by its
    // extent, with finite transforms.
    for b in [Bounds { min: [2.0, -3.0, 5.0], max: [2.0, -3.0, 5.0] }, Bounds { min: [0.0, 0.0, 1.0], max: [4.0, 2.0, 1.0] }] {
        for perspective in [true, false] {
            let camera = Camera::fit(&b, [0.0, 0.0, 1.0], UP, perspective, 1.5).unwrap();
            assert!(camera.eye.iter().chain(&camera.target).all(|c| c.is_finite()), "{camera:?}");
            let mut s = box_scene(&b, 30, 20);
            camera.apply(&mut s).unwrap();
            let [x, y, z] = ndc(&s, b.min);
            assert!(x.abs() <= 1.0 && y.abs() <= 1.0 && (0.0..=1.0).contains(&z), "{b:?} {perspective}: {x} {y} {z}");
        }
    }
}

#[test]
fn a_huge_finite_box_has_a_finite_radius_and_is_never_framed_as_radius_1() {
    // The full diagonal overflows f64, so the previous code fell back to a radius of 1.
    let b = Bounds { min: [-1e308, -1e308, 0.0], max: [1e308, 1e308, 0.0] };
    assert!(b.radius().is_finite() && b.radius() > 1e308, "{}", b.radius());
    assert_eq!(pc::Bounds { min: b.min, max: b.max }.radius(), f64::INFINITY);
    for perspective in [true, false] {
        assert_eq!(bad(Camera::fit(&b, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], perspective, 1.0)), CameraError::OutOfRange);
    }
    // The previous code put the eye 2 units from the box center, inside the box.
    let prev = pc::Camera::fit(&pc::Bounds { min: b.min, max: b.max }, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], false, 1.0).unwrap();
    assert_eq!(prev.eye, [-2.0, 0.0, 0.0]);
}

#[test]
fn an_eye_that_rounds_onto_its_target_is_an_error() {
    // A point near 1e300: a step of a few units is below its precision.
    let b = Bounds { min: [1e300; 3], max: [1e300; 3] };
    for perspective in [true, false] {
        assert_eq!(bad(Camera::fit(&b, [1.0, 0.0, 0.0], UP, perspective, 1.0)), CameraError::OutOfRange);
    }
    // A point at 1e8 fits in f64, but its eye 0.001 away rounds onto the target in f32, so the
    // perspective camera would clip the point: the fit is an error, and so is applying a camera
    // placed there by hand.
    let b = Bounds { min: [1e8, 0.0, 0.0], max: [1e8, 0.0, 0.0] };
    assert_eq!(bad(Camera::fit(&b, [1.0, 0.0, 0.0], UP, true, 1.0)), CameraError::OutOfRange);
    let camera = Camera { eye: [1e8 - 0.001, 0.0, 0.0], target: b.min, up: UP, projection: Projection::Perspective { fov_y: 30.0 } };
    let mut s = box_scene(&b, 8, 8);
    assert_eq!(bad(camera.apply(&mut s)), CameraError::OutOfRange);
}

#[test]
fn bad_aspects_directions_and_up_vectors_are_rejected() {
    let b = Bounds { min: [0.0; 3], max: [1.0; 3] };
    for aspect in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(bad(Camera::fit(&b, [0.0, 0.0, 1.0], UP, true, aspect)), CameraError::Aspect, "{aspect}");
    }
    for dir in [[0.0; 3], [f64::NAN, 0.0, 1.0], [f64::INFINITY, 0.0, 0.0]] {
        assert_eq!(bad(Camera::fit(&b, dir, UP, true, 1.0)), CameraError::Direction, "{dir:?}");
    }
    for up in [[0.0; 3], [0.0, 0.0, 2.0], [0.0, f64::NAN, 0.0], [0.0, f64::INFINITY, 0.0]] {
        assert_eq!(bad(Camera::fit(&b, [0.0, 0.0, 1.0], up, false, 1.0)), CameraError::Up, "{up:?}");
    }
    // An up direction whose cross product with the view overflows is still usable.
    assert!(Camera::fit(&b, [0.0, 0.0, 1.0], [0.0, 1e308, 0.0], false, 1.0).is_ok());
    // A scene with no drawn vertex cannot be fitted.
    let empty = box_scene(&b, 8, 8);
    let empty = Scene { indices: vec![], opaque_count: 0, ..empty };
    assert_eq!(bad(Camera::fit_scene(&empty, [0.0, 0.0, 1.0], UP, true, 1.0)), CameraError::Empty);
}

#[test]
fn applying_a_camera_checks_its_projection_and_range_and_leaves_the_scene_on_error() {
    let b = Bounds { min: [0.0; 3], max: [1.0; 3] };
    let base = Camera { eye: [0.5, 0.5, -5.0], target: [0.5; 3], up: UP, projection: Projection::Perspective { fov_y: 40.0 } };
    let scene = box_scene(&b, 16, 16);
    let cases = [
        (Camera { projection: Projection::Perspective { fov_y: 180.0 }, ..base }, CameraError::Projection),
        (Camera { projection: Projection::Perspective { fov_y: 0.0 }, ..base }, CameraError::Projection),
        (Camera { projection: Projection::Perspective { fov_y: f64::NAN }, ..base }, CameraError::Projection),
        (Camera { projection: Projection::Perspective { fov_y: 1e-310 }, ..base }, CameraError::Projection),
        (Camera { projection: Projection::Orthographic { height: 0.0 }, ..base }, CameraError::Projection),
        (Camera { projection: Projection::Orthographic { height: f64::INFINITY }, ..base }, CameraError::Projection),
        (Camera { target: base.eye, ..base }, CameraError::OutOfRange),
        (Camera { up: [0.0, 0.0, 1.0], ..base }, CameraError::Up),
        // Opposite ends of f64: the eye does not fit f32.
        (Camera { eye: [-1e308; 3], target: [1e308; 3], ..base }, CameraError::OutOfRange),
        // A near plane that underflows f32.
        (Camera { eye: [0.0, 0.0, -1e-40], target: [0.0; 3], ..base }, CameraError::OutOfRange),
        // An orthographic depth range past f32.
        (Camera { eye: [0.0, 0.0, -1e300], target: [0.0; 3], projection: Projection::Orthographic { height: 2.0 }, ..base }, CameraError::OutOfRange),
        // A visible height so large that both lateral clip rows round to zero in f32.
        (Camera { projection: Projection::Orthographic { height: 1e100 }, ..base }, CameraError::OutOfRange),
    ];
    for (camera, want) in cases {
        let mut s = scene.clone();
        assert_eq!(bad(camera.apply(&mut s)), want, "{camera:?}");
        assert_eq!((s.clip, s.view, s.eye, s.cam_fwd, s.ortho), (scene.clip, scene.view, scene.eye, scene.cam_fwd, scene.ortho));
    }
    // On a raster 2^30 times wider than tall, only the horizontal row rounds to zero.
    let wide = Scene { width: 1 << 30, height: 1, ..scene.clone() };
    let mut s = wide.clone();
    let camera = Camera { projection: Projection::Orthographic { height: 1e37 }, ..base };
    assert_eq!(bad(camera.apply(&mut s)), CameraError::OutOfRange);
    assert_eq!(s.clip, wide.clip);
    // With a height that keeps both rows, the same raster applies.
    Camera { projection: Projection::Orthographic { height: 1e30 }, ..base }.apply(&mut s).unwrap();
    let mut s = Scene { width: 0, ..scene.clone() };
    assert_eq!(base.apply(&mut s), Err(RenderError::BadSize(0, 16)));
}

#[test]
fn the_default_limits_sit_at_the_render_ceilings() {
    let l = Limits::default();
    assert_eq!(
        (l.max_pixels, l.max_work, l.max_triangles, l.max_vertices),
        (MAX_RENDER_PIXELS, MAX_RENDER_WORK, MAX_RENDER_TRIANGLES, MAX_RENDER_VERTICES)
    );
    assert_eq!((MAX_RENDER_PIXELS, MAX_RENDER_WORK, MAX_RENDER_TRIANGLES, MAX_RENDER_VERTICES), (1 << 24, 1 << 31, 1 << 24, 1 << 24));
    // At the pixel ceiling the size passes; one pixel row or column more is refused.
    assert_eq!(l.check(4096, 4096), Ok(1 << 24));
    assert_eq!(l.check(1 << 24, 1), Ok(1 << 24));
    assert_eq!(l.check(4097, 4096), Err(RenderError::TooLarge(4097, 4096)));
    assert_eq!(l.check((1 << 24) + 1, 1), Err(RenderError::TooLarge((1 << 24) + 1, 1)));
    let n = 1 << 24;
    assert_eq!(l.check_geometry(n, n), Ok(()));
    assert_eq!(l.check_geometry(n + 1, n), Err(RenderError::TooManyTriangles { count: n + 1, limit: n }));
    assert_eq!(l.check_geometry(n, n + 1), Err(RenderError::TooManyVertices { count: n + 1, limit: n }));
}

#[test]
fn work_at_the_limit_passes_and_one_over_is_refused() {
    // Two triangles covering 6 × 6 pixels: two 6 × 6 boxes plus one each, 74 tests.
    let s = quad_scene(8, 8, &[([1.0, 1.0, 7.0, 7.0], 100.0, [1.0; 4])]);
    assert!(try_render(&s, &limits(|l| l.max_work = 74)).is_ok());
    assert_eq!(try_render(&s, &limits(|l| l.max_work = 73)), Err(RenderError::TooMuchWork { needed: 74, limit: 73 }));
}

#[test]
fn the_search_under_discarded_pixels_spends_work() {
    // The same quad, single-sided or alpha-masked: its triangles can discard fragments, so the
    // search under discarded pixels may visit both boxes again, 74 more tests.
    for f in [|m: &mut Material| m.double_sided = false, |m: &mut Material| m.alpha_mode = 1] {
        let mut s = quad_scene(8, 8, &[([1.0, 1.0, 7.0, 7.0], 100.0, [1.0; 4])]);
        f(&mut s.materials[0]);
        assert!(try_render(&s, &limits(|l| l.max_work = 148)).is_ok());
        assert_eq!(try_render(&s, &limits(|l| l.max_work = 147)), Err(RenderError::TooMuchWork { needed: 148, limit: 147 }));
    }
}
