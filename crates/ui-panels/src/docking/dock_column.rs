//! `DockColumn`: a column of a side dock with its column bar and stacked groups, and the pure size rules of columns.
//!
//! The column is one container. Its bar is a `TitleRow` (a disclosure that collapses the column to icons and a menu
//! action) over a drag band; the groups below it stack at the heights `group_heights` gives and are shown by the
//! caller's group view. The column registers its edge bands and seams as drop spots of the dock and reports what the
//! user did; the dock decides what a drop changes.

use super::divider::{self, Divider, Drag};
use super::group::strip_surface;
use super::{CardInset, DividerKind, DockSizes, DockSpot, DockTargets, DockWords, DragUnit, PanelKey, PanelSpec, Splitter};
use crate::contract::{
    Access, BodyCx, Common, Component, Container, ContainerProps, Cx, NoValue, Outcome, Pad, PaintUi, PartsCx, Pointer as _, Props as _, Role,
    ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::menu::GroupItem;
use crate::runtime::env::Label;
use crate::runtime::focus::KeyUse;
use crate::style::{ComponentStyle, Corners, StyleCx, SurfaceEdge, SurfaceKind, SurfaceStyle, UiTokens};
use crate::tabs::PanelTabLook;
use craft_behavior::divider::{self as push, Segment};
use craft_behavior::dock::{Column, DockEdge, Group};
use craft_behavior::{Axis, DockSide, Flow, Metrics, PartKey, Sense, Value};
use egui::{Context, Id, Pos2, Rect, Response, Ui};
use std::fmt::Debug;
use std::hash::Hash;

#[cfg(feature = "suite")]
pub(crate) mod stories;

mod bar;
mod sizing;

use bar::panel_names;

pub(super) use bar::{BarRecord, CHEVRON, ColumnGrip, bar_band, bar_name, bar_slots, describe_chevron, paint_chevron, show_bar};
use sizing::group_sizes;
pub use sizing::{GroupSize, column_cells, fit_widths, group_heights, scale_widths, stacked, width_bounds};

const DIVIDER: PartKey = PartKey::new("divider");
const EDGE: PartKey = PartKey::new("edge");

/// Where a column's bar is. A dock column and a floating window are both one column; the window's title row is the
/// bar of the column it holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ColumnBar {
    /// The column shows its own bar (the double arrow and the band that moves the column and opens its menu), the
    /// divider of its inner edge, its frame and the drop bands of its two edges: a column of a side dock.
    #[default]
    Own,
    /// The column fills a floating window, whose frame, edges and title row carry what `Own` shows: the column shows
    /// no bar, no edge divider and no frame, and registers no edge bands, since nothing goes beside a floating column.
    Window,
}

impl ColumnBar {
    /// The height of the column's bar: the dock's bar height for `Own`, none for `Window`.
    pub(super) fn height(self, sizes: &DockSizes) -> f32 {
        match self {
            ColumnBar::Own => sizes.bar,
            ColumnBar::Window => 0.0,
        }
    }
}

/// What a column's double arrow does, which names it and sets where it points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, crate::contract::MemoProps)]
#[non_exhaustive]
pub enum ColumnArrow {
    /// Collapses the column to icons, or expands it.
    #[default]
    Collapse,
    /// Switches a stepped column ([`DockSizes::stepped`](super::DockSizes::stepped)) between one and two steps. Holds the steps it shows.
    Steps(u16),
    /// Nothing: the column neither collapses nor has steps. It shows no double arrow, and its menu offers no
    /// collapse.
    Fixed,
}

impl ColumnArrow {
    /// The arrow of `column`: an expanded column whose panels do not collapse (`collapses` false) switches steps
    /// when it is stepped and has none otherwise; any other column collapses or expands.
    pub fn of<P>(column: &Column<P>, sizes: &DockSizes, spec: &dyn Fn(&P) -> PanelSpec, collapses: bool) -> Self {
        if collapses || column.iconic {
            return ColumnArrow::Collapse;
        }
        sizes.stepped(column, spec).map_or(ColumnArrow::Fixed, |(_, steps)| ColumnArrow::Steps(steps))
    }

