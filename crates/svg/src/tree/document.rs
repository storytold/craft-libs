//! A format-neutral vector scene made from a bounded SVG tree.
//!
//! The scene preserves paths, groups, paints, transforms, opacity, strokes and image omissions
//! without depending on an application's document model. Coordinates and stroke widths use the
//! SVG tree's CSS-pixel coordinate system.

use super::usvg;

/// A parsed SVG scene. Text is represented by its outlined paths.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub width: f64,
    pub height: f64,
    pub nodes: Vec<Node>,
    /// Number of image nodes omitted from the vector scene.
    pub images: usize,
    /// True if nodes deeper than the requested depth were omitted.
    pub too_deep: bool,
}

/// A group or path in document order.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Group { children: Vec<Node>, opacity: f32 },
    Path(Path),
}

/// A transformed path with its resolved fill and stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub segments: Vec<Segment>,
    pub fill: Option<Paint>,
    pub stroke: Option<Stroke>,
}

/// A path segment in CSS pixels after its absolute transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    MoveTo([f64; 2]),
    LineTo([f64; 2]),
    QuadTo([f64; 2], [f64; 2]),
    CubicTo([f64; 2], [f64; 2], [f64; 2]),
    Close,
}

/// A resolved color or an unsupported SVG paint server.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Color { rgb: [u8; 3], opacity: f32 },
    Gradient { first_rgb: Option<[u8; 3]>, opacity: f32 },
    Pattern,
}

/// A resolved outline.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub paint: Paint,
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f64,
    pub dash_array: Option<Vec<f64>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

impl Document {
    /// Convert a bounded usvg tree to a scene, omitting nodes beyond `max_depth`.
    pub fn from_tree(tree: &usvg::Tree, max_depth: usize) -> Self {
        let size = tree.size();
        let mut document = Self { width: f64::from(size.width()), height: f64::from(size.height()), ..Self::default() };
        document.nodes = convert_group(tree.root(), 0, max_depth, &mut document);
        document
    }
}

fn color(paint: &usvg::Paint, opacity: f32) -> Paint {
    match paint {
        usvg::Paint::Color(c) => Paint::Color { rgb: [c.red, c.green, c.blue], opacity },
        usvg::Paint::LinearGradient(g) => {
            Paint::Gradient { first_rgb: g.stops().first().map(|s| [s.color().red, s.color().green, s.color().blue]), opacity }
        }
        usvg::Paint::RadialGradient(g) => {
            Paint::Gradient { first_rgb: g.stops().first().map(|s| [s.color().red, s.color().green, s.color().blue]), opacity }
        }
        usvg::Paint::Pattern(_) => Paint::Pattern,
    }
}

fn point(transform: usvg::Transform, p: usvg::tiny_skia_path::Point) -> [f64; 2] {
    let mut p = p;
    transform.map_point(&mut p);
    [f64::from(p.x), f64::from(p.y)]
}

fn convert_group(group: &usvg::Group, depth: usize, max_depth: usize, doc: &mut Document) -> Vec<Node> {
    let mut result = Vec::new();
    for node in group.children() {
        match node {
            usvg::Node::Path(path) => {
                if !path.is_visible() || path.data().is_empty() {
                    continue;
                }
                let transform = path.abs_transform();
                let segments = path
                    .data()
                    .segments()
                    .map(|segment| {
                        use usvg::tiny_skia_path::PathSegment as S;
                        match segment {
                            S::MoveTo(a) => Segment::MoveTo(point(transform, a)),
                            S::LineTo(a) => Segment::LineTo(point(transform, a)),
                            S::QuadTo(a, b) => Segment::QuadTo(point(transform, a), point(transform, b)),
                            S::CubicTo(a, b, c) => Segment::CubicTo(point(transform, a), point(transform, b), point(transform, c)),
                            S::Close => Segment::Close,
                        }
                    })
                    .collect();
                let fill = path.fill().map(|f| color(f.paint(), f.opacity().get()));
                let stroke = path.stroke().map(|s| {
                    let scale = f64::from((transform.sx * transform.sy - transform.kx * transform.ky).abs().sqrt());
                    Stroke {
                        paint: color(s.paint(), s.opacity().get()),
                        width: f64::from(s.width().get()) * scale,
                        cap: match s.linecap() {
                            usvg::LineCap::Butt => LineCap::Butt,
                            usvg::LineCap::Round => LineCap::Round,
                            usvg::LineCap::Square => LineCap::Square,
                        },
                        join: match s.linejoin() {
                            usvg::LineJoin::Round => LineJoin::Round,
                            usvg::LineJoin::Bevel => LineJoin::Bevel,
                            usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => LineJoin::Miter,
                        },
                        miter_limit: f64::from(s.miterlimit().get()),
                        dash_array: s.dasharray().map(|d| d.iter().map(|v| f64::from(*v) * scale).collect()),
                    }
                });
                result.push(Node::Path(Path { segments, fill, stroke }));
            }
            usvg::Node::Group(child) => {
                if depth >= max_depth {
                    doc.too_deep = true;
                    continue;
                }
                let children = convert_group(child, depth + 1, max_depth, doc);
                if !children.is_empty() {
                    result.push(Node::Group { children, opacity: child.opacity().get() });
                }
            }
            usvg::Node::Text(text) => {
                if depth >= max_depth {
                    doc.too_deep = true;
                    continue;
                }
                let children = convert_group(text.flattened(), depth + 1, max_depth, doc);
                if !children.is_empty() {
                    result.push(Node::Group { children, opacity: 1.0 });
                }
            }
            usvg::Node::Image(_) => doc.images += 1,
        }
    }
    result
}
