use egui::{Pos2, Rangef, Rect, Vec2, pos2};

use super::{Placement, Side, Target, Zone};

/// One visible group, supplied in back-to-front order to [`resolve_drop`]. An empty tab strip
/// or a group that rejects tabs may still accept side splits, e.g. a protected document canvas.
/// The dock area's docking profile ([`crate::docking::DockStyle::new_dock_zones`]) gives the
/// body of a frame or floating group that rejects tabs no target at all.
#[derive(Clone, Debug)]
pub struct DropTarget<P> {
    pub anchor: P,
    pub rect: Rect,
    pub tab_strip: Option<Rect>,
    pub tabs: Vec<(P, Rect)>,
    pub allow_splits: bool,
    pub accepts_tabs: bool,
    /// The strip runs right to left: the first tab is rightmost, and an insertion slot sits
    /// at the right edge of the tab it precedes.
    pub right_to_left: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DropProposal<P> {
    pub anchor: P,
    pub placement: Placement<P>,
    pub preview: Rect,
    pub insertion: Option<Rect>,
    /// The pointer addresses the tab's current insertion slot. The renderer also marks a drop
    /// that would leave every panel where it is, changing only sizes.
    pub noop: bool,
}

pub(super) fn usable(rect: Rect) -> bool {
    rect.is_finite() && rect.width() > 0.0 && rect.height() > 0.0 && rect.width() <= 1_000_000.0 && rect.height() <= 1_000_000.0
}

/// Depth of a group's side drop zones with [`crate::docking::DockStyle::side_edge_zones`]: the
/// `depth` that [`drop_zone`], [`zone_shape`] and [`resolve_drop`] take there.
pub const SIDE_EDGE_ZONE_DEPTH: f32 = 40.0;

/// Width of the band along the layout's outer edge with
/// [`crate::docking::DockStyle::side_edge_zones`], in place of
/// [`crate::docking::DockStyle::edge_band`].
pub(super) const SIDE_EDGE_BAND: f32 = 8.0;

/// The picture-frame zones of [`crate::docking::DockStyle::side_edge_zones`]. Tab header (the
/// top `tab_height` of `rect`) is the center; otherwise the nearest side zone of the body below
/// it wins. Every side zone reaches the same depth into the body: `depth`, or a
/// quarter of the body's smaller side when that is less, so a body always keeps a center and a
/// narrow body's top and bottom zones are no deeper than its left and right ones. A `depth` of
/// zero or less leaves only the center. Outside, non-finite, empty and inverted rectangles have
/// no target.
pub fn drop_zone(rect: Rect, tab_height: f32, depth: f32, pointer: Pos2) -> Option<Zone> {
    if !usable(rect) || !pointer.is_finite() || !rect.contains(pointer) || !tab_height.is_finite() || tab_height < 0.0 {
        return None;
    }
    let body = body(rect, tab_height);
    if pointer.y < body.top() || !usable(body) {
        return Some(Zone::Center);
    }
    let Vec2 { x, y } = side_depths(body, depth);
    let sides = [
        (pointer.x - body.left(), x, Zone::Left),
        (body.right() - pointer.x, x, Zone::Right),
        (pointer.y - body.top(), y, Zone::Top),
        (body.bottom() - pointer.y, y, Zone::Bottom),
    ];
    let nearest =
        sides.into_iter().filter(|(_, depth, _)| *depth > 0.0).map(|(gap, depth, zone)| (gap / depth, zone)).min_by(|a, b| a.0.total_cmp(&b.0));
    Some(match nearest {
        Some((distance, zone)) if distance <= 1.0 => zone,
        _ => Zone::Center,
    })
}

/// The zones of the default profile. Tab header (the top `tab_height` of `rect`) is the center.
/// In the body below it, the outer `fraction` of its width from the left or right edge, or of
/// its height from the top or bottom edge, is that side's zone; where two sides' zones meet, the
/// side the pointer is relatively nearer wins. The rest is the center. Without `center` (a group
/// that takes no tabs) the whole body goes to the relatively nearest side, as the dock area has
/// it without [`crate::docking::DockStyle::new_dock_zones`]; with it, such a body is no target.
/// A `fraction` of zero or less leaves only the center; more than one half acts as one half.
/// Outside, non-finite, empty and inverted rectangles have no target.
pub fn body_zone(rect: Rect, tab_height: f32, fraction: f32, center: bool, pointer: Pos2) -> Option<Zone> {
    body_zone_on(rect, tab_height, fraction, center, Sides::ALL, pointer)
}

/// [`body_zone`] where a body without `center` maps only to the nearest of `sides`, and to no
/// target without any.
pub(super) fn body_zone_on(rect: Rect, tab_height: f32, fraction: f32, center: bool, sides: Sides, pointer: Pos2) -> Option<Zone> {
    if !usable(rect) || !pointer.is_finite() || !rect.contains(pointer) || !tab_height.is_finite() || tab_height < 0.0 {
        return None;
    }
    let body = body(rect, tab_height);
    if pointer.y < body.top() || !usable(body) || fraction.is_nan() || fraction <= 0.0 {
        return Some(Zone::Center);
    }
    let fraction = fraction.min(0.5);
    let (w, h) = (body.width(), body.height());
    let allowed = |zone: &Zone| center || sides.contains(*zone);
    let (distance, zone) = [
        ((pointer.x - body.left()) / w, Zone::Left),
        ((body.right() - pointer.x) / w, Zone::Right),
        ((pointer.y - body.top()) / h, Zone::Top),
        ((body.bottom() - pointer.y) / h, Zone::Bottom),
    ]
    .into_iter()
    .filter(|(_, zone)| allowed(zone))
    .min_by(|a, b| a.0.total_cmp(&b.0))?;
    Some(if center && distance > fraction { Zone::Center } else { zone })
}

/// The preview line of a drop beside `rect` on `zone`'s side: `width` thick along that whole
/// edge, inside `rect`. None for the center.
pub(super) fn edge_line(rect: Rect, zone: Zone, width: f32) -> Option<Rect> {
    if !usable(rect) {
        return None;
    }
    let (w, h) = (width.max(1.0).min(rect.width()), width.max(1.0).min(rect.height()));
    Some(match zone {
        Zone::Left => Rect::from_x_y_ranges(rect.left()..=(rect.left() + w), rect.y_range()),
        Zone::Right => Rect::from_x_y_ranges((rect.right() - w)..=rect.right(), rect.y_range()),
        Zone::Top => Rect::from_x_y_ranges(rect.x_range(), rect.top()..=(rect.top() + h)),
        Zone::Bottom => Rect::from_x_y_ranges(rect.x_range(), (rect.bottom() - h)..=rect.bottom()),
        Zone::Center => return None,
    })
}

/// The part of a group drawn at `rect` below its tab header.
fn body(rect: Rect, tab_height: f32) -> Rect {
    Rect::from_min_max(pos2(rect.left(), rect.top() + tab_height.min(rect.height())), rect.max)
}

/// How deep the side zones of `body` reach in from its left and right edges (`x`) and its top
/// and bottom edges (`y`): the same on all four sides.
fn side_depths(body: Rect, depth: f32) -> Vec2 {
    let depth = if depth > 0.0 { depth } else { 0.0 };
    Vec2::splat(depth.min(body.width().min(body.height()) / 4.0))
}

/// The outline of a drop zone of the group drawn at `rect`, as [`drop_zone`] finds it with the
/// same `tab_height` and `depth`: a side's trapezoid (its outer edge the whole side of the body,
/// its inner edge on the center rectangle), or the center rectangle. The tab header, a grouping
/// zone too, is not part of any shape. None for a side without depth.
pub fn zone_shape(rect: Rect, tab_height: f32, depth: f32, zone: Zone) -> Option<Vec<Pos2>> {
    if !usable(rect) || !tab_height.is_finite() || tab_height < 0.0 {
        return None;
    }
    let body = body(rect, tab_height);
    let depths = side_depths(body, depth);
    let flat = match zone {
        Zone::Left | Zone::Right => depths.x <= 0.0,
        Zone::Top | Zone::Bottom => depths.y <= 0.0,
        Zone::Center => false,
    };
    if !usable(body) || flat {
        return None;
    }
    let inner = body.shrink2(depths);
    Some(match zone {
        Zone::Left => vec![body.left_top(), inner.left_top(), inner.left_bottom(), body.left_bottom()],
        Zone::Right => vec![inner.right_top(), body.right_top(), body.right_bottom(), inner.right_bottom()],
        Zone::Top => vec![body.left_top(), body.right_top(), inner.right_top(), inner.left_top()],
        Zone::Bottom => vec![inner.left_bottom(), inner.right_bottom(), body.right_bottom(), body.left_bottom()],
        Zone::Center => vec![inner.left_top(), inner.right_top(), inner.right_bottom(), inner.left_bottom()],
    })
}

/// Height of the tab header of `target` that [`resolve_drop`] treats as a grouping zone: from the
/// top of its rect to the bottom of its tab strip.
pub(super) fn header_height<P>(target: &DropTarget<P>) -> f32 {
    target.tab_strip.map(|strip| strip.intersect(target.rect)).filter(|strip| usable(*strip)).map_or(0.0, |strip| strip.bottom() - target.rect.top())
}

/// The region a split or group merge occupies: the half of `rect` on `zone`'s side, or all of
/// it. [`resolve_drop`] previews with it.
pub fn zone_rect(rect: Rect, zone: Zone) -> Option<Rect> {
    if !usable(rect) {
        return None;
    }
    let center = rect.min + rect.size() * 0.5;
    Some(match zone {
        Zone::Center => rect,
        Zone::Left => Rect::from_min_max(rect.min, pos2(center.x, rect.bottom())),
        Zone::Right => Rect::from_min_max(pos2(center.x, rect.top()), rect.max),
        Zone::Top => Rect::from_min_max(rect.min, pos2(rect.right(), center.y)),
        Zone::Bottom => Rect::from_min_max(pos2(rect.left(), center.y), rect.max),
    })
}

/// Bounded insertion marker for a logical slot in a measured tab strip. Hidden tabs
/// retain their order; their slot uses the next visible boundary or the visible strip end.
pub fn tab_insertion_marker<P: Eq>(target: &DropTarget<P>, before: Option<&P>) -> Option<Rect> {
    if target.tabs.len() > super::MAX_PANELS || !target.accepts_tabs || !usable(target.rect) {
        return None;
    }
    let strip = target.tab_strip?.intersect(target.rect);
    if !usable(strip) {
        return None;
    }
    let index = before.and_then(|before| target.tabs.iter().position(|(panel, _)| panel == before));
    let rtl = target.right_to_left;
    let leading = |rect: &Rect| if rtl { rect.right() } else { rect.left() };
    let visible = || target.tabs.iter().filter(|(_, rect)| usable(*rect)).map(|(_, rect)| rect);
    let x = index
        .and_then(|index| target.tabs.get(index..))
        .and_then(|tabs| tabs.iter().find(|(_, rect)| usable(*rect)).map(|(_, rect)| leading(rect)))
        .unwrap_or_else(|| {
            if rtl {
                visible().map(|rect| rect.left()).fold(strip.right(), f32::min).max(strip.left())
            } else {
                visible().map(|rect| rect.right()).fold(strip.left(), f32::max).min(strip.right())
            }
        });
    let width = strip.width().min(2.0);
    let left = (x - width * 0.5).max(strip.left()).min((strip.right() - width).max(strip.left()));
    Some(Rect::from_min_max(pos2(left, strip.top()), pos2((left + width).min(strip.right()), strip.bottom())))
}

/// Resolve a pointer against visible groups. The frontmost containing group occludes groups
/// behind it even if it rejects the drop. A strip reorders by midpoint; the body of the source
/// group is a no-op at center. The body's side zones are [`drop_zone`]'s with `zone_depth`
/// ([`SIDE_EDGE_ZONE_DEPTH`] in the docking area): one depth on all four sides, at most a quarter
/// of the body's smaller side. A split previews the half of the group it takes ([`zone_rect`]).
/// Floating groups normally set `allow_splits` to false. These are the split-tree zones; a host
/// that resolves drops for a [`crate::docking::DockStyle`] calls [`resolve_drop_by`] with its
/// [`crate::docking::DockStyle::zone_rule`].
pub fn resolve_drop<P: Clone + Eq>(panel: &P, pointer: Pos2, targets: &[DropTarget<P>], zone_depth: f32) -> Option<DropProposal<P>> {
    resolve_drop_by(panel, pointer, targets, ZoneRule::Depth(zone_depth))
}

/// A set of sides: of the layout's outer edge, or of a group's body
/// ([`crate::docking::DockStyle::drop_edges`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sides {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Sides {
    pub const ALL: Self = Self { left: true, right: true, top: true, bottom: true };
    pub const NONE: Self = Self { left: false, right: false, top: false, bottom: false };
    pub const LEFT_RIGHT: Self = Self { left: true, right: true, top: false, bottom: false };
    pub const TOP_BOTTOM: Self = Self { left: false, right: false, top: true, bottom: true };

