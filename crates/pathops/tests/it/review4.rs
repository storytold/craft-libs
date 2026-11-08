//! Claims made in the fourth review of the pull request, each as the smallest test that fails when
//! the claim is true.

use craft_pathops::*;
use kurbo::{BezPath, ParamCurveArclen, Point, Rect, Shape as _};

use crate::common::{poly, rectangle, rel_close as close};

const NZ: FillRule = FillRule::NonZero;

fn open(pts: &[(f64, f64)]) -> BezPath {
    poly(pts, false)
}

/// The default limits with `set` applied.
fn limits(set: impl FnOnce(&mut Limits)) -> Limits {
    let mut l = Limits::default();
    set(&mut l);
    l
}

fn length(path: &BezPath) -> f64 {
    path.segments().map(|s| s.arclen(1e-9)).sum()
}

/// One cubic from `p[0]`, closed or not.
fn cubic(p: [(f64, f64); 4], shut: bool) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(p[0]);
    path.curve_to(p[1], p[2], p[3]);
    if shut {
        path.close_path();
    }
    path
}

/// `m` thin strokes slanting one way (`down` false) or the other, as one path: the two halves of the
/// crate's test hatch, which do not cross themselves and cross each other `m²` times.
fn strokes(m: usize, down: bool) -> BezPath {
    let mut b = BezPath::new();
    let span = m as f64 * 4.0;
    for i in 0..m {
        if down {
            let x = i as f64 * 4.0 + span / 2.0 + 0.37;
            b.move_to((x, 0.0));
            b.line_to((x + 1.0, 0.0));
            b.line_to((x + 1.0 - span, span));
            b.line_to((x - span, span));
        } else {
            let x = i as f64 * 4.0 - span / 2.0;
            b.move_to((x, 0.0));
            b.line_to((x + 1.0, 0.0));
            b.line_to((x + span + 1.0, span));
            b.line_to((x + span, span));
        }
        b.close_path();
    }
    b
}

// ---- stroke regions and unions

/// Outlines within the element limit each, whose union is far over it, are refused by `try_stroke_region`
/// and `try_unite_all` (and are no region for the forms without a `Result`), not made into contours.
#[test]
fn the_union_of_several_outlines_is_held_to_the_element_limit() {
    // Each half alone is 100 elements and crosses nothing; together they cross 400 times.
    let (a, b) = (strokes(20, false), strokes(20, true));
    let few = limits(|l| l.elements = 1_000);
    for one in [&a, &b] {
        assert!(with_limits(few, || try_stroke_region(std::slice::from_ref(one))).is_ok(), "each outline alone is within the limit");
    }
    let both = [a.clone(), b.clone()];
    assert_eq!(with_limits(few, || try_stroke_region(&both)), Err(PathOpsError::TooComplex));
    assert!(with_limits(few, || stroke_region(&both)).elements().is_empty());
    // The same for a union of the two as filled paths.
    let filled = [(&a, NZ), (&b, NZ)];
    assert_eq!(with_limits(few, || try_unite_all(&filled)), Err(PathOpsError::TooComplex));
    assert!(with_limits(few, || unite_all(&filled)).elements().is_empty());
    // With room, all are made, and within the limit.
    let roomy = limits(|l| l.elements = 10_000);
    let region = with_limits(roomy, || try_stroke_region(&both)).expect("the region");
    assert!(!region.elements().is_empty() && region.elements().len() <= 10_000);
    assert!(with_limits(roomy, || try_unite_all(&filled)).is_ok());
}