    /// The steps a click gives a stepped column: two from one, else one. `None` for any other arrow.
    pub fn switched_steps(self) -> Option<u16> {
        match self {
            ColumnArrow::Steps(steps) => Some(if steps == 1 { 2 } else { 1 }),
            ColumnArrow::Collapse | ColumnArrow::Fixed => None,
        }
    }

    /// Whether the column shows a double arrow.
    pub(super) fn shown(self) -> bool {
        self != ColumnArrow::Fixed
    }

    /// Whether a click sends the column towards the edge it keeps to: collapsing the `expanded` column, or narrowing a
    /// stepped one.
    pub(super) fn towards_edge(self, expanded: bool) -> bool {
        match self {
            ColumnArrow::Collapse | ColumnArrow::Fixed => expanded,
            ColumnArrow::Steps(steps) => steps > 1,
        }
    }

    /// The arrow's name: what a click on it does to the `expanded` or collapsed column.
    pub(super) fn name(self, words: &DockWords, expanded: bool) -> Label {
        let word = match self {
            ColumnArrow::Collapse | ColumnArrow::Fixed if expanded => &words.collapse_column,
            ColumnArrow::Collapse | ColumnArrow::Fixed => &words.expand_column,
            ColumnArrow::Steps(1) => &words.two_steps,
            ColumnArrow::Steps(_) => &words.one_step,
        };
        Label::from(word.as_str().to_owned())
    }
}

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
    /// The dividers drawn this pass: between each two open groups and on the inner edge, each with the rect of its
    /// handle and the id of the handle's accessibility node.
    pub dividers: Vec<(DividerKind<P>, Rect, Id)>,
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
    /// The card of a column shown as one ([`card`](DockColumn::card)).
    card: SurfaceStyle,
}

/// One column of a side dock: the column bar over the stacked groups of `column`. Show it with
/// [`show_groups`](DockColumn::show_groups); the closure shows one group in the rect it is given, through
/// `DockPanels::group_view` or a `DockGroup`. A double click on the bar collapses the column to icons; the bar's
/// drag starts the move of the column through the drag session; Enter on the bar's menu button, the Menu key and a right
/// click open the column menu, whose dock rows come back in `picked`. A divider between two open groups resizes
/// them by pointer or with the arrow keys.
///
/// The column registers the drop bands of its two edges (`DockSpot::column`) and of the seams between its groups
/// (`DockSpot::seam`) with the dock `dock` every pass.
///
/// In a floating window ([`ColumnBar::Window`]) the window's title row is the column's bar and the window's edges
/// size it: the column shows its groups and their dividers only, and registers its seams.
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
    /// Where the column's bar is: its own (a dock column) or its floating window's title row.
    #[prop]
    bar: ColumnBar,
    /// What the double arrow of the column's own bar does ([`ColumnArrow::of`]).
    #[prop]
    arrow: ColumnArrow,
    /// The rect the dock may grow within, which bounds how wide its inner edge can make the column.
    #[prop]
    within: Rect,
    /// Which inset the outer side of the column's card takes, when it shows as one
    /// ([`DockConfig::column_cards`](super::DockConfig::column_cards), [`DockSizes::card_place`]): its bar and groups
    /// sit inside the card, inset by that on its outer side and by [`DockSizes::card`] on its top and bottom.
    #[prop]
    card: Option<CardInset>,
    /// The look of the dock's panel tabs ([`DockConfig::tab_look`](super::DockConfig::tab_look)): the column's bar
    /// lies on the surface the groups' tab strips do.
    #[prop]
    tab_look: PanelTabLook,
    /// The plan `parts` laid out this pass, which update, paint and access read.
    laid: Option<Plan>,
    resized: Vec<(P, f32)>,
    width: Option<f32>,
    /// The bar's chevron was clicked this pass: the column collapses to icons.
    collapse: bool,
}

