//! The transparency flattener: transparent art becomes opaque fills that print the same.
//!
//! Items split into transitive groups of overlapping items ([`geom::transparent_groups`]). Groups
//! without transparency come back unchanged, so their text stays live. The items of the other
//! groups are cut into planar regions ([`geom::regions`]); the same paints cover all of a region,
//! so one composited color is exact for it. Colors composite as ink vectors, one subtractive value
//! per ink of the list's [`InkSet`], process and spot inks alike (ISO 32000-2 §11.3). Overprint
//! applies per ink while compositing (§11.7.4, `CompatibleOverprint`), never through blend modes or
//! knockout groups. Regions a gradient, image or soft mask reaches have no single color: they are
//! listed in [`Flattened::complex`] for the raster step.
//!
//! Overprint after flattening: a region fill has nothing under it, and its color already holds
//! every plate's result, so it does not overprint. The exception keeps a paint as it was: where a
//! region's color equals its top paint painted alone, the fill takes that paint's color and
//! overprint attributes, which print the same over paper. Kept items keep their attributes, since
//! what they overprint is still under them.
//!
//! Ported from VectorCraft (MIT OR Apache-2.0) at commit c2b15982: `Composer`, `region_fill` and
//! the raster/vector balance (`rasterize_all`) in `crates/engine/src/cmd/flatten.rs`, and the
//! blend formulas in `crates/color/src/blend.rs`. VectorCraft composites RGB or CMY+K planes;
//! here each ink is a plane, and groups follow the §11.4 formulas.

use std::collections::HashMap;
use std::ops::Range;

use kurbo::{BezPath, Rect, Shape as _};

use super::geom::{self, ClipPath, GeomError, Shape};
use super::{BlendMode, DisplayList, FillItem, FillRule, GroupItem, Item, Paint};
use crate::ink::{InkColor, InkId, InkSet, Overprint, Process};

/// How [`flatten`] cuts transparent art.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlattenOptions {
    /// Regions narrower than this (in points) are dropped ([`geom::regions_with`]).
    pub sliver: f64,
    /// A group that cuts into more regions than this goes to [`Flattened::complex`] whole,
    /// as VectorCraft's raster/vector balance rasterizes it.
    pub max_regions: usize,
    /// The resolution complex regions are rasterized at ([`raster`](super::raster)).
    pub raster_dpi: f32,
}

impl Default for FlattenOptions {
    fn default() -> Self {
        // VectorCraft's medium preset (balance 75) allows 10^(75/20), about 5600 regions.
        Self { sliver: geom::DEFAULT_SLIVER, max_regions: 5000, raster_dpi: 300.0 }
    }
}

/// A page with no transparency left.
#[derive(Clone, Debug, PartialEq)]
pub struct Flattened {
    /// Kept items and region fills, in painting order. No group in it is transparent.
    pub items: Vec<Item>,
    /// Areas left for rasterizing, each disjoint from every item but those it paints before.
    pub complex: Vec<ComplexRegion>,
    pub report: FlattenReport,
}

/// An area without one flat color.
#[derive(Clone, Debug, PartialEq)]
pub struct ComplexRegion {
    /// The area, nonzero rule.
    pub path: BezPath,
    /// Indices into [`DisplayList::items`] of the top-level items painting in the area, in
    /// painting order. Rendered alone and clipped to `path`, they give its look.
    pub items: Vec<usize>,
    /// Paints before [`Flattened::items`]`[at]` (after all of them when `at` is their count).
    pub at: usize,
    /// Set when the region is a whole group ([`FlattenReport::complex_groups`]).
    pub whole: Option<ComplexReason>,
}

/// What [`flatten`] did.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct FlattenReport {
    /// Top-level items returned unchanged.
    pub kept: usize,
    /// Top-level items cut into regions.
    pub flattened: usize,
    /// Regions computed.
    pub regions: usize,
    /// Region fills written.
    pub fills: usize,
    /// Entries of [`Flattened::complex`].
    pub complex_regions: usize,
    /// Groups that went to [`Flattened::complex`] whole.
    pub complex_groups: Vec<ComplexGroup>,
}