    /// Whether `zone`'s side is in the set; never the center.
    pub fn contains(self, zone: Zone) -> bool {
        match zone {
            Zone::Left => self.left,
            Zone::Right => self.right,
            Zone::Top => self.top,
            Zone::Bottom => self.bottom,
            Zone::Center => false,
        }
    }
}

/// How [`resolve_drop_by`] divides a group's body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoneRule {
    /// [`drop_zone`] with this depth; a split previews the half it takes.
    Depth(f32),
    /// [`body_zone`] with this fraction; a split previews a `line` thick line along the
    /// group's edge on that side.
    Fraction { fraction: f32, line: f32 },
}

/// [`resolve_drop`] with the body zones of `rule`: [`crate::docking::DockStyle::zone_rule`]
/// gives the ones the dock area uses.
pub fn resolve_drop_by<P: Clone + Eq>(panel: &P, pointer: Pos2, targets: &[DropTarget<P>], rule: ZoneRule) -> Option<DropProposal<P>> {
    resolve_drop_on(panel, pointer, targets, rule, Sides::ALL)
}

/// [`resolve_drop_by`] where a group that takes no tabs splits only on `sides` with
/// [`ZoneRule::Fraction`], and with none is no target.
pub(super) fn resolve_drop_on<P: Clone + Eq>(
    panel: &P,
    pointer: Pos2,
    targets: &[DropTarget<P>],
    rule: ZoneRule,
    sides: Sides,
) -> Option<DropProposal<P>> {
    if !pointer.is_finite() || targets.len() > super::MAX_PANELS {
        return None;
    }
    let target = targets.iter().rev().find(|target| usable(target.rect) && target.rect.contains(pointer))?;
    if target.tabs.len() > super::MAX_PANELS {
        return None;
    }
    let anchor = if &target.anchor == panel { target.tabs.iter().find(|(p, _)| p != panel).map(|(p, _)| p).unwrap_or(panel) } else { &target.anchor };
    let strip = target.tab_strip.map(|strip| strip.intersect(target.rect)).filter(|r| usable(*r) && r.contains(pointer));
    if strip.is_some() {
        if !target.accepts_tabs {
            return None;
        }
        let ahead = |r: &Rect| if target.right_to_left { pointer.x > r.center().x } else { pointer.x < r.center().x };
        let before = target.tabs.iter().filter(|(_, r)| usable(*r)).find(|(_, r)| ahead(r));
        let before_panel = before.map(|(p, _)| p.clone());
        let insertion = tab_insertion_marker(target, before_panel.as_ref())?;
        let old_order: Vec<_> = target.tabs.iter().map(|(p, _)| p).collect();
        let mut order = old_order.clone();
        let noop = if before_panel.as_ref() == Some(panel) {
            old_order.contains(&panel)
        } else if old_order.contains(&panel) {
            order.retain(|p| *p != panel);
            let at = before_panel.as_ref().and_then(|before| order.iter().position(|p| *p == before)).unwrap_or(order.len());
            order.insert(at, panel);
            order == old_order
        } else {
            false
        };
        return Some(DropProposal {
            anchor: anchor.clone(),
            placement: Placement::Tab { before: before_panel },
            preview: target.rect,
            insertion: Some(insertion),
            noop,
        });
    }
    let header = header_height(target);
    let zone = match rule {
        _ if !target.allow_splits => Zone::Center,
        ZoneRule::Depth(depth) => drop_zone(target.rect, header, depth, pointer)?,
        ZoneRule::Fraction { fraction, .. } => body_zone_on(target.rect, header, fraction, target.accepts_tabs, sides, pointer)?,
    };
    if zone == Zone::Center && target.accepts_tabs && target.tabs.iter().any(|(p, _)| p == panel) {
        return Some(DropProposal {
            anchor: anchor.clone(),
            placement: Placement::Tab { before: Some(panel.clone()) },
            preview: target.rect,
            insertion: None,
            noop: true,
        });
    }
    if anchor == panel {
        return None;
    }
    if zone == Zone::Center {
        if !target.accepts_tabs || target.tabs.iter().any(|(p, _)| p == panel) {
            return None;
        }
    } else if !target.allow_splits {
        return None;
    }
    Some(DropProposal {
        anchor: anchor.clone(),
        placement: Placement::Split(zone),
        preview: match rule {
            ZoneRule::Fraction { line, .. } if zone != Zone::Center => edge_line(target.rect, zone, line)?,
            _ => zone_rect(target.rect, zone)?,
        },
        insertion: None,
        noop: false,
    })
}

