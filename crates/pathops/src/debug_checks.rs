//! A panic inside `linesweeper` is caught at every place the crate sweeps, and is the refusal the crate gives for a sweep that fails
//! ([`PathOpsError::Degenerate`]): never a panic out of a public function, in any build and on any platform.
//!
//! What makes `linesweeper` panic on an input of ordinary size is its own check, made in a build with debug assertions only
//! (`intersect_cubics_rec` in `curve/mod.rs`): the order it gives two curves must be the order their positions have where it changes. Where
//! that check fails depends on the arithmetic of the platform that made the input (a circle or an arc is made with `sin`, `cos` and `tan`,
//! which the standard libraries of Linux, macOS and Windows do not round alike), so no test here takes it for granted that an input trips the
//! check: each finds out whether it does here ([`sweep_panics`]) and holds what the crate promises either way.

use std::panic::{AssertUnwindSafe, catch_unwind};

use kurbo::{Arc, BezPath, Point, SvgArc, Vec2};
use linesweeper::topology::Topology;

use crate::boolean::eps_for;
use crate::meter::{Allowance, GATE_PIECES, dry_sweep_over, segments_of, within, within_charging, within_for_a_map};
use crate::pathfinder::Shape;
use crate::{
    BoolOp, Cap, DEFAULT_PRECISION, FillRule, Join, PathOpsError, PathfinderOp, area, boolean, boolean_n, interior_point, live_paint, merge_regions,
    normalize, offset_path, outline_stroke, pathfinder, pathfinder_with, region_at, regions, regions_within, shape_builder, stroke_region, try_area,
    try_boolean, try_boolean_n, try_normalize, try_offset_path, try_outline_stroke, try_pathfinder, try_pathfinder_with, try_regions_within,
    try_shape_builder, try_stroke_region, try_unite_all, try_unite_all_with, unite_all, unite_all_with,
};

const NZ: FillRule = FillRule::NonZero;

