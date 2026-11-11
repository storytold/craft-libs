//! The drop-spot registry of a dock and the payload of a whole-group drag.
//!
//! Group and column rendering write `DockSpot`s with their hit rects while a pass runs; drop resolution reads the
//! previous pass's registry, because a drop is decided against what was on screen when the pointer moved. Every
//! zone is a rect. A spot names a group by an anchor panel, as the dock model does.

use super::{DragUnit, PanelKey, PanelSpec, width_bounds};
use crate::contract::DropMark;
use crate::runtime::frame::{self, PassStep, PerViewport, Stamped};
use crate::style::StyleCx;
use craft_behavior::dock::{Column, ZoneSize, zones};
use craft_behavior::{DockSide, Flow};
use egui::{Context, Id, Pos2, Rect, Vec2};

pub use craft_behavior::dock::{SIDE_ZONE_DEPTH_SHARE, SIDE_ZONE_FRACTION};

/// The sizes of a dock's drop bands and zones and of the windows it floats, resolved from the theme's tokens.
/// A dock's component style calls [`Self::style`] from `Component::style`, so the sizes are cached with it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DockSizes {
    /// Depth of a group's side zones with `DockConfig::side_edge_zones` (the rail width).
    pub zone_depth: f32,
    /// Width of the band along the layout's outer edge that splits at the root (the standard control height).
    pub edge_band: f32,
    /// Width of the outer edge band with `DockConfig::side_edge_zones` (the least hit size).
    pub side_edge_band: f32,
    /// Height of the band centred on the seam between two stacked groups in a dock column (the standard control
    /// height).
    pub seam_band: f32,
    /// Width of the band centred on each edge of a dock column, where a drop makes a new column (the standard
    /// control height).
    pub column_edge_band: f32,
    /// Width of the band along the docked area's start and end edges, and inside an empty dock, that makes a new
    /// dock (the standard control height).
    pub new_dock_band: f32,
    /// Offset of a floated unit from the area's top-left corner, and of each further one from the last (the float
    /// cascade size).
    pub cascade_inset: f32,
    /// The size a command floats a panel at when the panel gives none (the float window size).
    pub float_size: Vec2,
    /// The divider between two docked groups or between a dock and its neighbour (the inline gap).
    pub gap: f32,
    /// The least extent of a pane along a split, and of a dock (the pane's least size).
    pub least_pane: f32,
    /// The height of a group's tab strip or section title row (the panel header height), which every group height includes.
    pub strip: f32,
    /// The height of a column bar (a small control).
    pub bar: f32,
    /// The least body size of a column's group and the least width of an expanded column (the panel header height).
    pub least: f32,
    /// How thick a divider is across its boundary (the least hit size).
    pub reach: f32,
    /// One arrow press on a divider (the control gap).
    pub step: f32,
    /// One Shift-arrow or Page press on a divider (the standard control height).
    pub jump: f32,
    /// The least width of an iconic column (the rail width).
    pub iconic_least: f32,
    /// The width of an expanded column the user has not resized (the dock column width).
    pub column: f32,
}

impl DockSizes {
    /// The sizes of the style context `cx`.
    pub fn style(cx: &StyleCx<'_>) -> Self {
        let size = &cx.tokens.size;
        DockSizes {
            zone_depth: size.rail.width,
            edge_band: size.control.md,
            side_edge_band: size.hit.target,
            seam_band: size.control.md,
            column_edge_band: size.control.md,
            new_dock_band: size.control.md,
            cascade_inset: size.window.cascade,
            float_size: Vec2::new(size.float.width, size.float.height),
            gap: cx.tokens.space.gap.inline,
            least_pane: size.pane.least,
            strip: size.panel_header,
            bar: size.control.sm,
            least: size.panel_header,
            reach: size.hit.target,
            step: cx.control().gap,
            jump: cx.control().height,
            iconic_least: size.rail.width,
            column: size.dock.column,
        }
    }

    /// The width bounds of an expanded column from the limits of its panels.
    pub fn width_bounds<P>(&self, column: &Column<P>, spec: &dyn Fn(&P) -> PanelSpec) -> (f32, f32) {
        let limits = column.groups.iter().flat_map(|group| group.panels.iter()).map(|panel| spec(panel).limits);
        width_bounds(self.least, limits)
    }

