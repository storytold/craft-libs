//! `DockColumn`: a column of a side dock with its column bar and stacked groups, and the pure size rules of columns.
//!
//! The column is one container. Its bar is a `TitleRow` (a disclosure that collapses the column to icons and a menu
//! action) over a drag band; the groups below it stack at the heights `group_heights` gives and are shown by the
//! caller's group view. The column registers its edge bands and seams as drop spots of the dock and reports what the
//! user did; the dock decides what a drop changes.

use super::divider::{self, Divider, Drag};
use super::group::GroupGrip;
use super::{DockSizes, DockSpot, DockTargets, DockWords, DragUnit, PanelKey, PanelSpec, Splitter};
use crate::contract::{
    Access, BodyCx, Common, Component, Container, ContainerProps, Cx, NoValue, Outcome, Pad, PaintUi, PartsCx, Props as _, Role, ShowContainer as _,
    Sizing, WordSetCx as _,
};
use crate::menu::GroupItem;
use crate::runtime::env::Label;
use crate::runtime::focus::KeyUse;
use crate::style::{ComponentStyle, Corners, StyleCx, SurfaceKind};
use craft_behavior::divider::{self as push, Segment};
use craft_behavior::dock::{Column, DockEdge, Group};
use craft_behavior::{Axis, DockSide, Flow, Metrics, PartKey, Sense, Value};
use egui::{Context, Id, Pos2, Rect, Response, Ui};
use std::hash::Hash;

#[cfg(feature = "testing")]
mod stories;

mod bar;
mod sizing;

use bar::panel_names;
pub(super) use bar::{BarRecord, bar_name, show_bar};
use sizing::group_sizes;
pub use sizing::{GroupSize, fit_widths, group_heights, stacked, width_bounds};

const DIVIDER: PartKey = PartKey::new("divider");
const EDGE: PartKey = PartKey::new("edge");

/// The caller's view of one group: the group, the rect it takes and the body step's context. The dock passes the
/// group to `DockPanels::group_view` or shows a `DockGroup` in it.
pub type GroupView<'a, P> = Box<dyn FnMut(&Group<P>, Rect, &mut Ui, &mut BodyCx<'_>) + 'a>;

/// What a dock column reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct DockColumnOutput<P> {
    /// The column's response.
    pub response: Response,
    /// The rect the column took, for `DockAreaOutput::columns`.
    pub placed: Rect,
    /// The user asked to collapse the column to icons (`true`); the host sets `Column::iconic`.
    pub iconic: Option<bool>,
    /// The dock row the user picked in the column menu.
    pub picked: Option<GroupItem>,
    /// The groups whose height the user changed this pass, each named by its first panel, with the new height.
    pub resized: Vec<(P, f32)>,
    /// The width the user gave the column on its inner edge this pass.
    pub width: Option<f32>,
}

impl<P> Outcome for DockColumnOutput<P> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The style of a dock column: the sizes its bar, groups and drop bands take.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct DockColumnStyle {
    #[style(metrics)]
    metrics: Metrics,
    sizes: DockSizes,
}

/// One column of a side dock: the column bar over the stacked groups of `column`. Show it with
/// [`show_groups`](DockColumn::show_groups); the closure shows one group in the rect it is given, through
/// `DockPanels::group_view` or a `DockGroup`. A double click on the bar collapses the column to icons; the bar's
/// drag starts the move of the column through the drag session; Enter on the bar's menu button, the Menu key and a right
/// click open the column menu, whose dock rows come back in `picked`. A divider between two open groups resizes
/// them by pointer or with the arrow keys.
///
/// The column registers the drop bands of its two edges (`DockSpot::column`) and of the seams between its groups
/// (`DockSpot::seam`) with the dock `dock` every pass. A dock hidden by the shell shows nothing.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container, probe(DockColumn<'static, u8>))]
pub struct DockColumn<'a, P: Value + Hash + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    dock: Id,
    column: &'a Column<P>,
    spec: &'a dyn Fn(&P) -> PanelSpec,
    /// Whether Auto-Collapse Iconic Panels is on, which the column menu shows as checked.
    #[prop]
    auto_collapse: bool,
    /// The window edge the column's dock sits on; the column's inner edge faces away from it.
    #[prop]
    edge: DockEdge,
    /// The rect the dock may grow within, which bounds how wide its inner edge can make the column.
    #[prop]
    within: Rect,
    /// The plan `parts` laid out this pass, which update, paint and access read.
    laid: Option<Plan>,
    resized: Vec<(P, f32)>,
    width: Option<f32>,
}