/// The outline of a stroke 20 million across of a run of two arcs 600 across, at about 10 million from the origin, as it goes to the sweep
/// (72 cubics and the line that closes them): the sweep of it fails the check in a build with debug assertions on Linux, where the run
/// and its width were found (the run at (3,000, 4,000) is one of the 700 that a grid of them makes, of which one fails at 1e7).
const OUTLINE: &str = "\
        M-9227769.23076923,-3842153.846153846 C-9060184.781173492,-4244356.525183618 \
        -8866492.805520594,-4635176.452509806 -8647931.700436374,-5012114.873137948 \
        C-8429370.595352177,-5389053.2937660515 -8186407.644283968,-5751304.313690103 \
        -7920596.258931798,-6096551.833858102 C-7654784.873579367,-6441799.354026409 \
        -7366693.377015544,-6769305.211431853 -7058163.720634047,-7076975.458180397 \
        C-6749634.064252447,-7384645.704929044 -6421325.884182718,-7671822.542307893 \
        -6075338.2580769,-7936669.866905935 C-5729350.631971197,-8201517.191503891 \
        -5366423.30523336,-8443468.741477188 -4988876.700960461,-8660977.57165024 \
        C-4611330.096687545,-8878486.401823303 -4219971.409182021,-9071087.478511702 \
        -3817302.8380766246,-9237549.379885443 C-3414634.2669710643,-9404011.281259242 \
        -3001516.7454389255,-9543978.100293914 -2580591.593202775,-9656554.944343826 \
        C-2159666.440966649,-9769131.78839373 -1731833.5956257395,-9854077.968266904 \
        -1299828.4596519491,-9910850.366994347 C-867823.3236778362,-9967622.765721818 -432569.5538430774,-9996100 \
        3150.0000000000005,-9996100 C438869.5538430913,-9996100 874123.3236778637,-9967622.765721774 \
        1306128.4596516334,-9910850.366994347 C1738133.5956257395,-9854077.968266863 \
        2165966.440966649,-9769131.78839373 2586891.593202775,-9656554.944343826 \
        C3007816.7454389255,-9543978.100293914 3420934.2669710643,-9404011.281259177 \
        3823602.8380764644,-9237549.379885443 C4226271.409182008,-9071087.47851164 \
        4617630.096687522,-8878486.401823295 4995176.7009604275,-8660977.57165024 \
        C5372723.305233396,-8443468.74147715 5735650.631971295,-8201517.191503982 \
        6081638.258077101,-7936669.866905935 C6427625.884182718,-7671822.542308047 \
        6755934.064252447,-7384645.704929044 7064463.720634047,-7076975.458180397 \
        C7372993.377015544,-6769305.211431853 7661084.873579367,-6441799.354026053 \
        7926896.258931525,-6096551.833858102 C8192707.644283968,-5751304.313689747 \
        8435670.595352177,-5389053.2937660515 8654231.700436374,-5012114.873137948 \
        C8872792.805520594,-4635176.452509806 9066484.781173492,-4244356.525183618 \
        9234069.23076923,-3842153.846153846 C9066490.233401831,-4244343.439835602 \
        8872804.554374503,-4635150.666654132 8654250.54484989,-5012076.850022447 \
        C8435696.535325298,-5389003.033390723 8192741.47875073,-5751242.279302778 \
        7926938.741363044,-6096478.567114086 C7661136.003975097,-6441714.854925702 \
        7373053.871671174,-6769210.069914838 7064534.231667321,-7076870.328186368 \
        C6756014.591663366,-7384530.586457999 6427717.080089023,-7671698.089299126 \
        6081740.710418661,-7936536.797297642 C5735764.340748414,-8201375.505296072 \
        5372848.809997516,-8443319.193648638 4995314.462729052,-8660820.962424062 \
        C4617780.115460571,-8878322.731199495 4226434.14597698,-9070917.546713367 \
        3823778.6753770313,-9237374.032379992 C3421123.204776919,-9403830.518046675 \
        3008019.1099179615,-9543792.790122407 2587107.6229655123,-9656365.979378814 \
        C2166196.136013088,-9768939.168635216 1738377.1945663989,-9853882.585880281 \
        1306386.1135401346,-9910653.137562357 C874395.0325137598,-9967423.689244447 439155.4082610395,-9995900 \
        3450.0000000000005,-9995900 C-432255.4082610535,-9995900 -867495.0325137876,-9967423.689244444 \
        -1299486.1135401346,-9910653.137562357 C-1731477.1945663989,-9853882.585880281 \
        -2159296.136013088,-9768939.168635216 -2580207.6229655123,-9656365.979378814 \
        C-3001119.1099179615,-9543792.790122407 -3414223.204776919,-9403830.51804661 \
        -3816878.675376871,-9237374.032379992 C-4219534.145976967,-9070917.546713307 \
        -4610880.115460548,-8878322.731199488 -4988414.462729018,-8660820.962424062 \
        C-5365948.809997551,-8443319.193648601 -5728864.340748512,-8201375.505296162 \
        -6074840.710418862,-7936536.797297642 C-6420817.080089023,-7671698.08929928 \
        -6749114.591663366,-7384530.586457999 -7057634.231667321,-7076870.328186368 \
        C-7366153.871671174,-6769210.069914838 -7654236.003975097,-6441714.854925347 \
        -7920038.741362771,-6096478.567114086 C-8185841.47875073,-5751242.279302422 \
        -8428796.535325298,-5389003.033390723 -8647350.54484989,-5012076.850022447 \
        C-8865904.554374503,-4635150.666654132 -9059590.233401831,-4244343.439835602 \
        -9227169.23076923,-3842153.846153846 L9234369.23076923,3850153.846153846 \
        C9066784.781173492,4252356.525183618 8873092.805520594,4643176.452509806 8654531.700436374,5020114.873137948 \
        C8435970.595352177,5397053.2937660515 8193007.644283968,5759304.313689391 \
        7927196.258931525,6104551.833857746 C7661384.873579367,6449799.354025697 7373293.377015544,6777305.211431853 \
        7064763.720634047,7084975.458180397 C6756234.064252447,7392645.704929044 6427925.884182718,7679822.542307893 \
        6081938.258077101,7944669.8669057805 C5735950.631971295,8209517.191503827 \
        5373023.305233396,8451468.741477128 4995476.7009604275,8668977.571650218 \
        C4617930.096687522,8886486.401823273 4226571.409182008,9079087.478511576 \
        3823902.8380764644,9245549.379885377 C3421234.2669710643,9412011.281259112 \
        3008116.7454389255,9551978.100293914 2587191.593202775,9664554.944343826 C2166266.440966649,9777131.78839373 \
        1738433.5956257395,9862077.968266863 1306428.4596519491,9918850.366994306 \
        C874423.3236780758,9975622.76572176 439169.55384319753,10004100 3450.0000000000005,10004100 \
        C-432269.55384318356,10004100 -867523.323678048,9975622.765721764 -1299528.4596519491,9918850.366994306 \
        C-1731533.5956257395,9862077.968266863 -2159366.440966649,9777131.78839373 \
        -2580291.593202775,9664554.944343826 C-3001216.7454389255,9551978.100293914 \
        -3414334.2669710643,9412011.281259177 -3817002.8380766246,9245549.379885377 \
        C-4219671.409182021,9079087.478511637 -4611030.096687545,8886486.40182328 \
        -4988576.700960461,8668977.571650218 C-5366123.30523336,8451468.741477165 \
        -5729050.631971197,8209517.191503736 -6075038.2580769,7944669.8669057805 \
        C-6421025.884182718,7679822.542307738 -6749334.064252447,7392645.704929044 \
        -7057863.720634047,7084975.458180397 C-7366393.377015544,6777305.211431853 \
        -7654484.873579367,6449799.354026053 -7920296.258931798,6104551.833857746 \
        C-8186107.644283968,5759304.313689747 -8429070.595352177,5397053.2937660515 \
        -8647631.700436374,5020114.873137948 C-8866192.805520594,4643176.452509806 \
        -9059884.781173492,4252356.525183618 -9227469.23076923,3850153.846153846 \
        C-9059890.233401831,4252343.439835602 -8866204.554374503,4643150.666654132 \
        -8647650.54484989,5020076.850022447 C-8429096.535325298,5397003.033390723 \
        -8186141.47875073,5759242.279302066 -7920338.741362771,6104478.56711373 \
        C-7654536.003975097,6449714.854924991 -7366453.871671174,6777210.069914838 \
        -7057934.231667321,7084870.328186368 C-6749414.591663366,7392530.586457999 \
        -6421117.080089023,7679698.089299126 -6075140.710418862,7944536.797297487 \
        C-5729164.340748512,8209375.505296008 -5366248.809997551,8451319.193648579 \
        -4988714.462729018,8668820.96242404 C-4611180.115460548,8886322.731199466 \
        -4219834.145976967,9078917.546713242 -3817178.675376871,9245374.032379927 \
        C-3414523.204776919,9411830.518046545 -3001419.1099179615,9551792.790122407 \
        -2580507.6229655123,9664365.979378814 C-2159596.136013088,9776939.168635216 \
        -1731777.1945663989,9861882.58588024 -1299786.113539819,9918653.137562357 \
        C-867795.0325135754,9975423.689244416 -432555.40826094727,10003900 3150.0000000000005,10003900 \
        C438855.40826093336,10003900 874095.0325135479,9975423.68924446 1306086.1135401346,9918653.137562357 \
        C1738077.1945663989,9861882.585880281 2165896.136013088,9776939.168635216 \
        2586807.6229655123,9664365.979378814 C3007719.1099179615,9551792.790122407 \
        3420823.204776919,9411830.51804661 3823478.6753770313,9245374.032379927 C4226134.14597698,9078917.546713302 \
        4617480.115460571,8886322.731199473 4995014.462729052,8668820.96242404 C5372548.809997516,8451319.193648616 \
        5735464.340748414,8209375.505295917 6081440.710418661,7944536.797297487 C6427417.080089023,7679698.089298971 \
        6755714.591663366,7392530.586457999 7064234.231667321,7084870.328186368 C7372753.871671174,6777210.069914838 \
        7660836.003975097,6449714.854925347 7926638.741363044,6104478.56711373 C8192441.47875073,5759242.279302422 \
        8435396.535325298,5397003.033390723 8653950.54484989,5020076.850022447 C8872504.554374503,4643150.666654132 \
        9066190.233401831,4252343.439835602 9233769.23076923,3850153.846153846 Z
    ";

