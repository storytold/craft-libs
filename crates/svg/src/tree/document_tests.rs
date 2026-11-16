#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing, clippy::float_cmp)]

use super::*;
use crate::tree::{Limits, parse};

fn tree(body: &str) -> usvg::Tree {
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">{body}</svg>"#);
    parse(svg.as_bytes(), &Limits::default(), &usvg::Options::default()).unwrap()
}

fn apply(m: [f64; 6], p: [f64; 2]) -> [f64; 2] {
    [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]]
}

fn then(outer: [f64; 6], inner: [f64; 6]) -> [f64; 6] {
    let [a, b, c, d, e, f] = outer;
    let [p, q, r, s, t, u] = inner;
    [a * p + c * q, b * p + d * q, a * r + c * s, b * r + d * s, a * t + c * u + e, b * t + d * u + f]
}

fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
}

/// Every path's points, mapped to document pixels through the scene's transforms.
fn document_points(nodes: &[Node], outer: [f64; 6], local: bool, out: &mut Vec<[f64; 2]>) {
    for node in nodes {
        match node {
            Node::Group { children, transform, .. } => {
                let next = if local { then(outer, *transform) } else { outer };
                document_points(children, next, local, out);
            }
            Node::Path(path) => {
                let m = then(outer, path.transform);
                for segment in &path.segments {
                    match segment {
                        Segment::MoveTo(p) | Segment::LineTo(p) => out.push(apply(m, *p)),
                        Segment::QuadTo(a, b) => out.extend([apply(m, *a), apply(m, *b)]),
                        Segment::CubicTo(a, b, c) => out.extend([apply(m, *a), apply(m, *b), apply(m, *c)]),
                        Segment::Close => {}
                    }
                }
            }
            Node::Image(_) => {}
        }
    }
}

fn paths(nodes: &[Node]) -> Vec<&Path> {
    let mut out = Vec::new();
    for node in nodes {
        match node {
            Node::Group { children, .. } => out.extend(paths(children)),
            Node::Path(path) => out.push(path),
            Node::Image(_) => {}
        }
    }
    out
}

fn group_ids(nodes: &[Node]) -> Vec<(String, [f64; 6])> {
    let mut out = Vec::new();
    for node in nodes {
        if let Node::Group { children, id, transform, .. } = node {
            out.push((id.clone(), *transform));
            out.extend(group_ids(children));
        }
    }
    out
}

const NESTED: &str = r##"<g id="outer" transform="translate(10 20) scale(2)">
    <g id="inner" transform="rotate(90)">
        <rect id="box" x="1" y="2" width="3" height="4" fill="#f00" stroke="#000" stroke-width="2" stroke-dasharray="1 3"/>
    </g>
    <path id="tri" transform="translate(5 0)" d="M0 0 L4 0 L4 4 Z" fill="#0f0"/>
</g>"##;

#[test]
fn a_local_scene_keeps_user_space_geometry_and_relative_transforms() {
    let tree = tree(NESTED);
    let local = Document::from_tree_local(&tree, 64);
    assert_eq!(local.coordinates, Coordinates::Local);
    let groups = group_ids(&local.nodes);
    let outer = groups.iter().find(|(id, _)| id == "outer").unwrap();
    assert!(close([outer.1[0], outer.1[3]], [2.0, 2.0]) && close([outer.1[4], outer.1[5]], [10.0, 20.0]), "{outer:?}");
    let inner = groups.iter().find(|(id, _)| id == "inner").unwrap();
    assert!(close([inner.1[0], inner.1[1]], [0.0, 1.0]) && close([inner.1[4], inner.1[5]], [0.0, 0.0]), "relative, not absolute: {inner:?}");

    let all = paths(&local.nodes);
    let rect = all.iter().find(|p| p.id == "box").unwrap();
    let Segment::MoveTo(start) = rect.segments[0] else { panic!("{:?}", rect.segments) };
    assert!(close(start, [1.0, 2.0]), "user space: {start:?}");
    let stroke = rect.stroke.as_ref().unwrap();
    assert_eq!(stroke.width, 2.0, "not scaled by the groups");
    assert_eq!(stroke.dash_array.as_deref(), Some([1.0, 3.0].as_slice()));
}

