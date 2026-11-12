//! Hard caps on the memory resvg needs to draw a subtree of a [`usvg::Tree`], checked before it
//! draws.
//!
//! A cap on the target pixmap bounds the target only. While it draws, resvg also allocates, and
//! a failed allocation aborts the process rather than returning an error:
//! - a layer for every isolated group (opacity, blend mode, clip, mask, filter), clamped to 5 × 5
//!   times the target, and one more per link of a clip-path chain, per clip child that has its own
//!   clip-path, and per link of a mask-on-mask chain;
//! - for a filter, a result per primitive (all kept until the filter ends) and a clone of the layer.
//!   The filter region is NOT clamped: `x`, `y`, `width` and `height` in the file set the size;
//! - a pattern tile of `width × height × scale`;
//! - for an embedded SVG image or an `feImage`, a pixmap of the current layer or filter region with
//!   a nested drawing inside.
//!
//! The checks are worst-case products, not a model of resvg's phases. A node is checked against
//! the layers of its ancestors (which are alive while it is drawn) and nothing else, and each
//! shared definition is walked once per size class, so the walk is linear in the file.
//!
//! The allocation counts were read from resvg 0.45.1 (PhotoCraft's `svg/limits.rs`, where this
//! comes from). This crate does not depend on resvg; a caller that draws with another version
//! should treat the byte figures as an estimate with the same shape.

use std::collections::HashMap;

use usvg::Transform;

/// Filter primitives per filter. A result of every primitive is kept; real filters have 1 to 12
/// (Inkscape's drop shadow has 5), and 64 leaves headroom for generated ones.
const MAX_PRIMITIVES: usize = 64;
/// Links of one clip-path chain, mask-on-mask chain or nested clip children with their own
/// clip-path. Each link keeps a layer-sized buffer alive while the next is applied; real files use
/// one or two, and 16 layers of a 24 MP canvas (1.5 GiB) is the whole budget anyway.
const MAX_LINKS: usize = 16;
/// Patterns drawn inside patterns. Each tile is alive while the inner one is drawn.
const MAX_PATTERN_DEPTH: usize = 8;
/// Embedded SVG images inside embedded SVG images. Each draws on a pixmap of the enclosing
/// layer, which the live-bytes check charges, so deep chains of small images are harmless and
/// chains of big ones stop at the budget long before this.
const MAX_IMAGE_DEPTH: usize = 32;
/// Nested isolated groups, patterns, embedded images and filter images followed. It keeps this
/// walk, and resvg's own recursion, far from a stack overflow; real drawings stay under a few dozen.
const MAX_DEPTH: usize = 200;
/// Nested groups of any kind. usvg expands `<use>` chains up to its own 1024-level limit, possibly
/// two groups per level; resvg recurses three frames per level too, so a walk that stops here is
/// lighter on the stack than the drawing it guards.
const MAX_NEST: usize = 2_100;
/// Nodes the walk visits before it gives up as too complex to check. A node costs a few
/// nanoseconds, so this bounds the walk to well under a second.
const MAX_NODES: usize = 2_000_000;

/// Why a drawing should not be drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// Too many elements to check, rather than too much memory.
    pub complex: bool,
    /// Over the memory budget, as opposed to a nesting, chain or element-count cap.
    pub memory: bool,
    pub message: String,
}

fn mib(bytes: f64) -> String {
    if bytes < 1e15 { format!("about {} MiB", (bytes / 1_048_576.0).ceil()) } else { "an enormous amount of memory".to_string() }
}

fn memory(what: &str, bytes: f64, budget: f64) -> Refusal {
    let limit = budget as u64 >> 20;
    Refusal { complex: false, memory: true, message: format!("{what} needs {}, over the {limit} MiB limit", mib(bytes)) }
}

fn limit(what: &str, value: usize) -> Refusal {
    Refusal { complex: false, memory: false, message: format!("{what} (limit {value})") }
}

/// A pixel count that is never NaN: unknown sizes count as unbounded.
pub fn px(v: f64) -> f64 {
    if v.is_nan() { f64::INFINITY } else { v }
}

/// What is alive where a node is drawn.
#[derive(Clone, Copy)]
struct Cx {
    /// Bytes held by the target and the layers of the ancestors.
    live: f64,
    /// Pixels of the pixmap being drawn on.
    layer: f64,
    /// Pixels of the largest group layer resvg makes in it (5 × 5 times its first target).
    cap: f64,
    /// Nested layers (isolated groups, patterns, embedded images, filter images).
    depth: usize,
    patterns: usize,
    images: usize,
    /// Nested groups of any kind: resvg and this walk recurse once per level.
    nest: usize,
}