pub(crate) fn outline() -> BezPath {
    BezPath::from_svg(OUTLINE).expect("the outline is a path")
}

/// Whether the sweep of `path` panics here: what `Topology::from_path`, which every sweep of the crate is, does for an input it checks.
pub(crate) fn sweep_panics(path: &BezPath) -> bool {
    let Ok(eps) = eps_for([path]) else { return false };
    catch_unwind(AssertUnwindSafe(|| drop(Topology::<i32>::from_path(path, eps)))).is_err()
}

/// Whether the dry sweep of the meter, which sweeps as the topology does, panics on `path` here.
pub(crate) fn dry_sweep_panics(path: &BezPath) -> bool {
    let (Ok(eps), Some(segments)) = (eps_for([path]), segments_of([path])) else { return false };
    catch_unwind(AssertUnwindSafe(|| {
        let _ = dry_sweep_over(&segments, eps, u64::MAX, &mut || 0);
    }))
    .is_err()
}

/// An open run of two arcs, each of chord 0.3 and bowing by 0.1 (one one way, one the other), starting at `(x, y)`, all scaled by 1,000: the run
/// whose stroke, 20 million across, the outline above is the sweep of.
fn s_run(x: f64, y: f64) -> BezPath {
    let at = |dx: f64| Point::new((x + dx) * 1_000.0, y * 1_000.0);
    let arc_to = |path: &mut BezPath, from: Point, to: Point, bow: f64| {
        let chord = (to - from).hypot();
        let radius = (chord * chord / 4.0 + bow * bow) / (2.0 * bow.abs());
        let arc = SvgArc { from, to, radii: Vec2::new(radius, radius), x_rotation: 0.0, large_arc: false, sweep: bow > 0.0 };
        if let Some(arc) = Arc::from_svg_arc(&arc) {
            arc.to_cubic_beziers(1e-6, |a, b, c| path.curve_to(a, b, c));
        }
    };
    let mut path = BezPath::new();
    path.move_to(at(0.0));
    arc_to(&mut path, at(0.0), at(0.3), 100.0);
    arc_to(&mut path, at(0.3), at(0.6), -100.0);
    path
}

