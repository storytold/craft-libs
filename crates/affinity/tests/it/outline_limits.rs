//! Original synthetic graph tests for bounded `.af` outline expansion. These model the reader's
//! accepted schema, without using Affinity software, third-party parser code or external assets.

use craft_affinity::{
    Document, Limits,
    model::{Kind, Path, Point},
    synth::{self, F, Method, tag},
};

fn compound(id: u32, children: Vec<F>) -> F {
    F::Def(id, vec![tag(b"Comp")], vec![(tag(b"Chld"), F::Shared(children))])
}

fn shape(id: u32, class: &[u8; 4], parameters: Vec<(craft_affinity::stream::Tag, F)>) -> F {
    F::Def(
        id,
        vec![tag(b"ShpN")],
        vec![(tag(b"ShpB"), F::F64s(vec![0.0, 0.0, 10.0, 20.0])), (tag(b"Shpe"), F::Def(id + 10_000, vec![tag(class)], parameters))],
    )
}

fn read(definitions: Vec<F>, top: u32) -> Document {
    try_read(definitions, top).unwrap()
}

fn try_read(definitions: Vec<F>, top: u32) -> Result<Document, craft_affinity::Error> {
    let data = synth::stream(&[
        (tag(b"Defs"), F::Shared(definitions)),
        (
            tag(b"DocR"),
            F::Def(
                1,
                vec![tag(b"DocN")],
                vec![(
                    tag(b"Chld"),
                    F::Shared(vec![F::Def(
                        2,
                        vec![tag(b"Sprd")],
                        vec![(tag(b"SprB"), F::F64s(vec![0.0, 0.0, 400.0, 300.0])), (tag(b"Chld"), F::Shared(vec![F::Ref(top)]))],
                    )]),
                )],
            ),
        ),
    ]);
    let bytes = synth::container(&[("doc.dat", &data, Method::Zlib)], None);
    craft_affinity::read(&bytes, Limits::default())
}

fn path(document: &Document) -> &Path {
    match &document.spreads[0].nodes[0].kind {
        Kind::Shape { path, .. } => path,
        other => panic!("expected shape, got {other:?}"),
    }
}

fn rejected(document: &Document, warning: &str) {
    assert!(matches!(document.spreads[0].nodes[0].kind, Kind::Unsupported));
    assert!(document.warnings.iter().any(|value| value.contains(warning)), "{:?}", document.warnings);
}

#[test]
fn self_cycle_and_mutual_cycle_discard_the_entire_compound() {
    // A valid first operand ensures a malformed later operand cannot leave a plausible partial path.
    let rectangle = shape(100, b"ShNR", vec![]);
    let self_cycle = compound(101, vec![F::Ref(100), F::Ref(101)]);
    rejected(&read(vec![rectangle.clone(), self_cycle], 101), "compound outline that contains itself");
    let mutual_cycle = compound(101, vec![F::Ref(100), compound(102, vec![F::Ref(101)])]);
    rejected(&read(vec![rectangle, mutual_cycle], 101), "compound outline that contains itself");
}

#[test]
fn deep_reference_chain_is_bounded_independently_of_stream_nesting() {
    let mut definitions = vec![shape(100, b"ShNR", vec![])];
    for id in 101..=250 {
        definitions.push(compound(id, vec![F::Ref(id - 1)]));
    }
    rejected(&read(definitions, 250), "compound outlines nested deeper than 128 levels");
}

#[test]
fn outline_at_the_depth_limit_is_still_imported() {
    let mut definitions = vec![shape(100, b"ShNR", vec![])];
    for id in 101..=227 {
        definitions.push(compound(id, vec![F::Ref(id - 1)]));
    }
    let document = read(definitions, 227);
    assert!(document.warnings.is_empty(), "{:?}", document.warnings);
    assert_eq!(path(&document).subpaths.len(), 1);
    assert_eq!(path(&document).subpaths[0].segments.len(), 3);
}

