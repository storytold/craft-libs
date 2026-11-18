use super::read::MAX_TREE_DEPTH;
use super::{Affine, Kind, Node, ObjId, Point, Rect, Stream, Tag};

/// The bounds of what visible `nodes` draw (control points of outlines, corners of images, frames
/// and tables), in document pixels.
pub fn nodes_bounds(nodes: &[Node]) -> Option<Rect> {
    let mut out = None;
    walk_bounds(nodes, true, 0, &mut out);
    out
}

/// The bounds of `node` and everything in it, hidden or not.
pub(super) fn tree_bounds(node: &Node) -> Option<Rect> {
    let mut out = None;
    walk_bounds(std::slice::from_ref(node), false, 0, &mut out);
    out
}

fn walk_bounds(nodes: &[Node], visible_only: bool, depth: usize, out: &mut Option<Rect>) {
    if depth > MAX_TREE_DEPTH {
        return;
    }
    for n in nodes.iter().filter(|n| n.visible || !visible_only) {
        kind_points(&n.kind, &mut |p| add_point(out, p));
        walk_bounds(&n.children, visible_only, depth + 1, out);
    }
}

/// What a pixel mask over a node of `kind` with `children` is read over: the bounds of what they
/// draw, strokes included, or `None` (all of the mask) when some of it has no bounds known here:
/// artistic text, whose glyphs only the layout places.
pub(super) fn mask_extent(kind: &Kind, children: &[Node]) -> Option<Rect> {
    fn visit(kind: &Kind, children: &[Node], depth: usize, out: &mut Option<Rect>) -> bool {
        if depth > MAX_TREE_DEPTH || matches!(kind, Kind::Text(t) if t.frame.is_none()) {
            return false;
        }
        let mut own = kind_bounds(kind);
        if let (Some(r), Kind::Shape { strokes, .. } | Kind::Artboard { strokes, .. }) = (own, kind) {
            // As far as a stroke can reach out of its path, miter joins included.
            let reach = strokes.iter().map(|s| s.width * s.miter_limit.clamp(1.0, 10.0)).filter(|w| w.is_finite()).fold(0.0, f64::max);
            own = Some(Rect { x0: r.x0 - reach, y0: r.y0 - reach, x1: r.x1 + reach, y1: r.y1 + reach });
        }
        if let Some(r) = own {
            *out = Some(out.map_or(r, |o| union(o, r)));
        }
        children.iter().filter(|n| n.visible).all(|n| visit(&n.kind, &n.children, depth + 1, out))
    }
    let mut out = None;
    if visit(kind, children, 0, &mut out) { out } else { None }
}

/// The bounds of what one node's own kind draws (not its children): its outline's points, its
/// image's or frame's corners, or artistic text's anchor.
pub fn kind_bounds(kind: &Kind) -> Option<Rect> {
    let mut out = None;
    kind_points(kind, &mut |p| add_point(&mut out, p));
    out
}

fn add_point(out: &mut Option<Rect>, p: Point) {
    if p.x.is_finite() && p.y.is_finite() {
        let r = Rect { x0: p.x, y0: p.y, x1: p.x, y1: p.y };
        *out = Some(out.map_or(r, |o| union(o, r)));
    }
}

fn kind_points(kind: &Kind, add: &mut dyn FnMut(Point)) {
    let corners = |r: Rect, m: Affine| [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)].map(|(x, y)| m.apply(Point { x, y }));
    match kind {
        Kind::Shape { path, .. } | Kind::Artboard { path, .. } => {
            for sp in &path.subpaths {
                add(sp.start);
                for seg in &sp.segments {
                    seg.iter().for_each(|p| add(*p));
                }
            }
        }
        Kind::Image(img) => {
            corners(Rect { x0: 0.0, y0: 0.0, x1: f64::from(img.width), y1: f64::from(img.height) }, img.transform).into_iter().for_each(add)
        }
        Kind::Text(t) => match t.frame {
            Some(f) => corners(f, t.transform).into_iter().for_each(add),
            None => add(t.transform.apply(t.anchor)),
        },
        Kind::Table(t) => {
            if let (Some(&x0), Some(&x1), Some(&y0), Some(&y1)) = (t.columns.first(), t.columns.last(), t.rows.first(), t.rows.last()) {
                corners(Rect { x0, y0, x1, y1 }, t.transform).into_iter().for_each(add);
            }
        }
        Kind::Layer | Kind::Group | Kind::Unsupported | Kind::MasterInstance { .. } => {}
    }
}

/// Page rectangles: `SpMd.PagR[].rctp` (Publisher 2, Affinity 3) or `SprB` split into `PagC`
/// equal pages (Publisher 1 facing spreads).
pub(super) fn pages(s: &Stream, spread: ObjId) -> Vec<Rect> {
    if let Some(md) = s.obj(spread, b"SpMd") {
        let v: Vec<Rect> =
            s.objs(md, b"PagR").into_iter().filter_map(|p| s.floats::<4>(p, b"rctp").map(rect)).filter(|r| r.x1 > r.x0 && r.y1 > r.y0).collect();
        if !v.is_empty() {
            return v;
        }
    }
    match (s.floats::<4>(spread, b"SprB").map(rect), s.int(spread, b"PagC")) {
        (Some(b), Some(n @ 2..=16)) => {
            let w = (b.x1 - b.x0) / n as f64;
            (0..n).map(|i| Rect { x0: b.x0 + w * i as f64, x1: b.x0 + w * (i + 1) as f64, ..b }).collect()
        }
        _ => Vec::new(),
    }
}

pub(crate) fn union(a: Rect, b: Rect) -> Rect {
    Rect { x0: a.x0.min(b.x0), y0: a.y0.min(b.y0), x1: a.x1.max(b.x1), y1: a.y1.max(b.y1) }
}

pub(super) fn is_container(class: Tag) -> bool {
    class == Tag::of(b"Scop") || class == Tag::of(b"Grup")
}

pub(crate) fn rect(v: [f64; 4]) -> Rect {
    Rect { x0: v[0], y0: v[1], x1: v[2], y1: v[3] }
}

/// A readable name for a layer class in warnings.
pub(crate) fn describe(class: Tag) -> &'static str {
    match &class.0.to_be_bytes() {
        b"PCrv" => "curve",
        b"ShpN" | b"SNEN" | b"SNRR" | b"ShRN" => "shape",
        b"Comp" => "compound",
        b"Rstr" => "pixel",
        b"ImgN" => "image",
        b"EmbN" => "embedded document",
        b"MPIN" => "master page instance",
        b"FRst" => "fill",
        b"MRst" => "mask",
        b"Slic" | b"SlcP" => "slice",
        b"3DLA" => "3D",
        [_, _, b'R', b'A'] => "adjustment",
        _ => "unknown",
    }
}