    /// The width an expanded column's panels draw at, whether the column is expanded or iconic (its flyout and a
    /// floated iconic column use it): the stored width kept within the panels' limits.
    pub fn expanded_width<P>(&self, column: &Column<P>, spec: &dyn Fn(&P) -> PanelSpec) -> f32 {
        let (lo, hi) = self.width_bounds(column, spec);
        column.width.unwrap_or(self.column).clamp(lo, hi)
    }

    /// The width of a column before the dock's room shrinks it.
    pub fn width<P>(&self, column: &Column<P>, spec: &dyn Fn(&P) -> PanelSpec) -> f32 {
        if column.iconic {
            // Widening an iconic column stops at the width it expands to.
            let most = self.expanded_width(column, spec).max(self.iconic_least);
            column.iconic_width.unwrap_or(self.iconic_least).clamp(self.iconic_least, most)
        } else {
            self.expanded_width(column, spec)
        }
    }

    /// The `(width, floor)` of every column, for [`fit_widths`](super::fit_widths): an expanded column shrinks down to
    /// its least width, an iconic one keeps its width.
    pub fn natural<P>(&self, columns: &[Column<P>], spec: &dyn Fn(&P) -> PanelSpec) -> Vec<(f32, Option<f32>)> {
        columns.iter().map(|column| (self.width(column, spec), (!column.iconic).then(|| self.width_bounds(column, spec).0.max(1.0)))).collect()
    }
}

/// Where a drop would land. A group is named by an anchor: any panel in it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DockSpot<P> {
    /// A tab insertion in the group of `anchor`, before the tab at `index` (the tab count for the end). The hit
    /// rect starts where the new tab would.
    #[non_exhaustive]
    Tab {
        /// A panel of the group whose strip takes the tab.
        anchor: P,
        /// The position the tab takes among the group's tabs.
        index: usize,
    },
    /// A side of the group of `anchor`: beside it, or into its tabs for `DockSide::Center`.
    #[non_exhaustive]
    Group {
        /// A panel of the group.
        anchor: P,
        /// The side.
        side: DockSide,
    },
    /// The layout's outer edge on `side`: a new root split.
    Edge(DockSide),
    /// The seam between two stacked groups of a dock column: a new group between them.
    #[non_exhaustive]
    Seam {
        /// A panel of the group above the seam.
        before: P,
        /// A panel of the group below it.
        after: P,
    },
    /// An edge of the dock column that holds the group of `anchor`: a new column beside it.
    #[non_exhaustive]
    Column {
        /// A panel of a group of the column.
        anchor: P,
        /// The column's side.
        side: DockSide,
    },
    /// A new dock along the docked area's start or end edge.
    NewDock(DockSide),
    /// A rect that takes no drop and hides the spots registered before it, such as a canvas that takes none.
    Blocked,
}

impl<P> DockSpot<P> {
    /// A tab insertion in the group of `anchor` before the tab at `index`.
    pub const fn tab(anchor: P, index: usize) -> Self {
        DockSpot::Tab { anchor, index }
    }

    /// The `side` of the group of `anchor`.
    pub const fn group(anchor: P, side: DockSide) -> Self {
        DockSpot::Group { anchor, side }
    }

    /// The seam between the stacked groups of `before` and `after`.
    pub const fn seam(before: P, after: P) -> Self {
        DockSpot::Seam { before, after }
    }

    /// The `side` edge of the dock column that holds the group of `anchor`.
    pub const fn column(anchor: P, side: DockSide) -> Self {
        DockSpot::Column { anchor, side }
    }

    /// Which spot wins where rects overlap: the outer bands, then seams and columns, then a tab strip, then the
    /// sides of a group, then its middle. A border band overlaps a tab strip only by its own thickness, so the
    /// strip keeps the rest of its height.
    const fn priority(&self) -> u8 {
        match self {
            DockSpot::Blocked => 0,
            DockSpot::Group { side: DockSide::Center, .. } => 1,
            DockSpot::Group { .. } => 2,
            DockSpot::Tab { .. } => 3,
            DockSpot::Seam { .. } | DockSpot::Column { .. } => 4,
            DockSpot::Edge(_) | DockSpot::NewDock(_) => 5,
        }
    }