#[test]
fn excessive_declared_curve_count_is_rejected_before_expansion() {
    let curve = F::Def(
        100,
        vec![tag(b"PCrv")],
        vec![(tag(b"Crvs"), F::Def(101, vec![tag(b"PCvD")], vec![(tag(b"Data"), F::Pos(vec![F::U8(0), F::U32(u32::MAX)]))]))],
    );
    rejected(&read(vec![curve], 100), "compound outlines exceeding the geometry work limit");
}

#[test]
fn empty_shared_fanout_cannot_bypass_the_operand_visit_limit() {
    // Only 21 on-disk objects, but an unbounded traversal would visit over two million operands.
    let mut definitions = vec![compound(100, vec![])];
    for id in 101..=120 {
        definitions.push(compound(id, vec![F::Ref(id - 1), F::Ref(id - 1)]));
    }
    rejected(&read(definitions, 120), "compound outlines exceeding the operand visit limit");
}

#[test]
fn shared_large_operands_are_charged_for_cumulative_geometry_work() {
    // A small reference graph expands a 20,000-segment star. Count work across every reuse and
    // transform, rather than permitting each individual operand's allocation limit repeatedly.
    let mut definitions = vec![shape(100, b"ShSt", vec![(tag(b"Pnts"), F::I32(10_000))])];
    for id in 101..=112 {
        definitions.push(compound(id, vec![F::Ref(id - 1), F::Ref(id - 1)]));
    }
    rejected(&read(definitions, 112), "compound outlines exceeding the geometry work limit");
}

#[test]
fn ordinary_nested_compound_preserves_repeated_operands_and_transforms() {
    let mut rectangle = shape(100, b"ShNR", vec![]);
    if let F::Def(_, _, fields) = &mut rectangle {
        fields.push((tag(b"Xfrm"), F::F64s(vec![1.0, 0.0, 3.0, 0.0, 1.0, 7.0])));
    }
    let inner = compound(101, vec![F::Ref(100), F::Ref(100)]);
    let mut outer = compound(102, vec![F::Ref(101)]);
    if let F::Def(_, _, fields) = &mut outer {
        fields.push((tag(b"Xfrm"), F::F64s(vec![1.0, 0.0, 100.0, 0.0, 1.0, 200.0])));
    }
    let document = read(vec![rectangle, inner, outer], 102);
    assert!(document.warnings.is_empty(), "{:?}", document.warnings);
    let path = path(&document);
    assert_eq!(path.subpaths.len(), 2);
    assert_eq!(path.subpaths[0], path.subpaths[1]);
    assert_eq!(path.subpaths[0].start, Point { x: 103.0, y: 207.0 });
    assert_eq!(path.subpaths[0].segments.last().unwrap()[2], Point { x: 103.0, y: 227.0 });
}

#[test]
fn overflowing_compound_transform_is_reported_before_processing_geometry() {
    let mut child = shape(100, b"ShNR", vec![]);
    if let F::Def(_, _, fields) = &mut child {
        fields.push((tag(b"Xfrm"), F::F64s(vec![1.0e308, 0.0, 0.0, 0.0, 1.0, 0.0])));
    }
    let mut outer = compound(101, vec![F::Ref(100)]);
    if let F::Def(_, _, fields) = &mut outer {
        fields.push((tag(b"Xfrm"), F::F64s(vec![1.0e308, 0.0, 0.0, 0.0, 1.0, 0.0])));
    }
    rejected(&read(vec![child, outer], 101), "compound operand with an invalid transform");
}

#[test]
fn pie_angles_with_huge_finite_values_do_not_loop_or_emit_nonfinite_points() {
    let document = read(vec![shape(100, b"ShPi", vec![(tag(b"AngS"), F::F64(1.0e308)), (tag(b"AngE"), F::F64(-1.0e308))])], 100);
    for subpath in &path(&document).subpaths {
        assert!(subpath.start.x.is_finite() && subpath.start.y.is_finite());
        assert!(subpath.segments.len() <= 20);
        assert!(subpath.segments.iter().flatten().all(|point| point.x.is_finite() && point.y.is_finite()));
    }
}