/// What a subtree needs, relative to what is alive where it starts. A shared definition is walked
/// once per transform and size, and every later reference re-checks its profile against where it
/// is used, so nothing is skipped and the walk stays linear.
#[derive(Clone, Copy, Default)]
struct Profile {
    /// Most bytes it holds beyond the incoming live bytes.
    extra: f64,
    /// Most extra levels of nested layers, patterns, embedded images and groups below it.
    depth: usize,
    patterns: usize,
    images: usize,
    nest: usize,
}

impl Profile {
    fn max(self, o: Self) -> Self {
        Self {
            extra: self.extra.max(o.extra),
            depth: self.depth.max(o.depth),
            patterns: self.patterns.max(o.patterns),
            images: self.images.max(o.images),
            nest: self.nest.max(o.nest),
        }
    }

    /// This profile seen from a parent that adds `bytes` and one level of each kind in `levels`
    /// (layers, patterns, images, groups).
    fn under(self, bytes: f64, levels: [usize; 4]) -> Self {
        Self {
            extra: self.extra + bytes,
            depth: self.depth + levels[0],
            patterns: self.patterns + levels[1],
            images: self.images + levels[2],
            nest: self.nest + levels[3],
        }
    }
}

/// A shared definition at one linear transform, layer size and clamp.
type Key = (usize, [u32; 4], u64, u64);

fn key(ptr: usize, ts: Transform, cx: Cx) -> Key {
    (ptr, [ts.sx.to_bits(), ts.kx.to_bits(), ts.ky.to_bits(), ts.sy.to_bits()], cx.layer.to_bits(), cx.cap.to_bits())
}

struct Walk {
    /// Bytes of RGBA8 buffers alive at once that the drawing may need.
    budget: f64,
    visited: usize,
    shared: HashMap<Key, Profile>,
    clips: HashMap<usize, usize>,
    masks: HashMap<usize, usize>,
}

/// Check that drawing `nodes` at `ts` onto a `width × height` pixmap keeps the RGBA8 buffers alive
/// at once inside `budget` bytes, given `held` bytes already in use (anything the caller keeps;
/// the target itself is counted here).
pub fn check(nodes: &[usvg::Node], ts: Transform, width: u32, height: u32, held: u64, budget: u64) -> Result<(), Refusal> {
    let target = f64::from(width) * f64::from(height);
    let cx = Cx { live: held as f64 + target * 4.0, layer: target, cap: 25.0 * target, depth: 0, patterns: 0, images: 0, nest: 0 };
    let mut walk = Walk { budget: budget as f64, visited: 0, shared: HashMap::new(), clips: HashMap::new(), masks: HashMap::new() };
    walk.nodes(nodes, ts, cx).map(|_| ())
}

impl Walk {
    fn need(&self, live: f64, what: impl FnOnce() -> String) -> Result<(), Refusal> {
        if live > self.budget { Err(memory(&what(), live, self.budget)) } else { Ok(()) }
    }
}

impl Walk {
    fn nodes(&mut self, nodes: &[usvg::Node], ts: Transform, cx: Cx) -> Result<Profile, Refusal> {
        let mut profile = Profile::default();
        for n in nodes {
            profile = profile.max(self.node(n, ts, cx)?);
        }
        Ok(profile)
    }

    // The recursion through plain groups is the deep one (usvg allows 1,000+ levels), so only small
    // functions sit on it; everything with many locals is kept off it with `inline(never)`.
    fn node(&mut self, node: &usvg::Node, ts: Transform, cx: Cx) -> Result<Profile, Refusal> {
        self.visited += 1;
        if self.visited > MAX_NODES {
            return Err(Refusal { complex: true, memory: false, message: format!("the drawing has more than {MAX_NODES} elements to check") });
        }
        match node {
            usvg::Node::Group(g) if !g.should_isolate() => self.plain(g, ts, cx),
            usvg::Node::Group(g) => self.group(g, ts, cx),
            usvg::Node::Text(t) => self.group(t.flattened(), ts, cx),
            usvg::Node::Path(p) => self.path(p, ts, cx),
            usvg::Node::Image(image) => self.image(image, ts, cx),
        }
    }

