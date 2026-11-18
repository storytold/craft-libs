//! The drop-spot registry of a dock and the payload of a whole-group drag.
//!
//! Group and column rendering write `DockSpot`s with their hit rects while a pass runs; drop resolution reads the
//! previous pass's registry, because a drop is decided against what was on screen when the pointer moved. Every
//! zone is a rect. A spot names a group by an anchor panel, as the dock model does.

use super::{DragUnit, PanelKey, PanelSpec, scale_widths, stepped, width_bounds};
use crate::contract::DropMark;
use crate::runtime::frame::{self, PassStep, PerViewport, Stamped, Stored};
use crate::style::StyleCx;
use craft_behavior::DockSide;
use craft_behavior::dock::{Action, Column, DockEdge, Layout, Node, ZoneSize, node_at, zones};
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
    /// How far a band (edge, seam, column edge, new dock) reaches into a tab strip from its drop line (the least hit
    /// size): the rest of the strip joins the group.
    pub strip_reach: f32,
    /// The least extent of a pane along a split, and of a dock (the pane's least size).
    pub least_pane: f32,
    /// The height of a group's tab strip or section title row (the panel header height), which every group height includes.
    pub strip: f32,
    /// The height of a column bar (a small control).
    pub bar: f32,
    /// The height of a floating window's title row, which is the bar of the column the window holds: every height of
    /// a window's body or content is the window's height less this.
    pub title_row: f32,
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
    /// The sizes in the root scope's style, for code outside a style pass:
    /// `StyleCx::with_root(ctx, DockSizes::style)`.
    pub fn of(ctx: &Context) -> Self {
        StyleCx::with_root(ctx, DockSizes::style)
    }

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
            strip_reach: size.hit.target,
            least_pane: size.pane.least,
            strip: size.panel_header,
            bar: size.control.sm,
            title_row: super::floating::title_row(cx.tokens),
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
    /// floated iconic column use it): the stored width kept within the panels' limits, or a stepped column's width
    /// at its steps ([`stepped`]).
    pub fn expanded_width<P>(&self, column: &Column<P>, spec: &dyn Fn(&P) -> PanelSpec) -> f32 {
        if let Some((grid, steps)) = stepped(column, spec) {
            return grid.width(steps);
        }
        let (lo, hi) = self.width_bounds(column, spec);
        column.width.unwrap_or(self.column).clamp(lo, hi)
    }

    /// The bounds [`width`](Self::width) keeps a column in: an expanded column's [`width_bounds`](Self::width_bounds);
    /// a stepped one's width at one step up to its width at all its grid's columns; an iconic column's rail width up
    /// to the width it expands to.
    pub fn shown_bounds<P>(&self, column: &Column<P>, spec: &dyn Fn(&P) -> PanelSpec) -> (f32, f32) {
        if column.iconic {
            // Widening an iconic column stops at the width it expands to.
            (self.iconic_least, self.expanded_width(column, spec).max(self.iconic_least))
        } else if let Some((grid, _)) = stepped(column, spec) {
            (grid.width(1), grid.width(grid.most))
        } else {
            self.width_bounds(column, spec)
        }
    }

    /// The width of a column before the dock's room shrinks it.
    pub fn width<P>(&self, column: &Column<P>, spec: &dyn Fn(&P) -> PanelSpec) -> f32 {
        if column.iconic {
            let (lo, hi) = self.shown_bounds(column, spec);
            column.iconic_width.unwrap_or(lo).clamp(lo, hi)
        } else {
            self.expanded_width(column, spec)
        }
    }

    /// The actions that give the dock on `side` of `layout` the total width `total`, its columns scaled within
    /// their [`shown_bounds`](Self::shown_bounds) ([`scale_widths`]): [`Action::ResizeColumn`] for an expanded column,
    /// [`Action::ResizeIconic`] for an iconic one, which keeps its expanded width. A stepped column keeps its width,
    /// which only its steps set. Apply them together
    /// ([`Workspace::apply_all`](craft_behavior::dock::Workspace::apply_all)). Empty when the side has no dock or the
    /// columns already add up to the width; a column without panels keeps its width.
    pub fn resize_side<P: Clone + Eq + std::hash::Hash>(
        &self,
        layout: &Layout<P>,
        side: DockEdge,
        spec: &dyn Fn(&P) -> PanelSpec,
        total: f32,
    ) -> Vec<Action<P>> {
        let columns = match layout.dock_path(side).and_then(|path| node_at(layout.root.as_ref()?, &path)) {
            Some(Node::Dock { columns, .. }) => columns,
            _ => return Vec::new(),
        };
        let fixed = |column: &Column<P>| !column.iconic && stepped(column, spec).is_some();
        let current: Vec<(f32, (f32, f32))> = columns
            .iter()
            .map(|column| {
                let width = self.width(column, spec);
                (width, if fixed(column) { (width, width) } else { self.shown_bounds(column, spec) })
            })
            .collect();
        let widths = scale_widths(&current, total);
        columns
            .iter()
            .zip(current.iter().zip(widths))
            .filter(|(column, ((width, _), scaled))| !fixed(column) && (scaled - width).abs() >= 0.01)
            .filter_map(|(column, (_, width))| {
                let panel = column.groups.iter().find_map(|group| group.panels.first())?.clone();
                Some(if column.iconic { Action::resize_iconic(panel, Some(width)) } else { Action::resize_column(panel, width) })
            })
            .collect()
    }

    /// The `(width, floor)` of every column, for [`fit_widths`](super::fit_widths): an expanded column shrinks down to
    /// its least width, an iconic or stepped one keeps its width.
    pub fn natural<P>(&self, columns: &[Column<P>], spec: &dyn Fn(&P) -> PanelSpec) -> Vec<(f32, Option<f32>)> {
        columns
            .iter()
            .map(|column| {
                let keeps = column.iconic || stepped(column, spec).is_some();
                (self.width(column, spec), (!keeps).then(|| self.width_bounds(column, spec).0.max(1.0)))
            })
            .collect()
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
    /// sides of a group, then its middle. Over a tab strip a band takes the drop only near its line
    /// ([`DockTargets::register_band`]), so the strip keeps the rest of its height.
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

    /// What the drag ghost paints for a drop on this spot when the dock cannot say where the moved panels land
    /// (`DockDropTarget::placing`); `None` for [`DockSpot::Blocked`], which takes no drop.
    ///
    /// `rect` is the rect the spot applies to: the group's rect for [`DockSpot::Group`]; the rect it registered with
    /// (`DockTargets::register_in`, else its hit rect) for the others. A group's middle marks the group as the region
    /// the panels join; a side of it marks a line along that edge of the group (mirrored under `rtl`). A tab insertion
    /// marks a line across the hit rect's start edge. A band (edge, seam, column, new dock) marks a line along its
    /// middle.
    pub fn mark(&self, rect: Rect, rtl: bool) -> Option<DropMark> {
        let (start, end) = if rtl { (rect.right(), rect.left()) } else { (rect.left(), rect.right()) };
        let across = |x: f32| DropMark::line(Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom()));
        let along = |y: f32| DropMark::line(Pos2::new(rect.left(), y), Pos2::new(rect.right(), y));
        match self {
            DockSpot::Blocked => None,
            DockSpot::Group { side, .. } => Some(match side {
                DockSide::Start => across(start),
                DockSide::End => across(end),
                DockSide::Top => along(rect.top()),
                DockSide::Bottom => along(rect.bottom()),
                _ => DropMark::region(rect),
            }),
            DockSpot::Tab { .. } => self.tab_line(rect, rtl).map(|[from, to]| DropMark::line(from, to)),
            DockSpot::Edge(_) | DockSpot::Seam { .. } | DockSpot::Column { .. } | DockSpot::NewDock(_) => {
                let centre = rect.center();
                Some(if rect.width() <= rect.height() { across(centre.x) } else { along(centre.y) })
            }
        }
    }

    /// The tab line of a [`DockSpot::Tab`] at `rect`, the rect it registered with: across its start edge.
    pub(crate) fn tab_line(&self, rect: Rect, rtl: bool) -> Option<[Pos2; 2]> {
        let x = if rtl { rect.right() } else { rect.left() };
        matches!(self, DockSpot::Tab { .. }).then(|| [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())])
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
    /// The part of `rect` where the spot takes a drop over a tab strip: all of it, except for a band.
    over_strip: Rect,
}

