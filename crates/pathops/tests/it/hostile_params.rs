// Carved from VectorCraft's crates/pathops (tests/it/hostile_params.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Absurd but finite or non-finite numeric parameters: no panic, hang or runaway allocation.

use craft_pathops::*;
use kurbo::{BezPath, Point};

use crate::common::{poly, polyline};

const HOSTILE: [f64; 9] = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -0.0, 1e-300, 1e300, -1e300, f64::MAX];

fn square() -> BezPath {
    poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], true)
}

fn zigzag() -> BezPath {
    let pts: Vec<Point> = (0..40).map(|i| Point::new(f64::from(i) * 3.0, if i % 2 == 0 { 0.0 } else { 7.0 })).collect();
    polyline(&pts, false)
}

#[test]
fn edit_functions_survive_hostile_numbers() {
    for paths in [square(), zigzag()] {
        for &v in &HOSTILE {
            let _ = simplify(&paths, v);
            let _ = simplify_with(&paths, &SimplifyOptions::default().tolerance(v).corner_angle_deg(v));
            let _ = simplify_with(&paths, &SimplifyOptions::default().corner_angle_deg(v).straight_lines(true));
            let _ = simplify_with(&paths, &SimplifyOptions::default().smooth(v));
            let _ = remove_redundant_points(&paths, v);
            let _ = join(&[paths.clone(), zigzag()], v);
        }
    }
}

/// A stroke width of 1e200 or more with round joins used to panic inside the sweep (`Left == Ish`).
#[test]
fn stroke_and_offset_survive_hostile_numbers() {
    for paths in [square(), zigzag()] {
        for &w in &HOSTILE {
            for &m in &[f64::NAN, 0.0, 1.0, 1e300] {
                for join in [Join::Miter, Join::Round, Join::Bevel] {
                    for cap in [Cap::Butt, Cap::Round] {
                        let _ = outline_stroke(&paths, w, cap, join, m);
                    }
                    let _ = offset_path(&paths, w, join, m);
                }
            }
        }
    }
}

/// Hostile coordinates reach every operation without a panic or a hang: each answers an error, an
/// empty path or a bounded result.
#[test]
fn every_operation_survives_hostile_coordinates() {
    let rule = FillRule::NonZero;
    for &v in &HOSTILE {
        let bad = poly(&[(0.0, 0.0), (v, 0.0), (v, v), (0.0, v)], true);
        let ok = square();
        for op in [BoolOp::Union, BoolOp::Intersect, BoolOp::Difference, BoolOp::Xor] {
            let _ = boolean(&bad, rule, &ok, rule, op);
            let _ = try_boolean(&ok, rule, &bad, rule, op, DEFAULT_PRECISION);
            let _ = try_boolean(&ok, rule, &bad, rule, op, v);
        }
        let _ = area(&bad, rule);
        let _ = normalize(&bad, rule);
        let _ = unite_all(&[(&bad, rule), (&ok, rule)]);
        let shapes = [Shape::new(ok.clone(), rule, 0), Shape::new(bad.clone(), rule, 1)];
        for op in [
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
        ] {
            let _ = pathfinder(op, &shapes);
            let _ = pathfinder_with(op, &shapes, v);
        }
        let _ = regions(&shapes);
        let _ = shape_builder(&shapes, true);
        let _ = live_paint(&shapes);
        let _ = interior_point(&bad);
        let _ = stroke_region(&[bad.clone(), ok.clone()]);
        let _ = try_stroke_region(&[bad.clone(), ok.clone()]);
        // The edits and the stroker take the same coordinates.
        let _ = offset_path(&bad, 1.0, Join::Round, 4.0);
        let _ = try_offset_path(&bad, 1.0, Join::Miter, 4.0);
        let _ = outline_stroke(&bad, 1.0, Cap::Round, Join::Round, 4.0);
        let _ = simplify(&bad, 0.5);
        let _ = simplify_with(&bad, &SimplifyOptions::default().smooth(1.0));
        let _ = remove_redundant_points(&bad, 0.1);
        let _ = join(&[bad.clone(), ok.clone()], 1.0);
    }
}

/// Curves with hostile control points (and the places hostile numbers can hide in a curve) reach
/// every operation too: kurbo's flattening and arclength sub-divide without end on some of them.
#[test]
fn curves_with_hostile_control_points_survive() {
    let rule = FillRule::NonZero;
    for &v in &HOSTILE {
        let mut bad = BezPath::new();
        bad.move_to((0.0, 0.0));
        bad.curve_to((v, 0.0), (0.0, v), (10.0, 10.0));
        bad.quad_to((v, v), (0.0, 10.0));
        bad.close_path();
        let ok = square();
        let _ = boolean(&bad, rule, &ok, rule, BoolOp::Union);
        let _ = area(&bad, rule);
        let _ = interior_point(&bad);
        let shapes = [Shape::new(ok.clone(), rule, 0), Shape::new(bad.clone(), rule, 1)];
        for op in [PathfinderOp::Divide, PathfinderOp::Outline, PathfinderOp::Trim] {
            let _ = pathfinder(op, &shapes);
        }
        let _ = shape_builder(&shapes, true);
        let _ = live_paint(&shapes);
        let _ = offset_path(&bad, 1.0, Join::Round, 4.0);
        let _ = outline_stroke(&bad, 1.0, Cap::Round, Join::Round, 4.0);
        let _ = simplify(&bad, 0.5);
        let _ = remove_redundant_points(&bad, 0.1);
        let _ = join(&[bad.clone(), ok.clone()], 1.0);
    }
}