    /// A plain group is no layer.
    fn plain(&mut self, g: &usvg::Group, ts: Transform, cx: Cx) -> Result<Profile, Refusal> {
        if cx.nest >= MAX_NEST {
            return Err(limit("groups are nested too deeply", MAX_NEST));
        }
        let inside = self.nodes(g.children(), ts.pre_concat(g.transform()), Cx { nest: cx.nest + 1, ..cx })?;
        Ok(inside.under(0.0, [0, 0, 0, 1]))
    }

    #[inline(never)]
    fn path(&mut self, p: &usvg::Path, ts: Transform, cx: Cx) -> Result<Profile, Refusal> {
        let mut profile = Profile::default();
        if !p.is_visible() {
            return Ok(profile);
        }
        for paint in p.fill().map(|f| f.paint()).into_iter().chain(p.stroke().map(|s| s.paint())) {
            if let usvg::Paint::Pattern(pattern) = paint {
                profile = profile.max(self.pattern(pattern, ts, cx)?);
            }
        }
        Ok(profile)
    }

    /// resvg draws an embedded SVG on a pixmap the size of the current one, with its own 5 × 5 clamp.
    #[inline(never)]
    fn image(&mut self, image: &usvg::Image, ts: Transform, cx: Cx) -> Result<Profile, Refusal> {
        let usvg::ImageKind::SVG(tree) = image.kind() else { return Ok(Profile::default()) };
        if cx.images >= MAX_IMAGE_DEPTH {
            return Err(limit("embedded SVG images are nested too deeply", MAX_IMAGE_DEPTH));
        }
        let bytes = cx.layer * 4.0;
        self.need(cx.live + bytes, || "an embedded SVG image".to_string())?;
        let nested = Cx { live: cx.live + bytes, cap: 25.0 * cx.layer, depth: cx.depth + 1, images: cx.images + 1, nest: cx.nest + 1, ..cx };
        deeper(nested)?;
        let inside = self.nodes(tree.root().children(), ts, nested)?;
        Ok(Profile { extra: bytes, ..Profile::default() }.max(inside.under(bytes, [1, 0, 1, 1])))
    }

    /// Walk a definition shared by reference (pattern, mask, filter image) once per transform and
    /// size; a repeat only re-checks its profile where it is used now.
    fn shared(&mut self, key: Key, cx: Cx, walk: impl FnOnce(&mut Self) -> Result<Profile, Refusal>) -> Result<Profile, Refusal> {
        let Some(p) = self.shared.get(&key).copied() else {
            let p = walk(self)?;
            self.shared.insert(key, p);
            return Ok(p);
        };
        self.need(cx.live + p.extra, || "a shared definition used here".to_string())?;
        if cx.depth + p.depth > MAX_DEPTH {
            return Err(limit("layers are nested too deeply", MAX_DEPTH));
        }
        if cx.patterns + p.patterns > MAX_PATTERN_DEPTH {
            return Err(limit("patterns are nested too deeply", MAX_PATTERN_DEPTH));
        }
        if cx.images + p.images > MAX_IMAGE_DEPTH {
            return Err(limit("embedded SVG images are nested too deeply", MAX_IMAGE_DEPTH));
        }
        if cx.nest + p.nest > MAX_NEST {
            return Err(limit("groups are nested too deeply", MAX_NEST));
        }
        Ok(p)
    }

