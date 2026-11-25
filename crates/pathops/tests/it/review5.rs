//! Claims made in the fifth review of the pull request, and an input found when the fourth round was
//! checked again, each as the smallest test that fails when the claim is true.

use craft_pathops::*;
use kurbo::{BezPath, Point, Shape as _};

use crate::common::{poly, rel_close as close};
use crate::review4::deviation;

/// The two-way deviation between `path` and its simplification at `tol` (default options otherwise).
fn simplified_within(path: &str, tol: f64) -> (f64, BezPath, BezPath) {
    let path = BezPath::from_svg(path).expect("an SVG path");
    let simpler = simplify_with(&path, &SimplifyOptions::default().tolerance(tol));
    (deviation(&path, &simpler).max(deviation(&simpler, &path)), path, simpler)
}

// ---- simplify

/// A run that a nearly straight cubic fits within the tolerance is that cubic's chord only where the
/// chord is within the tolerance of the run too. A bump half a unit high on a run 75 long, at a
/// tolerance of 0.5, refits to a cubic that bows a little towards it, and its chord, the one line it
/// came out as, was 1.0026 of the tolerance from the top of the bump.
#[test]
fn a_nearly_straight_refit_is_its_chord_only_where_the_chord_is_within_the_tolerance() {
    let bump = "M0,0 C12.599983333333332,0.0036077075223443973 25.19996666666666,0.4779334247178249 37.799949999999995,0.500584843939615 \
                C50.39993333333333,0.523236263161405 62.99991666666666,0.0047330603239298785 75.59989999999999,0";
    let tol = 0.5;
    let (dev, path, simpler) = simplified_within(bump, tol);
    assert!(dev <= tol + 1e-9, "{dev} from the run at a tolerance of {tol}: {simpler:?}");
    assert!(simpler.elements().len() < path.elements().len(), "it is still simplified: {simpler:?}");
}

/// The refit is held to the tolerance where the run is off the polyline of its samples between two of
/// them: four cubics at a tolerance of 0.25, which came out 1.0093 of the tolerance from their refit
/// with that distance left out of the bound, and are 0.468 of it.
#[test]
fn a_refit_is_within_the_tolerance_where_the_run_leaves_the_polyline_of_its_samples() {
    let run = "M0,0 C0.31517966136753,0 0.7396758478251511,0.19739978067465472 1.143700191002585,0.3052231156732847 \
               C1.7108136146526371,0.4565705819658921 1.8233562823945917,0.1120302355026037 2.211629052951122,0.0016633370289438343 \
               C3.121766080218643,-0.25704396490617415 4.129739087417817,-0.5475839350620679 5.6863341660939355,-0.9933120660610077 \
               C6.086929838199549,-1.1080218982196457 6.097550227321668,-0.6206005725509969 6.230265784870834,-0.5003119497805706";
    let tol = 0.25;
    let (dev, path, simpler) = simplified_within(run, tol);
    assert!(dev <= tol + 1e-9, "{dev} from the run at a tolerance of {tol}: {simpler:?}");
    // And refitted, not left as it is (which is within any tolerance).
    assert!(simpler.elements().len() < path.elements().len(), "the run is left as it is: {simpler:?}");
}

// ---- interior points

/// The shallow bulge of the fourth review's test, moved a quadrillion out in y: its vertex is a tenth
/// off the line through its neighbours across that line (in x), where rounding is a millionth of a
/// millionth, though a unit in the last place of its y is an eighth.
#[test]
fn a_shallow_bend_far_out_along_its_line_is_a_height_of_the_search() {
    let y = 1e15;
    let bulge = poly(&[(0.0, y), (0.0, y + 4e11), (0.1, y + 4.005e11), (0.0, y + 4.01e11), (0.0, y + 1e12)], true);
    assert!(close(bulge.area().abs(), 5e7, 1e-9), "{}", bulge.area());
    let p = interior_point(&bulge).expect("a point in the bulge");
    assert!(bulge.winding(p) != 0, "{p:?}");
    assert!(crate::review4::strictly_in_the_bulge(p, y), "{p:?} is on the edge of the bulge, or outside it");
}

// ---- offset

/// The four cubics of a circle of radius `r` about `(r, r)`, from its bottom point round, after the
/// path `start` (which starts at the bottom point), closed.
fn circle_after(r: f64, start: &mut BezPath) {
    let k = 0.552_284_749_830_793_4 * r;
    let b = Point::new(r, 0.0);
    let pts = [b, Point::new(2.0 * r, r), Point::new(r, 2.0 * r), Point::new(0.0, r), b];
    let tangents = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0), (1.0, 0.0)];
    for i in 0..4 {
        let (p, q) = (pts[i], pts[i + 1]);
        let (tp, tq) = (tangents[i], tangents[i + 1]);
        start.curve_to(Point::new(p.x + k * tp.0, p.y + k * tp.1), Point::new(q.x - k * tq.0, q.y - k * tq.1), q);
    }
    start.close_path();
}

/// An inset does not stroke a cubic that its subpath goes out and back along, whose middle lies on the
/// outline: from `(5e7, 0)` back to it with handles a unit above and below, it climbs 0.29 into the fill
/// and drops 0.29 out of it along one vertical line, and the sweep cancels it whole. On a square 1e8
/// across (lines), and on a circle of that size (cubics), whose lengths are measured to less.
#[test]
fn an_inset_does_not_stroke_a_cubic_its_subpath_retraces() {
    let spur = |p: &mut BezPath| {
        p.move_to((5e7, 0.0));
        p.curve_to((5e7, 1.0), (5e7, -1.0), (5e7, 0.0));
    };
    let mut square = BezPath::new();
    spur(&mut square);
    for q in [(1e8, 0.0), (1e8, 1e8), (0.0, 1e8), (0.0, 0.0)] {
        square.line_to(q);
    }
    square.close_path();
    let mut circle = BezPath::new();
    spur(&mut circle);
    circle_after(5e7, &mut circle);
    // 2.2 above the bottom, over the spur: in the inset by 2, and within 2 of the spur's top (0.29).
    // A hundred along, the inset is where it is without the spur, a little over 2 above the bottom.
    let over = Point::new(5e7, 2.2);
    let (beside_in, beside_out) = (Point::new(5e7 + 100.0, 2.2), Point::new(5e7 + 100.0, 1.9));
    for (name, path) in [("square", square), ("circle", circle)] {
        let inset = try_offset_path(&path, -2.0, Join::Round, 4.0).expect("an inset");
        assert!(inset.winding(over) != 0, "{name}: a notch is cut round the spur");
        assert!(inset.winding(beside_in) != 0 && inset.winding(beside_out) == 0, "{name}");
    }
}