impl<'a, P: Value + Hash + Debug + 'static> DockColumn<'a, P> {
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
            bar: ColumnBar::Own,
            arrow: ColumnArrow::Collapse,
            within: Rect::NOTHING,
            card: None,
            tab_look: PanelTabLook::Plain,
            laid: None,
            resized: Vec::new(),
            width: None,
            collapse: false,
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
        let (column, spec, auto_collapse, edge, own, arrow, card) =
            (self.column, self.spec, self.auto_collapse, self.edge, self.bar, self.arrow, self.card);
        let mut bar = BarRecord::default();
        let shown = &mut bar;
        let body = move |ui: &mut Ui, body: &mut BodyCx<'_>| {
            let Some(style) = body.style::<DockColumn<'_, P>>() else { return };
            let sizes = style.sizes;
            let inner = card_rect(body.rect, card, &sizes, edge, body.env.direction().is_rtl());
            if inner != body.rect {
                // The groups' own surfaces keep to the card's rounded corners.
                crate::primitives::frame_surfaces(ui, inner, style.card.radius);
            }
            // The bar and the groups at the heights this pass's drag leaves them, in the measuring run and the real
            // run alike: the same plan `parts` lays out, so the dividers sit at the seams.
            let drags = |key: u64, divider: &Divider| divider::drag(body.part_press(DIVIDER.at(key)).phase, body.rect, false, divider);
            let plan = column_plan(column, spec, inner, own, &sizes, drags);
            if own == ColumnBar::Own {
                let band = bar_band(ui, plan.bar, edge);
                *shown = show_bar(ui, band, true, auto_collapse, arrow, false, column_grip(column, spec, body));
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
        output.iconic = bar.iconic.or(output.iconic);
        output.picked = bar.picked;
        output
    }

    /// The divider on the column's inner edge, which moves between the width bounds of its panels, or of its steps
    /// for a stepped column ([`stepped`](super::stepped)), whose host takes the width as whole steps; a card's inset
    /// on top.
    fn edge_divider(&self, rect: Rect, sizes: &DockSizes) -> Divider {
        let (least, most) = sizes.shown_bounds(self.column, self.spec);
        let inset = self.card.map_or(0.0, |place| sizes.inset(place));
        let (least, most) = (least + inset, most + inset);
        Divider::column_edge(rect, self.edge, (least, most.min((self.within.width() - sizes.least_pane).max(rect.width()))), sizes.reach)
    }

    /// Takes the width the coordinate `coord` of the inner edge asks for, kept within the divider's travel.
    fn move_edge(&mut self, rect: Rect, divider: &Divider, coord: f32) {
        let width = Divider::column_width(rect, self.edge, coord.clamp(divider.lo, divider.hi));
        if (width - rect.width()).abs() > f32::EPSILON {
            self.width = Some(width);
        }
    }

    /// Where the column's bar and groups sit in `rect`: inside its card, if it shows as one.
    fn inner(&self, rect: Rect, rtl: bool, sizes: &DockSizes) -> Rect {
        card_rect(rect, self.card, sizes, self.edge, rtl)
    }

    fn anchor(&self, group: usize) -> Option<&P> {
        self.column.groups.get(group).and_then(|group| group.panels.first())
    }
}

/// The card of a docked column in `rect`, the column's rect in a dock on the window edge `edge`: `rect` inset by
/// the card's outer inset at `card` ([`DockSizes::inset`]) on its outer side, the one towards `edge`, and by
/// [`DockSizes::card`] on its top and bottom; `rect` for a column that is no card.
pub(crate) fn card_rect(rect: Rect, card: Option<CardInset>, sizes: &DockSizes, edge: DockEdge, rtl: bool) -> Rect {
    let Some(outer) = card.map(|place| sizes.inset(place)) else { return rect };
    let inner = rect.shrink2(egui::vec2(0.0, sizes.card));
    if (edge == DockEdge::Start) != rtl { inner.with_min_x(inner.left() + outer) } else { inner.with_max_x(inner.right() - outer) }
}

/// The look of a docked column's card ([`DockConfig::column_cards`](super::DockConfig::column_cards)): `surface.panel`
/// with `shape.radius.surface` corners and no edge.
pub(crate) fn card_style(cx: &StyleCx<'_>) -> SurfaceStyle {
    SurfaceStyle { radius: UiTokens::corner(cx.tokens.shape.radius.surface), edge: SurfaceEdge::None, shadow: None, ..cx.surface(SurfaceKind::Panel) }
}

