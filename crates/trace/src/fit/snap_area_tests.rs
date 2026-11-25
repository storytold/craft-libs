use super::*;
use crate::{Raster, preset, trace};

#[test]
fn snap_preserves_the_interior_of_a_thin_zig_at_different_positions() {
    for (ox, oy) in [(10, 10), (210, 85)] {
        let image = Raster::from_fn(256, 128, |x, y| {
            if [(ox, oy), (ox + 1, oy), (ox + 1, oy + 1), (ox + 2, oy + 1)].contains(&(x, y)) { [0, 0, 0, 255] } else { [255; 4] }
        });
        for corners in [0.0, 25.0, 50.0, 75.0, 100.0] {
            for snap in [false, true] {
                let mut p = preset("Black and White Logo").unwrap();
                p.paths = 50.0;
                p.corners = corners;
                p.noise = 1;
                p.ignore_white = true;
                p.snap_curves_to_lines = snap;
                let out = trace(&image, &p);
                assert_eq!(out.paths.len(), 1);
                assert!(out.paths[0].path.subpaths[0].area().abs() > 1.0, "position=({ox},{oy}), corners={corners}, snap={snap}");
                p.noise = 5;
                assert!(trace(&image, &p).paths.is_empty(), "Noise must still remove the four-pixel mark");
            }
        }
    }
}

fn thin_outline() -> Vec<Point> {
    vec![
        Point::new(0.0, 0.0),
        Point::new(2.00390625, 0.501953125),
        Point::new(2.501953125, 1.0),
        Point::new(3.00390625, 2.00390625),
        Point::new(1.0, 1.501953125),
        Point::new(0.501953125, 1.00390625),
    ]
}

fn hard_style() -> Style {
    let mut p = crate::preset("Black and White Logo").unwrap();
    p.paths = 50.0;
    p.corners = 0.0;
    let mut st = p.outline_style();
    st.tol *= 2.5;
    st.line_tol *= 1.8;
    st.circle_tol *= 1.8;
    st.sigma = 0.9;
    st
}

#[test]
fn a_thin_closed_outline_and_its_hole_keep_opposite_winding() {
    let points = thin_outline();
    let forward = fit_closed(&points, &hard_style()).unwrap();
    let area = subpath(&forward, true).unwrap().area();
    assert!(area > 1.0);
    for shift in 0..points.len() {
        let mut backward = points.clone();
        backward.rotate_left(shift);
        backward.reverse();
        let reverse = fit_closed(&backward, &hard_style()).unwrap();
        assert_eq!(reverse, reversed(forward.clone()), "shared closed boundary must be the same curve in reverse");
        assert!((subpath(&reverse, true).unwrap().area() + area).abs() < 1e-10);
    }
}

#[test]
fn a_thin_white_hole_remains_open_with_snap_enabled() {
    let image = Raster::from_fn(32, 32, |x, y| {
        if (4..28).contains(&x) && (4..28).contains(&y) && ![(14, 14), (15, 14), (15, 15), (16, 15)].contains(&(x, y)) {
            [0, 0, 0, 255]
        } else {
            [255; 4]
        }
    });
    let mut p = preset("Black and White Logo").unwrap();
    p.paths = 50.0;
    p.corners = 0.0;
    p.noise = 1;
    p.ignore_white = true;
    let out = trace(&image, &p);
    assert_eq!(out.paths.len(), 1);
    let subs = &out.paths[0].path.subpaths;
    assert_eq!(subs.len(), 2);
    assert!(subs[0].area() * subs[1].area() < -1.0, "hole must retain a nonzero opposite interior");
    p.noise = 5;
    assert_eq!(trace(&image, &p).paths[0].path.subpaths.len(), 1, "Noise must still fill the four-pixel hole");
}

#[test]
fn a_narrow_rectangle_remains_a_filled_region() {
    let points = vec![Point::new(0.0, 0.0), Point::new(20.0, 0.0), Point::new(20.0, 1.0), Point::new(0.0, 1.0)];
    let curves = fit_closed(&points, &hard_style()).unwrap();
    let area = subpath(&curves, true).unwrap().area().abs();
    assert!((15.0..25.0).contains(&area), "a narrow filled region must remain visible: {area}");
}
