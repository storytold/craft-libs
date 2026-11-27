//! `SplitViews`: one canvas area shown as views of one document, or a grid of panes the caller fills (one document
//! each). One view is active and is the full interactive view (tools, overlays, keys); the others show their document
//! from their own view and become active on a click. [`PanePress`] chooses whether that click also reaches the view.
//! The dividers between panes move with the pointer and, when
//! focused, with the arrow keys.
//!
//! The panes are the cells [`arrange_rects`] gives for the arrangement the caller names, with the dividers moved to
//! the [`SplitAt`] the caller binds. [`SplitViews::show_panes`] gives each pane its own body, at the pane's rect as the
//! views lay it out in the pass. A linked pane follows the active view's zoom and position ([`SplitViews::follow`]).
//!
//! With [`SplitViews::takes_tabs`] the views take a dragged document strip tab: an unsplit body offers its reading-end
//! half, a split body each whole pane, and the drop comes back in [`SplitViewsOutput::tab_drop`].

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::words::SplitWords;
use crate::contract::{
    Access, Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, DropMark, DropOperation, DropTarget, Edit as _, EditResponse,
    Inner, ListDrag, Name, Outcome, Pad, PaintUi, PartsCx, Pointer as _, Props as _, Role, ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::docking::divider::{self, Divider, Drag};
use crate::docking::{DockSizes, Splitter};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::runtime::env::{AppValues, EnvValue, Label, in_room};
use crate::runtime::focus::KeyUse;
use crate::style::{ComponentStyle, StyleColor, StyleCx};
use craft_behavior::dock::arrange_rects;
use craft_behavior::{Axis, CanvasView, Flow, HitLevel, HitPriority, Metrics, PartKey, Sense};
use egui::{CornerRadius, Pos2, Rect, Response, Ui, Vec2};
use std::sync::Arc;

const DIVIDER: PartKey = PartKey::new("divider");
const PANE: PartKey = PartKey::new("pane");

/// How the canvas area divides: the Window › Arrange arrangements. The n-up and tile arrangements divide it into panes
/// in the reading order [`arrange_rects`] gives (pane 0 at the top start corner); any other entry, or fewer than two
/// panes, shows one pane over the whole area.
pub use craft_behavior::dock::Arrangement as SplitLayout;

/// How a pointer press in an inactive pane activates it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PanePress {
    /// The first press selects the pane; its contents receive later presses.
    #[default]
    Select,
    /// The press selects the pane and also reaches its contents.
    Interact,
}

/// Where the dividers lie, as fractions of the area counted from its start edge and its top: one entry per column line
/// and per row line of the layout, in order. A list that does not fit the layout (the wrong length, not ascending, or
/// outside 0–1) lays out that axis in even cells. A saved value missing a field takes that field's default lines.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct SplitAt {
    /// The column lines (the vertical dividers), each over the width.
    pub columns: Vec<f32>,
    /// The row lines (the horizontal dividers), each over the height.
    pub rows: Vec<f32>,
}

impl SplitAt {
    /// One column line at `x` and one row line at `y`: the dividers of two side by side views and of four views.
    pub fn new(x: f32, y: f32) -> Self {
        SplitAt { columns: vec![x], rows: vec![y] }
    }

    /// The column lines at `columns` and the row lines at `rows`.
    pub fn lines(columns: Vec<f32>, rows: Vec<f32>) -> Self {
        SplitAt { columns, rows }
    }

    /// The panes' rects in `full` for `layout` showing `count` documents (a tile arrangement shows that many panes,
    /// an n-up one its own number), with the lines at these fractions counted from the left, in reading order.
    pub fn rects(&self, layout: SplitLayout, count: usize, full: Rect) -> Vec<Rect> {
        lay_out(layout, count, self, full).panes
    }

    /// The panes' rects in `full` placed for the layout direction: [`rects`](Self::rects) reflected across the middle
    /// of `full` under `rtl`, so pane 0 sits at the top start corner and the fractions count from the start edge.
    pub fn placed(&self, layout: SplitLayout, count: usize, full: Rect, rtl: bool) -> Vec<Rect> {
        let flow = Flow::new(full, rtl);
        self.rects(layout, count, full).into_iter().map(|rect| flow.to_physical(rect)).collect()
    }
}