/// One element more than the operations read: a zigzag of `MAX_ELEMENTS` line segments, a move
/// and nothing else.
fn past_the_cap() -> BezPath {
    let mut bp = BezPath::new();
    bp.move_to((0.0, 0.0));
    for i in 0..MAX_ELEMENTS {
        bp.line_to((i as f64, if i % 2 == 0 { 7.0 } else { 0.0 }));
    }
    assert!(bp.elements().len() > MAX_ELEMENTS);
    bp
}

const TOO_COMPLEX: PathOpsError = PathOpsError::TooComplex;

/// A path of more than half a million elements (and one of millions) is refused or returned as it
/// is, by every operation, before any of them copies or sweeps it: the booleans.
#[test]
fn booleans_refuse_paths_past_the_segment_cap() {
    let rule = FillRule::NonZero;
    let (huge, ok) = (past_the_cap(), square());
    for op in [BoolOp::Union, BoolOp::Intersect, BoolOp::Difference, BoolOp::Xor] {
        assert_eq!(try_boolean(&huge, rule, &ok, rule, op, DEFAULT_PRECISION), Err(TOO_COMPLEX));
        assert_eq!(try_boolean(&ok, rule, &huge, rule, op, DEFAULT_PRECISION), Err(TOO_COMPLEX));
        assert!(boolean(&huge, rule, &ok, rule, op).elements().is_empty());
    }
    assert_eq!(try_boolean_n(&[(&ok, rule), (&huge, rule)], |m| m[0]), Err(TOO_COMPLEX));
    assert!(boolean_n(&[(&huge, rule)], |m| m[0]).elements().is_empty());
    assert_eq!(try_unite_all(&[(&ok, rule), (&huge, rule)]), Err(TOO_COMPLEX));
    assert!(unite_all(&[(&huge, rule)]).elements().is_empty());
    assert_eq!(try_area(&huge, rule), Err(TOO_COMPLEX));
    assert_eq!(area(&huge, rule), 0.0);
    assert_eq!(try_normalize(&huge, rule), Err(TOO_COMPLEX));
    assert!(normalize(&huge, rule).elements().is_empty());
}

/// The Pathfinder and the planar maps give no result for such a path.
#[test]
fn pathfinder_and_planar_maps_refuse_paths_past_the_segment_cap() {
    let rule = FillRule::NonZero;
    let shapes = [Shape::new(square(), rule, 0), Shape::new(past_the_cap(), rule, 1)];
    for op in [
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
    ] {
        assert!(pathfinder(op, &shapes).is_empty(), "{op:?}");
    }
    assert!(regions(&shapes).is_empty());
    assert!(region_at(&shapes, Point::new(5.0, 5.0)).is_none());
    assert_eq!(shape_builder(&shapes, true), BuilderArrangement::default());
    assert_eq!(live_paint(&shapes), (vec![], vec![]));
    assert!(has_open_path(&shapes));
    assert_eq!(interior_point(&shapes[1].path), None);
}

/// Offset, stroke and the edits leave such a path as it is or refuse it.
#[test]
fn offsets_strokes_and_edits_refuse_paths_past_the_segment_cap() {
    let (huge, ok) = (past_the_cap(), square());
    assert_eq!(try_offset_path(&huge, 1.0, Join::Round, 4.0), Err(TOO_COMPLEX));
    assert_eq!(offset_path(&huge, 1.0, Join::Round, 4.0), huge, "an offset that fails leaves the path as it is");
    assert_eq!(try_outline_stroke(&huge, 1.0, Cap::Butt, Join::Round, 4.0), Err(TOO_COMPLEX));
    assert!(outline_stroke(&huge, 1.0, Cap::Butt, Join::Round, 4.0).elements().is_empty());
    assert_eq!(try_stroke_region(std::slice::from_ref(&huge)), Err(TOO_COMPLEX));
    assert_eq!(simplify(&huge, 0.5), huge);
    assert_eq!(remove_redundant_points(&huge, 0.5), huge);
    assert_eq!(join(&[huge.clone(), ok.clone()], 1.0).elements().len(), huge.elements().len() + ok.elements().len());
}