#[test]
fn wrapped_pie_angles_keep_the_same_geometry_as_the_positive_span() {
    let make = |end| read(vec![shape(100, b"ShPi", vec![(tag(b"AngS"), F::F64(0.0)), (tag(b"AngE"), F::F64(end))])], 100);
    assert_eq!(path(&make(-std::f64::consts::FRAC_PI_2)), path(&make(3.0 * std::f64::consts::FRAC_PI_2)));
}

/// `CTyp`: an array of corner-type enums (top left, top right, bottom right, bottom left).
fn corner_types(ids: [u16; 4]) -> F {
    let mut raw = vec![0xaa];
    raw.extend(tag(b"CTyp").0.to_le_bytes());
    raw.extend(4u32.to_le_bytes());
    raw.extend(0u16.to_le_bytes());
    raw.extend(ids.iter().flat_map(|id| id.to_le_bytes()));
    F::Raw(raw)
}

#[test]
fn unlocked_rectangle_corners_keep_their_own_radius_and_type() {
    let p = |x: f64, y: f64| Point { x, y };
    // 10×20 box: relative radii scale by 10. Round 5, round 1, straight 3, none.
    let corners = |lock: Option<bool>| {
        let mut fields = vec![(tag(b"ShCR"), F::F64s(vec![0.5, 0.1, 0.3, 0.0])), (tag(b"CTyp"), corner_types([0, 0, 1, 4]))];
        fields.extend(lock.map(|l| (tag(b"Lock"), F::Bool(l))));
        read(vec![shape(100, b"ShNR", fields)], 100)
    };

    let unlocked = corners(Some(false));
    assert!(unlocked.warnings.is_empty(), "{:?}", unlocked.warnings);
    let sub = &path(&unlocked).subpaths[0];
    assert_eq!(sub.start, p(5.0, 0.0));
    let ends: Vec<Point> = sub.segments.iter().map(|s| s[2]).collect();
    assert_eq!(ends, [p(9.0, 0.0), p(10.0, 1.0), p(10.0, 17.0), p(7.0, 20.0), p(0.0, 20.0), p(0.0, 5.0), p(5.0, 0.0)]);
    assert_eq!(sub.segments[3], [p(7.0, 20.0); 3], "the straight corner is a line");
    assert_ne!(sub.segments[1][0], p(9.0, 0.0), "the round corner is a curve");

    // Locked (the default): every corner follows the top left one.
    let locked = corners(None);
    let sub = &path(&locked).subpaths[0];
    assert_eq!(sub.segments.len(), 8);
    assert_eq!(sub.segments[3][2], p(5.0, 20.0));
}

/// The special shapes come in as their own outlines: one closed contour of finite points inside
/// (or, for a callout's tail, just below) the node's box, and none as the bounding ellipse.
#[test]
fn special_shapes_have_their_own_closed_outlines() {
    for class in [b"ShCl", b"ShHt", b"ShCg", b"ShCR", b"ShCE", b"ShDA", b"ShDS", b"ShTr", b"ShCr", b"ShpD", b"ShSg"] {
        let name = String::from_utf8_lossy(class).into_owned();
        let document = read(vec![shape(100, class, vec![])], 100);
        assert!(!document.warnings.iter().any(|w| w.contains("bounding ellipse")), "{name}: {:?}", document.warnings);
        let (Kind::Shape { path, even_odd, .. }, cog) = (&document.spreads[0].nodes[0].kind, class == b"ShCg") else { panic!("{name}: no shape") };
        // A cog is its teeth and its hole, filled even-odd.
        assert_eq!((path.subpaths.len(), *even_odd), if cog { (2, true) } else { (1, false) }, "{name}");
        for sub in &path.subpaths {
            assert!(sub.closed && sub.segments.len() >= 2, "{name}");
            for p in std::iter::once(sub.start).chain(sub.segments.iter().flatten().copied()) {
                assert!(p.x.is_finite() && p.y.is_finite(), "{name}: {p:?}");
                assert!((-5.0..=15.0).contains(&p.x) && (-10.0..=30.0).contains(&p.y), "{name}: {p:?}");
            }
        }
    }
    // A class the reader does not know is still its bounding ellipse, and said.
    let document = read(vec![shape(100, b"ShZZ", vec![])], 100);
    assert!(document.warnings.iter().any(|w| w.contains("unrecognized parametric shape")), "{:?}", document.warnings);
}