impl Default for SplitAt {
    fn default() -> Self {
        SplitAt::new(0.5, 0.5)
    }
}

/// One dividing line between panes, in reading order: its coordinate along its axis and the extent it spans across.
#[derive(Clone, Copy, Debug)]
struct Line {
    at: f32,
    from: f32,
    to: f32,
}

/// A layout laid out in a rect: the panes and the lines between them, unmirrored.
#[derive(Debug)]
struct Laid {
    panes: Vec<Rect>,
    columns: Vec<Line>,
    rows: Vec<Line>,
}

/// How close two cell edges along an extent of `len` are when they are the same line: [`arrange_rects`] builds each
/// edge from a sum of equal shares, so one line can differ in the last bits between neighbouring cells.
fn tolerance(len: f32) -> f32 {
    len.abs().max(1.0) * 1e-4
}

/// The distinct coordinates of `edges` strictly inside `start..end`, ascending.
fn interior(edges: impl Iterator<Item = f32>, start: f32, end: f32, tol: f32) -> Vec<f32> {
    let mut inside: Vec<f32> = edges.filter(|&edge| edge > start + tol && edge < end - tol).collect();
    inside.sort_by(f32::total_cmp);
    inside.dedup_by(|next, kept| (*next - *kept).abs() <= tol);
    inside
}

/// The lines' coordinates: `fractions` of `len` from `start` when they fit `even` (as many, ascending, inside 0–1),
/// else the even lines themselves.
fn targets(fractions: &[f32], even: &[f32], start: f32, len: f32) -> Vec<f32> {
    let inside = fractions.iter().all(|fraction| (0.0..=1.0).contains(fraction));
    let ascending = fractions.windows(2).all(|pair| matches!(pair, [a, b] if a < b));
    if fractions.len() == even.len() && inside && ascending {
        fractions.iter().map(|fraction| start + len * fraction).collect()
    } else {
        even.to_vec()
    }
}

/// `edge` moved to the target of the even line it lies on, or kept when it lies on none (an outer edge).
fn snapped(edge: f32, even: &[f32], targets: &[f32], tol: f32) -> f32 {
    even.iter().zip(targets).find(|(line, _)| (**line - edge).abs() <= tol).map_or(edge, |(_, target)| *target)
}

/// Each even line of `even` with its target in `targets`, spanning the extent (by `across`) of the panes whose cells
/// have an edge (by `edges`) on it.
fn spans(
    even: &[f32],
    targets: &[f32],
    cells: &[Rect],
    panes: &[Rect],
    tol: f32,
    edges: impl Fn(&Rect) -> [f32; 2],
    across: impl Fn(&Rect) -> (f32, f32),
) -> Vec<Line> {
    even.iter()
        .zip(targets)
        .map(|(&line, &at)| {
            let touching = cells.iter().zip(panes).filter(|(cell, _)| edges(cell).iter().any(|edge| (edge - line).abs() <= tol));
            let (from, to) = touching.fold((f32::INFINITY, f32::NEG_INFINITY), |(from, to), (_, pane)| {
                let (start, end) = across(pane);
                (from.min(start), to.max(end))
            });
            Line { at, from, to }
        })
        .collect()
}