    /// What the drag ghost paints for a drop on this spot; `None` for [`DockSpot::Blocked`], which takes no drop.
    ///
    /// `rect` is the rect the spot applies to: the group's rect for [`DockSpot::Group`], whose side marks the half
    /// of it a docked panel would take (mirrored under `rtl`); the rect it registered with (`DockTargets::register_in`, else its hit rect) for the others. A tab
    /// insertion marks a line across the hit rect's start edge. A band (edge, seam, column, new dock) marks a line
    /// along its middle.
    pub fn mark(&self, rect: Rect, rtl: bool) -> Option<DropMark> {
        match self {
            DockSpot::Blocked => None,
            DockSpot::Group { side: DockSide::Center, .. } => Some(DropMark::Into(rect)),
            DockSpot::Group { side, .. } => Some(DropMark::Split(Flow::new(rect, rtl).to_physical(side.area(rect)))),
            DockSpot::Tab { .. } => {
                let x = if rtl { rect.right() } else { rect.left() };
                Some(DropMark::line(Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())))
            }
            DockSpot::Edge(_) | DockSpot::Seam { .. } | DockSpot::Column { .. } | DockSpot::NewDock(_) => {
                let centre = rect.center();
                Some(if rect.width() <= rect.height() {
                    DropMark::line(Pos2::new(centre.x, rect.top()), Pos2::new(centre.x, rect.bottom()))
                } else {
                    DropMark::line(Pos2::new(rect.left(), centre.y), Pos2::new(rect.right(), centre.y))
                })
            }
        }
    }
}

/// How a group's body divides into side zones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ZoneRule {
    /// Zones `DockSizes::zone_depth` deep, at most `SIDE_ZONE_DEPTH_SHARE` of the body's smaller side.
    Depth,
    /// Zones `SIDE_ZONE_FRACTION` of the body's width (start, end) and height (top, bottom).
    Fraction,
}