    #[inline(never)]
    fn group(&mut self, g: &usvg::Group, ts: Transform, cx: Cx) -> Result<Profile, Refusal> {
        if !g.should_isolate() {
            return self.plain(g, ts, cx);
        }
        let ts = ts.pre_concat(g.transform());
        if cx.nest >= MAX_NEST {
            return Err(limit("groups are nested too deeply", MAX_NEST));
        }
        let Some(bb) = g.layer_bounding_box().transform(ts) else { return Ok(Profile::default()) };
        let own = px((f64::from(bb.width()).ceil() + 4.0) * (f64::from(bb.height()).ceil() + 4.0)).min(cx.cap);
        let inner = Cx { live: cx.live + own * 4.0, layer: own, depth: cx.depth + 1, nest: cx.nest + 1, ..cx };
        deeper(inner)?;
        // Filter, clip and mask are applied one after another; each needs its own extra buffers
        // beside the group's layer. A clip or mask link is a 4 B/px pixmap and a 1 B/px mask.
        let clip = g.clip_path().map_or(Ok(0), |c| self.clip_links(c, 0))?;
        let mask = g.mask().map_or(Ok(0), |m| self.mask_links(m, 0))?;
        let links = clip.max(mask);
        let layers = own * 4.0 + own * 5.0 * links as f64;
        self.need(cx.live + layers, || format!("a group with {links} clip or mask layers of {} px", own as u64))?;
        let mut profile = Profile { extra: layers, ..Profile::default() };
        for filter in g.filters() {
            profile = profile.max(self.filter(filter, ts, cx, own)?);
        }
        let mut mask_ref = g.mask();
        let mut link = 1.0;
        while let Some(mask) = mask_ref {
            // The mask content is drawn while this and every earlier link's pixmap and mask are alive.
            let bytes = own * 4.0 + own * 5.0 * link;
            let content = Cx { live: cx.live + bytes, ..inner };
            let inside =
                self.shared(key(std::ptr::from_ref(mask) as usize, ts, content), content, |w| w.nodes(mask.root().children(), ts, content))?;
            profile = profile.max(inside.under(bytes, [1, 0, 0, 1]));
            mask_ref = mask.mask();
            link += 1.0;
        }
        let inside = self.nodes(g.children(), ts, inner)?;
        Ok(profile.max(inside.under(own * 4.0, [1, 0, 0, 1])))
    }

    /// Layers a clip-path keeps alive: its own, the deepest clip child with a clip-path of its own,
    /// or the next link of the chain, whichever is deepest. Nothing is walked twice.
    fn clip_links(&mut self, clip: &usvg::ClipPath, depth: usize) -> Result<usize, Refusal> {
        let key = std::ptr::from_ref(clip) as usize;
        if let Some(n) = self.clips.get(&key) {
            return Ok(*n);
        }
        let too_long = || limit("clip-paths are chained or nested too deeply", MAX_LINKS);
        if depth >= MAX_LINKS {
            return Err(too_long());
        }
        let inside = self.clip_children(clip.root(), depth + 1)?;
        let next = clip.clip_path().map_or(Ok(0), |c| self.clip_links(c, depth + 1))?;
        let n = 1 + inside.max(next);
        if n > MAX_LINKS {
            return Err(too_long());
        }
        self.clips.insert(key, n);
        Ok(n)
    }

    /// resvg's `clip_group`: a clip child with its own clip-path is drawn on a layer of its own,
    /// kept while its descendants are drawn and its clip is applied.
    fn clip_children(&mut self, group: &usvg::Group, depth: usize) -> Result<usize, Refusal> {
        let mut deepest = 0;
        for child in group.children() {
            let inner = match child {
                usvg::Node::Group(g) => {
                    let inside = self.clip_children(g, depth + usize::from(g.clip_path().is_some()))?;
                    match g.clip_path() {
                        Some(c) => 1 + inside.max(self.clip_links(c, depth + 1)?),
                        None => inside,
                    }
                }
                usvg::Node::Text(t) => self.clip_children(t.flattened(), depth)?,
                _ => 0,
            };
            if inner + depth > MAX_LINKS {
                return Err(limit("clip-paths are chained or nested too deeply", MAX_LINKS));
            }
            deepest = deepest.max(inner);
        }
        Ok(deepest)
    }

    /// Layers a mask chain keeps alive: a mask pixmap per link while the next is applied.
    fn mask_links(&mut self, mask: &usvg::Mask, depth: usize) -> Result<usize, Refusal> {
        let key = std::ptr::from_ref(mask) as usize;
        if let Some(n) = self.masks.get(&key) {
            return Ok(*n);
        }
        if depth >= MAX_LINKS {
            return Err(limit("masks are chained too deeply", MAX_LINKS));
        }
        let n = 1 + mask.mask().map_or(Ok(0), |m| self.mask_links(m, depth + 1))?;
        // A cached tail can make the chain longer than `depth` shows; the cap is on the whole chain.
        if n > MAX_LINKS {
            return Err(limit("masks are chained too deeply", MAX_LINKS));
        }
        self.masks.insert(key, n);
        Ok(n)
    }