/// `layout` showing `count` documents in `full`, with the lines at `at`: [`arrange_rects`]' even cells with each edge
/// on an inner line moved to where `at` puts that line. A line spans the panes whose edges lie on it, so the row line
/// of 3-up Stacked spans only the second column.
fn lay_out(layout: SplitLayout, count: usize, at: &SplitAt, full: Rect) -> Laid {
    let cells =
        layout.group_count(count).and_then(|n| arrange_rects(layout, full, n)).filter(|cells| !cells.is_empty()).unwrap_or_else(|| vec![full]);
    let (tol_x, tol_y) = (tolerance(full.width()), tolerance(full.height()));
    let even_columns = interior(cells.iter().flat_map(|cell| [cell.left(), cell.right()]), full.left(), full.right(), tol_x);
    let even_rows = interior(cells.iter().flat_map(|cell| [cell.top(), cell.bottom()]), full.top(), full.bottom(), tol_y);
    let columns = targets(&at.columns, &even_columns, full.left(), full.width());
    let rows = targets(&at.rows, &even_rows, full.top(), full.height());
    let x = |edge: f32| snapped(edge, &even_columns, &columns, tol_x);
    let y = |edge: f32| snapped(edge, &even_rows, &rows, tol_y);
    let panes: Vec<Rect> =
        cells.iter().map(|cell| Rect::from_min_max(Pos2::new(x(cell.left()), y(cell.top())), Pos2::new(x(cell.right()), y(cell.bottom())))).collect();
    let column_lines = spans(&even_columns, &columns, &cells, &panes, tol_x, |cell| [cell.left(), cell.right()], |pane| (pane.top(), pane.bottom()));
    let row_lines = spans(&even_rows, &rows, &cells, &panes, tol_y, |cell| [cell.top(), cell.bottom()], |pane| (pane.left(), pane.right()));
    Laid { panes, columns: column_lines, rows: row_lines }
}

/// The number of panes `layout` shows with `count` documents.
fn pane_count(layout: SplitLayout, count: usize) -> usize {
    lay_out(layout, count, &SplitAt::lines(Vec::new(), Vec::new()), Rect::from_min_size(Pos2::ZERO, Vec2::splat(1.0))).panes.len()
}

/// One pane's view: its own, or linked to the main view's zoom and position.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SplitPane {
    /// The pane's own view; ignored for centre and zoom while `linked`.
    pub view: CanvasView,
    /// The pane follows the main view's centre and zoom (its rotation stays its own).
    pub linked: bool,
}

impl SplitPane {
    /// A pane showing `view`.
    pub const fn new(view: CanvasView, linked: bool) -> Self {
        SplitPane { view, linked }
    }
}

/// One pane as [`SplitViews::show_panes`] gives it to the pane's body.
#[derive(Clone, Copy, Debug, PartialEq, crate::contract::MemoProps)]
#[non_exhaustive]
pub struct SplitPaneAt {
    /// The pane's number in reading order, as [`SplitViewsOutput::activated`] and [`TabDropTarget::Pane`] count it.
    pub index: usize,
    /// The pane's rect in this pass, placed for the layout direction: where the dividers lie as the views draw them.
    pub rect: Rect,
    /// The pane is the active one.
    pub active: bool,
}

/// The panes of the innermost split views, as the views laid them out in the pass, for their body.
#[derive(Clone, Debug, PartialEq, crate::contract::MemoProps)]
struct SplitPanes(Arc<[SplitPaneAt]>);

impl EnvValue for SplitPanes {
    fn root() -> Self {
        SplitPanes(Arc::from(Vec::new()))
    }
}

/// The dividers of `laid` in `rect`: the column lines first, then the row lines, each in order. Each can move until
/// the panes on both sides of it are `least_pane` points across; a line with no room for that stays where it is.
fn dividers(laid: &Laid, rect: Rect, sizes: &DockSizes) -> Vec<Divider> {
    let range = |lines: &[Line], index: usize, start: f32, end: f32| {
        let at = lines.get(index).map_or(start, |line| line.at);
        let before = index.checked_sub(1).and_then(|i| lines.get(i)).map_or(start, |line| line.at);
        let after = lines.get(index + 1).map_or(end, |line| line.at);
        let (lo, hi) = (before + sizes.least_pane, after - sizes.least_pane);
        if lo <= hi { (lo, hi) } else { (at, at) }
    };
    let columns = laid.columns.iter().enumerate().map(|(index, line)| {
        let span = Rect::from_min_max(Pos2::new(line.at, line.from), Pos2::new(line.at, line.to));
        Divider::new(Axis::X, line.at, span, range(&laid.columns, index, rect.left(), rect.right()), sizes.reach)
    });
    let rows = laid.rows.iter().enumerate().map(|(index, line)| {
        let span = Rect::from_min_max(Pos2::new(line.from, line.at), Pos2::new(line.to, line.at));
        Divider::new(Axis::Y, line.at, span, range(&laid.rows, index, rect.top(), rect.bottom()), sizes.reach)
    });
    columns.chain(rows).collect()
}