#[test]
fn an_absolute_scene_is_unchanged_by_the_local_option() {
    let tree = tree(NESTED);
    let absolute = Document::from_tree(&tree, 64);
    assert_eq!(absolute.coordinates, Coordinates::Absolute);
    let rect = paths(&absolute.nodes).into_iter().find(|p| p.id == "box").unwrap();
    assert_eq!(rect.transform, IDENTITY);
    // (1, 2) rotated 90° is (-2, 1), then scaled by 2 and moved by (10, 20).
    let Segment::MoveTo(start) = rect.segments[0] else { panic!("{:?}", rect.segments) };
    assert!(close(start, [6.0, 22.0]), "{start:?}");
    assert_eq!(rect.stroke.as_ref().unwrap().width, 4.0);
}

#[test]
fn local_and_absolute_scenes_put_every_point_in_the_same_place() {
    let tree = tree(NESTED);
    let (mut local, mut absolute) = (Vec::new(), Vec::new());
    document_points(&Document::from_tree_local(&tree, 64).nodes, IDENTITY, true, &mut local);
    document_points(&Document::from_tree(&tree, 64).nodes, IDENTITY, false, &mut absolute);
    assert_eq!(local.len(), absolute.len());
    assert!(!local.is_empty());
    for (l, a) in local.iter().zip(&absolute) {
        assert!(close(*l, *a), "{l:?} vs {a:?}");
    }
}

#[test]
fn fill_rule_and_ids_are_kept() {
    let tree =
        tree(r##"<path id="hole" fill-rule="evenodd" d="M0 0 H10 V10 H0 Z M2 2 H8 V8 H2 Z"/><path d="M0 0 H1 V1 Z" stroke="#000" fill="none"/>"##);
    for document in [Document::from_tree(&tree, 64), Document::from_tree_local(&tree, 64)] {
        let all = paths(&document.nodes);
        assert_eq!(all.len(), 2);
        assert_eq!((all[0].id.as_str(), all[0].fill_rule), ("hole", FillRule::EvenOdd));
        assert_eq!((all[1].id.as_str(), all[1].fill_rule), ("", FillRule::NonZero), "no fill: the default");
        assert!(all[1].fill.is_none());
    }
}

#[test]
fn a_local_scene_respects_the_depth_limit() {
    let deep = format!("{}<rect width=\"1\" height=\"1\"/>{}", "<g opacity=\"0.5\">".repeat(6), "</g>".repeat(6));
    let tree = tree(&deep);
    let shallow = Document::from_tree_local(&tree, 2);
    assert!(shallow.too_deep);
    assert!(paths(&shallow.nodes).is_empty());
    assert!(!Document::from_tree_local(&tree, 64).too_deep);
}

#[test]
fn an_embedded_svg_in_a_local_scene_is_local_too() {
    use std::io::Write as _;
    let inner = r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><g transform="scale(2)"><rect width="1" height="1"/></g></svg>"##;
    let mut b64 = Vec::new();
    {
        // A tiny base64 encoder keeps the test free of extra dependencies.
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        for chunk in inner.as_bytes().chunks(3) {
            let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
            for i in 0..4 {
                if i <= chunk.len() {
                    b64.push(A[((n >> (18 - 6 * i)) & 63) as usize]);
                } else {
                    b64.push(b'=');
                }
            }
        }
    }
    let mut body = Vec::new();
    write!(body, r#"<image x="10" y="10" width="4" height="4" href="data:image/svg+xml;base64,{}"/>"#, String::from_utf8(b64).unwrap()).unwrap();
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">{}</svg>"#, String::from_utf8(body).unwrap());
    let options = usvg::Options { image_href_resolver: crate::tree::image_resolver(Limits::default()), ..usvg::Options::default() };
    let tree = parse(svg.as_bytes(), &Limits::default(), &options).unwrap();
    let local = Document::from_tree_local(&tree, 64);
    let image = local
        .nodes
        .iter()
        .find_map(|n| match n {
            Node::Image(image) => Some(image),
            Node::Group { children, .. } => children.iter().find_map(|c| if let Node::Image(image) = c { Some(image) } else { None }),
            Node::Path(_) => None,
        })
        .unwrap();
    let nested = image.svg.as_ref().unwrap();
    assert_eq!(nested.coordinates, Coordinates::Local);
    let rect = paths(&nested.nodes)[0];
    let Segment::LineTo(p) = rect.segments[1] else { panic!("{:?}", rect.segments) };
    assert!(close(p, [1.0, 0.0]), "inside its own scaled group: {p:?}");
}