/// [D] A dock drop target, built once per frame next to the [`DropTarget`]s. The pointer selects
/// it inside `band`; `line` is the edge it docks to.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct DockDrop<P> {
    pub(super) target: DockTarget<P>,
    pub(super) band: Rect,
    pub(super) line: Rect,
    /// A band along the window edge: a new dock, or a new column outside the outermost one.
    /// Its preview is `line`, not the rect the drop takes.
    pub(super) outer: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum DockTarget<P> {
    /// Column edge: a new column beside the column of `anchor`, further from the window edge
    /// when `after` (P5).
    ColumnEdge { anchor: P, after: bool },
    /// The seam between two stacked groups: a new group below the group of `anchor` (P4).
    Seam { anchor: P },
    /// The top or bottom zone of a group body, or the free room under a column's last group (P3).
    GroupBand { anchor: P, below: bool },
    /// A window edge without a dock, or an empty dock (P5, P6).
    NewDock { side: Side },
}

impl<P: Clone> DockTarget<P> {
    pub(super) fn to(&self) -> Target<P> {
        match self {
            Self::ColumnEdge { anchor, after } => Target::At { anchor: anchor.clone(), placement: Placement::Column { after: *after } },
            Self::Seam { anchor } => Target::At { anchor: anchor.clone(), placement: Placement::Group { below: true, height: None } },
            Self::GroupBand { anchor, below } => Target::At { anchor: anchor.clone(), placement: Placement::Group { below: *below, height: None } },
            Self::NewDock { side } => Target::NewDock { side: *side },
        }
    }