/// The split with divider `index` of `laid` (numbered as [`dividers`] numbers them) moved to the coordinate `coord`
/// of `rect`, within the divider's range. Every other line stays where `laid` has it, so the lists written always
/// fit the layout.
fn moved(laid: &Laid, index: usize, divider: &Divider, coord: f32, rect: Rect) -> SplitAt {
    let coord = coord.clamp(divider.lo, divider.hi);
    let mut columns: Vec<f32> = laid.columns.iter().map(|line| (line.at - rect.left()) / rect.width()).collect();
    let mut rows: Vec<f32> = laid.rows.iter().map(|line| (line.at - rect.top()) / rect.height()).collect();
    if let Some(column) = columns.get_mut(index) {
        *column = (coord - rect.left()) / rect.width();
    } else if let Some(row) = index.checked_sub(laid.columns.len()).and_then(|row| rows.get_mut(row)) {
        *row = (coord - rect.top()) / rect.height();
    }
    SplitAt { columns, rows }
}

/// The style of split views: the sizes the dividers and their keys read, and the edge of the active view.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SplitStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The docking sizes: the dividers' reach and key steps, and the least extent of a pane along a divider's axis.
    sizes: DockSizes,
    /// The edge of the active pane.
    active_edge: StyleColor,
    /// The width of that edge.
    edge: f32,
}

/// What split views report: the dividers' edits and the pane the user chose.
#[derive(Debug)]
#[non_exhaustive]
pub struct SplitViewsOutput<D = ()> {
    /// The divider edits and the area's response.
    pub edit: EditResponse<SplitAt>,
    /// The pane pressed, or activated by key or assistive technology, this pass that was not the active one: the app
    /// makes it active.
    pub activated: Option<usize>,
    /// The documents dropped on the views this pass, and where.
    pub tab_drop: Option<TabDrop<D>>,
}

/// Where a dragged document strip tab lands on split views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TabDropTarget {
    /// A whole pane of a split body.
    Pane(usize),
    /// The reading-end half of an unsplit body.
    SplitEnd,
}

/// The documents of a dragged tab group and the target they were dropped on.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TabDrop<D> {
    /// The dragged documents.
    pub documents: Vec<D>,
    /// Where they were dropped.
    pub target: TabDropTarget,
}

/// Where a dragged tab may land on `laid` in `rect`, in physical space: each pane of a split body, numbered as `laid`
/// numbers them, or the reading-end half of an unsplit body (the left half under right-to-left).
fn tab_areas(laid: &Laid, rect: Rect, rtl: bool) -> Vec<(TabDropTarget, Rect)> {
    let flow = Flow::new(rect, rtl);
    if laid.panes.len() > 1 {
        laid.panes.iter().enumerate().map(|(pane, area)| (TabDropTarget::Pane(pane), flow.to_physical(*area))).collect()
    } else {
        let end = Rect::from_min_max(Pos2::new(rect.center().x, rect.top()), rect.max);
        vec![(TabDropTarget::SplitEnd, flow.to_physical(end))]
    }
}

/// The views' target for a dragged document strip tab: the area under the pointer, when the caller's rule takes the
/// dragged documents there. The drop carries the area, which the ghost layer marks.
struct TabTarget<'t, D> {
    areas: Vec<(TabDropTarget, Rect)>,
    accepts: &'t dyn Fn(&[D], TabDropTarget) -> bool,
}

impl<D: Clone> DropTarget<ListDrag<D>> for TabTarget<'_, D> {
    type Drop = (TabDrop<D>, Rect);

    fn accept(&self, payload: &ListDrag<D>, at: Pos2, _: DropOperation) -> Option<Self::Drop> {
        let &(target, area) = self.areas.iter().find(|(_, area)| area.contains(at))?;
        (self.accepts)(&payload.keys, target).then(|| (TabDrop { documents: payload.keys.clone(), target }, area))
    }

    fn mark(&self, (drop, area): &Self::Drop) -> DropMark {
        match drop.target {
            TabDropTarget::SplitEnd => DropMark::region(*area),
            TabDropTarget::Pane(_) => DropMark::Into(*area),
        }
    }
}