impl<'a, P: Value + Hash + 'static> DockColumn<'a, P> {
    /// A column named `label` of the dock `dock`, over `column`; `spec` reads each panel's limits and heights.
    pub fn new(label: impl Into<Label>, dock: Id, column: &'a Column<P>, spec: &'a dyn Fn(&P) -> PanelSpec) -> Self {
        let props = ContainerProps::new().sizing(Sizing::Fill).pad(Pad::None);
        DockColumn {
            common: Common::new(),
            props,
            dock,
            column,
            spec,
            auto_collapse: false,
            edge: DockEdge::Start,
            within: Rect::NOTHING,
            laid: None,
            resized: Vec::new(),
            width: None,
        }
        .label(label)
    }

    /// Shows the column; `view` shows one group in the rect it gets.
    pub fn show_groups(self, ui: &mut Ui, view: impl FnMut(&Group<P>, Rect, &mut Ui, &mut BodyCx<'_>) + 'a) -> DockColumnOutput<P> {
        self.show_groups_in(ui, None, view)
    }

    /// Shows the column at `rect` with no allocation, so its paint is clipped by the dock and not by a region of its
    /// own: the accent line of its edge reaches into the gutter beside it.
    #[doc(hidden)]
    pub fn show_groups_at(self, ui: &mut Ui, rect: Rect, view: impl FnMut(&Group<P>, Rect, &mut Ui, &mut BodyCx<'_>) + 'a) -> DockColumnOutput<P> {
        self.show_groups_in(ui, Some(rect), view)
    }

    fn show_groups_in(
        self,
        ui: &mut Ui,
        at: Option<Rect>,
        mut view: impl FnMut(&Group<P>, Rect, &mut Ui, &mut BodyCx<'_>) + 'a,
    ) -> DockColumnOutput<P> {
        let (column, spec, auto_collapse, edge) = (self.column, self.spec, self.auto_collapse, self.edge);
        let mut bar = BarRecord::default();
        let shown = &mut bar;
        let body = move |ui: &mut Ui, body: &mut BodyCx<'_>| {
            if body.shell().is_some_and(|shell| !shell.dock_visible()) {
                return;
            }
            let Some(sizes) = body.style::<DockColumn<'_, P>>().map(|style| style.sizes) else { return };
            // The bar and the groups at the heights this pass's drag leaves them, in the measuring run and the real
            // run alike: the same plan `parts` lays out, so the dividers sit at the seams.
            let drags = |key: u64, divider: &Divider| divider::drag(body.part_press(DIVIDER.at(key)).phase, body.rect, false, divider);
            let plan = column_plan(column, spec, body.rect, &sizes, drags);
            if let Some(grip) = column_grip(column, spec, body) {
                *shown = show_bar(ui, body, plan.bar, true, auto_collapse, edge, grip);
            }
            for (group, rect) in column.groups.iter().zip(&plan.groups) {
                view(group, *rect, ui, body);
            }
        };
        let mut output = match at {
            Some(rect) => self.show_at(ui, rect, body),
            None => self.show(ui, body),
        }
        .output;
        output.iconic = bar.iconic;
        output.picked = bar.picked;
        output
    }

    /// The divider on the column's inner edge, which moves between the width bounds of its panels.
    fn edge_divider(&self, rect: Rect, sizes: &DockSizes) -> Divider {
        let (least, most) = sizes.width_bounds(self.column, self.spec);
        Divider::column_edge(rect, self.edge, (least, most.min((self.within.width() - sizes.least_pane).max(rect.width()))), sizes.reach)
    }

    /// Takes the width the coordinate `coord` of the inner edge asks for, kept within the divider's travel.
    fn move_edge(&mut self, rect: Rect, divider: &Divider, coord: f32) {
        let width = Divider::column_width(rect, self.edge, coord.clamp(divider.lo, divider.hi));
        if (width - rect.width()).abs() > f32::EPSILON {
            self.width = Some(width);
        }
    }

    fn anchor(&self, group: usize) -> Option<&P> {
        self.column.groups.get(group).and_then(|group| group.panels.first())
    }
}

/// The bar and body of the column in `rect`, and the groups stacked in the body, with the drags `drags` makes of
/// the dividers between open groups (the divider's part index and the divider as laid out before the drag).
fn column_plan<P: Value + Hash>(
    column: &Column<P>,
    spec: &dyn Fn(&P) -> PanelSpec,
    rect: Rect,
    sizes: &DockSizes,
    drags: impl Fn(u64, &Divider) -> Option<Drag>,
) -> Plan {
    let bar = Rect::from_min_max(rect.min, Pos2::new(rect.right(), (rect.top() + sizes.bar).min(rect.bottom())));
    let body = Rect::from_min_max(Pos2::new(rect.left(), bar.bottom()), rect.max);
    body_plan(column, spec, bar, body, sizes, drags)
}

/// [`column_plan`] for a column whose `bar` and `body` rects are known.
fn body_plan<P: Value + Hash>(
    column: &Column<P>,
    spec: &dyn Fn(&P) -> PanelSpec,
    bar: Rect,
    body: Rect,
    sizes: &DockSizes,
    drags: impl Fn(u64, &Divider) -> Option<Drag>,
) -> Plan {
    let group_sizes = group_sizes(column, spec, sizes);
    let heights = group_heights(&group_sizes, sizes.strip, body.height());
    let groups = stacked(body, &heights);
    let open = (0..group_sizes.len()).filter(|at| !group_sizes[*at].minimized).collect();
    let mut plan = Plan { bar, body, groups, heights, mins: group_sizes.iter().map(|size| size.min).collect(), open };
    for (key, index, divider) in plan.dividers(body, sizes.reach) {
        if let Some(Drag::To(coord)) = drags(key, &divider) {
            plan.move_divider(index, coord - divider.at);
        }
    }
    plan
}

/// Where the parts of a column sit.
struct Plan {
    bar: Rect,
    body: Rect,
    groups: Vec<Rect>,
    heights: Vec<f32>,
    mins: Vec<f32>,
    open: Vec<usize>,
}

impl Plan {
    /// Moves the divider after the `index`-th open group by `delta` points; the groups are stacked at their new heights.
    fn move_divider(&mut self, index: usize, delta: f32) {
        let row = self.row();
        let at: f32 = row.iter().take(index + 1).map(|segment| segment.extent).sum();
        let sizes = push::resize(&row, index, at + delta);
        for (group, next) in self.open.clone().into_iter().zip(sizes) {
            self.heights[group] = next;
        }
        self.groups = stacked(self.body, &self.heights);
    }

    /// The open groups as a row of segments, for the push rule of the dividers between them.
    fn row(&self) -> Vec<Segment> {
        self.open.iter().filter_map(|at| Some(Segment::unbounded(*self.heights.get(*at)?, *self.mins.get(*at)?))).collect()
    }

    /// The divider after each open group but the last: its part index (the group above it), its position among the open
    /// groups and the divider.
    fn dividers(&self, rect: Rect, reach: f32) -> Vec<(u64, usize, Divider)> {
        let row = self.row();
        self.open
            .windows(2)
            .enumerate()
            .filter_map(|(index, pair)| {
                let at = self.groups.get(pair[0])?.bottom();
                let (lo, hi) = push::travel(&row, index);
                Some((pair[0] as u64, index, Divider::new(Axis::Y, at, rect, (at + lo, at + hi), reach)))
            })
            .collect()
    }
}

impl<P: Value + Hash + 'static> Component for DockColumn<'_, P> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::SPLITTER;
    type Style = DockColumnStyle;
    type Output = DockColumnOutput<P>;

    /// The column's edge line lies centred on the edge, so half of it is outside the column.
    fn paint_outset(tokens: &crate::style::UiTokens) -> f32 {
        tokens.stroke.thick / 2.0
    }

    fn style(cx: &StyleCx<'_>, _: DockColumnStyleKey<P>) -> DockColumnStyle {
        DockColumnStyle { metrics: Metrics::new(), sizes: DockSizes::style(cx) }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// The bar's drag band under the bar's own controls, and a divider between every two open groups.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let sizes = cx.style.sizes;
        let plan = column_plan(self.column, self.spec, rect, &sizes, |_, _| None);
        let dividers = plan.dividers(rect, sizes.reach);
        for (key, _, divider) in dividers {
            divider::declare(cx, rect, DIVIDER, key, &divider);
        }
        divider::declare(cx, rect, EDGE, 0, &self.edge_divider(rect, &sizes));
        self.laid = Some(plan);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let sizes = cx.style.sizes;
        let (rect, rtl, ctx) = (cx.rect, cx.flow.is_rtl(), cx.core.ctx.clone());
        let Some(base) = self.laid.take() else { return };
        let mut plan = column_plan(self.column, self.spec, rect, &sizes, |key, divider| divider::pointer(cx, DIVIDER, key, divider));
        let dividers = base.dividers(rect, sizes.reach);
        let keyed: Vec<(u64, Divider)> = dividers.iter().map(|(key, _, divider)| (*key, *divider)).collect();
        if let Some((key, coord)) = divider::keys(cx, DIVIDER, &keyed, sizes.step, sizes.jump)
            && let Some((_, index, divider)) = dividers.iter().find(|(at, ..)| *at == key)
        {
            plan.move_divider(*index, coord - divider.at);
        }
        // The groups whose height the drag or the keys changed, each named by its first panel.
        for group in 0..plan.heights.len() {
            if plan.heights[group] != base.heights[group]
                && let Some(anchor) = self.anchor(group)
            {
                self.resized.push((anchor.clone(), plan.heights[group]));
            }
        }
        let edge = self.edge_divider(rect, &sizes);
        if let Some(Drag::To(coord)) = divider::pointer(cx, EDGE, 0, &edge) {
            self.move_edge(rect, &edge, coord);
        }
        if let Some((_, coord)) = divider::keys(cx, EDGE, &[(0, edge)], sizes.step, sizes.jump) {
            self.move_edge(rect, &edge, coord);
        }
        self.register(&ctx, rect, rtl, &plan, &sizes);
        self.laid = Some(plan);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(plan) = self.laid.as_ref() else { return };
        // Painted here, not as the container's surface, which lies over what `paint` draws.
        ui.draw().surface(cx.rect, SurfaceKind::Panel, Corners::All);
        ui.draw().surface(plan.bar, SurfaceKind::Background, Corners::None);
    }

    /// The column's frame, then the dividers over it.
    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(plan) = self.laid.as_ref() else { return };
        ui.draw().surface(cx.rect, SurfaceKind::Frame, Corners::None);
        for pair in plan.open.windows(2) {
            ui.face(DIVIDER.at(pair[0] as u64), Splitter::new(Axis::Y));
        }
        // The edge's line lies on the column's inner edge, over the frame; its handle stays inside the column.
        let (rect, edge) = (cx.rect, self.edge_divider(cx.rect, &cx.style.sizes));
        let x = if cx.flow.is_rtl() { rect.right() - (edge.at - rect.left()) } else { edge.at };
        ui.face(EDGE.at(0), Splitter::new(Axis::X).boundary(x));
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<DockWords>();
        let sizes = cx.style.sizes;
        let Some(plan) = self.laid.as_ref() else { return };
        for (key, _, divider) in plan.dividers(cx.rect, sizes.reach) {
            divider::describe(access, DIVIDER, key, &divider, words.resize.as_str(), sizes.step);
        }
        let edge = self.edge_divider(cx.rect, &sizes);
        divider::describe(access, EDGE, 0, &edge, words.resize.as_str(), sizes.step);
    }

    fn finish(self, cx: Cx<'_, Self>) -> DockColumnOutput<P> {
        DockColumnOutput { response: cx.response.clone(), placed: cx.rect, iconic: None, picked: None, resized: self.resized, width: self.width }
    }
}