/// A region is never more elements than the limit, however few the sides of its boundary are: the
/// union of the two halves of a hatch is hundreds of contours, each written with a move and a close
/// beyond its sides, and a limit of one under what it is written in is refused, not passed with a
/// region over it. A region that is made instead is the whole one in fewer elements: its area, and no
/// further from the full region than the refit allows.
#[test]
fn a_region_is_never_more_elements_than_the_limit() {
    let both = [strokes(20, false), strokes(20, true)];
    let filled = [(&both[0], NZ), (&both[1], NZ)];
    let region = try_stroke_region(&both).expect("the region");
    let union = try_unite_all(&filled).expect("the union");
    let held = |full: &BezPath, got: Result<BezPath, PathOpsError>| {
        let limit = full.elements().len() - 1;
        match got {
            Err(e) => assert_eq!(e, PathOpsError::TooComplex),
            Ok(p) => {
                assert!(!p.elements().is_empty() && p.elements().len() <= limit, "{} elements under {limit}", p.elements().len());
                assert!(close(area(&p, NZ), area(full, NZ), 1e-9), "{} against {}", area(&p, NZ), area(full, NZ));
                assert!(deviation(&p, full).max(deviation(full, &p)) <= DEFAULT_PRECISION, "not the full region");
            }
        }
    };
    let elements = |p: &BezPath| p.elements().len() - 1;
    held(&region, with_limits(limits(|l| l.elements = elements(&region)), || try_stroke_region(&both)));
    held(&union, with_limits(limits(|l| l.elements = elements(&union)), || try_unite_all(&filled)));
}

// ---- simplify

/// The largest distance from a point of `from` to `to`, at 2,001 points of each segment of `from`.
pub(crate) fn deviation(from: &BezPath, to: &BezPath) -> f64 {
    use kurbo::{ParamCurve, ParamCurveNearest};
    let mut dev: f64 = 0.0;
    for seg in from.segments() {
        for i in 0..=2_000 {
            let q = seg.eval(f64::from(i) / 2_000.0);
            dev = dev.max(to.segments().map(|s| s.nearest(q, 1e-12).distance_sq).fold(f64::INFINITY, f64::min).sqrt());
        }
    }
    dev
}

/// A chain of cubics from `(0, 0)`, each given by its two handles and its end.
fn chain_of(cubics: &[[(f64, f64); 3]]) -> BezPath {
    let mut chain = BezPath::new();
    chain.move_to((0.0, 0.0));
    for &[a, b, c] in cubics {
        chain.curve_to(a, b, c);
    }
    chain
}

/// A refit cubic is held to the tolerance between the samples, not only at them. Two chains of cubics
/// each up to about four tolerances long, turning up to 1.5 rad, came out 1.0116 and 1.0033 of the
/// tolerance from their refits (between two samples, where the cubic and the chain bend apart).
#[test]
fn a_refit_curve_is_within_the_tolerance_between_its_samples() {
    let tight = chain_of(&[
        [(0.7762602009288002, 0.0), (0.9519770551661517, -0.5260968359785947), (1.0041236967149634, -0.8591331359083565)],
        [(1.1343355090238778, -1.690735304149959), (1.0994143013532665, -1.598968692843822), (1.4011730541850653, -2.586795457582525)],
        [(1.497606753632297, -2.902477400335786), (1.5717715899397366, -3.271178343888079), (1.1513930756924282, -3.7514329076529185)],
        [(0.8849927882769535, -4.055777562490058), (0.8615603854718964, -4.053621153952524), (0.6160215073077214, -4.206915807713447)],
        [(0.0201850725556737, -4.57890797712617), (-0.5241039699865782, -4.731449131278201), (-0.8422763419344562, -4.8012130379952636)],
        [(-1.3979140255408806, -4.923044658039109), (-1.5457918876199566, -5.143047408937598), (-1.675003655006829, -5.287631259183077)],
        [(-2.2676741997596235, -5.950810712729895), (-2.8878888847316375, -5.815368006257858), (-3.213433818842733, -5.714689288518768)],
    ]);
    let looser = chain_of(&[
        [(0.8251886787017998, 0.0), (0.9658240471376174, 0.1714358508010806), (1.3519899214194389, 0.6206164529324928)],
        [(1.7764207378399268, 1.114306088344837), (1.7314215336802594, 1.0749405133849923), (1.9522835752387258, 1.5592870765381803)],
        [(2.156632511538936, 2.007420691000021), (2.284408425691744, 2.7936772545562225), (2.24277115706313, 3.2588640587707625)],
        [(2.220560250195486, 3.507012439410832), (2.220231083312957, 3.6835681758276966), (1.9293822740608488, 4.063097334945395)],
        [(1.5300730901441404, 4.584156641289398), (1.2007746473074423, 4.850031255011433), (0.8309568500676727, 5.1021205782221815)],
        [(0.3891959519520386, 5.40325050398057), (0.25029848469413896, 5.376083121921157), (-0.14900210331249453, 5.271932269984106)],
        [(-1.1834332661844282, 5.002118274915416), (-1.3437013649251441, 5.144331482958443), (-1.9979272244426083, 5.64309637896825)],
        [(-2.1303489096456194, 5.744051251186901), (-2.261830329743545, 5.929053860985305), (-2.313854333603694, 6.084660408048256)],
        [(-2.5847511455327035, 6.894927100092016), (-2.4942703214749335, 7.506459530156277), (-2.3708539657259267, 7.966523876990724)],
    ]);
    for (chain, tol) in [(tight, 0.5), (looser, 1.0)] {
        let simpler = simplify_with(&chain, &SimplifyOptions::default().tolerance(tol).corner_angle_deg(179.0));
        let dev = deviation(&chain, &simpler).max(deviation(&simpler, &chain));
        assert!(dev <= tol, "{dev} from the chain at a tolerance of {tol}");
        assert!(simpler.elements().len() < chain.elements().len(), "it is still simplified: {simpler:?}");
    }
}