/// `path` three times over, as three subpaths: more pieces than [`GATE_PIECES`], so that the meter sweeps it before the sweep is made.
fn three_times(path: &BezPath) -> BezPath {
    let mut all = BezPath::new();
    for _ in 0..3 {
        all.extend(path.iter());
    }
    all
}

/// What each function that sweeps and says why it refuses gave for `path` as the one shape or path it sweeps (the operations on two operands
/// have it for both): the name, whether the sweep of `path` alone is the first sweep it makes (so that the check fails there if it fails for
/// `path`), and what it gave.
fn refusals(path: &BezPath) -> Vec<(String, bool, Result<(), PathOpsError>)> {
    let shapes = [Shape::new(path.clone(), NZ, 0)];
    let mut all: Vec<(String, bool, Result<(), PathOpsError>)> = Vec::new();
    let mut add = |name: String, alone: bool, got: Result<(), PathOpsError>| all.push((name, alone, got));
    add("try_normalize".into(), true, try_normalize(path, NZ).map(drop));
    add("try_area".into(), true, try_area(path, NZ).map(drop));
    for op in [BoolOp::Union, BoolOp::Intersect, BoolOp::Difference, BoolOp::Xor] {
        add(format!("try_boolean {op:?}"), false, try_boolean(path, NZ, path, NZ, op, DEFAULT_PRECISION).map(drop));
    }
    add("try_boolean_n".into(), false, try_boolean_n(&[(path, NZ), (path, NZ)], |_| true).map(drop));
    add("try_unite_all".into(), true, try_unite_all(&[(path, NZ)]).map(drop));
    add("try_unite_all_with".into(), true, try_unite_all_with(&[(path, NZ)], DEFAULT_PRECISION).map(drop));
    add("try_offset_path inset".into(), true, try_offset_path(path, -1_000.0, Join::Miter, 4.0).map(drop));
    add("try_offset_path outset".into(), true, try_offset_path(path, 1_000.0, Join::Round, 4.0).map(drop));
    add("try_stroke_region".into(), true, try_stroke_region(std::slice::from_ref(path)).map(drop));
    for op in ALL_OPS {
        add(format!("try_pathfinder {op:?}"), true, try_pathfinder(op, &shapes).map(drop));
        add(format!("try_pathfinder_with {op:?}"), true, try_pathfinder_with(op, &shapes, 0.5).map(drop));
    }
    add("try_regions_within".into(), true, try_regions_within(&shapes, 100).map(drop));
    add("try_shape_builder with edges".into(), true, try_shape_builder(&shapes, true).map(drop));
    add("try_shape_builder".into(), true, try_shape_builder(&shapes, false).map(drop));
    all
}

const ALL_OPS: [PathfinderOp; 10] = [
    PathfinderOp::Unite,
    PathfinderOp::MinusFront,
    PathfinderOp::Intersect,
    PathfinderOp::Exclude,
    PathfinderOp::Divide,
    PathfinderOp::Trim,
    PathfinderOp::Merge,
    PathfinderOp::Crop,
    PathfinderOp::Outline,
    PathfinderOp::MinusBack,
];