/// The grip of the column bar: a drag moves the whole column, its ghost the column's size and its panels' names.
pub(super) fn column_grip<P: Value + Hash>(column: &Column<P>, spec: &dyn Fn(&P) -> PanelSpec, body: &BodyCx<'_>) -> Option<GroupGrip<P>> {
    let first = column.groups.first()?;
    let anchor = first.panels.first()?;
    let active = PanelKey::of(first.active_panel().unwrap_or(anchor));
    let name = bar_name(column, spec, &body.words::<DockWords>().move_column);
    let ghost = (body.rect.size(), Label::from(panel_names(column, spec)));
    Some(GroupGrip::new(DragUnit::Column(anchor.clone()), active).ghost(ghost).label(name))
}

/// Registers the drop bands of the two edges of the column in `rect` with the dock `dock`: a drop there makes a new
/// column beside the one that holds `anchor`.
pub(super) fn register_edges<P: Clone + Send + 'static>(ctx: &Context, dock: Id, anchor: &P, rect: Rect, rtl: bool, sizes: &DockSizes) {
    let flow = Flow::new(rect, rtl);
    // A narrow column keeps a third of its width between its two edge bands, so a drop inside it still reaches its
    // groups.
    let band = sizes.column_edge_band.min(rect.width() * 2.0 / 3.0);
    for (side, x) in [(DockSide::Start, rect.left()), (DockSide::End, rect.right())] {
        let edge = Rect::from_x_y_ranges((x - band * 0.5)..=(x + band * 0.5), rect.y_range());
        DockTargets::register(ctx, dock, DockSpot::column(anchor.clone(), side), flow.to_physical(edge));
    }
}

impl<P: Value + Hash + 'static> DockColumn<'_, P> {
    /// Registers the drop bands of the column's two edges and of the seams between its groups.
    fn register(&self, ctx: &Context, rect: Rect, rtl: bool, plan: &Plan, sizes: &DockSizes) {
        let Some(first) = self.anchor(0) else { return };
        register_edges(ctx, self.dock, first, rect, rtl, sizes);
        let (mut before, half) = (first, sizes.seam_band * 0.5);
        for (at, group) in plan.groups.iter().enumerate().skip(1) {
            let Some(after) = self.anchor(at) else { continue };
            let seam = Rect::from_x_y_ranges(rect.x_range(), (group.top() - half)..=(group.top() + half));
            DockTargets::register(ctx, self.dock, DockSpot::seam(before.clone(), after.clone()), seam);
            before = after;
        }
    }
}

impl<P: Value + Hash + 'static> Container for DockColumn<'_, P> {}