    /// Rank in resolution order: column edges reach past their columns, so they come first.
    fn rank(&self) -> u8 {
        match self {
            Self::ColumnEdge { .. } => 0,
            Self::Seam { .. } => 1,
            Self::GroupBand { .. } => 2,
            Self::NewDock { .. } => 3,
        }
    }
}

/// The dock target under the pointer: column edges, then seams, then group bands, then new docks.
pub(super) fn resolve_dock<P: Clone>(pointer: Pos2, targets: &[DockDrop<P>]) -> Option<&DockDrop<P>> {
    if !pointer.is_finite() {
        return None;
    }
    targets.iter().filter(|t| usable(t.band) && t.band.contains(pointer)).min_by_key(|t| t.target.rank())
}

/// How far from its line a border band reaches into a tab strip that takes tabs: the rest of
/// the strip joins the group.
pub(super) const STRIP_REACH: f32 = 4.0;

/// Band and line of a column edge at `x`: the band `band` wide, centred on the edge, and the
/// line `line` wide.
pub(super) fn column_edge(x: f32, ys: Rangef, band: f32, line: f32) -> (Rect, Rect) {
    let half = band.max(0.0) * 0.5;
    (Rect::from_x_y_ranges((x - half)..=(x + half), ys), vertical_line(x, ys, line))
}

pub(super) fn vertical_line(x: f32, ys: Rangef, width: f32) -> Rect {
    Rect::from_x_y_ranges((x - width * 0.5)..=(x + width * 0.5), ys)
}

pub(super) fn horizontal_line(y: f32, xs: Rangef, width: f32) -> Rect {
    Rect::from_x_y_ranges(xs, (y - width * 0.5)..=(y + width * 0.5))
}