/// The check that fails on Linux is the premise of the tests below, and is held to there, where it was found (x86-64, in a build with debug
/// assertions): without it they would pass for any input, and say nothing. Elsewhere they find out whether the check fails.
fn fails_on_linux() -> bool {
    cfg!(all(target_os = "linux", target_arch = "x86_64", debug_assertions))
}

#[test]
fn the_outline_fails_the_sweeps_check_on_linux_in_a_debug_build() {
    let (p, three) = (outline(), three_times(&outline()));
    if fails_on_linux() {
        assert!(sweep_panics(&p) && dry_sweep_panics(&p), "the outline's sweep panics");
        assert!(sweep_panics(&three) && dry_sweep_panics(&three), "its three copies' sweep panics");
    }
    assert!(segments_of([&three]).is_some_and(|s| s.len() > GATE_PIECES), "the copies are over the gate of the meter");
}

/// Every function that sweeps and says why it refuses gives a refusal for an input whose sweep panics, and never a panic (a panic fails the test
/// where it is made: nothing here catches it): the outline; three of it, which the meter sweeps by itself first (it is over the gate), so that the panic is the dry sweep's before it is the topology's; and the
/// outline moved in its last place, whose sweep the planar map's frame makes panic and the topology's does not. Where the sweep of the input
/// panics here, a function whose first sweep is of that input alone says `Degenerate`; where it does not, a function is free to make the input.
#[test]
fn every_function_that_sweeps_refuses_in_words_a_sweep_that_panics_and_never_panics() {
    let p = outline();
    for (what, path) in [("the outline", p.clone()), ("three of it", three_times(&p)), ("the outline moved in its last place", outline_in_the_map())]
    {
        let panics = sweep_panics(&path);
        for (name, alone, got) in refusals(&path) {
            if panics && alone {
                assert_eq!(got, Err(PathOpsError::Degenerate), "{what}: {name} for an input whose sweep panics");
            }
            // Where it was found (the first two of these), the sweep panics in every operation.
            if fails_on_linux() && !what.contains("moved") {
                assert_eq!(got, Err(PathOpsError::Degenerate), "{what}: {name}");
            }
        }
    }
}

/// The planar map's sweep fails the check for a variant of the outline that the topology's passes (on Linux, as above): the map that `live_paint`
/// gives is none, and the shape builder says `Degenerate`.
#[test]
fn the_planar_maps_sweep_is_guarded_as_the_topologys_is() {
    let moved = outline_in_the_map();
    let shapes = [Shape::new(moved.clone(), NZ, 0)];
    let (faces, edges) = live_paint(&shapes);
    let shaped = try_shape_builder(&shapes, true).map(drop);
    if fails_on_linux() {
        assert!(!sweep_panics(&moved), "the topology's sweep passes it");
        assert!(faces.is_empty() && edges.is_empty(), "the map is none where its sweep panics");
        assert_eq!(shaped, Err(PathOpsError::Degenerate));
    }
}

/// The meter's own sweep is under a guard at all its entries: for an input that is over the gate and whose dry sweep panics, each counts nothing
/// (`Ok`), as the sweep that follows is the one that refuses.
#[test]
fn the_meter_counts_nothing_for_a_dry_sweep_that_panics() {
    let three = three_times(&outline());
    let eps = eps_for([&three]).expect("finite");
    assert_eq!(within([&three], eps), Ok(()));
    assert_eq!(within_for_a_map([&three], eps), Ok(()));
    let mut allowance = Allowance::new();
    assert_eq!(within_charging([&three], eps, &mut allowance), Ok(()));
    assert!(!allowance.exhausted());
}