fn group(id: u32, children: Vec<F>) -> F {
    F::Def(id, vec![tag(b"Grup")], vec![(tag(b"Chld"), F::Shared(children))])
}

/// A list's null slots yield no node, operand or segment, but each visit of a shared object scans
/// them all: they are charged to the document's work before the scan.
#[test]
fn null_slots_of_shared_lists_are_charged_for_every_visit() {
    // A compound of a million null operands, placed 20 times: 20 million slots, over the 2^24.
    let document = read(vec![compound(100, vec![F::Null; 1_000_000]), group(200, vec![F::Ref(100); 20])], 200);
    assert!(document.warnings.iter().any(|w| w.contains("geometry work limit")), "{:?}", document.warnings);
    let shapes = document.spreads[0].nodes[0].children.iter().filter(|n| matches!(n.kind, Kind::Shape { .. })).count();
    assert_eq!(shapes, 16, "each placement costs a million slots of the 2^24");
    // A group of a million null children, placed 20 times, fails the read like too many layers.
    let result = try_read(vec![group(100, vec![F::Null; 1_000_000]), group(200, vec![F::Ref(100); 20])], 200);
    assert!(matches!(result, Err(craft_affinity::Error::Limit(_))), "{:?}", result.map(|d| d.warnings));
    // Within the work of one document the same lists are read, nulls skipped.
    let document = read(vec![group(100, vec![F::Null; 1_000]), group(200, vec![F::Ref(100); 20])], 200);
    assert_eq!((document.spreads[0].nodes[0].children.len(), document.warnings.len()), (20, 0));
}

fn boxed(class: &[u8; 4], b: Vec<f64>) -> F {
    F::Def(100, vec![tag(b"ShpN")], vec![(tag(b"ShpB"), F::F64s(b)), (tag(b"Shpe"), F::Def(10_100, vec![tag(class)], vec![]))])
}

/// A callout rectangle (`ShCR`) whose box runs right to left (x0 > x1) panicked in `f64::clamp`
/// (min > max), found by the `stream` fuzz target. It reads, inside its box.
#[test]
fn callout_rectangle_with_a_reversed_box_reads_inside_it() {
    for b in [vec![10.0_f64, 0.0, 0.0, 20.0], vec![6.8e38, 0.0, 10.0, 20.0]] {
        let (lo, hi) = (b[0].min(b[2]), b[0].max(b[2]));
        let document = read(vec![boxed(b"ShCR", b)], 100);
        let sub = &path(&document).subpaths[0];
        let points = std::iter::once(&sub.start).chain(sub.segments.iter().flatten());
        assert!(points.into_iter().all(|q| q.x.is_finite() && q.y.is_finite() && (lo..=hi).contains(&q.x)), "{sub:?}");
    }
}

/// A box with finite corners whose width overflows gave infinite and NaN outline points. Such a
/// shape imports as its box, with a warning.
#[test]
fn a_shape_too_large_to_compute_imports_as_its_box() {
    for class in [b"ShCR", b"ShpE", b"ShCg"] {
        let document = read(vec![boxed(class, vec![f64::MAX, 0.0, -f64::MAX, 20.0])], 100);
        let sub = &path(&document).subpaths;
        assert_eq!(sub.len(), 1, "{class:?}");
        let corners: Vec<Point> = std::iter::once(sub[0].start).chain(sub[0].segments.iter().map(|s| s[2])).collect();
        assert!(corners.iter().all(|p| p.x.abs() == f64::MAX && (p.y == 0.0 || p.y == 20.0)), "{class:?}: {corners:?}");
        assert!(document.warnings.iter().any(|w| w.contains("too large to compute")), "{:?}", document.warnings);
    }
}
