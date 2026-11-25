//! Claims made in the sixth review of the pull request, each as the smallest test that fails when the
//! claim is true.

use craft_pathops::*;
use kurbo::{BezPath, Point, Shape as _};

use crate::common::rel_close as close;

const NZ: FillRule = FillRule::NonZero;

/// A 100 x 100 square with a slit `w` wide from x = 50, y from 20 to 80; and, with `far`, a unit square
/// at 1e11, which widens the sweep's tolerance to 1.4e-3.
fn slit(w: f64, far: bool) -> BezPath {
    let mut p = BezPath::new();
    for pts in [[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], [(50.0, 20.0), (50.0, 80.0), (50.0 + w, 80.0), (50.0 + w, 20.0)]] {
        p.move_to(pts[0]);
        for q in &pts[1..] {
            p.line_to(*q);
        }
        p.close_path();
    }
    if far {
        p.move_to((1e11, 1e11));
        p.line_to((1e11 + 1.0, 1e11));
        p.line_to((1e11 + 1.0, 1e11 + 1.0));
        p.line_to((1e11, 1e11 + 1.0));
        p.close_path();
    }
    p
}

// ---- the inset's boundary

/// A slit narrower than twice the sweep's tolerance is a hole, not two passes of one curve: its sides
/// are its width apart, where the passes of a retraced curve are under a unit in the last place of
/// their coordinates apart. One 1.5e-6 wide (the tolerance is 1e-6), and ones 2e-3 and 1.5e-3 wide beside a
/// square at 1e11 (1.4e-3, which is what 64 units in the last place of the slit's own coordinates is not: that
/// is 1.4e-12, and nothing under 1.4e-3 is kept by the sweep itself, below), grow by the inset all round, round joins rounding their ends; they were left out,
/// and the inset was the square's. (The far square's part of the inset is taken away as made alone.)
#[test]
fn an_inset_widens_a_slit_narrower_than_twice_the_sweeps_tolerance() {
    let d = 0.25;
    let alone = |path: &BezPath| area(&try_offset_path(path, -d, Join::Round, 4.0).expect("an inset"), NZ);
    let far_part = alone(&slit(0.0, true)) - alone(&slit(0.0, false));
    for (w, far) in [(1.5e-6, false), (2e-3, true), (1.5e-3, true)] {
        let inset = try_offset_path(&slit(w, far), -d, Join::Round, 4.0).expect("an inset");
        // The slit grown by `d` all round.
        let hole = w * 60.0 + 2.0 * d * (w + 60.0) + std::f64::consts::PI * d * d;
        let want = (100.0 - 2.0 * d) * (100.0 - 2.0 * d) - hole;
        let got = area(&inset, NZ) - if far { far_part } else { 0.0 };
        assert!(close(got, want, 1e-6), "slit {w} wide (far square {far}): {got}, not {want}");
        // Its middle, and just past its sides, are out of the inset; further out, and past its ends, in.
        for (p, inside) in
            [((50.0 + w / 2.0, 50.0), false), ((49.76, 50.0), false), ((49.74, 50.0), true), ((50.0 + w + 0.26, 50.0), true), ((50.0, 19.7), true)]
        {
            assert_eq!(inset.winding(Point::new(p.0, p.1)) != 0, inside, "slit {w} wide, at {p:?}");
        }
    }
}

