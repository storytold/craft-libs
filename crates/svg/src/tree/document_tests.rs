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

// The embedded SVG reaches the scene only through the data-URL image resolver.
#[cfg(feature = "tree")]
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

fn fill_pattern(path: &Path) -> Option<&Pattern> {
    match path.fill.as_ref()? {
        Paint::Pattern { pattern, .. } => pattern.as_ref(),
        _ => None,
    }
}

const PATTERN: &str = r##"<defs><pattern id="dots" width="4" height="4" patternUnits="userSpaceOnUse" patternTransform="translate(1 2)">
    <rect width="2" height="2" fill="#f00"/>
</pattern></defs>
<g transform="scale(2)"><rect id="a" width="10" height="10" fill="url(#dots)"/><rect id="b" x="20" width="10" height="10" fill="url(#dots)" fill-opacity="0.5"/></g>"##;

#[test]
fn a_pattern_paint_carries_its_tile_and_content() {
    let tree = tree(PATTERN);
    let local = Document::from_tree_local(&tree, 64);
    let all = paths(&local.nodes);
    let a = all.iter().find(|p| p.id == "a").unwrap();
    let pattern = fill_pattern(a).unwrap();
    assert_eq!(pattern.id, "dots");
    assert_eq!(pattern.rect, [0.0, 0.0, 4.0, 4.0]);
    assert!(close([pattern.transform[4], pattern.transform[5]], [1.0, 2.0]) && close([pattern.transform[0], pattern.transform[3]], [1.0, 1.0]));
    let tile = paths(&pattern.content);
    assert_eq!(tile.len(), 1);
    assert_eq!(tile[0].fill, Some(Paint::Color { rgb: [255, 0, 0], opacity: 1.0 }));

    // Every use shares one converted tile, and keeps its own opacity.
    let b = all.iter().find(|p| p.id == "b").unwrap();
    assert!(std::sync::Arc::ptr_eq(&pattern.content, &fill_pattern(b).unwrap().content));
    assert!(matches!(b.fill, Some(Paint::Pattern { opacity, .. }) if (opacity - 0.5).abs() < 1e-6));
}

#[test]
fn an_absolute_scene_maps_the_tile_to_the_document() {
    let tree = tree(PATTERN);
    let absolute = Document::from_tree(&tree, 64);
    let a = paths(&absolute.nodes).into_iter().find(|p| p.id == "a").unwrap();
    let pattern = fill_pattern(a).unwrap();
    // translate(1 2), then the group's scale(2).
    assert!(close([pattern.transform[0], pattern.transform[3]], [2.0, 2.0]), "{:?}", pattern.transform);
    assert!(close([pattern.transform[4], pattern.transform[5]], [2.0, 4.0]), "{:?}", pattern.transform);
    // The tile's content stays in tile space.
    let Segment::LineTo(p) = paths(&pattern.content)[0].segments[1] else { panic!() };
    assert!(close(p, [2.0, 0.0]), "{p:?}");
}

#[test]
fn bounding_box_pattern_units_are_resolved() {
    let tree = tree(
        r##"<defs><pattern id="half" width=".5" height=".5"><rect width="1" height="1"/></pattern></defs><rect x="0" width="10" height="20" fill="url(#half)"/>"##,
    );
    let document = Document::from_tree_local(&tree, 64);
    let pattern = fill_pattern(paths(&document.nodes)[0]).unwrap();
    assert_eq!(pattern.rect, [0.0, 0.0, 5.0, 10.0]);
}

#[test]
fn nested_patterns_convert_within_the_depth_limit() {
    let svg = r##"<defs>
        <pattern id="inner" width="2" height="2" patternUnits="userSpaceOnUse"><rect width="1" height="1"/></pattern>
        <pattern id="outer" width="4" height="4" patternUnits="userSpaceOnUse"><rect width="4" height="4" fill="url(#inner)"/></pattern>
    </defs><rect width="8" height="8" fill="url(#outer)"/>"##;
    let tree = tree(svg);
    let deep = Document::from_tree_local(&tree, 64);
    let outer = fill_pattern(paths(&deep.nodes)[0]).unwrap();
    let inner = fill_pattern(paths(&outer.content)[0]).unwrap();
    assert_eq!(inner.id, "inner");
    assert!(!deep.too_deep);

    let shallow = Document::from_tree_local(&tree, 1);
    assert!(shallow.too_deep);
    let outer = fill_pattern(paths(&shallow.nodes)[0]).unwrap();
    assert!(fill_pattern(paths(&outer.content)[0]).is_none(), "past the limit: no content");
}

