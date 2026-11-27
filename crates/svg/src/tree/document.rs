//! A format-neutral vector scene made from a bounded SVG tree.
//!
//! The scene preserves paths, groups, paints, transforms, opacity, strokes and image sources
//! without depending on an application's document model.
//!
//! [`Document::from_tree`] gives every coordinate and stroke width in the document's CSS pixels
//! ([`Coordinates::Absolute`]). [`Document::from_tree_local`] keeps each node in its own user
//! space with its transform relative to its parent ([`Coordinates::Local`]), for an app that
//! keeps a node's geometry editable apart from where it is placed.

#[cfg(feature = "tree-render")]
use super::draw::Refusal;
use super::usvg;

/// A parsed SVG scene. Text is represented by its outlined paths.
#[derive(Clone, Debug, Default)]
pub struct Document {
    pub width: f64,
    pub height: f64,
    pub nodes: Vec<Node>,
    /// Number of image nodes omitted from the vector scene.
    pub images: usize,
    /// True if nodes deeper than the requested depth were omitted.
    pub too_deep: bool,
    /// The space segments, transforms, bounds and stroke widths are given in.
    pub coordinates: Coordinates,
    /// Group effects the scene flags ([`GroupFeatures`]) but doesn't carry, counted.
    pub omitted: Omitted,
    source: Option<Box<usvg::Tree>>,
}

/// How many groups have an effect the scene doesn't carry: an app that draws only the scene
/// draws them without it, and can say so. Elements usvg itself leaves out (animation, scripts…)
/// are listed by [`super::Preflight::unsupported_elements`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Omitted {
    /// Groups with `filter`.
    pub filters: usize,
    /// Groups with `clip-path`.
    pub clip_paths: usize,
    /// Groups with `mask`.
    pub masks: usize,
    /// Groups with a `mix-blend-mode` other than normal.
    pub blends: usize,
}

/// Where a scene's geometry is given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Coordinates {
    /// Document CSS pixels: path segments, stroke widths and dashes are already transformed, a
    /// path's `transform` is the identity, and a group's or image's `transform` is its full
    /// transform to the document.
    #[default]
    Absolute,
    /// Each node in its own user space: path segments, stroke widths, dashes, gradients and
    /// bounds are as the SVG gives them, and every `transform` maps the node into its parent
    /// (the document for top-level nodes).
    Local,
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.nodes == other.nodes
            && self.images == other.images
            && self.too_deep == other.too_deep
            && self.coordinates == other.coordinates
            && self.omitted == other.omitted
    }
}

/// Stable node indices from the root of the source usvg tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodePath(Vec<usize>);

impl NodePath {
    pub fn indices(&self) -> &[usize] {
        &self.0
    }
}

/// A group or path in document order.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Group {
        children: Vec<Node>,
        opacity: f32,
        transform: [f64; 6],
        bounds: [f64; 4],
        features: GroupFeatures,
        source_path: NodePath,
        /// The element's `id`, empty when it has none.
        id: String,
    },
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
    pub source_path: NodePath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Jpeg,
    Png,
    Gif,
    WebP,
    Svg,
}

/// A path with its resolved fill and stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub segments: Vec<Segment>,
    pub fill: Option<Paint>,
    /// How the fill decides what is inside (`fill-rule`); `NonZero` when there is no fill.
    pub fill_rule: FillRule,
    pub stroke: Option<Stroke>,
    pub source_path: NodePath,
    /// The element's `id`, empty when it has none.
    pub id: String,
    /// Maps the segments into the parent ([`Coordinates`]); the identity in absolute scenes.
    pub transform: [f64; 6],
}

/// The `fill-rule` property.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

impl Path {
    /// The basic shape (rectangle, rounded rectangle or ellipse) this path draws, if any
    /// ([`super::primitive::recognize`]).
    pub fn primitive(&self) -> Option<super::primitive::Primitive> {
        super::primitive::recognize(&self.segments)
    }
}

/// A path segment, in the space the scene's [`Coordinates`] name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    MoveTo([f64; 2]),
    LineTo([f64; 2]),
    QuadTo([f64; 2], [f64; 2]),
    CubicTo([f64; 2], [f64; 2], [f64; 2]),
    Close,
}