/// The spots of one dock in one pass with their hit rects, and the room the dock laid its panels out in. Spots are
/// added back to front.
#[derive(Clone, Debug, PartialEq)]
pub struct DockTargets<P> {
    targets: Vec<Target<P>>,
    room: Option<Rect>,
}

impl<P> Default for DockTargets<P> {
    fn default() -> Self {
        DockTargets { targets: Vec::new(), room: None }
    }
}

impl<P: Clone> DockTargets<P> {
    fn new() -> Self {
        DockTargets::default()
    }

    /// Adds `spot`, taking drops over `rect`, over a tab strip only in `over_strip`, and marked against `applies`.
    /// An empty or non-finite `rect` takes none.
    fn add(&mut self, spot: DockSpot<P>, rect: Rect, applies: Rect, over_strip: Rect) {
        if rect.is_finite() && rect.is_positive() {
            self.targets.push(Target { spot, rect, applies, over_strip });
        }
    }

    /// The spot a drop at `at` lands on: among the spots whose rect holds `at`, the highest priority (see
    /// [`DockSpot`]), then the one added last. A [`DockSpot::Blocked`] hides the spots added before it, so a rect
    /// that takes no drop registers after what lies under it, with the rect its mark applies to. Where a tab strip
    /// ([`DockSpot::Tab`]) holds `at`, a band holds it only near its line. `None` where no spot holds `at` and where
    /// the winner is `Blocked`.
    fn at(&self, at: Pos2) -> Option<(DockSpot<P>, Rect)> {
        let hits: Vec<(usize, &Target<P>)> = self.targets.iter().enumerate().filter(|(_, target)| target.rect.contains(at)).collect();
        let hidden_below = hits.iter().rev().find(|(_, target)| matches!(target.spot, DockSpot::Blocked)).map(|(order, _)| *order);
        let shown: Vec<(usize, &Target<P>)> = hits.into_iter().filter(|(order, _)| hidden_below.is_none_or(|blocked| *order >= blocked)).collect();
        let on_strip = shown.iter().any(|(_, target)| matches!(target.spot, DockSpot::Tab { .. }));
        let winner = shown
            .into_iter()
            .filter(|(_, target)| !on_strip || target.over_strip.contains(at))
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

impl<P: Send + 'static> Stored for Pass<P> {}

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

    /// Runs `f` on the registry of `dock` in the running pass, made empty the first time.
    fn with_dock(ctx: &Context, dock: Id, f: impl FnOnce(&mut DockTargets<P>)) {
        Self::with_pass(ctx, |pass| {
            let at = match pass.now.iter().position(|(id, _)| *id == dock) {
                Some(at) => at,
                None => {
                    pass.now.push((dock, DockTargets::new()));
                    pass.now.len() - 1
                }
            };
            f(&mut pass.now[at].1);
        });
    }

    /// The room the dock `dock` laid its panels out in the previous pass ([`Self::register_room`]); `None` outside a
    /// pass and where the dock registered none.
    pub fn room(ctx: &Context, dock: Id) -> Option<Rect> {
        Self::with_pass(ctx, |pass| pass.before.iter().find(|(id, _)| *id == dock).and_then(|(_, targets)| targets.room)).flatten()
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

/// The registrations of a pass. A settled region's run record keeps each one to make it again, so the panel type is `Sync`.
impl<P: Clone + Send + Sync + 'static> DockTargets<P> {
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
        Self::journal(ctx, || {
            let spot = spot.clone();
            move |ctx: &Context| Self::register_in(ctx, dock, spot.clone(), hit, applies)
        });
        Self::with_dock(ctx, dock, |targets| targets.add(spot, hit, applies, hit));
    }

    /// Registers a band `spot` (an edge, seam, column edge or new dock) over `band`, whose drop line is `line`: over a
    /// tab strip the band takes the drop only within `reach` of the line (`DockSizes::strip_reach`), and the rest of
    /// the strip joins its group.
    pub fn register_band(ctx: &Context, dock: Id, spot: DockSpot<P>, band: Rect, line: Rect, reach: f32) {
        if !crate::runtime::measure::effect(ctx, "dock::register_target") {
            return;
        }
        Self::journal(ctx, || {
            let spot = spot.clone();
            move |ctx: &Context| Self::register_band(ctx, dock, spot.clone(), band, line, reach)
        });
        let near = line.expand(reach).intersect(band);
        Self::with_dock(ctx, dock, |targets| targets.add(spot, band, band, near));
    }

    /// Registers `room`, the rect the dock `dock` lays its panels out in this pass: the docked tree's rect for a dock
    /// area, the column's rect for a floating window. A drop's mark lays the panels out in it as they land
    /// ([`placed_rect`](super::placed_rect)).
    pub fn register_room(ctx: &Context, dock: Id, room: Rect) {
        if !crate::runtime::measure::effect(ctx, "dock::register_room") {
            return;
        }
        Self::journal(ctx, || move |ctx: &Context| Self::register_room(ctx, dock, room));
        Self::with_dock(ctx, dock, |targets| targets.room = Some(room));
    }

    /// Notes a registration in the records of the settling regions around the running code, so a region that settles
    /// registers its drop spots again when it replays its last run.
    fn journal<F: Fn(&Context) + Send + Sync + 'static>(ctx: &Context, make: impl FnOnce() -> F) {
        crate::runtime::journal(ctx, make);
    }
}

/// What a drag of a whole group, window or column carries, beside `PanelDrag` for one tab. Every such drag starts
/// through [`start_group_drag`](super::start_group_drag) or [`start_icon_drag`](super::start_icon_drag), so the type
/// is `non_exhaustive` and only the crate builds it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct GroupDrag<P> {
    /// What the drag moves, named by a panel of it.
    pub unit: DragUnit<P>,
    /// The size of the group, window or column where the drag started, which a float of it takes; `None` where
    /// that size is not the unit's own (an iconic column's cells).
    pub source: Option<Vec2>,
    /// The size of the icons the drag started on, for a unit dragged out of a collapsed column
    /// ([`start_icon_drag`](super::start_icon_drag)): a float of it lands collapsed, its strip under the pointer.
    pub strip: Option<Vec2>,
    /// The panel the unit shows, for the drag's preview.
    pub active: PanelKey,
    /// The arrangement the drag started on, when it started in a dock area.
    pub(crate) started: Option<super::drop::Arrangement>,
}