/// Paints a column in `rect` whose content sits in `card`, with its own `bar` if it has one, beside tabs in `look`: the
/// card over the dock's background where the column is one, else the column's panel surface; the bar on the surface
/// of the dock's strips. A card's bar is part of the card, unless the strips have a surface of their own
/// ([`PanelTabLook::Block`]): then it is a square strip across the card's top, and only the card's foot is rounded.
pub(crate) fn paint_card(ui: &mut PaintUi<'_>, rect: Rect, card: Rect, style: &SurfaceStyle, bar: Option<Rect>, look: PanelTabLook) {
    let strip = strip_surface(look);
    if card == rect {
        ui.draw().surface(rect, SurfaceKind::Panel, Corners::All);
        if let Some(bar) = bar {
            ui.draw().surface(bar, strip, Corners::None);
        }
        return;
    }
    ui.draw().surface(rect, SurfaceKind::Background, Corners::None);
    match bar.filter(|_| look == PanelTabLook::Block) {
        Some(bar) => {
            let mut foot = *style;
            (foot.radius.nw, foot.radius.ne) = (0, 0);
            ui.draw().surface_styled(card, &foot);
            ui.draw().surface(bar, strip, Corners::None);
        }
        None => ui.draw().surface_styled(card, style),
    }
}

/// Where the groups of `column` sit in `rect`, the column's rect with its bar `bar`, as the column lays them out while
/// no divider between them is dragged.
pub fn group_rects<P>(column: &Column<P>, spec: &dyn Fn(&P) -> PanelSpec, rect: Rect, bar: ColumnBar, sizes: &DockSizes) -> Vec<Rect> {
    column_plan(column, spec, rect, bar, sizes, |_, _| None).groups
}

/// The bar and body of the column in `rect`, and the groups stacked in the body, with the drags `drags` makes of
/// the dividers between open groups (the divider's part index and the divider as laid out before the drag). A
/// column in a floating window has an empty bar on top.
fn column_plan<P>(
    column: &Column<P>,
    spec: &dyn Fn(&P) -> PanelSpec,
    rect: Rect,
    own: ColumnBar,
    sizes: &DockSizes,
    drags: impl Fn(u64, &Divider) -> Option<Drag>,
) -> Plan {
    let bar = Rect::from_min_max(rect.min, Pos2::new(rect.right(), (rect.top() + own.height(sizes)).min(rect.bottom())));
    let body = Rect::from_min_max(Pos2::new(rect.left(), bar.bottom()), rect.max);
    body_plan(column, spec, bar, body, sizes, drags)
}

/// [`column_plan`] for a column whose `bar` and `body` rects are known.
fn body_plan<P>(
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
        let sizes = push::resize(&row, index, at + delta, push::Give::Neighbours);
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
    /// The room the dividers between the groups take in the column's `rect`: the body, inside the column's card if it
    /// shows as one, in the frame of `rect` mirrored for right-to-left that dividers are declared in.
    fn divider_room(&self, rect: Rect, rtl: bool) -> Rect {
        Flow::new(rect, rtl).to_physical(self.body)
    }

    fn dividers(&self, rect: Rect, reach: f32) -> Vec<(u64, usize, Divider)> {
        let row = self.row();
        self.open
            .windows(2)
            .enumerate()
            .filter_map(|(index, pair)| {
                let at = self.groups.get(pair[0])?.bottom();
                let (lo, hi) = push::travel(&row, index, push::Give::Neighbours);
                Some((pair[0] as u64, index, Divider::new(Axis::Y, at, rect, (at + lo, at + hi), reach)))
            })
            .collect()
    }
}