/// A resolved color, gradient or pattern.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Color {
        rgb: [u8; 3],
        opacity: f32,
    },
    Gradient {
        kind: GradientKind,
        stops: Vec<GradientStop>,
        transform: [f64; 6],
        spread: Spread,
        opacity: f32,
    },
    /// A `<pattern>`; `None` when its content is past the depth limit or refers back to the
    /// pattern itself.
    Pattern {
        pattern: Option<Pattern>,
        opacity: f32,
    },
}

/// A resolved `<pattern>` tile. usvg has already applied `patternUnits`,
/// `patternContentUnits` and `viewBox`, so the tile and its content share one space.
#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    /// The element's `id`.
    pub id: String,
    /// The tile `[x, y, width, height]` in tile space.
    pub rect: [f64; 4],
    /// Maps tile space into the painted path's space: `patternTransform` in a local scene; in
    /// an absolute scene, `patternTransform` followed by the path's transform to the document.
    pub transform: [f64; 6],
    /// What one tile draws, in tile space ([`Coordinates::Local`] whatever the scene's
    /// coordinates). Shared by every use of the same pattern.
    pub content: std::sync::Arc<Vec<Node>>,
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
    pub dash_offset: f64,
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
    /// Convert a bounded usvg tree to a scene in document pixels ([`Coordinates::Absolute`]),
    /// omitting nodes beyond `max_depth`.
    pub fn from_tree(tree: &usvg::Tree, max_depth: usize) -> Self {
        Self::convert(tree, max_depth, Coordinates::Absolute)
    }

    /// Convert a bounded usvg tree to a scene that keeps every node in its own user space with a
    /// transform relative to its parent ([`Coordinates::Local`]), omitting nodes beyond
    /// `max_depth`.
    pub fn from_tree_local(tree: &usvg::Tree, max_depth: usize) -> Self {
        Self::convert(tree, max_depth, Coordinates::Local)
    }

    /// Build a scene in document pixels that retains its source tree for fallback rendering.
    pub fn from_owned_tree(tree: usvg::Tree, max_depth: usize) -> Self {
        let mut document = Self::convert(&tree, max_depth, Coordinates::Absolute);
        document.source = Some(Box::new(tree));
        document
    }

    fn convert(tree: &usvg::Tree, max_depth: usize, coordinates: Coordinates) -> Self {
        let size = tree.size();
        let mut document = Self { width: f64::from(size.width()), height: f64::from(size.height()), coordinates, ..Self::default() };
        let mut cx = Convert { doc: &mut document, max_depth, local: coordinates == Coordinates::Local, patterns: std::collections::HashMap::new() };
        let nodes = cx.group(tree.root(), tree.root().abs_transform(), 0, &[]);
        document.nodes = nodes;
        document
    }

    /// Draw a source subtree named by a [`NodePath`] without exposing usvg traversal to callers.
    #[cfg(feature = "tree-render")]
    pub fn draw_subtree(
        &self,
        path: &NodePath,
        transform: usvg::Transform,
        target: &mut super::draw::tiny_skia::Pixmap,
        held_bytes: u64,
        budget_bytes: u64,
    ) -> Result<bool, Refusal> {
        let tree = self.source.as_deref().ok_or_else(|| Refusal {
            complex: true,
            memory: false,
            message: "the SVG scene does not retain its source tree".into(),
        })?;
        let mut group = tree.root();
        let mut node = None;
        for (i, index) in path.0.iter().enumerate() {
            node = Some(group.children().get(*index).ok_or_else(|| Refusal {
                complex: true,
                memory: false,
                message: "the SVG subtree path is no longer valid".into(),
            })?);
            if i + 1 < path.0.len() {
                group = match node {
                    Some(usvg::Node::Group(g)) => g,
                    Some(usvg::Node::Text(t)) => t.flattened(),
                    _ => return Err(Refusal { complex: true, memory: false, message: "the SVG subtree path does not name a group".into() }),
                };
            }
        }
        let node = node.ok_or_else(|| Refusal { complex: true, memory: false, message: "the SVG subtree path is empty".into() })?;
        super::draw::render_node(node, transform, target, held_bytes, budget_bytes)
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

fn color(paint: &usvg::Paint, opacity: f32, pattern: impl FnOnce(&std::sync::Arc<usvg::Pattern>) -> Option<Pattern>) -> Paint {
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
        usvg::Paint::Pattern(source) => Paint::Pattern { pattern: pattern(source), opacity },
    }
}

