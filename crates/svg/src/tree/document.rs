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
    Group { children: Vec<Node>, opacity: f32, transform: [f64; 6], bounds: [f64; 4], features: GroupFeatures },
    Path(Box<Path>),
    Image(Image),
}

/// Features that need a renderer capable of SVG compositing or external resources.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GroupFeatures {
    pub filters: bool,
    pub clip_path: bool,
    pub mask: bool,
    pub non_normal_blend: bool,
    pub isolation: bool,
}

/// An embedded image node and its bounded source data/placement.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub kind: ImageKind,
    pub visible: bool,
    pub transform: [f64; 6],
    pub bounds: [f64; 4],
    pub encoded: Option<std::sync::Arc<Vec<u8>>>,
    pub svg: Option<Box<Document>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Jpeg,
    Png,
    Gif,
    WebP,
    Svg,
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
    Gradient { kind: GradientKind, stops: Vec<GradientStop>, transform: [f64; 6], spread: Spread, opacity: f32 },
    Pattern,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GradientKind {
    Linear { start: [f64; 2], end: [f64; 2] },
    Radial { center: [f64; 2], radius: f64, focus: [f64; 2], focus_radius: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    pub offset: f64,
    pub rgb: [u8; 3],
    pub opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spread {
    Pad,
    Reflect,
    Repeat,
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

fn transform(t: usvg::Transform) -> [f64; 6] {
    [f64::from(t.sx), f64::from(t.ky), f64::from(t.kx), f64::from(t.sy), f64::from(t.tx), f64::from(t.ty)]
}

fn rect(r: usvg::Rect) -> [f64; 4] {
    [f64::from(r.x()), f64::from(r.y()), f64::from(r.width()), f64::from(r.height())]
}

fn spread(s: usvg::SpreadMethod) -> Spread {
    match s {
        usvg::SpreadMethod::Pad => Spread::Pad,
        usvg::SpreadMethod::Reflect => Spread::Reflect,
        usvg::SpreadMethod::Repeat => Spread::Repeat,
    }
}

fn color(paint: &usvg::Paint, opacity: f32) -> Paint {
    match paint {
        usvg::Paint::Color(c) => Paint::Color { rgb: [c.red, c.green, c.blue], opacity },
        usvg::Paint::LinearGradient(g) => Paint::Gradient {
            kind: GradientKind::Linear { start: [f64::from(g.x1()), f64::from(g.y1())], end: [f64::from(g.x2()), f64::from(g.y2())] },
            stops: g
                .stops()
                .iter()
                .map(|s| GradientStop {
                    offset: f64::from(s.offset().get()),
                    rgb: [s.color().red, s.color().green, s.color().blue],
                    opacity: s.opacity().get(),
                })
                .collect(),
            transform: transform(g.transform()),
            spread: spread(g.spread_method()),
            opacity,
        },
        usvg::Paint::RadialGradient(g) => Paint::Gradient {
            kind: GradientKind::Radial {
                center: [f64::from(g.cx()), f64::from(g.cy())],
                radius: f64::from(g.r().get()),
                focus: [f64::from(g.fx()), f64::from(g.fy())],
                focus_radius: f64::from(g.fr().get()),
            },
            stops: g
                .stops()
                .iter()
                .map(|s| GradientStop {
                    offset: f64::from(s.offset().get()),
                    rgb: [s.color().red, s.color().green, s.color().blue],
                    opacity: s.opacity().get(),
                })
                .collect(),
            transform: transform(g.transform()),
            spread: spread(g.spread_method()),
            opacity,
        },
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
                result.push(Node::Path(Box::new(Path { segments, fill, stroke })));
            }
            usvg::Node::Group(child) => {
                if depth >= max_depth {
                    doc.too_deep = true;
                    continue;
                }
                let children = convert_group(child, depth + 1, max_depth, doc);
                if !children.is_empty() {
                    result.push(Node::Group {
                        children,
                        opacity: child.opacity().get(),
                        transform: transform(child.abs_transform()),
                        bounds: rect(child.abs_layer_bounding_box().to_rect()),
                        features: GroupFeatures {
                            filters: !child.filters().is_empty(),
                            clip_path: child.clip_path().is_some(),
                            mask: child.mask().is_some(),
                            non_normal_blend: child.blend_mode() != usvg::BlendMode::Normal,
                            isolation: child.should_isolate(),
                        },
                    });
                }
            }
            usvg::Node::Text(text) => {
                if depth >= max_depth {
                    doc.too_deep = true;
                    continue;
                }
                let children = convert_group(text.flattened(), depth + 1, max_depth, doc);
                if !children.is_empty() {
                    result.push(Node::Group {
                        children,
                        opacity: 1.0,
                        transform: transform(text.abs_transform()),
                        bounds: rect(text.abs_bounding_box()),
                        features: GroupFeatures::default(),
                    });
                }
            }
            usvg::Node::Image(image) => {
                doc.images += 1;
                let kind = match image.kind() {
                    usvg::ImageKind::JPEG(_) => ImageKind::Jpeg,
                    usvg::ImageKind::PNG(_) => ImageKind::Png,
                    usvg::ImageKind::GIF(_) => ImageKind::Gif,
                    usvg::ImageKind::WEBP(_) => ImageKind::WebP,
                    usvg::ImageKind::SVG(_) => ImageKind::Svg,
                };
                let (encoded, svg) = match image.kind() {
                    usvg::ImageKind::JPEG(data) | usvg::ImageKind::PNG(data) | usvg::ImageKind::GIF(data) | usvg::ImageKind::WEBP(data) => {
                        (Some(data.clone()), None)
                    }
                    usvg::ImageKind::SVG(tree) => (None, Some(Box::new(Document::from_tree(tree, max_depth.saturating_sub(depth + 1))))),
                };
                result.push(Node::Image(Image {
                    kind,
                    visible: image.is_visible(),
                    transform: transform(image.abs_transform()),
                    bounds: rect(image.abs_bounding_box()),
                    encoded,
                    svg,
                }));
            }
        }
    }
    result
}