impl<D> Outcome for SplitViewsOutput<D> {
    type Value = SplitAt;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// Whether a tab drop of these documents may land on a target.
type AcceptsDrop<'a, D> = Box<dyn Fn(&[D], TabDropTarget) -> bool + 'a>;

/// One canvas area shown as views of one document, or a grid of panes the caller fills (one document each), in the
/// panes of a [`SplitLayout`]. The active pane is the app's: it routes tools and keys there. An inactive pane is a
/// Tab stop: a press, Enter or Space reports it in [`SplitViewsOutput::activated`]. By default the first press selects
/// the pane; [`PanePress::Interact`] also sends it to the view. With [`takes_tabs`](SplitViews::takes_tabs) a document
/// strip tab dropped on the views is reported in [`SplitViewsOutput::tab_drop`].
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container, probe(SplitViews<'static, ()>))]
pub struct SplitViews<'a, D: Clone + Send + Sync + 'static = ()> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    layout: SplitLayout,
    at: Bind<'a, SplitAt>,
    /// The active pane: the full interactive view (0 by default).
    #[prop]
    active: usize,
    /// How a press in an inactive pane selects it.
    #[prop]
    pane_press: PanePress,
    /// The documents a tile arrangement shows, one pane each (0 by default); the n-up arrangements show their own
    /// number of panes. [`show_panes`](Self::show_panes) gives each of them a body.
    #[prop]
    count: usize,
    activated: Option<usize>,
    accepts: Option<AcceptsDrop<'a, D>>,
    /// The tab drop released over the views in this pass.
    tab_drop: Option<TabDrop<D>>,
}

impl<'a> SplitViews<'a> {
    /// Split views named `label` with `layout`, the dividers at `at`. A mixed split shows no dividers. Pane 0 is
    /// active until [`active`](Self::active) says otherwise.
    pub fn new(label: impl Into<Label>, layout: SplitLayout, at: Bind<'a, SplitAt>) -> Self {
        let props = ContainerProps::new().arena(true).sizing(Sizing::Fill).pad(Pad::None);
        SplitViews {
            common: Common::new(),
            props,
            layout,
            at,
            active: 0,
            pane_press: PanePress::default(),
            count: 0,
            activated: None,
            accepts: None,
            tab_drop: None,
        }
        .label(label)
    }

    /// Takes a dragged document strip tab over the views: an unsplit body offers its reading-end half, a split body
    /// each whole pane, each only when `accepts` returns true.
    #[must_use]
    pub fn takes_tabs<D: Clone + Send + Sync + 'static>(self, accepts: impl Fn(&[D], TabDropTarget) -> bool + 'a) -> SplitViews<'a, D> {
        let SplitViews { common, props, layout, at, active, pane_press, count, activated, accepts: _, tab_drop: _ } = self;
        SplitViews { common, props, layout, at, active, pane_press, count, activated, accepts: Some(Box::new(accepts)), tab_drop: None }
    }

    /// Copies the main view's centre and zoom into the linked panes, each keeping its own rect and rotation.
    pub fn follow(main: &CanvasView, panes: &mut [SplitPane]) {
        for pane in panes.iter_mut().filter(|pane| pane.linked) {
            pane.view.center = main.center;
            pane.view.zoom = main.zoom;
        }
    }
}

impl<D: Clone + Send + Sync + 'static> SplitViews<'_, D> {
    fn active_pane(&self) -> usize {
        self.active.min(pane_count(self.layout, self.count).saturating_sub(1))
    }

    /// The layout laid out in `rect` with the lines at `at`.
    fn laid(&self, at: &SplitAt, rect: Rect) -> Laid {
        lay_out(self.layout, self.count, at, rect)
    }