impl<P: Value + Debug + Hash + 'static> Component for DockColumn<'_, P> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::SPLITTER;
    type Style = DockColumnStyle;
    type Output = DockColumnOutput<P>;

    /// The column's edge line lies centred on the edge, so half of it is outside the column.
    fn paint_outset(tokens: &crate::style::UiTokens) -> f32 {
        tokens.stroke.thick / 2.0
    }

    fn style(cx: &StyleCx<'_>, _: DockColumnStyleKey<P>) -> DockColumnStyle {
        DockColumnStyle { metrics: Metrics::new(), sizes: DockSizes::style(cx), card: card_style(cx) }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// The chevron at the bar's outer end, a divider between every two open groups and the inner edge's divider; a
    /// column in a floating window has the group dividers only.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let sizes = cx.style.sizes;
        let plan = column_plan(self.column, self.spec, self.inner(rect, cx.flow.is_rtl(), &sizes), self.bar, &sizes, |_, _| None);
        if self.bar == ColumnBar::Own && self.arrow.shown() {
            let (chevron, _) = bar_slots(plan.bar, self.edge, cx.flow.is_rtl());
            let name = self.arrow.name(&cx.words::<DockWords>(), true);
            let _ = cx.part(CHEVRON, chevron).sense(Sense::CLICK | Sense::FOCUS).focusable().tooltip(name);
        }
        let dividers = plan.dividers(plan.divider_room(rect, cx.flow.is_rtl()), sizes.reach);
        for (key, _, divider) in dividers {
            divider::declare(cx, rect, DIVIDER, key, &divider);
        }
        if self.bar == ColumnBar::Own {
            divider::declare(cx, rect, EDGE, 0, &self.edge_divider(rect, &sizes));
        }
        self.laid = Some(plan);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let sizes = cx.style.sizes;
        let (rect, rtl, ctx) = (cx.rect, cx.flow.is_rtl(), cx.core.ctx.clone());
        let own = self.bar == ColumnBar::Own;
        self.collapse = own && self.arrow.shown() && cx.part_activated(CHEVRON).is_some();
        let Some(base) = self.laid.take() else { return };
        let inner = self.inner(rect, rtl, &sizes);
        let mut plan = column_plan(self.column, self.spec, inner, self.bar, &sizes, |key, divider| divider::pointer(cx, DIVIDER, key, divider));
        let dividers = base.dividers(base.divider_room(rect, rtl), sizes.reach);
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
        if own {
            let edge = self.edge_divider(rect, &sizes);
            if let Some(Drag::To(coord)) = divider::pointer(cx, EDGE, 0, &edge) {
                self.move_edge(rect, &edge, coord);
            }
            if let Some((_, coord)) = divider::keys(cx, EDGE, &[(0, edge)], sizes.step, sizes.jump) {
                self.move_edge(rect, &edge, coord);
            }
        }
        self.register(&ctx, rect, rtl, &plan, &sizes);
        self.laid = Some(plan);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(plan) = self.laid.as_ref() else { return };
        // Painted here, not as the container's surface, which lies over what `paint` draws.
        let card = self.inner(cx.rect, cx.flow.is_rtl(), &cx.style.sizes);
        let bar = (self.bar == ColumnBar::Own).then_some(plan.bar);
        paint_card(ui, cx.rect, card, &cx.style.card, bar, self.tab_look);
        if self.bar == ColumnBar::Own && self.arrow.shown() {
            paint_chevron(ui, self.edge, self.arrow, true, cx.flow.is_rtl(), &cx.words::<DockWords>());
        }
    }

    /// The column's frame, then the dividers over it. The frame of a column in a floating window is the window's.
    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(plan) = self.laid.as_ref() else { return };
        if self.bar == ColumnBar::Window {
            for pair in plan.open.windows(2) {
                ui.face(DIVIDER.at(pair[0] as u64), Splitter::new(Axis::Y));
            }
            return;
        }
        // A card has no frame.
        if self.card.is_none() {
            ui.draw().surface(cx.rect, SurfaceKind::Frame, Corners::None);
        }
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
        for (key, _, divider) in plan.dividers(plan.divider_room(cx.rect, cx.flow.is_rtl()), sizes.reach) {
            divider::describe(access, DIVIDER, key, &divider, words.resize.as_str(), sizes.step);
        }
        if self.bar == ColumnBar::Own {
            let edge = self.edge_divider(cx.rect, &sizes);
            divider::describe(access, EDGE, 0, &edge, words.resize.as_str(), sizes.step);
            if self.arrow.shown() {
                describe_chevron(access, &self.arrow.name(&words, true));
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> DockColumnOutput<P> {
        let node = |part| crate::runtime::access::sub_node_id(cx.core.id, part);
        let mut dividers = Vec::new();
        for pair in self.laid.as_ref().map_or(&[][..], |plan| &plan.open[..]).windows(2) {
            let part = DIVIDER.at(pair[0] as u64);
            if let (Some(shown), Some(anchor)) = (cx.part_if_declared(part), self.anchor(pair[0])) {
                dividers.push((DividerKind::Groups(anchor.clone()), shown.rect, node(part)));
            }
        }
        let edge = EDGE.at(0);
        if let (Some(shown), Some(first)) = (cx.part_if_declared(edge), self.anchor(0)) {
            dividers.push((DividerKind::ColumnEdge(first.clone()), shown.rect, node(edge)));
        }
        DockColumnOutput {
            dividers,
            response: cx.response.clone(),
            placed: cx.rect,
            iconic: self.collapse.then_some(true),
            picked: None,
            resized: self.resized,
            width: self.width,
        }
    }
}

/// The grip of the column bar: a drag moves the whole column, its ghost the column's size and its panels' names.
pub(super) fn column_grip<P: Value + Hash>(column: &Column<P>, spec: &dyn Fn(&P) -> PanelSpec, body: &BodyCx<'_>) -> Option<ColumnGrip<P>> {
    let first = column.groups.first()?;
    let anchor = first.panels.first()?;
    let active = PanelKey::of(first.active_panel().unwrap_or(anchor));
    let name = Label::from(bar_name(column, spec, &body.words::<DockWords>().move_column));
    let ghost = (body.rect.size(), Label::from(panel_names(column, spec)));
    Some(ColumnGrip { unit: DragUnit::Column(anchor.clone()), active, ghost, name })
}

/// Registers the drop bands of the two edges of the column in `rect` with the dock `dock`: a drop there makes a new
/// column beside the one that holds `anchor`.
pub(super) fn register_edges<P: Clone + Send + Sync + PartialEq + Debug + 'static>(
    ctx: &Context,
    dock: Id,
    anchor: &P,
    rect: Rect,
    rtl: bool,
    sizes: &DockSizes,
) {
    let flow = Flow::new(rect, rtl);
    // A narrow column keeps a third of its width between its two edge bands, so a drop inside it still reaches its
    // groups.
    let band = sizes.column_edge_band.min(rect.width() * 2.0 / 3.0);
    for (side, x) in [(DockSide::Start, rect.left()), (DockSide::End, rect.right())] {
        let edge = Rect::from_x_y_ranges((x - band * 0.5)..=(x + band * 0.5), rect.y_range());
        let line = Rect::from_x_y_ranges(x..=x, rect.y_range());
        DockTargets::register_band(
            ctx,
            dock,
            DockSpot::column(anchor.clone(), side),
            flow.to_physical(edge),
            flow.to_physical(line),
            sizes.strip_reach,
        );
    }
}

impl<P: Value + Debug + Hash + 'static> DockColumn<'_, P> {
    /// Registers the drop bands of the column's two edges, for a dock column, and of the seams between its groups.
    fn register(&self, ctx: &Context, rect: Rect, rtl: bool, plan: &Plan, sizes: &DockSizes) {
        let Some(first) = self.anchor(0) else { return };
        if self.bar == ColumnBar::Own {
            register_edges(ctx, self.dock, first, rect, rtl, sizes);
        }
        let (mut before, half) = (first, sizes.seam_band * 0.5);
        for (at, group) in plan.groups.iter().enumerate().skip(1) {
            let Some(after) = self.anchor(at) else { continue };
            let seam = Rect::from_x_y_ranges(rect.x_range(), (group.top() - half)..=(group.top() + half));
            let line = Rect::from_x_y_ranges(rect.x_range(), group.top()..=group.top());
            DockTargets::register_band(ctx, self.dock, DockSpot::seam(before.clone(), after.clone()), seam, line, sizes.strip_reach);
            before = after;
        }
    }
}

impl<P: Value + Hash + Debug + 'static> Container for DockColumn<'_, P> {}