/// A group of items rasterized whole.
#[derive(Clone, Debug, PartialEq)]
pub struct ComplexGroup {
    /// Indices into [`DisplayList::items`].
    pub items: Vec<usize>,
    pub reason: ComplexReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ComplexReason {
    /// The region sweep failed ([`GeomError::Degenerate`]).
    Degenerate,
    /// More regions than [`FlattenOptions::max_regions`].
    TooManyRegions(usize),
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum FlattenError {
    #[error("a color names ink {0:?}, which is not in the list's ink set")]
    UnknownInk(InkId),
    #[error(transparent)]
    Geometry(#[from] GeomError),
}

/// `list` with its transparency flattened. Items that neither involve transparency nor overlap
/// an item that does come back unchanged, and so do opaque items with no transparency painted
/// above them where they paint.
pub fn flatten(list: &DisplayList, opts: &FlattenOptions) -> Result<Flattened, FlattenError> {
    let nan = Rect::new(f64::NAN, f64::NAN, f64::NAN, f64::NAN);
    let bounds: Vec<Rect> = list.items.iter().map(|i| bounds(i).unwrap_or(nan)).collect();
    let transparent: Vec<bool> = list.items.iter().map(is_transparent).collect();

    #[derive(Clone, Copy)]
    enum Plan {
        Keep,
        Drop,
        Emit(usize),
    }
    let groups = geom::transparent_groups(&bounds, &transparent);
    let mut plan = vec![Plan::Keep; list.items.len()];
    let mut members: Vec<Vec<usize>> = Vec::with_capacity(groups.len());
    for g in &groups {
        // Opaque items above the group's last transparent item paint over flat regions as they
        // would over the composited art, overprint included: they stay as they are.
        let Some(&last) = g.iter().rev().find(|&&i| transparent[i]) else { continue };
        let m: Vec<usize> = g.iter().copied().filter(|&i| i <= last).collect();
        for &i in &m {
            plan[i] = Plan::Drop;
        }
        plan[last] = Plan::Emit(members.len());
        members.push(m);
    }

    let mut out = Flattened { items: Vec::new(), complex: Vec::new(), report: FlattenReport::default() };
    for (i, item) in list.items.iter().enumerate() {
        match plan[i] {
            Plan::Keep => {
                out.items.push(without_groups(item));
                out.report.kept += 1;
            }
            Plan::Drop => {}
            Plan::Emit(g) => flatten_group(list, &members[g], &bounds, &transparent, opts, &mut out)?,
        }
    }
    out.report.complex_regions = out.complex.len();
    Ok(out)
}

fn flatten_group(
    list: &DisplayList,
    members: &[usize],
    bounds: &[Rect],
    transparent: &[bool],
    opts: &FlattenOptions,
    out: &mut Flattened,
) -> Result<(), FlattenError> {
    let mut b = Builder::new(&list.inks);
    let mut nodes = Vec::new();
    for &i in members {
        b.top = i;
        if let Some(n) = b.node(&list.items[i], false)? {
            nodes.push(n);
        }
    }
    let whole = |out: &mut Flattened, path: BezPath, reason| {
        out.complex.push(ComplexRegion { path, items: members.to_vec(), at: out.items.len(), whole: Some(reason) });
        out.report.complex_groups.push(ComplexGroup { items: members.to_vec(), reason });
    };
    let regions = match geom::regions_with(&b.shapes, &b.clips, opts.sliver) {
        Ok(r) => r,
        Err(GeomError::Degenerate) => {
            // Each member's box is clear of every other group's items.
            let mut path = BezPath::new();
            for &i in members {
                if !bounds[i].is_nan() {
                    path.extend(bounds[i].abs().path_elements(0.1));
                }
            }
            whole(out, path, ComplexReason::Degenerate);
            out.report.flattened += members.len();
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    out.report.regions += regions.len();
    if regions.len() > opts.max_regions {
        let mut path = BezPath::new();
        for r in &regions {
            path.extend(r.path.iter());
        }
        whole(out, path, ComplexReason::TooManyRegions(regions.len()));
        out.report.flattened += members.len();
        return Ok(());
    }

    // Members drawn as they are, over the region fills of the others: opaque, with only kept
    // members above them wherever they paint. Painting them in order over the composite of what
    // is under them gives the same page, and text among them stays live text.
    let kept = kept_members(members, transparent, &b.owners, &regions);
    let is_kept = |s: usize| kept.binary_search(&b.owners[s]).is_ok();

    // A region's look depends only on its visible sources: the last one that covers what is
    // under it ([`Builder::covers`]) and those above it. Regions with the same visible sources
    // are one area of one look, so they are written as one: art hidden under an opaque fill
    // must not cut it into pieces, whose edges anti-aliasing viewers show (vectorcraft#2161).
    let mut by_visible: HashMap<Vec<usize>, BezPath> = HashMap::new();
    let mut order = Vec::new();
    for r in regions {
        // Kept members come last in a region's sources. The fill is the look of what is under
        // them, also where one covers it: the region then joins its neighbors of that look
        // instead of leaving a hole whose edge anti-aliasing viewers show.
        let below = r.sources.iter().position(|&s| is_kept(s)).unwrap_or(r.sources.len());
        if below == 0 {
            continue;
        }
        let sources = &r.sources[..below];
        let from = sources.iter().rposition(|&s| b.covers[s]).unwrap_or(0);
        let visible = sources[from..].to_vec();
        let path = oriented(r.path);
        match by_visible.get_mut(&visible) {
            Some(p) => p.extend(path),
            None => {
                order.push(visible.clone());
                by_visible.insert(visible, path);
            }
        }
    }

    let mut complex = Vec::new();
    let mut fills: Vec<(Option<usize>, FillItem)> = Vec::new();
    for sources in order {
        let Some(path) = by_visible.remove(&sources) else { continue };
        let r = geom::Region { path, area: 0.0, sources };
        if r.sources.iter().any(|&s| b.paints[s].is_none()) {
            let mut items: Vec<usize> = r.sources.iter().map(|&s| b.owners[s]).collect();
            items.dedup();
            complex.push(ComplexRegion { path: r.path, items, at: 0, whole: None });
            continue;
        }
        let c = Composer { sources: &r.sources, paints: &b.paints };
        if nodes.iter().fold(0.0, |a, n| union(a, c.alpha(n))) < 0.5 / 255.0 {
            continue;
        }
        // The page group composites over paper: no ink, opaque.
        let paper = Px { c: vec![0.0; list.inks.len()], a: 1.0 };
        let px = nodes.iter().fold(paper, |acc, n| c.draw(n, acc));
        let top = r.sources.last().and_then(|&s| b.paints[s].as_ref());
        let (color, overprint) = match top {
            Some(t) if px.c.iter().zip(&t.value).all(|(a, b)| (a - b).abs() < 1e-4) => (t.color.clone(), t.overprint),
            _ => (ink_color(&px.c, &list.inks), Overprint::default()),
        };
        let tag = r.sources.iter().rev().find_map(|&s| b.tags[s]);
        fills.push((tag, FillItem { path: r.path, rule: FillRule::NonZero, paint: Paint::Solid(color), overprint }));
    }
    // Region fills are disjoint and nothing else of the page is under them, so they can be
    // reordered: those showing tagged text go into one plain group per text.
    out.report.fills += fills.len();
    let mut tagged: Vec<Vec<Item>> = vec![Vec::new(); b.texts.len()];
    for (tag, fill) in fills {
        match tag.and_then(|t| tagged.get_mut(t)) {
            Some(children) => children.push(Item::Fill(fill)),
            None => out.items.push(Item::Fill(fill)),
        }
    }
    for (text, children) in b.texts.into_iter().zip(tagged).filter(|(_, c)| !c.is_empty()) {
        out.items.push(Item::Group(GroupItem {
            opacity: 1.0,
            blend: BlendMode::Normal,
            isolated: false,
            knockout: false,
            clip: None,
            mask: None,
            actual_text: Some(text),
            children,
        }));
    }
    let at = out.items.len();
    out.complex.extend(merge_touching(complex).into_iter().map(|c| ComplexRegion { at, ..c }));
    for &i in &kept {
        out.items.push(without_groups(&list.items[i]));
    }
    out.report.kept += kept.len();
    out.report.flattened += members.len() - kept.len();
    Ok(())
}

/// The members of a group that can be drawn as they are ([`flatten_group`]), ascending: not
/// transparent, and in every region a member's shapes are in, only kept members are above it.
fn kept_members(members: &[usize], transparent: &[bool], owners: &[usize], regions: &[geom::Region]) -> Vec<usize> {
    let mut regions_of: HashMap<usize, Vec<usize>> = HashMap::new();
    for (r, region) in regions.iter().enumerate() {
        let mut last = None;
        for &s in &region.sources {
            let o = owners[s];
            if last != Some(o) {
                regions_of.entry(o).or_default().push(r);
                last = Some(o);
            }
        }
    }
    // From the top down, so that a member's decision sees every member above it decided.
    // `clear[r]`: every member decided so far that is in region `r` is kept.
    let mut clear = vec![true; regions.len()];
    let mut kept = Vec::new();
    for &m in members.iter().rev() {
        let rs = regions_of.get(&m).map_or(&[][..], Vec::as_slice);
        let keep = !transparent[m] && rs.iter().all(|&r| clear[r]);
        if keep {
            kept.push(m);
        } else {
            for &r in rs {
                clear[r] = false;
            }
        }
    }
    kept.reverse();
    kept
}

/// `path` with its subpaths reversed when its signed area is negative. Faces of one look are
/// written as one path; with one orientation, their shared edges cancel in an anti-aliasing
/// rasterizer's coverage instead of leaving a seam.
fn oriented(path: BezPath) -> BezPath {
    if path.area() < 0.0 { path.reverse_subpaths() } else { path }
}

/// Complex regions whose boxes touch, merged, so that one image spans their shared edges. A
/// merged region lists the items of all its parts: an item paints nothing in a part that
/// doesn't list it, so rendering it there changes nothing.
fn merge_touching(regions: Vec<ComplexRegion>) -> Vec<ComplexRegion> {
    if regions.len() < 2 {
        return regions;
    }
    let boxes: Vec<Rect> = regions.iter().map(|r| r.path.bounding_box().inflate(1e-6, 1e-6)).collect();
    geom::overlap_groups(&boxes)
        .into_iter()
        .map(|g| {
            let (mut path, mut items) = (BezPath::new(), Vec::new());
            for i in g {
                path.extend(regions[i].path.iter());
                items.extend_from_slice(&regions[i].items);
            }
            items.sort_unstable();
            items.dedup();
            ComplexRegion { path, items, at: 0, whole: None }
        })
        .collect()
}

/// Composited ink values as a color: the inks with ink on them, or no ink on the process plates.
fn ink_color(c: &[f32], inks: &InkSet) -> InkColor {
    let mut tints: Vec<(InkId, f32)> = inks.iter().zip(c).map(|((id, _), v)| (id, unit(*v))).filter(|(_, v)| *v > 1e-6).collect();
    if tints.is_empty() {
        tints = Process::ALL.iter().map(|p| (InkId::process(*p), 0.0)).collect();
    }
    InkColor::Tints(tints)
}

fn bounds(item: &Item) -> Option<Rect> {
    let path = |p: &BezPath| (!p.elements().is_empty()).then(|| p.bounding_box());
    match item {
        Item::Fill(f) => path(&f.path),
        Item::Text(t) => path(&t.outlines),
        Item::Image(im) => Some(im.transform.transform_rect_bbox(Rect::new(0.0, 0.0, 1.0, 1.0))),
        Item::Group(g) => {
            let inner = g.children.iter().filter_map(bounds).reduce(|a, b| a.union(b))?;
            match g.clip.as_ref().and_then(path) {
                Some(clip) => Some(inner.intersect(clip)),
                None if g.clip.is_some() => None,
                None => Some(inner),
            }
        }
    }
}

/// Whether `item` paints differently composited than painted opaquely in order.
fn is_transparent(item: &Item) -> bool {
    match item {
        Item::Fill(_) | Item::Text(_) => false,
        Item::Image(im) => im.smask.is_some(),
        Item::Group(g) => {
            g.opacity < 1.0
                || g.blend != BlendMode::Normal
                || g.mask.is_some()
                || ((g.isolated || g.knockout) && g.children.iter().any(overprints))
                || g.children.iter().any(is_transparent)
        }
    }
}

/// A kept item with its isolated and knockout groups made plain groups: without transparency
/// inside ([`is_transparent`]), they paint the same, and a flattened page has no transparency
/// groups.
fn without_groups(item: &Item) -> Item {
    match item {
        Item::Group(g) => {
            Item::Group(GroupItem { isolated: false, knockout: false, children: g.children.iter().map(without_groups).collect(), ..g.clone() })
        }
        other => other.clone(),
    }
}

fn overprints(item: &Item) -> bool {
    match item {
        Item::Fill(f) => f.overprint.fill,
        Item::Text(t) => t.overprint.fill,
        Item::Image(im) => im.overprint.fill,
        Item::Group(g) => g.children.iter().any(overprints),
    }
}

/// A flat paint as the compositor uses it.
#[derive(Debug)]
struct Source {
    /// One value per ink, `0.0..=1.0`; unnamed inks 0.
    value: Vec<f32>,
    /// With overprint: which inks the paint names. `None` when it names all of them.
    named: Option<Vec<bool>>,
    color: InkColor,
    overprint: Overprint,
}

impl Source {
    /// Fills, outlined strokes included, overprint by `overprint.fill`. An overprinting paint
    /// names the inks it lists; with nonzero overprint mode a process-only color (DeviceCMYK)
    /// leaves out its zero components too. `None` for the `None` colorant, which paints nothing.
    fn new(color: &InkColor, overprint: Overprint, inks: &InkSet) -> Result<Option<Self>, FlattenError> {
        let n = inks.len();
        let (value, named) = match color {
            InkColor::None => return Ok(None),
            InkColor::All(t) => (vec![unit(*t); n], vec![true; n]),
            InkColor::Tints(tints) => {
                let (mut value, mut named) = (vec![0.0; n], vec![false; n]);
                for &(id, t) in tints {
                    let (v, k) = value.get_mut(id.index()).zip(named.get_mut(id.index())).ok_or(FlattenError::UnknownInk(id))?;
                    *v = 1.0 - (1.0 - *v) * (1.0 - unit(t));
                    *k = true;
                }
                if overprint.nonzero && color.as_cmyk().is_some() {
                    for (k, v) in named.iter_mut().zip(&value).take(4) {
                        *k &= *v > 0.0;
                    }
                }
                (value, named)
            }
        };
        let named = overprint.fill.then_some(named);
        Ok(Some(Self { value, named, color: color.clone(), overprint }))
    }
}

enum Node {
    /// A shape and its paint (`Builder::paints`).
    Leaf(usize),
    Group(GroupNode),
}

struct GroupNode {
    opacity: f32,
    blend: BlendMode,
    isolated: bool,
    knockout: bool,
    /// [`is_plain`].
    plain: bool,
    /// The shapes of every leaf inside, which come in this range by construction.
    shapes: Range<usize>,
    children: Vec<Node>,
}

impl GroupNode {
    /// The children, with those of plain groups ([`is_plain`]) in their place, recursively.
    fn elements(self) -> Vec<Node> {
        let mut out = Vec::with_capacity(self.children.len());
        for c in self.children {
            match c {
                Node::Group(g) if g.plain => out.extend(g.elements()),
                n => out.push(n),
            }
        }
        out
    }
}

/// A group that only clips: it composites nothing of its own.
fn is_plain(item: &Item) -> bool {
    matches!(item, Item::Group(g) if g.opacity >= 1.0 && g.blend == BlendMode::Normal && !g.isolated && !g.knockout && g.mask.is_none())
}

/// Turns items into the region sweep's shapes and the compositor's tree.
struct Builder<'a> {
    inks: &'a InkSet,
    shapes: Vec<Shape>,
    clips: Vec<ClipPath>,
    clip_stack: Vec<usize>,
    /// Per shape: its flat paint, `None` for art without one (gradients, images, masked art).
    paints: Vec<Option<Source>>,
    /// Per shape: the top-level item it is part of.
    owners: Vec<usize>,
    top: usize,
    /// Per shape: the text it shows, an index into `texts`. The outermost source wins: a group's
    /// `actual_text`, else a text run's own text.
    tags: Vec<Option<usize>>,
    texts: Vec<String>,
    tag: Option<usize>,
    /// Per shape: it paints its color over whatever is under it, so nothing under it shows.
    covers: Vec<bool>,
    /// Every open group composites at full opacity with Normal blending.
    plain_path: bool,
    /// Some open group is a knockout group.
    knockout_path: bool,
}

impl<'a> Builder<'a> {
    fn new(inks: &'a InkSet) -> Self {
        Self {
            inks,
            shapes: Vec::new(),
            clips: Vec::new(),
            clip_stack: Vec::new(),
            paints: Vec::new(),
            owners: Vec::new(),
            top: 0,
            tags: Vec::new(),
            texts: Vec::new(),
            tag: None,
            covers: Vec::new(),
            plain_path: true,
            knockout_path: false,
        }
    }

    fn leaf(&mut self, path: BezPath, rule: FillRule, paint: Option<Source>) -> Option<Node> {
        if path.elements().is_empty() {
            return None;
        }
        self.shapes.push(Shape::new(path, rule).clipped(self.clip_stack.iter().copied()));
        // An opaque paint without overprint in groups that composite it unchanged (an isolated
        // group too: the paint replaces its backdrop either way). Not in a knockout group: a later
        // element there composites over the group's backdrop, so what is under the paint shows.
        self.covers.push(self.plain_path && !self.knockout_path && paint.as_ref().is_some_and(|p| p.named.is_none()));
        self.paints.push(paint);
        self.owners.push(self.top);
        self.tags.push(self.tag);
        Some(Node::Leaf(self.shapes.len() - 1))
    }

    fn painted(&mut self, path: &BezPath, rule: FillRule, paint: &Paint, overprint: Overprint, masked: bool) -> Result<Option<Node>, FlattenError> {
        let source = match paint {
            Paint::Solid(color) => match Source::new(color, overprint, self.inks)? {
                Some(s) => (!masked).then_some(s),
                None => return Ok(None),
            },
            Paint::Gradient { .. } => None,
        };
        Ok(self.leaf(path.clone(), rule, source))
    }

    /// Makes `text` the current tag unless one is set; returns the tag to restore.
    fn push_tag(&mut self, text: Option<&str>) -> Option<usize> {
        let outer = self.tag;
        if let (None, Some(text)) = (outer, text) {
            self.texts.push(text.to_string());
            self.tag = Some(self.texts.len() - 1);
        }
        outer
    }

    fn node(&mut self, item: &Item, masked: bool) -> Result<Option<Node>, FlattenError> {
        match item {
            Item::Fill(f) => self.painted(&f.path, f.rule, &f.paint, f.overprint, masked),
            Item::Text(t) => {
                let outer = self.push_tag(t.text.as_deref());
                let node = self.painted(&t.outlines, FillRule::NonZero, &t.paint, t.overprint, masked);
                self.tag = outer;
                node
            }
            Item::Image(im) => Ok(self.leaf(im.transform * Rect::new(0.0, 0.0, 1.0, 1.0).to_path(0.1), FillRule::NonZero, None)),
            Item::Group(g) => {
                let start = self.shapes.len();
                let outer = self.push_tag(g.actual_text.as_deref());
                if let Some(clip) = &g.clip {
                    self.clip_stack.push(self.clips.len());
                    self.clips.push(ClipPath { path: clip.clone(), rule: FillRule::NonZero });
                }
                let masked = masked || g.mask.is_some();
                let (plain_path, knockout_path) = (self.plain_path, self.knockout_path);
                self.plain_path &= g.opacity >= 1.0 && g.blend == BlendMode::Normal;
                self.knockout_path |= g.knockout;
                let mut children = Vec::with_capacity(g.children.len());
                for c in &g.children {
                    match self.node(c, masked)? {
                        // A clip (or a plain form) is not an element of a knockout group: the
                        // elements inside it are, and knock each other out.
                        Some(Node::Group(inner)) if g.knockout && is_plain(c) => children.extend(inner.elements()),
                        Some(n) => children.push(n),
                        None => {}
                    }
                }
                if g.clip.is_some() {
                    self.clip_stack.pop();
                }
                (self.plain_path, self.knockout_path) = (plain_path, knockout_path);
                self.tag = outer;
                Ok((!children.is_empty()).then(|| {
                    Node::Group(GroupNode {
                        opacity: unit(g.opacity),
                        blend: g.blend,
                        isolated: g.isolated,
                        knockout: g.knockout,
                        plain: is_plain(item),
                        shapes: start..self.shapes.len(),
                        children,
                    })
                }))
            }
        }
    }
}

/// A color and its alpha at one point: one subtractive value per ink (0 = no ink).
#[derive(Clone, Debug)]
struct Px {
    c: Vec<f32>,
    a: f32,
}

/// Composites the items over one region (ISO 32000-2 §11.3–11.4). Shapes are 0 or 1 across a
/// region, so a knockout group shows its last element that covers it.
struct Composer<'a> {
    /// The shapes covering the region, ascending.
    sources: &'a [usize],
    paints: &'a [Option<Source>],
}

impl Composer<'_> {
    fn covers(&self, n: &Node) -> bool {
        match n {
            Node::Leaf(s) => self.sources.binary_search(s).is_ok(),
            Node::Group(g) => {
                let i = self.sources.partition_point(|&s| s < g.shapes.start);
                self.sources.get(i).is_some_and(|&s| s < g.shapes.end)
            }
        }
    }

    /// `n`'s alpha as an element of its parent group.
    fn alpha(&self, n: &Node) -> f32 {
        match n {
            _ if !self.covers(n) => 0.0,
            Node::Leaf(_) => 1.0,
            Node::Group(g) => g.opacity * self.group_alpha(g),
        }
    }

    /// The group's own alpha, without its backdrop (§11.4.8, α_gn).
    fn group_alpha(&self, g: &GroupNode) -> f32 {
        if g.knockout {
            g.children.iter().rev().find(|c| self.covers(c)).map_or(0.0, |c| self.alpha(c))
        } else {
            g.children.iter().fold(0.0, |a, c| union(a, self.alpha(c)))
        }
    }

    /// `n` composited over `acc`.
    fn draw(&self, n: &Node, acc: Px) -> Px {
        if !self.covers(n) {
            return acc;
        }
        let g = match n {
            Node::Leaf(s) => match &self.paints[*s] {
                Some(p) => return composite(&acc, &p.value, p.named.as_deref(), 1.0, BlendMode::Normal),
                None => return acc,
            },
            Node::Group(g) => g,
        };
        if g.opacity >= 1.0 && g.blend == BlendMode::Normal && !g.isolated && !g.knockout {
            return g.children.iter().fold(acc, |acc, c| self.draw(c, acc));
        }
        let init = if g.isolated { Px { c: vec![0.0; acc.c.len()], a: 0.0 } } else { acc.clone() };
        let cn = if g.knockout {
            match g.children.iter().rev().find(|c| self.covers(c)) {
                Some(c) => self.draw(c, init.clone()),
                None => init.clone(),
            }
        } else {
            g.children.iter().fold(init.clone(), |acc, c| self.draw(c, acc))
        };
        let ag = self.group_alpha(g);
        if ag <= 0.0 {
            return acc;
        }
        let c = if g.isolated {
            cn.c
        } else {
            // Take the backdrop's share out of a non-isolated group's result (§11.4.8).
            let k = init.a / ag - init.a;
            cn.c.iter().zip(&init.c).map(|(n, b)| unit(n + (n - b) * k)).collect()
        };
        composite(&acc, &c, None, ag * g.opacity, g.blend)
    }
}

/// `s` with alpha `a_s` composited over `b` with blend mode `mode` (§11.3.3). Inks that `named`
/// leaves out keep the backdrop's value (`CompatibleOverprint`, §11.7.4.3).
fn composite(b: &Px, s: &[f32], named: Option<&[bool]>, a_s: f32, mode: BlendMode) -> Px {
    let a_r = union(b.a, a_s);
    if a_r <= 0.0 {
        return b.clone();
    }
    let mixed = blend(mode, &b.c, s);
    let t = a_s / a_r;
    let c = (0..b.c.len())
        .map(|i| {
            let (cb, cs) = (b.c[i], s[i]);
            if named.is_some_and(|n| !n[i]) { cb } else { unit((1.0 - t) * cb + t * ((1.0 - b.a) * cs + b.a * mixed[i])) }
        })
        .collect();
    Px { c, a: a_r }
}

/// B(c_b, c_s) per ink, on subtractive values: the additive formulas on complemented values
/// (§11.3.5). Non-separable modes blend C, M and Y as RGB, keep the backdrop's K (the source's
/// for Luminosity), and act as Normal on spot inks (§11.3.5.3).
fn blend(mode: BlendMode, b: &[f32], s: &[f32]) -> Vec<f32> {
    if mode == BlendMode::Normal {
        return s.to_vec();
    }
    if mode.is_non_separable() {
        let mut out = s.to_vec();
        if let ([b0, b1, b2, bk, ..], [s0, s1, s2, sk, ..]) = (b, s) {
            let m = non_separable(mode, [1.0 - b0, 1.0 - b1, 1.0 - b2], [1.0 - s0, 1.0 - s1, 1.0 - s2]);
            for (o, v) in out.iter_mut().zip(m) {
                *o = 1.0 - v;
            }
            out[3] = if mode == BlendMode::Luminosity { *sk } else { *bk };
        }
        return out;
    }
    let f = separable(mode);
    b.iter().zip(s).map(|(b, s)| 1.0 - f(1.0 - unit(*b), 1.0 - unit(*s))).collect()
}

fn separable(mode: BlendMode) -> fn(f32, f32) -> f32 {
    use BlendMode as B;
    match mode {
        B::Multiply => |b, s| b * s,
        B::Screen => screen,
        B::Overlay => |b, s| hard_light(s, b),
        B::Darken => f32::min,
        B::Lighten => f32::max,
        B::ColorDodge => |b, s| {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        },
        B::ColorBurn => |b, s| {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        },
        B::HardLight => hard_light,
        B::SoftLight => |b, s| {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
                b + (2.0 * s - 1.0) * (d - b)
            }
        },
        B::Difference => |b, s| (b - s).abs(),
        B::Exclusion => |b, s| b + s - 2.0 * b * s,
        B::Normal | B::Hue | B::Saturation | B::Color | B::Luminosity => |_, s| s,
    }
}

fn non_separable(mode: BlendMode, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    match mode {
        BlendMode::Hue => set_lum(set_sat(s, sat(b)), lum(b)),
        BlendMode::Saturation => set_lum(set_sat(b, sat(s)), lum(b)),
        BlendMode::Color => set_lum(s, lum(b)),
        BlendMode::Luminosity => set_lum(b, lum(s)),
        _ => s,
    }
}

fn screen(b: f32, s: f32) -> f32 {
    b + s - b * s
}

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 { b * 2.0 * s } else { screen(b, 2.0 * s - 1.0) }
}

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