    /// Shows the views with a body per pane: `pane` runs once for each pane, in reading order, in a `Ui` held to the
    /// pane's rect and clipped to it, with the pane ([`SplitPaneAt`]) as the views lay it out in this pass, so a
    /// divider drag moves the panes' content in the pass it moves the divider. Each pane is a layout room of its own
    /// ([`in_room`]): a view that fills takes its pane, not a share of the views' body with the other panes' content.
    /// The results come back in reading order.
    pub fn show_panes<R>(self, ui: &mut Ui, mut pane: impl FnMut(&mut Ui, &mut BodyCx<'_>, SplitPaneAt) -> R) -> Inner<SplitViewsOutput<D>, Vec<R>> {
        self.show(ui, |ui, body| {
            let panes = body.env.get::<SplitPanes>().0;
            panes.iter().map(|&at| in_room(ui, at.rect, |ui| pane(ui, body, at))).collect()
        })
    }
}

/// Dragging a divider moves its boundary (one gesture from the drag's start to its release), the arrow keys move the
/// focused divider by a step, Shift or Page by a jump, and assistive technology sets it to a value.
fn resize<D: Clone + Send + Sync + 'static>(cx: &mut Cx<'_, SplitViews<'_, D>>, at: &SplitAt, laid: &Laid, dividers: &[Divider]) {
    let rect = cx.rect;
    for (index, divider) in dividers.iter().enumerate() {
        match divider::pointer(cx, DIVIDER, index as u64, divider) {
            Some(Drag::Start) => cx.begin(),
            Some(Drag::To(coord)) => cx.propose(moved(laid, index, divider, coord, rect)),
            Some(Drag::Release) => cx.commit(),
            None => {}
        }
    }
    let sizes = cx.style.sizes;
    let keyed: Vec<(u64, Divider)> = dividers.iter().enumerate().map(|(index, divider)| (index as u64, *divider)).collect();
    if let Some((index, coord)) = divider::keys(cx, DIVIDER, &keyed, sizes.step, sizes.jump)
        && let Some(divider) = dividers.get(index as usize)
    {
        let next = moved(laid, index as usize, divider, coord, rect);
        if next != *at {
            cx.set(next);
        }
    }
}