// ---- offset

#[test]
fn an_inset_does_not_stroke_a_spur_its_subpath_retraces() {
    // A square 1e8 across whose subpath goes up 0.5 from its bottom edge and comes straight back.
    let pts = [(0.0, 0.0), (5e7, 0.0), (5e7, 0.5), (5e7, 0.0), (1e8, 0.0), (1e8, 1e8), (0.0, 1e8)];
    let spur = poly(&pts, true);
    // Round joins: the stroke of the spur reaches 2 past its tip (a mitre at a turn of 180 degrees is bevelled flat).
    let inset = try_offset_path(&spur, -2.0, Join::Round, 4.0).expect("an inset");
    // 2.3 above the bottom edge, over the spur: in the inset of the square, 1.8 from the spur's tip.
    let over = Point::new(5e7, 2.3);
    let plain = try_offset_path(&rectangle(Rect::new(0.0, 0.0, 1e8, 1e8)), -2.0, Join::Round, 4.0).expect("an inset");
    assert!(plain.winding(over) != 0);
    assert!(inset.winding(over) != 0, "no notch is cut round the spur");
}

// ---- coordinates that are not numbers, in a contour the fill leaves out

#[test]
fn a_loop_of_one_cubic_whose_handles_are_not_numbers_is_an_error_not_nothing() {
    let nan = cubic([(0.0, 0.0), (f64::NAN, 0.0), (0.0, 10.0), (0.0, 0.0)], true);
    assert_eq!(try_area(&nan, NZ), Err(PathOpsError::NonFinite));
    assert_eq!(try_normalize(&nan, NZ), Err(PathOpsError::NonFinite));
    let square = rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
    assert_eq!(try_boolean(&square, NZ, &nan, NZ, BoolOp::Union, DEFAULT_PRECISION), Err(PathOpsError::NonFinite));
    assert_eq!(try_boolean_n(&[(&square, NZ), (&nan, NZ)], |m| m[0] || m[1]), Err(PathOpsError::NonFinite));
    assert_eq!(try_unite_all(&[(&square, NZ), (&nan, NZ)]), Err(PathOpsError::NonFinite));
    // Handles so large that their products overflow: past the coordinates the sweep takes.
    let huge = cubic([(0.0, 0.0), (1e200, 0.0), (0.0, 1e200), (0.0, 0.0)], true);
    assert_eq!(try_area(&huge, NZ), Err(PathOpsError::Degenerate));
    assert_eq!(try_normalize(&huge, NZ), Err(PathOpsError::Degenerate));
    // A loop of ordinary numbers whose handles lie along one line is still nothing.
    let flat = cubic([(0.0, 0.0), (10.0, 0.0), (5.0, 0.0), (0.0, 0.0)], true);
    assert_eq!(try_area(&flat, NZ), Ok(0.0));
}

// ---- shape_builder