    /// A filter keeps a result per primitive, each as large as the filter region or the layer it
    /// filters, whichever is larger (usvg makes the layer the union of all of a group's filter
    /// regions, and `SourceGraphic` clones the whole layer). `own` is the size of that layer.
    fn filter(&mut self, f: &usvg::filter::Filter, ts: Transform, cx: Cx, own: f64) -> Result<Profile, Refusal> {
        use usvg::filter::Kind;
        let prims = f.primitives().len();
        if prims > MAX_PRIMITIVES {
            return Err(limit("a filter has too many primitives", MAX_PRIMITIVES));
        }
        let Some(region) = f.rect().transform(ts) else { return Ok(Profile::default()) };
        let region = px(f64::from(region.width()).ceil() * f64::from(region.height()).ceil());
        let slot = region.max(own) * 4.0;
        // Buffers a primitive allocates beside its result (resvg 0.45.1 `filter/`): a blur works
        // on one copy, an IIR blur (sigma under 2 after scaling) also on f64 scratch; blend,
        // composite and displacement map convert two inputs; a drop shadow holds the input, its
        // blur, the shadow and the result.
        let (sx, sy) = ts.get_scale();
        let transient = |kind: &Kind| match kind {
            Kind::GaussianBlur(b) if b.std_dev_x().get() * sx >= 2.0 || b.std_dev_y().get() * sy >= 2.0 => 1.0,
            Kind::GaussianBlur(_) => 2.0,
            Kind::Blend(_) | Kind::Composite(_) | Kind::DisplacementMap(_) => 2.0,
            // A tile of the source graphic converts and tiles two copies.
            Kind::Tile(t) if matches!(t.input(), usvg::filter::Input::SourceGraphic) => 2.0,
            Kind::DropShadow(_) => 4.0,
            _ => 1.0,
        };
        let most = f.primitives().iter().map(|p| transient(p.kind())).fold(0.0, f64::max);
        let extra = own * 4.0 + slot * (prims as f64 + most);
        self.need(cx.live + extra, || format!("a filter with {prims} primitives over a region of {} px", region.min(1e18) as u64))?;
        let mut profile = Profile { extra, ..Profile::default() };
        for (i, primitive) in f.primitives().iter().enumerate() {
            if let Kind::Image(fe) = primitive.kind() {
                // What an feImage draws sees the results kept so far.
                let bytes = own * 4.0 + slot * (i + 1) as f64;
                let (sx, sy) = ts.get_scale();
                let scaled = Transform::from_scale(sx, sy);
                let nested = Cx { live: cx.live + bytes, layer: region, cap: region, depth: cx.depth + 1, nest: cx.nest + 1, ..cx };
                deeper(nested)?;
                let inside =
                    self.shared(key(std::ptr::from_ref(fe) as usize, scaled, nested), nested, |w| w.nodes(fe.root().children(), scaled, nested))?;
                profile = profile.max(inside.under(bytes, [1, 0, 0, 1]));
            }
        }
        Ok(profile)
    }

    /// A pattern tile is `width × height × scale` pixels, whatever the target.
    fn pattern(&mut self, pattern: &usvg::Pattern, ts: Transform, cx: Cx) -> Result<Profile, Refusal> {
        let (sx, sy) = ts.pre_concat(pattern.transform()).get_scale();
        let rect = pattern.rect();
        let (tw, th) = (f64::from(rect.width() * sx).round(), f64::from(rect.height() * sy).round());
        // resvg draws no tile smaller than a pixel (or not a number).
        if !(tw >= 1.0 && th >= 1.0) {
            return Ok(Profile::default());
        }
        let tile = tw * th;
        let bytes = tile * 4.0;
        self.need(cx.live + bytes, || format!("a pattern tile of {tw} × {th} px"))?;
        if cx.patterns >= MAX_PATTERN_DEPTH {
            return Err(limit("patterns are nested too deeply", MAX_PATTERN_DEPTH));
        }
        let nested = Cx { live: cx.live + bytes, layer: tile, depth: cx.depth + 1, patterns: cx.patterns + 1, nest: cx.nest + 1, ..cx };
        deeper(nested)?;
        let inner_ts = Transform::from_scale(sx, sy);
        let inside = self
            .shared(key(std::ptr::from_ref(pattern) as usize, inner_ts, nested), nested, |w| w.nodes(pattern.root().children(), inner_ts, nested))?;
        Ok(Profile { extra: bytes, ..Profile::default() }.max(inside.under(bytes, [1, 1, 0, 1])))
    }
}

fn deeper(cx: Cx) -> Result<(), Refusal> {
    if cx.depth > MAX_DEPTH {
        return Err(limit("layers are nested too deeply", MAX_DEPTH));
    }
    Ok(())
}
