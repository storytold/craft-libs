//! `SplitViews`: one canvas area shown as views of one document, or a grid of panes the caller fills (one document
//! each). One view is active and is the full interactive view (tools, overlays, keys); the others show their document
//! from their own view and become active on a click. The dividers between panes move with the pointer and, when
//! focused, with the arrow keys.
//!
//! The panes are the cells [`arrange_rects`] gives for the arrangement the caller names, with the dividers moved to
//! the [`SplitAt`] the caller binds ([`SplitAt::placed`]). A linked pane follows the active view's zoom and position
//! ([`SplitViews::follow`]).

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::words::SplitWords;
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Container, ContainerProps, Cx, Edit as _, EditResponse, Name, Outcome, Pad, PaintUi, PartsCx,
    Pointer as _, Props as _, Role, Sizing, WordSetCx as _,
};
use crate::docking::divider::{self, Divider, Drag};
use crate::docking::{DockSizes, Splitter};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::runtime::env::Label;
use crate::runtime::focus::KeyUse;
use crate::style::{ComponentStyle, StyleColor, StyleCx};
use craft_behavior::dock::arrange_rects;
use craft_behavior::{Axis, CanvasView, Flow, HitLevel, HitPriority, Metrics, PartKey, Sense};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};

const DIVIDER: PartKey = PartKey::new("divider");
const PANE: PartKey = PartKey::new("pane");

/// How the canvas area divides: the Window › Arrange arrangements. The n-up and tile arrangements divide it into panes
/// in the reading order [`arrange_rects`] gives (pane 0 at the top start corner); any other entry, or fewer than two
/// panes, shows one pane over the whole area.
pub use craft_behavior::dock::Arrangement as SplitLayout;

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
pub struct SplitViewsOutput {
    /// The divider edits and the area's response.
    pub edit: EditResponse<SplitAt>,
    /// The pane pressed, or activated by key or assistive technology, this pass that was not the active one: the app
    /// makes it active.
    pub activated: Option<usize>,
}

impl Outcome for SplitViewsOutput {
    type Value = SplitAt;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// One canvas area shown as views of one document, or a grid of panes the caller fills (one document each), in the
/// panes of a [`SplitLayout`]. The active pane is the app's: it routes tools and keys there. An inactive pane is a
/// Tab stop over its view: a press, Enter or Space on it reports it in [`SplitViewsOutput::activated`] and its view
/// gets no press.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct SplitViews<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    layout: SplitLayout,
    at: Bind<'a, SplitAt>,
    /// The active pane: the full interactive view (0 by default). The other panes are display-only until clicked.
    #[prop]
    active: usize,
    /// The documents a tile arrangement shows, one pane each (0 by default); the n-up arrangements show their own
    /// number of panes. Pass the same number to [`SplitAt::placed`] for the body's panes.
    #[prop]
    count: usize,
    activated: Option<usize>,
}

impl<'a> SplitViews<'a> {
    /// Split views named `label` with `layout`, the dividers at `at`. A mixed split shows no dividers. Pane 0 is
    /// active until [`active`](Self::active) says otherwise.
    pub fn new(label: impl Into<Label>, layout: SplitLayout, at: Bind<'a, SplitAt>) -> Self {
        let props = ContainerProps::new().arena(true).sizing(Sizing::Fill).pad(Pad::None);
        SplitViews { common: Common::new(), props, layout, at, active: 0, count: 0, activated: None }.label(label)
    }

    /// Copies the main view's centre and zoom into the linked panes, each keeping its own rect and rotation.
    pub fn follow(main: &CanvasView, panes: &mut [SplitPane]) {
        for pane in panes.iter_mut().filter(|pane| pane.linked) {
            pane.view.center = main.center;
            pane.view.zoom = main.zoom;
        }
    }

    fn active_pane(&self) -> usize {
        self.active.min(pane_count(self.layout, self.count).saturating_sub(1))
    }

    /// The layout laid out in `rect` with the lines at `at`.
    fn laid(&self, at: &SplitAt, rect: Rect) -> Laid {
        lay_out(self.layout, self.count, at, rect)
    }
}

/// Dragging a divider moves its boundary (one gesture from the drag's start to its release), the arrow keys move the
/// focused divider by a step, Shift or Page by a jump, and assistive technology sets it to a value.
fn resize(cx: &mut Cx<'_, SplitViews<'_>>, at: &SplitAt, laid: &Laid, dividers: &[Divider]) {
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

impl Component for SplitViews<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::SPLITTER;
    type Style = SplitStyle;
    type Output = SplitViewsOutput;

    fn style(cx: &StyleCx<'_>, _: SplitViewsStyleKey) -> SplitStyle {
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

    /// The panes and the dividers. An inactive pane is a click and Tab stop above its view; the active one is an inert
    /// part under its view, so every pane has an accessible node. A divider is an edge part above the panes, its own
    /// Tab stop that the arrow keys move.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let at = cx.value().cloned().unwrap_or_default();
        let active = self.active_pane();
        let flow = Flow::new(rect, cx.flow.is_rtl());
        let laid = self.laid(&at, rect);
        for (pane, pane_rect) in laid.panes.iter().map(|pane| flow.to_physical(*pane)).enumerate() {
            if pane == active {
                cx.part(PANE.at(pane as u64), pane_rect).priority(HitPriority::new(HitLevel::Background, 0));
            } else {
                cx.part(PANE.at(pane as u64), pane_rect)
                    .sense(Sense::CLICK | Sense::TAKE_FOCUS)
                    .priority(HitPriority::new(HitLevel::Body, 0))
                    .focusable()
                    .over();
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
        let chosen = |pane: &usize| {
            let part = PANE.at(*pane as u64);
            cx.part_if_declared(part).is_some_and(|result| matches!(result.press.phase, craft_behavior::PressPhase::Down { .. }))
                || cx.part_activated(part).is_some()
        };
        self.activated = (0..pane_count(self.layout, self.count)).filter(|&pane| pane != active).find(chosen);
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

    fn finish(self, cx: Cx<'_, Self>) -> SplitViewsOutput {
        SplitViewsOutput { edit: cx.edit_response(), activated: self.activated }
    }
}

impl Container for SplitViews<'_> {}