/// A spur out and back along a slanted line on the edge of a square 1e8 across is still two passes of
/// one curve to the inset, and strokes no notch: one cubic that folds back on itself, or two straight
/// cubics, out and back, drawn at different paces. The passes are a few billionths apart, under a unit
/// in the last place of their coordinates (7.5e-9).
#[test]
fn an_inset_does_not_stroke_a_cubic_retraced_along_a_slanted_line() {
    for deg in [60.0f64, 45.0, 30.0] {
        let (dx, dy) = (deg.to_radians().cos(), deg.to_radians().sin());
        let o = (5e7, 0.0);
        let at = |s: f64| (o.0 + s * dx, o.1 + s * dy);
        // One cubic reaching 0.289 out along the line, or two reaching 0.3 (out, then back).
        let folded = |p: &mut BezPath| p.curve_to(at(1.0), at(-1.0), o);
        let pair = |p: &mut BezPath| {
            p.curve_to(at(0.1), at(0.2), at(0.3));
            p.curve_to(at(0.27), at(0.05), o);
        };
        for (shape, reach) in [(&folded as &dyn Fn(&mut BezPath), 0.289), (&pair, 0.3)] {
            let mut square = BezPath::new();
            square.move_to(o);
            shape(&mut square);
            for q in [(1e8, 0.0), (1e8, 1e8), (0.0, 1e8), (0.0, 0.0)] {
                square.line_to(q);
            }
            square.close_path();
            let inset = try_offset_path(&square, -2.0, Join::Round, 4.0).expect("an inset");
            // 2.1 above the spur's tip is in the inset, and under 2 from the tip.
            let tip = Point::new(at(reach).0, at(reach).1);
            let over = Point::new(tip.x, 2.1);
            assert!(over.distance(tip) < 2.0);
            assert!(inset.winding(over) != 0, "{deg} degrees, reaching {reach}: a notch is cut round the spur");
        }
    }
}

/// A spur cut short by the fill of another subpath: the sweep keeps the part of it below the other
/// subpath as two pieces, out to the cut and back from it, which come back over each other. A cubic
/// out and back at 45 degrees from the bottom edge of a square 1e8 across, under a rectangle whose
/// bottom edge is a tenth above that edge, inset by 2.
#[test]
fn an_inset_does_not_stroke_the_two_pieces_of_a_spur_cut_short() {
    let (dx, dy) = (std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2);
    let o = (5e7, 0.0);
    let mut path = BezPath::new();
    path.move_to(o);
    path.curve_to((o.0 + dx, o.1 + dy), (o.0 - dx, o.1 - dy), o);
    for q in [(1e8, 0.0), (1e8, 1e8), (0.0, 1e8), (0.0, 0.0)] {
        path.line_to(q);
    }
    path.close_path();
    path.move_to((5e7 - 1.0, 0.1));
    for q in [(5e7 + 1.0, 0.1), (5e7 + 1.0, 3.0), (5e7 - 1.0, 3.0)] {
        path.line_to(q);
    }
    path.close_path();
    let inset = try_offset_path(&path, -2.0, Join::Round, 4.0).expect("an inset");
    // The cut is at (5e7 + 0.1, 0.1): 1.95 above it is in the inset of the square.
    assert!(inset.winding(Point::new(5e7 + 0.1, 2.05)) != 0, "a notch is cut round the spur's pieces");
    assert!(inset.winding(Point::new(5e7 + 50.0, 1.95)) == 0 && inset.winding(Point::new(5e7 + 50.0, 2.05)) != 0);
}

// ---- the faces of an offset

/// A ring between two circles 0.6 apart, inset by 0.25, is a face 0.1 wide: narrower than a tenth of
/// the flattening the deep point of a face was looked for at (a hundredth of its size, 1), where a
/// point found between the flattened circles can be in the hole, by the path, and the face was dropped.
#[test]
fn an_inset_keeps_a_curved_face_narrower_than_the_flattening_of_its_deep_point() {
    let (c, outer, inner, d) = (Point::new(50.0, 50.0), 50.0, 49.4, 0.25);
    let mut ring = kurbo::Circle::new(c, outer).to_path(1e-9);
    ring.extend(kurbo::Circle::new(c, inner).to_path(1e-9).reverse_subpaths().iter());
    let inset = try_offset_path(&ring, -d, Join::Round, 4.0).expect("an inset");
    let want = std::f64::consts::PI * ((outer - d) * (outer - d) - (inner + d) * (inner + d));
    assert!(close(area(&inset, NZ), want, 1e-4), "{} of the ring, not {want}", area(&inset, NZ));
    for (r, inside) in [(49.7, true), (49.6, false), (49.8, false), (49.0, false)] {
        for a in [0.0f64, 1.0, 2.5, 4.0] {
            let p = Point::new(c.x + r * a.cos(), c.y + r * a.sin());
            assert_eq!(inset.winding(p) != 0, inside, "at radius {r}, angle {a}");
        }
    }
}