/// `c` moved back into gamut keeping its luminance. A gray is clamped instead of scaled by 0/0.
fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let (n, x) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
    let mut c = c;
    if n < 0.0 {
        c = if l - n > f32::EPSILON { c.map(|v| l + (v - l) * l / (l - n)) } else { c.map(|v| v.max(0.0)) };
    }
    if x > 1.0 {
        c = if x - l > f32::EPSILON { c.map(|v| l + (v - l) * (1.0 - l) / (x - l)) } else { c.map(|v| v.min(1.0)) };
    }
    c
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let (n, x) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
    if x <= n {
        return [0.0; 3];
    }
    c.map(|v| (v - n) * s / (x - n))
}

fn union(a: f32, b: f32) -> f32 {
    a + b - a * b
}

fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kurbo::{Affine, Point};

    use super::*;
    use crate::flatten::{GradientGeometry, GroupItem, PlacedGlyph, TextFont, TextItem};
    use crate::ink::{Alternate, Ink, Spot};

    const C: InkId = InkId::process(Process::Cyan);
    const M: InkId = InkId::process(Process::Magenta);
    const Y: InkId = InkId::process(Process::Yellow);
    const K: InkId = InkId::process(Process::Black);

    fn fill(r: Rect, tints: &[(InkId, f32)], overprint: Overprint) -> Item {
        Item::Fill(FillItem { path: r.to_path(0.1), rule: FillRule::NonZero, paint: Paint::Solid(InkColor::Tints(tints.to_vec())), overprint })
    }

    fn solid(r: Rect, tints: &[(InkId, f32)]) -> Item {
        fill(r, tints, Overprint::default())
    }

    fn group(opacity: f32, blend: BlendMode, children: Vec<Item>) -> GroupItem {
        GroupItem { opacity, blend, isolated: false, knockout: false, clip: None, mask: None, actual_text: None, children }
    }

    fn list(inks: InkSet, items: Vec<Item>) -> DisplayList {
        DisplayList { page: Rect::new(0.0, 0.0, 600.0, 800.0), inks, items }
    }

    fn flat(list: &DisplayList) -> Flattened {
        flatten(list, &FlattenOptions::default()).unwrap()
    }

    /// The fill painting `(x, y)` and its tint of each of `inks`.
    fn at<'a>(f: &'a Flattened, x: f64, y: f64, inks: &[InkId]) -> (&'a FillItem, Vec<f32>) {
        let p = Point::new(x, y);
        let hit = f.items.iter().rev().find_map(|i| match i {
            Item::Fill(fi) if fi.path.winding(p) != 0 => Some(fi),
            _ => None,
        });
        let fi = hit.unwrap_or_else(|| panic!("nothing paints {p:?}"));
        let Paint::Solid(InkColor::Tints(t)) = &fi.paint else { panic!("{:?}", fi.paint) };
        let tint = |id: InkId| t.iter().find(|(i, _)| *i == id).map_or(0.0, |(_, v)| (v * 1000.0).round() / 1000.0);
        (fi, inks.iter().map(|&id| tint(id)).collect())
    }

    #[test]
    fn half_black_over_cyan_composites_per_ink() {
        let l = list(
            InkSet::new(),
            vec![
                solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(C, 1.0)]),
                Item::Group(group(0.5, BlendMode::Normal, vec![solid(Rect::new(50.0, 0.0, 150.0, 100.0), &[(K, 1.0)])])),
            ],
        );
        let f = flat(&l);
        assert_eq!(at(&f, 75.0, 50.0, &[C, M, Y, K]).1, [0.5, 0.0, 0.0, 0.5]);
        assert_eq!(at(&f, 25.0, 50.0, &[C, K]).1, [1.0, 0.0]);
        assert_eq!(at(&f, 125.0, 50.0, &[C, K]).1, [0.0, 0.5]);
        assert_eq!((f.report.kept, f.report.flattened, f.report.regions, f.report.fills), (0, 2, 3, 3));
        assert!(f.complex.is_empty());
    }

    #[test]
    fn multiply_adds_spot_inks_and_non_separable_modes_are_normal_on_spots() {
        let mut inks = InkSet::new();
        let spot = |name: &str| Ink::Spot(Spot { name: name.into(), alternate: Alternate::Cmyk([0.0, 0.3, 1.0, 0.0]), solidity: None });
        let gold = inks.add(spot("Gold")).unwrap();
        let silver = inks.add(spot("Silver")).unwrap();
        let scene = |blend| {
            list(
                inks.clone(),
                vec![
                    solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(gold, 0.5)]),
                    Item::Group(group(1.0, blend, vec![solid(Rect::new(50.0, 0.0, 150.0, 100.0), &[(gold, 0.4), (silver, 0.6)])])),
                ],
            )
        };
        let f = flat(&scene(BlendMode::Multiply));
        assert_eq!(at(&f, 75.0, 50.0, &[gold, silver, K]).1, [0.7, 0.6, 0.0]);
        let f = flat(&scene(BlendMode::Hue));
        assert_eq!(at(&f, 75.0, 50.0, &[gold, silver, K]).1, [0.4, 0.6, 0.0]);
    }

    #[test]
    fn overprint_keeps_the_inks_a_paint_leaves_out() {
        let op = Overprint { fill: true, ..Overprint::default() };
        let opm = Overprint { nonzero: true, ..op };
        let cyan = || solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(C, 1.0)]);
        // A 50% yellow strip over both, so that neither is kept as it is.
        let veil = || Item::Group(group(0.5, BlendMode::Normal, vec![solid(Rect::new(0.0, 90.0, 150.0, 100.0), &[(Y, 1.0)])]));
        let top = |tints: &[(InkId, f32)], o| list(InkSet::new(), vec![cyan(), fill(Rect::new(50.0, 0.0, 150.0, 100.0), tints, o), veil()]);

        let f = flat(&top(&[(K, 1.0)], op));
        let (fi, v) = at(&f, 75.0, 50.0, &[C, M, Y, K]);
        assert_eq!(v, [1.0, 0.0, 0.0, 1.0], "K100 overprints cyan");
        assert_eq!(fi.overprint, Overprint::default(), "the composite needs no overprint");
        let (fi, _) = at(&f, 125.0, 50.0, &[K]);
        assert_eq!((fi.paint.clone(), fi.overprint), (Paint::Solid(InkColor::Tints(vec![(K, 1.0)])), op), "a paint shown as it is stays");
        assert_eq!(at(&flat(&top(&[(K, 1.0)], Overprint::default())), 75.0, 50.0, &[C, K]).1, [0.0, 1.0], "knocks out");

        let magenta = [(C, 0.0), (M, 1.0), (Y, 0.0), (K, 0.0)];
        assert_eq!(at(&flat(&top(&magenta, opm)), 75.0, 50.0, &[C, M]).1, [1.0, 1.0], "OPM 1: a zero component keeps the backdrop");
        assert_eq!(at(&flat(&top(&magenta, op)), 75.0, 50.0, &[C, M]).1, [0.0, 1.0], "OPM 0: zero is a value");
    }

    #[test]
    fn knockout_group_replaces_what_is_under_the_top_element() {
        let scene = |knockout| {
            let half_magenta = Item::Group(group(0.5, BlendMode::Normal, vec![solid(Rect::new(50.0, 0.0, 150.0, 100.0), &[(M, 1.0)])]));
            let g =
                GroupItem { knockout, ..group(1.0, BlendMode::Normal, vec![solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(C, 1.0)]), half_magenta]) };
            list(InkSet::new(), vec![Item::Group(g)])
        };
        assert_eq!(at(&flat(&scene(true)), 75.0, 50.0, &[C, M]).1, [0.0, 0.5]);
        assert_eq!(at(&flat(&scene(false)), 75.0, 50.0, &[C, M]).1, [0.5, 0.5]);
    }

    #[test]
    fn knockout_group_shows_its_backdrop_under_a_later_element() {
        let half_yellow = Item::Group(group(0.5, BlendMode::Normal, vec![solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(Y, 1.0)])]));
        let g =
            GroupItem { knockout: true, ..group(1.0, BlendMode::Normal, vec![solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(M, 1.0)]), half_yellow]) };
        let l = list(InkSet::new(), vec![solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(C, 1.0)]), Item::Group(g)]);
        assert_eq!(at(&flat(&l), 50.0, 50.0, &[C, M, Y]).1, [0.5, 0.0, 0.5]);
    }

    #[test]
    fn knockout_reaches_elements_inside_a_clip() {
        // A form's bounding box arrives as a clip around the knockout group's elements.
        let half_magenta = Item::Group(group(0.5, BlendMode::Normal, vec![solid(Rect::new(50.0, 0.0, 150.0, 100.0), &[(M, 1.0)])]));
        let clip = GroupItem {
            clip: Some(Rect::new(0.0, 0.0, 150.0, 100.0).to_path(0.1)),
            ..group(1.0, BlendMode::Normal, vec![solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(C, 1.0)]), half_magenta])
        };
        let g = GroupItem { knockout: true, ..group(1.0, BlendMode::Normal, vec![Item::Group(clip)]) };
        assert_eq!(at(&flat(&list(InkSet::new(), vec![Item::Group(g)])), 75.0, 50.0, &[C, M]).1, [0.0, 0.5]);
    }

    #[test]
    fn isolated_group_blends_without_the_backdrop() {
        let scene = |isolated| {
            let multiply = Item::Group(group(1.0, BlendMode::Multiply, vec![solid(Rect::new(50.0, 0.0, 150.0, 100.0), &[(M, 1.0)])]));
            let outer = GroupItem { isolated, ..group(1.0, BlendMode::Normal, vec![multiply]) };
            list(InkSet::new(), vec![solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(C, 1.0)]), Item::Group(outer)])
        };
        assert_eq!(at(&flat(&scene(false)), 75.0, 50.0, &[C, M]).1, [1.0, 1.0]);
        assert_eq!(at(&flat(&scene(true)), 75.0, 50.0, &[C, M]).1, [0.0, 1.0]);
    }

    #[test]
    fn opaque_items_away_from_transparency_and_above_it_stay_as_they_are() {
        let font = Arc::new(TextFont { data: Arc::new(vec![0_u8; 8]), index: 0, key: 7 });
        let text = |x: f64| {
            Item::Text(TextItem {
                font: Some(font.clone()),
                size: 12.0,
                transform: Affine::translate((x, 50.0)),
                glyphs: vec![PlacedGlyph { id: 3, origin: Point::ZERO, text: 0..1 }],
                text: Some("A".into()),
                outlines: Rect::new(x, 40.0, x + 8.0, 50.0).to_path(0.1),
                paint: Paint::Solid(InkColor::Tints(vec![(K, 1.0)])),
                overprint: Overprint { fill: true, ..Overprint::default() },
            })
        };
        let far = solid(Rect::new(300.0, 300.0, 400.0, 400.0), &[(M, 1.0)]);
        let l = list(
            InkSet::new(),
            vec![
                far.clone(),
                text(500.0),
                solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(C, 1.0)]),
                Item::Group(group(0.5, BlendMode::Normal, vec![solid(Rect::new(50.0, 0.0, 150.0, 100.0), &[(K, 1.0)])])),
                text(60.0),
            ],
        );
        let f = flat(&l);
        assert_eq!(f.items[0], far);
        assert_eq!(f.items[1], l.items[1]);
        assert_eq!(f.items.last(), Some(&l.items[4]), "text above the transparency, painted last");
        let Some(Item::Text(t)) = f.items.last() else { panic!() };
        assert!(Arc::ptr_eq(t.font.as_ref().unwrap(), &font));
        assert_eq!((f.report.kept, f.report.flattened, f.report.fills), (3, 2, 3));
    }

    #[test]
    fn opaque_members_with_no_transparency_above_them_stay_as_they_are() {
        // A background frame, body text on it, and a transparent object over the frame only: the
        // boxes put all three in one group, but only the frame is under transparency.
        let text = solid(Rect::new(10.0, 10.0, 50.0, 20.0), &[(K, 1.0)]);
        let veiled = |items: Vec<Item>| {
            let mut items = items;
            items.push(Item::Group(group(0.5, BlendMode::Normal, vec![solid(Rect::new(100.0, 100.0, 150.0, 150.0), &[(M, 1.0)])])));
            list(InkSet::new(), items)
        };
        let f = flat(&veiled(vec![solid(Rect::new(0.0, 0.0, 200.0, 200.0), &[(C, 0.2)]), text.clone()]));
        assert_eq!((f.report.kept, f.report.flattened), (1, 2));
        assert_eq!(f.items.last(), Some(&text), "the text is drawn as it is, over the fills");
        assert_eq!(at(&f, 125.0, 125.0, &[C, M]).1, [0.1, 0.5]);
        assert_eq!(at(&f, 5.0, 5.0, &[C, K]).1, [0.2, 0.0]);
        // Under the text, the frame's fill goes on.
        let fills = Flattened { items: f.items[..f.items.len() - 1].to_vec(), ..f.clone() };
        assert_eq!(at(&fills, 30.0, 15.0, &[C, K]).1, [0.2, 0.0]);

        // Under an opaque rectangle that is itself under the transparency, the text is flattened.
        let over = solid(Rect::new(40.0, 0.0, 120.0, 120.0), &[(Y, 1.0)]);
        let f = flat(&veiled(vec![text.clone(), over]));
        assert_eq!((f.report.kept, f.report.flattened), (0, 3));
        assert_eq!(at(&f, 20.0, 15.0, &[Y, K]).1, [0.0, 1.0]);
        assert_eq!(at(&f, 45.0, 15.0, &[Y, K]).1, [1.0, 0.0]);
    }

    #[test]
    fn art_hidden_under_an_opaque_fill_does_not_cut_it() {
        // K bars under a cyan fill, one where a half magenta overlaps it: three looks, three fills.
        let half_magenta = Item::Group(group(0.5, BlendMode::Normal, vec![solid(Rect::new(50.0, 0.0, 150.0, 100.0), &[(M, 1.0)])]));
        let l = list(
            InkSet::new(),
            vec![
                solid(Rect::new(10.0, 10.0, 20.0, 90.0), &[(K, 1.0)]),
                solid(Rect::new(60.0, 10.0, 70.0, 90.0), &[(K, 1.0)]),
                solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(C, 1.0)]),
                half_magenta,
            ],
        );
        let f = flat(&l);
        assert_eq!(f.report.fills, 3, "{:?}", f.items);
        assert_eq!(at(&f, 15.0, 50.0, &[C, K]).1, [1.0, 0.0]);
        assert_eq!(at(&f, 65.0, 50.0, &[C, M, K]).1, [0.5, 0.5, 0.0]);
    }

    #[test]
    fn kept_isolated_and_knockout_groups_come_back_plain() {
        // A placed page: an isolated group of opaque art, away from any transparency.
        let inner = GroupItem { knockout: true, ..group(1.0, BlendMode::Normal, vec![solid(Rect::new(0.0, 0.0, 10.0, 10.0), &[(C, 1.0)])]) };
        let placed = GroupItem { isolated: true, ..group(1.0, BlendMode::Normal, vec![Item::Group(inner)]) };
        let f = flat(&list(InkSet::new(), vec![Item::Group(placed)]));
        let Some(Item::Group(g)) = f.items.first() else { panic!("{:?}", f.items) };
        let Some(Item::Group(inner)) = g.children.first() else { panic!("{g:?}") };
        assert!(!g.isolated && !inner.knockout);
    }

    #[test]
    fn gradients_and_crowded_groups_go_to_complex() {
        let gradient = Item::Fill(FillItem {
            path: Rect::new(50.0, 0.0, 150.0, 100.0).to_path(0.1),
            rule: FillRule::NonZero,
            paint: Paint::Gradient {
                stops: vec![(0.0, InkColor::Tints(vec![(C, 0.0)])), (1.0, InkColor::Tints(vec![(C, 1.0)]))],
                geometry: GradientGeometry::Axial {
                    start: Point::new(50.0, 0.0),
                    end: Point::new(150.0, 0.0),
                    extend: [true; 2],
                    transform: Affine::IDENTITY,
                },
            },
            overprint: Overprint::default(),
        });
        let l = list(
            InkSet::new(),
            vec![
                solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(M, 1.0)]),
                Item::Group(group(0.5, BlendMode::Normal, vec![gradient])),
                solid(Rect::new(140.0, 0.0, 200.0, 100.0), &[(Y, 1.0)]),
            ],
        );
        let f = flat(&l);
        assert_eq!(f.report.complex_regions, 1, "touching regions merge: {:?}", f.complex);
        let both = f.complex.iter().find(|c| c.path.winding(Point::new(75.0, 50.0)) != 0).unwrap();
        assert_eq!(both.items, [0, 1]);
        assert_eq!(both.at, f.items.len() - 1, "before the yellow kept on top");
        assert_eq!(at(&f, 25.0, 50.0, &[M]).1, [1.0]);

        let f = flatten(&l, &FlattenOptions { max_regions: 2, ..FlattenOptions::default() }).unwrap();
        assert_eq!(f.report.complex_groups, [ComplexGroup { items: vec![0, 1], reason: ComplexReason::TooManyRegions(3) }]);
        assert_eq!((f.complex.len(), f.items.len()), (1, 1));
    }

    #[test]
    fn flattened_fills_keep_the_actual_text_they_show() {
        let tagged = GroupItem {
            actual_text: Some("fi".into()),
            ..group(1.0, BlendMode::Normal, vec![solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(C, 1.0)])])
        };
        let veil = group(0.5, BlendMode::Normal, vec![solid(Rect::new(50.0, 0.0, 150.0, 100.0), &[(K, 1.0)])]);
        let f = flat(&list(InkSet::new(), vec![Item::Group(tagged), Item::Group(veil)]));
        let groups: Vec<&GroupItem> = f
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Group(g) => Some(g),
                _ => None,
            })
            .collect();
        assert_eq!(groups.len(), 1, "{:?}", f.items);
        assert_eq!(groups[0].actual_text.as_deref(), Some("fi"));
        assert_eq!(groups[0].children.len(), 2, "the cyan alone and under the veil");
        assert_eq!((f.items.len(), f.report.fills), (2, 3));
        assert!(!is_transparent(&f.items[1]));
    }
}