/// `shape_builder` gives the pieces of a loop of one cubic, open or closed, as edges of the outline of its
/// fill (with `edges`) and never as lines, as it does a closed subpath.
#[test]
fn the_pieces_of_an_open_loop_of_one_cubic_are_outline_edges_not_lines() {
    let ring = cubic([(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (0.0, 0.0)], false);
    let shut = cubic([(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (0.0, 0.0)], true);
    let cutter = open(&[(-10.0, 10.0), (150.0, 10.0)]);
    for loop_path in [ring, shut] {
        let shapes = [Shape::new(loop_path.clone(), NZ, 0), Shape::new(cutter.clone(), NZ, 1)];
        let without = shape_builder(&shapes, false);
        assert!(
            without.lines.iter().all(|l| l.key == 1),
            "only the cutter's pieces are lines: {:?}",
            without.lines.iter().map(|l| l.key).collect::<Vec<_>>()
        );
        assert!(without.edges.is_empty());
        let with = shape_builder(&shapes, true);
        assert!(with.lines.iter().all(|l| l.key == 1));
        let edged: f64 = with.edges.iter().filter(|e| e.key == 0).map(|e| length(&e.path)).sum();
        assert!(close(edged, length(&loop_path), 1e-6), "the loop's outline is its edges: {edged} of {}", length(&loop_path));
        assert_eq!(without.regions.len(), 2);
    }
    // Behind a closed copy of itself, the open loop's outline lies under the copy's: that of a fill, not
    // a line's, so none of its pieces is a line under another path.
    let (ring, copy) =
        (cubic([(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (0.0, 0.0)], false), cubic([(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (0.0, 0.0)], true));
    let behind = shape_builder(&[Shape::new(ring, NZ, 0), Shape::new(copy, NZ, 1), Shape::new(cutter, NZ, 2)], false);
    assert!(behind.lines_under.iter().all(|l| l.key != 0), "{:?}", behind.lines_under.iter().map(|l| l.key).collect::<Vec<_>>());
}

// ---- interior points

/// A unit square narrower than the allowance for two rounded crossings at its x (0.9 each at 1e15) has a
/// point found in it: the crossings of its vertical sides are exact, with no rounding to allow for.
#[test]
fn an_interior_point_is_found_in_a_unit_square_at_a_quadrillion() {
    for x in [1e15, 4e15] {
        let square = rectangle(Rect::new(x, 0.0, x + 1.0, 1.0));
        let p = interior_point(&square).unwrap_or_else(|| panic!("no point in a unit square at x = {x}"));
        assert!(square.winding(p) != 0, "{p:?}");
    }
}

#[test]
fn a_shallow_bend_that_encloses_area_is_a_height_of_the_search() {
    // A bulge 0.1 wide and a billion high on a line a trillion long: its edges turn by under 1e-9 of a radian.
    let bulge = poly(&[(0.0, 0.0), (0.0, 4e11), (0.1, 4.005e11), (0.0, 4.01e11), (0.0, 1e12)], true);
    assert!(close(bulge.area().abs(), 5e7, 1e-9), "{}", bulge.area());
    let p = interior_point(&bulge).expect("a point in the bulge");
    assert!(bulge.winding(p) != 0, "{p:?}");
}

// ---- the frame of a planar map

#[test]
fn short_lines_level_in_each_of_the_ten_shears_are_mapped_in_one_between() {
    // Ten lines 9e-6 long (nine tolerances), one level in each of the ten shears, beside a rectangle: a
    // shear of 3/8 levels none of them.
    let ks = [32.0, 64.0, 16.0, 128.0, 8.0, 256.0, 512.0, 4.0, 1024.0, 2.0];
    let mut shapes: Vec<Shape> =
        ks.iter().enumerate().map(|(i, k)| Shape::new(open(&[(0.0, 100.0 * i as f64), (9e-6, 100.0 * i as f64 - 9e-6 / k)]), NZ, i as u64)).collect();
    shapes.push(Shape::new(rectangle(Rect::new(-500.0, -500.0, -400.0, -400.0)), NZ, 99));
    let (faces, edges) = live_paint(&shapes);
    assert_eq!(faces.len(), 1, "the rectangle's face");
    assert_eq!(faces[0].sources, [10]);
    assert!(edges.iter().any(|e| e.key == 99));
}
