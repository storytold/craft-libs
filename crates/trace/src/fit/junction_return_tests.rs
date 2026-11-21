//! Original geometry regressions for a shared piece returning to one junction.
use super::*;

fn hard_style() -> Style {
    let mut s = crate::TraceParams { corners: 0., paths: 0., ..crate::TraceParams::default() }.outline_style();
    s.tol *= 2.5;
    s.line_tol *= 1.8;
    s.circle_tol *= 1.8;
    s.sigma = 0.9;
    s
}

fn returning_rect(width: f64) -> Vec<Point> {
    let d = 1.0 / 512.0;
    vec![Point::new(width, 1.), Point::new(-d, 1. + d), Point::new(-d, -d), Point::new(width + d, -d), Point::new(width, 1.)]
}

#[test]
fn returning_piece_keeps_its_junction_and_interior() {
    let style = crate::TraceParams { corners: 0., paths: 0., ..crate::TraceParams::default() }.outline_style();
    for width in [1, 2] {
        for turns in 0..4 {
            for offset in [(10, 10), (1_000_000_000, 1_000_000_000)] {
                let pts: Vec<_> = [(width, 1), (0, 1), (0, 0), (width, 0)]
                    .into_iter()
                    .map(|(mut x, mut y)| {
                        for _ in 0..turns {
                            (x, y) = (-y, x);
                        }
                        (x + offset.0, y + offset.1)
                    })
                    .collect();
                let junction = pts[0];
                let at = |x, y| ((x, y) == junction).then(|| Point::new(x as f64, y as f64));
                let raw = Loop { pts: pts.clone(), area2: 2 * i64::from(width) };
                let w = offset.0 as usize + 20;
                let h = offset.1 as usize + 20;
                let (path, _) =
                    fit_loop(&raw, w, h, &|_| (0., 0, subpixel::EDGE_STEP), &at, &style).expect("a positive-area returning loop must be editable");
                assert_eq!(path.anchors[0].p, Point::new(junction.0 as f64, junction.1 as f64));
                let mut local = path.clone();
                let shift = Point::new(offset.0 as f64, offset.1 as f64).to_vec2();
                for a in &mut local.anchors {
                    a.p -= shift;
                    a.h_in -= shift;
                    a.h_out -= shift;
                }
                assert!(local.area().abs() > 0.9 * width as f64, "{width}, {turns}, {offset:?}");
                let reverse = Loop { pts: pts.into_iter().rev().collect(), area2: -raw.area2 };
                let (back, _) = fit_loop(&reverse, w, h, &|_| (0., 0, subpixel::EDGE_STEP), &at, &style).unwrap();
                let mut expected = path;
                expected.reverse();
                let start = expected.anchors.iter().position(|a| a.p == back.anchors[0].p).unwrap();
                expected.anchors.rotate_left(start);
                assert_eq!(back, expected);
            }
        }
    }
}

#[test]
fn returning_piece_does_not_promote_a_zero_area_chain() {
    let raw = Loop { pts: vec![(2, 2), (1, 2), (1, 1), (2, 1)], area2: 2 };
    let junction = |x, y| ((x, y) == (2, 2)).then_some(Point::new(1.5, 1.5));
    let style = crate::TraceParams { corners: 0., paths: 0., ..crate::TraceParams::default() }.outline_style();
    // All four subpixel corners coincide; the raw pixel has area but its
    // positioned boundary does not. The fallback must not invent ink.
    assert!(fit_loop(&raw, 4, 4, &|_| (-0.5, 0, subpixel::EDGE_STEP), &junction, &style).is_none());
}

#[test]
fn returning_piece_leaves_free_end_fitting_unchanged() {
    assert_eq!(fit_piece(&returning_rect(1.), &hard_style(), false).len(), 1);
    let pts = [Point::new(0., 0.), Point::new(2., 0.), Point::new(4., 0.)];
    assert_eq!(fit_piece(&pts, &hard_style(), true), fit_piece(&pts, &hard_style(), false));
}

fn trace_art(hole: bool, noise: u32) -> crate::TraceResult {
    let image = crate::Raster::from_fn(12, 12, |x, y| {
        let ink = if hole {
            (3..9).contains(&x) && (3..9).contains(&y) && !(x == 5 && y == 5)
        } else {
            ((4..9).contains(&x) && (4..9).contains(&y)) || (x == 3 && y == 3)
        };
        if ink { [0, 0, 0, 255] } else { [255, 255, 255, 255] }
    });
    crate::trace(
        &image,
        &crate::TraceParams {
            corners: if hole { 100. } else { 0. },
            paths: if hole { 100. } else { 0. },
            noise,
            ignore_white: true,
            ..crate::TraceParams::default()
        },
    )
}

#[test]
fn returning_piece_keeps_a_corner_touching_mark_until_noise_removes_it() {
    assert_eq!(trace_art(false, 0).paths.len(), 2);
    assert_eq!(trace_art(false, 2).paths.len(), 1);
}

#[test]
fn returning_piece_preserves_the_existing_hole_noise_control() {
    let holes = |noise| trace_art(true, noise).paths.iter().map(|p| p.path.subpaths.len().saturating_sub(1)).sum::<usize>();
    assert_eq!(holes(0), 1);
    assert_eq!(holes(2), 0);
}

#[test]
fn returning_piece_has_the_same_boundary_in_its_abutting_neighbour() {
    for inverse in [false, true] {
        for turns in 0..4 {
            let image = crate::Raster::from_fn(12, 12, |mut x, mut y| {
                for _ in 0..turns {
                    (x, y) = (y, 11 - x);
                }
                let mark = ((4..9).contains(&x) && (4..9).contains(&y)) || (x == 3 && y == 3);
                if mark != inverse { [0, 0, 0, 255] } else { [255, 255, 255, 255] }
            });
            let res =
                crate::trace(&image, &crate::TraceParams { corners: 0., paths: 0., noise: 0, ignore_white: false, ..crate::TraceParams::default() });
            let ink = if inverse { [255, 255, 255] } else { [0, 0, 0] };
            let dot = res.paths.iter().find(|p| p.color == ink && p.pixels == 1).expect("corner mark is retained");
            let cubics = |p: &craft_geom::PathData| {
                p.subpaths
                    .iter()
                    .flat_map(|s| {
                        (0..s.segment_count()).map(move |i| {
                            let c = s.segment(i);
                            [c.p0, c.p1, c.p2, c.p3]
                        })
                    })
                    .collect::<Vec<_>>()
            };
            let other: Vec<_> = res.paths.iter().filter(|p| p.color != ink).flat_map(|p| cubics(&p.path)).collect();
            for c in cubics(&dot.path) {
                assert!(other.contains(&[c[3], c[2], c[1], c[0]]), "inverse {inverse}, turns {turns}: unshared recovered edge: {c:?}");
            }
        }
    }
}