impl<D: Clone + Send + Sync + 'static> Component for SplitViews<'_, D> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::SPLITTER;
    type Style = SplitStyle;
    type Output = SplitViewsOutput<D>;

    fn style(cx: &StyleCx<'_>, _: SplitViewsStyleKey<D>) -> SplitStyle {
        SplitStyle {
            metrics: Metrics::new(),
            sizes: DockSizes::style(cx),
            active_edge: cx.color(cx.tokens.color.line.focus),
            edge: cx.tokens.stroke.thin,
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn bind(&mut self) -> Option<BindMut<'_, SplitAt>> {
        Some(self.at.reborrow())
    }

    /// The panes and the dividers. An inactive pane is a click and Tab stop; its press policy decides whether it sits
    /// above or below its view. The active one is inert under its view, so every pane has an accessible node. A divider
    /// is an edge part above the panes, its own Tab stop that the arrow keys move.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let at = cx.value().cloned().unwrap_or_default();
        let active = self.active_pane();
        let flow = Flow::new(rect, cx.flow.is_rtl());
        let laid = self.laid(&at, rect);
        for (pane, pane_rect) in laid.panes.iter().map(|pane| flow.to_physical(*pane)).enumerate() {
            if pane == active {
                cx.part(PANE.at(pane as u64), pane_rect).priority(HitPriority::new(HitLevel::Background, 0));
            } else {
                let part = cx
                    .part(PANE.at(pane as u64), pane_rect)
                    .sense(Sense::CLICK | Sense::TAKE_FOCUS)
                    .priority(HitPriority::new(if self.pane_press == PanePress::Select { HitLevel::Body } else { HitLevel::Background }, 0));
                if self.pane_press == PanePress::Select {
                    part.focusable().over();
                } else {
                    part.focusable();
                }
            }
        }
        if cx.value().is_none() {
            return;
        }
        let sizes = cx.style.sizes;
        for (index, divider) in dividers(&laid, rect, &sizes).iter().enumerate() {
            divider::declare(cx, rect, DIVIDER, index as u64, divider);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(at) = cx.value().cloned() {
            let sizes = cx.style.sizes;
            let laid = self.laid(&at, cx.rect);
            let dividers = dividers(&laid, cx.rect, &sizes);
            resize(cx, &at, &laid, &dividers);
        }
        let active = self.active_pane();
        // A press chooses the pane, as in Photoshop; Enter, Space and assistive technology activate it too.
        let pointer_pane = if self.pane_press == PanePress::Interact {
            let press = crate::runtime::pointer::frame(&cx.ctx).press.filter(|press| press.button == craft_behavior::PointerButton::Primary);
            press.and_then(|press| {
                let shapes = crate::runtime::pointer::shapes_at(&cx.ctx, press.origin);
                // A divider spans both panes; resizing it must not select either one.
                if shapes.iter().any(|(owner, part)| *owner == cx.id && part.is_some_and(|part| part.key == DIVIDER)) {
                    return None;
                }
                (0..pane_count(self.layout, self.count)).find(|pane| shapes.contains(&(cx.id, Some(PANE.at(*pane as u64)))))
            })
        } else {
            None
        };
        let chosen = |pane: &usize| {
            let part = PANE.at(*pane as u64);
            cx.part_if_declared(part).is_some_and(|result| matches!(result.press.phase, craft_behavior::PressPhase::Down { .. }))
                || cx.part_activated(part).is_some()
                || pointer_pane == Some(*pane)
        };
        self.activated = (0..pane_count(self.layout, self.count)).filter(|&pane| pane != active).find(chosen);
        // The views' own target over their rect; it ranks above a strip's tear-off, which takes the window.
        if let Some(accepts) = self.accepts.as_deref() {
            let at = cx.value().cloned().unwrap_or_default();
            let areas = tab_areas(&self.laid(&at, cx.rect), cx.rect, cx.flow.is_rtl());
            let target = TabTarget { areas, accepts };
            self.tab_drop = cx.drop_target::<ListDrag<D>, _>(&target).map(|(drop, _)| drop);
        }
    }

    /// The body reads the panes as this pass lays them out: after the dividers' edits of `update`.
    fn content_env(&self, cx: &Cx<'_, Self>, values: &mut AppValues) {
        let at = cx.value().cloned().unwrap_or_default();
        let flow = Flow::new(cx.rect, cx.flow.is_rtl());
        let active = self.active_pane();
        let panes: Vec<SplitPaneAt> = self
            .laid(&at, cx.rect)
            .panes
            .iter()
            .enumerate()
            .map(|(index, rect)| SplitPaneAt { index, rect: flow.to_physical(*rect), active: index == active })
            .collect();
        values.set(SplitPanes(Arc::from(panes)));
    }

    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let at = cx.value().cloned().unwrap_or_default();
        let laid = self.laid(&at, cx.rect);
        if cx.value().is_some() {
            for (index, divider) in dividers(&laid, cx.rect, &cx.style.sizes).iter().enumerate() {
                ui.face(DIVIDER.at(index as u64), Splitter::new(divider.axis));
            }
        }
        if laid.panes.len() > 1
            && let Some(rect) = laid.panes.get(self.active_pane())
        {
            let rect = Flow::new(cx.rect, cx.flow.is_rtl()).to_physical(*rect);
            ui.draw().stroke(&PaintShape::Rect(rect, CornerRadius::ZERO), StrokeWidth::Thin, cx.style.active_edge);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<SplitWords>();
        let at = cx.value().cloned().unwrap_or_default();
        let laid = self.laid(&at, cx.rect);
        for pane in 0..laid.panes.len() {
            let number = cx.format().number((pane + 1) as f64, 0);
            let name = words.view.fill(&[("n", number.as_str())]);
            access.node(PANE.at(pane as u64), Role::GROUP).name = Name::Text(name);
        }
        let shown = if cx.value().is_some() { dividers(&laid, cx.rect, &cx.style.sizes) } else { Vec::new() };
        for (index, divider) in shown.iter().enumerate() {
            divider::describe(access, DIVIDER, index as u64, divider, words.resize.as_str(), cx.style.sizes.step);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> SplitViewsOutput<D> {
        SplitViewsOutput { edit: cx.edit_response(), activated: self.activated, tab_drop: self.tab_drop }
    }
}

impl<D: Clone + Send + Sync + 'static> Container for SplitViews<'_, D> {}