/// The functions that give no reason give what each documents for an input that cannot be swept: nothing, the path as it was (an offset), or the
/// filled areas of it (the shape builder).
#[test]
fn the_functions_that_do_not_say_why_give_what_they_document_for_a_sweep_that_panics() {
    let p = outline();
    let shapes = [Shape::new(p.clone(), NZ, 0)];
    let panics = sweep_panics(&p);
    let (normal, size) = (normalize(&p, NZ), area(&p, NZ));
    let united = (unite_all(&[(&p, NZ)]), unite_all_with(&[(&p, NZ)], DEFAULT_PRECISION));
    let (both, many) = (boolean(&p, NZ, &p, NZ, BoolOp::Union), boolean_n(&[(&p, NZ), (&p, NZ)], |_| true));
    let stroked = stroke_region(std::slice::from_ref(&p));
    let offset = offset_path(&p, -1_000.0, Join::Miter, 4.0);
    let made = (pathfinder(PathfinderOp::Unite, &shapes), pathfinder_with(PathfinderOp::Divide, &shapes, 0.5), regions(&shapes));
    let (within, at) = (regions_within(&shapes, 10), region_at(&shapes, Point::new(0.0, 0.0)));
    // These have no reason to give, and their sweeps can pass where the others' fail: they only must not panic.
    let _ = (shape_builder(&shapes, true), live_paint(&shapes), interior_point(&p), merge_regions(&[]));
    if panics {
        assert!(normal.elements().is_empty() && size == 0.0, "normalize and area");
        assert!(united.0.elements().is_empty() && united.1.elements().is_empty() && stroked.elements().is_empty(), "unite_all and stroke_region");
        assert_eq!(offset, p, "offset_path keeps the path as it was");
        assert!(made.0.is_empty() && made.1.is_empty() && made.2.is_empty(), "pathfinder and regions");
        assert!(within.is_some_and(|r| r.is_empty()) && at.is_none(), "regions_within and region_at");
    }
    // The sweep of two copies of the outline is not that of one, and fails the check where it was found.
    if fails_on_linux() {
        assert!(both.elements().is_empty() && many.elements().is_empty(), "boolean and boolean_n");
    }
}

/// A stroke of the run that [`OUTLINE`] is the outline of is refused in words or made, and where it is refused the function with no error gives
/// an empty path. The run is made with the standard library's `atan2`, `cos`, `sin` and `sqrt`, so which of the two it is depends on the platform;
/// on Linux it is refused in a build with debug assertions.
#[test]
fn a_stroke_whose_outline_fails_the_check_is_refused_in_words_and_never_panics() {
    let run = s_run(3.0, 4.0);
    let got = try_outline_stroke(&run, 2.0e7, Cap::Butt, Join::Miter, 4.0);
    match &got {
        Ok(made) => assert!(!made.elements().is_empty()),
        Err(e) => assert!(cfg!(debug_assertions) && *e == PathOpsError::Degenerate, "{e:?}"),
    }
    assert_eq!(got.is_err(), outline_stroke(&run, 2.0e7, Cap::Butt, Join::Miter, 4.0).elements().is_empty());
    if fails_on_linux() {
        assert_eq!(got.map(drop), Err(PathOpsError::Degenerate));
    }
}

/// `path` with every coordinate moved by up to `ulps` units in the last place (each by a hash of its place and of `seed`).
fn nudged(path: &BezPath, seed: u64, ulps: i64) -> BezPath {
    use crate::testutil::hash01;
    let mut n = 0u64;
    let mut moved = |x: f64| {
        n += 1;
        let by = ((hash01(n, seed) * (2 * ulps + 1) as f64) as i64) - ulps;
        x + by as f64 * x.abs() * f64::EPSILON
    };
    let mut at = |p: Point| Point::new(moved(p.x), moved(p.y));
    let mut out = BezPath::new();
    for el in path.elements() {
        out.push(match *el {
            kurbo::PathEl::MoveTo(p) => kurbo::PathEl::MoveTo(at(p)),
            kurbo::PathEl::LineTo(p) => kurbo::PathEl::LineTo(at(p)),
            kurbo::PathEl::QuadTo(a, b) => kurbo::PathEl::QuadTo(at(a), at(b)),
            kurbo::PathEl::CurveTo(a, b, c) => kurbo::PathEl::CurveTo(at(a), at(b), at(c)),
            kurbo::PathEl::ClosePath => kurbo::PathEl::ClosePath,
        });
    }
    out
}

/// [`outline`] with its coordinates moved by up to one unit in the last place, as the 188th of the 300 seeds of a hash moves them. The check is
/// hair-trigger: of the outline and its 299 variants it fails in the topology's sweep for 3 (the outline and two of the variants) and in the
/// planar map's sweep for 1 (this one, which the topology's passes). The map is the one of `live_paint`, the shape builder and the faces of an
/// Outline, and is swept in another frame (the closed subpaths are written again, and snapped).
fn outline_in_the_map() -> BezPath {
    nudged(&outline(), 188, 1)
}
