#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing, clippy::float_cmp)]

use super::*;
use crate::tree::document::{Document, Node};
use crate::tree::{Limits, parse, usvg};

/// The segments usvg makes of `element`, the only shape in a document.
fn segments(element: &str) -> Vec<Segment> {
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">{element}</svg>"#);
    let tree = parse(svg.as_bytes(), &Limits::default(), &usvg::Options::default()).unwrap();
    let document = Document::from_tree(&tree, 16);
    fn first(nodes: &[Node]) -> Option<Vec<Segment>> {
        nodes.iter().find_map(|n| match n {
            Node::Path(p) => Some(p.segments.clone()),
            Node::Group { children, .. } => first(children),
            Node::Image(_) => None,
        })
    }
    first(&document.nodes).unwrap()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn a_rect_is_a_rect() {
    let Some(Primitive::Rect { x, y, width, height, rx, ry }) = recognize(&segments(r#"<rect x="10" y="20" width="30" height="40"/>"#)) else {
        panic!()
    };
    assert_eq!((x, y, width, height, rx, ry), (10.0, 20.0, 30.0, 40.0, 0.0, 0.0));
}

#[test]
fn a_rounded_rect_keeps_its_radii() {
    for (attrs, want) in [(r#"rx="5""#, (5.0, 5.0)), (r#"rx="6" ry="3""#, (6.0, 3.0)), (r#"ry="4""#, (4.0, 4.0))] {
        let element = format!(r#"<rect x="10" y="20" width="30" height="40" {attrs}/>"#);
        let Some(Primitive::Rect { x, y, width, height, rx, ry }) = recognize(&segments(&element)) else { panic!("{element}") };
        assert!(near(x, 10.0) && near(y, 20.0) && near(width, 30.0) && near(height, 40.0), "{element}");
        assert!(near(rx, want.0) && near(ry, want.1), "{element}: {rx} {ry}");
    }
}

#[test]
fn circles_and_ellipses_are_ellipses() {
    for (element, want) in [
        (r#"<circle cx="50" cy="60" r="25"/>"#, (50.0, 60.0, 25.0, 25.0)),
        (r#"<ellipse cx="50" cy="60" rx="40" ry="10"/>"#, (50.0, 60.0, 40.0, 10.0)),
        // A rectangle whose corners meet in the middle is an ellipse too.
        (r#"<rect x="0" y="0" width="20" height="10" rx="10" ry="5"/>"#, (10.0, 5.0, 10.0, 5.0)),
    ] {
        let Some(Primitive::Ellipse { cx, cy, rx, ry }) = recognize(&segments(element)) else { panic!("{element}") };
        assert!(near(cx, want.0) && near(cy, want.1) && near(rx, want.2) && near(ry, want.3), "{element}: {cx} {cy} {rx} {ry}");
    }
}

#[test]
fn other_paths_are_not_primitives() {
    for element in [
        // A triangle on the edges of its box encloses half of it.
        r#"<path d="M0 0 L10 0 L10 10 Z"/>"#,
        // Open, or two contours.
        r#"<path d="M0 0 H10 V10 H0" fill="none" stroke="black"/>"#,
        r#"<path d="M0 0 H10 V10 H0 Z M20 0 H30 V10 H20 Z"/>"#,
        // A rectangle drawn twice, a diamond, a star.
        r#"<path d="M0 0 H10 V10 H0 Z H10 V10 H0 Z"/>"#,
        r#"<path d="M5 0 L10 5 L5 10 L0 5 Z"/>"#,
        r#"<path d="M50 0 L61 35 L98 35 L68 57 L79 91 L50 70 L21 91 L32 57 L2 35 L39 35 Z"/>"#,
        // A curve that bulges past a circle's outline.
        r#"<path d="M0 50 C0 -20 100 -20 100 50 C100 120 0 120 0 50 Z"/>"#,
    ] {
        assert_eq!(recognize(&segments(element)), None, "{element}");
    }
}

#[test]
fn degenerate_and_oversized_input_is_refused() {
    assert_eq!(recognize(&[]), None);
    assert_eq!(recognize(&[Segment::LineTo([1.0, 1.0])]), None);
    assert_eq!(recognize(&[Segment::MoveTo([0.0, 0.0]), Segment::LineTo([10.0, 0.0]), Segment::Close]), None, "no height");
    let nan = [Segment::MoveTo([f64::NAN, 0.0]), Segment::LineTo([1.0, 0.0]), Segment::LineTo([1.0, 1.0]), Segment::Close];
    assert_eq!(recognize(&nan), None);
    let mut long = vec![Segment::MoveTo([0.0, 0.0])];
    long.extend((0..100).map(|i| Segment::LineTo([f64::from(i % 2) * 10.0, 0.0])));
    long.push(Segment::Close);
    assert_eq!(recognize(&long), None);
}

#[test]
fn hand_written_segments_are_recognised_too() {
    // Square corners written clockwise from the bottom left, closed by returning to the start.
    let square = [
        Segment::MoveTo([0.0, 10.0]),
        Segment::LineTo([0.0, 0.0]),
        Segment::LineTo([10.0, 0.0]),
        Segment::LineTo([10.0, 10.0]),
        Segment::LineTo([0.0, 10.0]),
    ];
    assert_eq!(recognize(&square), Some(Primitive::Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0, rx: 0.0, ry: 0.0 }));
    // A circle from four cubic quarters, through Path::primitive.
    let k = 0.552_284_749_830_793_6 * 5.0;
    let path = crate::tree::document::Path {
        segments: vec![
            Segment::MoveTo([10.0, 5.0]),
            Segment::CubicTo([10.0, 5.0 + k], [5.0 + k, 10.0], [5.0, 10.0]),
            Segment::CubicTo([5.0 - k, 10.0], [0.0, 5.0 + k], [0.0, 5.0]),
            Segment::CubicTo([0.0, 5.0 - k], [5.0 - k, 0.0], [5.0, 0.0]),
            Segment::CubicTo([5.0 + k, 0.0], [10.0, 5.0 - k], [10.0, 5.0]),
            Segment::Close,
        ],
        fill: None,
        fill_rule: crate::tree::document::FillRule::NonZero,
        stroke: None,
        source_path: crate::tree::document::NodePath::default(),
        id: String::new(),
        transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    };
    assert_eq!(path.primitive(), Some(Primitive::Ellipse { cx: 5.0, cy: 5.0, rx: 5.0, ry: 5.0 }));
}