#[test]
fn a_pattern_that_refers_to_itself_ends() {
    let tree = tree(
        r##"<defs><pattern id="loop" width="2" height="2" patternUnits="userSpaceOnUse"><rect width="2" height="2" fill="url(#loop)"/></pattern></defs><rect width="4" height="4" fill="url(#loop)"/>"##,
    );
    let document = Document::from_tree_local(&tree, 64);
    // usvg may drop the reference; if it keeps it, the inner use has no content.
    for path in paths(&document.nodes) {
        if let Some(pattern) = fill_pattern(path) {
            for inner in paths(&pattern.content) {
                assert!(fill_pattern(inner).is_none());
            }
        }
    }
}

#[test]
fn group_effects_the_scene_leaves_out_are_counted() {
    let tree = tree(
        r##"<defs><filter id="f"><feGaussianBlur stdDeviation="2"/></filter><clipPath id="c"><rect width="5" height="5"/></clipPath>
        <mask id="m"><rect width="5" height="5" fill="#fff"/></mask></defs>
        <g filter="url(#f)"><rect width="10" height="10"/></g>
        <g clip-path="url(#c)"><rect width="10" height="10"/></g>
        <g clip-path="url(#c)" mask="url(#m)"><rect width="10" height="10"/></g>
        <g style="mix-blend-mode: multiply"><rect width="10" height="10"/></g>
        <g opacity="0.5"><rect width="10" height="10"/></g>"##,
    );
    for document in [Document::from_tree(&tree, 64), Document::from_tree_local(&tree, 64)] {
        assert_eq!(document.omitted, Omitted { filters: 1, clip_paths: 2, masks: 1, blends: 1 });
    }
    assert_eq!(Document::from_tree(&self::tree(r#"<rect width="1" height="1"/>"#), 64).omitted, Omitted::default());
}

#[test]
fn elements_usvg_leaves_out_are_listed() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:html="http://www.w3.org/1999/xhtml" width="10" height="10">
        <rect width="10" height="10"><animate attributeName="x" to="5" dur="1s"/><set attributeName="y" to="1"/></rect>
        <animateTransform attributeName="transform" type="rotate" to="90" dur="1s"/>
        <script>void 0</script><foreignObject width="5" height="5"><html:div/></foreignObject>
        <g><animate attributeName="opacity" to="0" dur="1s"/></g>
    </svg>"##;
    let checked = crate::tree::preflight(svg, &Limits::default()).unwrap();
    assert_eq!(
        checked.unsupported_elements(),
        vec![("animate", 2), ("animateTransform", 1), ("foreignObject", 1), ("script", 1), ("set", 1)],
        "the XHTML div is not an SVG element"
    );
    // The same checked XML still parses.
    assert!(checked.parse(&usvg::Options::default()).is_ok());
    let plain = crate::tree::preflight(r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="1" height="1"/></svg>"#, &Limits::default()).unwrap();
    assert!(plain.unsupported_elements().is_empty());
}

#[cfg(feature = "tree-text")]
#[test]
fn local_text_is_placed_and_bounded_in_its_parents_frame() {
    let mut options = usvg::Options::default();
    let fonts = options.fontdb_mut();
    fonts.load_font_data(include_bytes!("../../../ui/tests/fonts/NotoSans-test.ttf").to_vec());
    let family = fonts.faces().next().unwrap().families[0].0.clone();
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><g id="move" transform="translate(50 0)"><text id="t" x="5" y="20" font-family="{family}" font-size="10">Hi</text></g></svg>"#
    );
    let tree = parse(svg.as_bytes(), &Limits::default(), &options).unwrap();
    let local = Document::from_tree_local(&tree, 64);
    let absolute = Document::from_tree(&tree, 64);

    let groups = group_ids(&local.nodes);
    let text = groups.iter().find(|(id, _)| id == "t").unwrap();
    assert!(close([text.1[0], text.1[3]], [1.0, 1.0]) && close([text.1[4], text.1[5]], [0.0, 0.0]), "text sits in its group's frame: {text:?}");

    let bounds = |nodes: &[Node]| -> [f64; 4] {
        let Node::Group { children, .. } = &nodes[0] else { panic!() };
        let Node::Group { bounds, .. } = &children[0] else { panic!() };
        *bounds
    };
    let (l, a) = (bounds(&local.nodes), bounds(&absolute.nodes));
    assert!(close([l[0] + 50.0, l[1]], [a[0], a[1]]) && close([l[2], l[3]], [a[2], a[3]]), "local {l:?} vs absolute {a:?}");
    assert!(l[0] > 0.0 && l[0] < 10.0, "near x=5 in the group's frame: {l:?}");

    let (mut lp, mut ap) = (Vec::new(), Vec::new());
    document_points(&local.nodes, IDENTITY, true, &mut lp);
    document_points(&absolute.nodes, IDENTITY, false, &mut ap);
    assert!(!lp.is_empty() && lp.len() == ap.len());
    assert!(lp.iter().zip(&ap).all(|(l, a)| close(*l, *a)));
}