fn point(transform: usvg::Transform, p: usvg::tiny_skia_path::Point) -> [f64; 2] {
    let mut p = p;
    transform.map_point(&mut p);
    [f64::from(p.x), f64::from(p.y)]
}

const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// A node's transform relative to `parent` (both absolute): `parent⁻¹ · node`. The identity
/// when `parent` cannot be inverted (a degenerate group draws nothing anyway).
fn relative(parent: usvg::Transform, node: usvg::Transform) -> usvg::Transform {
    parent.invert().map_or(usvg::Transform::identity(), |inverse| inverse.pre_concat(node))
}

struct Convert<'d> {
    doc: &'d mut Document,
    max_depth: usize,
    local: bool,
    /// Pattern content already converted, by the address of usvg's shared pattern; `None` while
    /// a pattern is being converted, so a pattern that refers back to itself gets no content.
    patterns: std::collections::HashMap<usize, Option<std::sync::Arc<Vec<Node>>>>,
}

impl Convert<'_> {
    /// The transform a node with absolute transform `node` gets under a group whose absolute
    /// transform is `parent`.
    fn placement(&self, parent: usvg::Transform, node: usvg::Transform) -> [f64; 6] {
        if self.local { transform(relative(parent, node)) } else { transform(node) }
    }

    /// `parent` is the absolute transform `group`'s leaves are placed against: its own, except
    /// for flattened text, whose outline paths carry the text's while the group has none.
    fn group(&mut self, group: &usvg::Group, parent: usvg::Transform, depth: usize, parent_path: &[usize]) -> Vec<Node> {
        let mut result = Vec::new();
        for (index, node) in group.children().iter().enumerate() {
            let mut source_path = parent_path.to_vec();
            source_path.push(index);
            match node {
                usvg::Node::Path(path) => {
                    if let Some(path) = self.path(path, parent, source_path, depth) {
                        result.push(Node::Path(Box::new(path)));
                    }
                }
                usvg::Node::Group(child) => {
                    if depth >= self.max_depth {
                        self.doc.too_deep = true;
                        continue;
                    }
                    let children = self.group(child, child.abs_transform(), depth + 1, &source_path);
                    if !children.is_empty() {
                        let omitted = &mut self.doc.omitted;
                        omitted.filters += usize::from(!child.filters().is_empty());
                        omitted.clip_paths += usize::from(child.clip_path().is_some());
                        omitted.masks += usize::from(child.mask().is_some());
                        omitted.blends += usize::from(child.blend_mode() != usvg::BlendMode::Normal);
                        let bounds = if self.local { child.layer_bounding_box().to_rect() } else { child.abs_layer_bounding_box().to_rect() };
                        result.push(Node::Group {
                            children,
                            opacity: child.opacity().get(),
                            // Locally, the group's own transform: usvg leaves the absolute one at
                            // the identity for colour and bitmap glyph groups in text.
                            transform: if self.local { transform(child.transform()) } else { transform(child.abs_transform()) },
                            bounds: rect(bounds),
                            features: GroupFeatures {
                                filters: !child.filters().is_empty(),
                                clip_path: child.clip_path().is_some(),
                                mask: child.mask().is_some(),
                                non_normal_blend: child.blend_mode() != usvg::BlendMode::Normal,
                                isolation: child.should_isolate(),
                            },
                            source_path: NodePath(source_path),
                            id: child.id().to_string(),
                        });
                    }
                }
                usvg::Node::Text(text) => {
                    if depth >= self.max_depth {
                        self.doc.too_deep = true;
                        continue;
                    }
                    let flat = text.flattened();
                    // The flattened group has no transform of its own, but its outline paths
                    // have the text's: place its leaves, and the wrapper, in the text's frame.
                    let frame = if self.local { text.abs_transform() } else { flat.abs_transform() };
                    let children = self.group(flat, frame, depth + 1, &source_path);
                    if !children.is_empty() {
                        let bounds = if self.local { text.bounding_box() } else { text.abs_bounding_box() };
                        result.push(Node::Group {
                            children,
                            opacity: 1.0,
                            transform: self.placement(parent, frame),
                            bounds: rect(bounds),
                            features: GroupFeatures::default(),
                            source_path: NodePath(source_path),
                            id: text.id().to_string(),
                        });
                    }
                }
                usvg::Node::Image(image) => {
                    self.doc.images += 1;
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
                        usvg::ImageKind::SVG(tree) => {
                            let max_depth = self.max_depth.saturating_sub(depth + 1);
                            let nested = if self.local {
                                Document::from_tree_local(tree, max_depth)
                            } else {
                                Document::from_owned_tree(tree.clone(), max_depth)
                            };
                            (None, Some(Box::new(nested)))
                        }
                    };
                    let bounds = if self.local { image.bounding_box() } else { image.abs_bounding_box() };
                    result.push(Node::Image(Image {
                        kind,
                        visible: image.is_visible(),
                        transform: self.placement(parent, image.abs_transform()),
                        bounds: rect(bounds),
                        encoded,
                        svg,
                        source_path: NodePath(source_path),
                    }));
                }
            }
        }
        result
    }

    /// A visible, non-empty path. Its own absolute transform is its group's in usvg, so in a
    /// local scene its segments stay as they are and its transform is normally the identity.
    /// The tile of `pattern` painted on a path at `depth` whose own transform to the document is
    /// `abs`.
    fn pattern(&mut self, pattern: &std::sync::Arc<usvg::Pattern>, abs: usvg::Transform, depth: usize) -> Option<Pattern> {
        let key = std::sync::Arc::as_ptr(pattern) as usize;
        let content = match self.patterns.get(&key) {
            Some(done) => done.clone(),
            None if depth >= self.max_depth => {
                self.doc.too_deep = true;
                None
            }
            None => {
                self.patterns.insert(key, None);
                let local = std::mem::replace(&mut self.local, true);
                let nodes = self.group(pattern.root(), pattern.root().abs_transform(), depth + 1, &[]);
                self.local = local;
                let content = Some(std::sync::Arc::new(nodes));
                self.patterns.insert(key, content.clone());
                content
            }
        }?;
        let placement = if self.local { pattern.transform() } else { abs.pre_concat(pattern.transform()) };
        Some(Pattern { id: pattern.id().to_string(), rect: rect(pattern.rect().to_rect()), transform: transform(placement), content })
    }

    fn path(&mut self, path: &usvg::Path, parent: usvg::Transform, source_path: Vec<usize>, depth: usize) -> Option<Path> {
        if !path.is_visible() || path.data().is_empty() {
            return None;
        }
        let abs = path.abs_transform();
        let (to_points, scale, placement) = if self.local {
            (usvg::Transform::identity(), 1.0, transform(relative(parent, abs)))
        } else {
            (abs, f64::from((abs.sx * abs.sy - abs.kx * abs.ky).abs().sqrt()), IDENTITY)
        };
        let segments = path
            .data()
            .segments()
            .map(|segment| {
                use usvg::tiny_skia_path::PathSegment as S;
                match segment {
                    S::MoveTo(a) => Segment::MoveTo(point(to_points, a)),
                    S::LineTo(a) => Segment::LineTo(point(to_points, a)),
                    S::QuadTo(a, b) => Segment::QuadTo(point(to_points, a), point(to_points, b)),
                    S::CubicTo(a, b, c) => Segment::CubicTo(point(to_points, a), point(to_points, b), point(to_points, c)),
                    S::Close => Segment::Close,
                }
            })
            .collect();
        let fill = path.fill().map(|f| color(f.paint(), f.opacity().get(), |p| self.pattern(p, abs, depth)));
        let fill_rule = match path.fill().map(usvg::Fill::rule) {
            Some(usvg::FillRule::EvenOdd) => FillRule::EvenOdd,
            _ => FillRule::NonZero,
        };
        let stroke = path.stroke().map(|s| Stroke {
            paint: color(s.paint(), s.opacity().get(), |p| self.pattern(p, abs, depth)),
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
            dash_offset: f64::from(s.dashoffset()) * scale,
        });
        Some(Path { segments, fill, fill_rule, stroke, source_path: NodePath(source_path), id: path.id().to_string(), transform: placement })
    }
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;