impl ZoneRule {
    /// The zones of `body`, the part of a group below its tab strip, by [`craft_behavior::dock::zones`]: `Start` is
    /// on the right when `rtl`.
    pub fn zones(self, body: Rect, sizes: &DockSizes, rtl: bool) -> [(DockSide, Rect); 5] {
        let size = match self {
            ZoneRule::Depth => ZoneSize::Depth(sizes.zone_depth),
            ZoneRule::Fraction => ZoneSize::Fraction,
        };
        zones(body, size, rtl)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Target<P> {
    spot: DockSpot<P>,
    /// Where a drop lands on the spot.
    rect: Rect,
    /// The rect the spot's mark applies to (see [`DockSpot::mark`]).
    applies: Rect,
}

/// The spots of one dock in one pass with their hit rects. Spots are added back to front.
#[derive(Clone, Debug, PartialEq)]
pub struct DockTargets<P> {
    targets: Vec<Target<P>>,
}

impl<P> Default for DockTargets<P> {
    fn default() -> Self {
        DockTargets { targets: Vec::new() }
    }
}

impl<P: Clone> DockTargets<P> {
    fn new() -> Self {
        DockTargets::default()
    }

    /// Adds `spot`, taking drops over `rect` and marked against `applies`. An empty or non-finite `rect` takes none.
    fn add(&mut self, spot: DockSpot<P>, rect: Rect, applies: Rect) {
        if rect.is_finite() && rect.is_positive() {
            self.targets.push(Target { spot, rect, applies });
        }
    }

    /// The spot a drop at `at` lands on: among the spots whose rect holds `at`, the highest priority (see
    /// [`DockSpot`]), then the one added last. A [`DockSpot::Blocked`] hides the spots added before it, so a rect
    /// that takes no drop registers after what lies under it, with the rect its mark applies to. `None` where no
    /// spot holds `at` and where the winner is `Blocked`.
    fn at(&self, at: Pos2) -> Option<(DockSpot<P>, Rect)> {
        let hits: Vec<(usize, &Target<P>)> = self.targets.iter().enumerate().filter(|(_, target)| target.rect.contains(at)).collect();
        let hidden_below = hits.iter().rev().find(|(_, target)| matches!(target.spot, DockSpot::Blocked)).map(|(order, _)| *order);
        let winner = hits
            .into_iter()
            .filter(|(order, _)| hidden_below.is_none_or(|blocked| *order >= blocked))
            .max_by_key(|(order, target)| (target.spot.priority(), *order))?;
        match &winner.1.spot {
            DockSpot::Blocked => None,
            spot => Some((spot.clone(), winner.1.applies)),
        }
    }
}

/// The registries of the running pass and of the pass before, one per dock.
struct Pass<P> {
    now: Vec<(Id, DockTargets<P>)>,
    before: Vec<(Id, DockTargets<P>)>,
}

impl<P> Default for Pass<P> {
    fn default() -> Self {
        Pass { now: Vec::new(), before: Vec::new() }
    }
}

impl<P: Clone + Send + 'static> DockTargets<P> {
    /// Runs `f` on the registries of the running pass; `None` outside a pass.
    fn with_pass<R>(ctx: &Context, f: impl FnOnce(&mut Pass<P>) -> R) -> Option<R> {
        let facts = frame::running(ctx)?;
        Some(PerViewport::<Stamped<Pass<P>>>::with_pass(
            ctx,
            facts.viewport,
            facts.pass_nr,
            |pass, step| {
                pass.before = match step {
                    PassStep::Next => std::mem::take(&mut pass.now),
                    PassStep::Skipped => Vec::new(),
                };
                pass.now.clear();
            },
            f,
        ))
    }

    /// Registers `spot` over `rect` for the dock `dock` in the running pass of this viewport. Group and column
    /// rendering call it; it does nothing outside a pass. A floating window registers on its own egui layer, and
    /// the drag session orders the layers, so no registry orders them.
    pub fn register(ctx: &Context, dock: Id, spot: DockSpot<P>, rect: Rect) {
        Self::register_in(ctx, dock, spot, rect, rect);
    }

    /// Registers `spot` taking drops over `hit`, with its mark drawn against `applies`: the group's rect for a side
    /// zone of a group, the boundary between two tabs for a tab insertion.
    pub fn register_in(ctx: &Context, dock: Id, spot: DockSpot<P>, hit: Rect, applies: Rect) {
        if !crate::runtime::measure::effect(ctx, "dock::register_target") {
            return;
        }
        Self::with_pass(ctx, |pass| {
            let at = match pass.now.iter().position(|(id, _)| *id == dock) {
                Some(at) => at,
                None => {
                    pass.now.push((dock, DockTargets::new()));
                    pass.now.len() - 1
                }
            };
            pass.now[at].1.add(spot, hit, applies);
        });
    }

    /// The spot a drop at `at` lands on in the dock `dock`, by the spots of the previous pass, with the rect its
    /// mark applies to (`DockSpot::mark` takes it); `None` outside a pass.
    ///
    /// A dock calls it only inside its own `DropTarget::accept`, so the drag session arbitrates between the dock
    /// and an app's own `DropTarget<PanelDrag>` in a panel body, and between docks, by layer order and
    /// registration order.
    pub fn resolve(ctx: &Context, dock: Id, at: Pos2) -> Option<(DockSpot<P>, Rect)> {
        Self::with_pass(ctx, |pass| pass.before.iter().find(|(id, _)| *id == dock).and_then(|(_, targets)| targets.at(at))).flatten()
    }
}

/// What a drag of a whole group, window or column carries, beside `PanelDrag` for one tab. Every such drag starts
/// through [`start_group_drag`](super::start_group_drag), so the type is `non_exhaustive` and only the crate builds it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct GroupDrag<P> {
    /// What the drag moves, named by a panel of it.
    pub unit: DragUnit<P>,
    /// The panel the unit shows, for the drag's preview.
    pub active: PanelKey,
    /// The arrangement the drag started on, when it started in a dock area.
    pub(crate) started: Option<super::drop::Arrangement>,
}
