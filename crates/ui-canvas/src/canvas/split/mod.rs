//! `SplitViews`: one canvas area shown as 1, 2 or 4 views of the same document. One view is active and is the full
//! interactive view (tools, overlays, keys); the others show the document from their own view and become active on a
//! click. The dividers between views move with the pointer and, when focused, with the arrow keys.
//!
//! The body shows one view per pane at the rects [`SplitLayout::rects`] gives for the split the caller binds. A
//! linked pane follows the active view's zoom and position ([`SplitViews::follow`]).

#[cfg(feature = "testing")]
mod stories;

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
use craft_behavior::{Axis, CanvasView, Flow, HitLevel, HitPriority, Metrics, PartKey, Sense};
use egui::{CornerRadius, Pos2, Rect, Response};

const DIVIDER: PartKey = PartKey::new("divider");
const PANE: PartKey = PartKey::new("pane");

/// How the canvas area divides.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SplitLayout {
    /// One view.
    #[default]
    One,
    /// Two views side by side, the main view on the right.
    Two,
    /// Four views in quarters, the main view at the bottom right.
    Four,
}

/// Where the dividers lie, as fractions of the area: `x` of its width for the vertical divider between side by side
/// views, `y` of its height for the horizontal one of four views.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SplitAt {
    /// The vertical divider's distance from the left edge, over the width.
    pub x: f32,
    /// The horizontal divider's distance from the top edge, over the height.
    pub y: f32,
}

impl SplitAt {
    /// Dividers at fractions `x` and `y` of the area.
    pub const fn new(x: f32, y: f32) -> Self {
        SplitAt { x, y }
    }
}

impl Default for SplitAt {
    fn default() -> Self {
        SplitAt::new(0.5, 0.5)
    }
}

impl SplitLayout {
    /// The number of panes.
    pub fn panes(self) -> usize {
        match self {
            SplitLayout::One => 1,
            SplitLayout::Two => 2,
            SplitLayout::Four => 4,
        }
    }

    /// The panes' rects in `full` placed for the layout direction: [`rects`](Self::rects) reflected across the middle
    /// of `full` under `rtl`, so the dividers' fractions count from the start edge.
    pub fn placed(self, at: SplitAt, full: Rect, rtl: bool) -> Vec<Rect> {
        let flow = Flow::new(full, rtl);
        self.rects(at, full).into_iter().map(|rect| flow.to_physical(rect)).collect()
    }

    /// The panes' rects in `full` with the dividers at `at` counted from the left, the main view (pane 0) first and
    /// the others in reading order.
    pub fn rects(self, at: SplitAt, full: Rect) -> Vec<Rect> {
        let (x, y) = (full.left() + full.width() * at.x, full.top() + full.height() * at.y);
        let quad = |x0, y0, x1, y1| Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1));
        match self {
            SplitLayout::One => vec![full],
            SplitLayout::Two => vec![quad(x, full.top(), full.right(), full.bottom()), quad(full.left(), full.top(), x, full.bottom())],
            SplitLayout::Four => vec![
                quad(x, y, full.right(), full.bottom()),
                quad(full.left(), full.top(), x, y),
                quad(x, full.top(), full.right(), y),
                quad(full.left(), y, x, full.bottom()),
            ],
        }
    }

    /// The panes in reading order: top left first, row by row. The pane numbered `n` in the accessible names is the
    /// `n`th of this order.
    pub fn reading_order(self) -> Vec<usize> {
        match self {
            SplitLayout::One => vec![0],
            SplitLayout::Two => vec![1, 0],
            SplitLayout::Four => vec![1, 2, 3, 0],
        }
    }
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

/// The dividers of `layout` in `rect` at `at`: the vertical one first, then the horizontal one of four views. Each
/// can move until a pane is `least` points across.
fn dividers(layout: SplitLayout, at: SplitAt, rect: Rect, sizes: &DockSizes) -> Vec<Divider> {
    let (x, y) = (rect.left() + rect.width() * at.x, rect.top() + rect.height() * at.y);
    let span = |from: f32, to: f32| (from + sizes.least_pane, (to - sizes.least_pane).max(from + sizes.least_pane));
    let vertical = || Divider::new(Axis::X, x, rect, span(rect.left(), rect.right()), sizes.reach);
    let horizontal = || Divider::new(Axis::Y, y, rect, span(rect.top(), rect.bottom()), sizes.reach);
    match layout {
        SplitLayout::One => Vec::new(),
        SplitLayout::Two => vec![vertical()],
        SplitLayout::Four => vec![vertical(), horizontal()],
    }
}

/// `at` with divider `divider` moved to the coordinate `coord` of `rect`.
fn moved(at: SplitAt, divider: &Divider, coord: f32, rect: Rect) -> SplitAt {
    let coord = coord.clamp(divider.lo, divider.hi);
    match divider.axis {
        Axis::X => SplitAt { x: (coord - rect.left()) / rect.width(), ..at },
        _ => SplitAt { y: (coord - rect.top()) / rect.height(), ..at },
    }
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

/// One canvas area divided into 1, 2 or 4 panes. The active pane is the app's: it routes tools and keys there. An
/// inactive pane is a Tab stop over its view: a press, Enter or Space on it reports it in [`SplitViewsOutput::activated`]
/// and its view gets no press.
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
    activated: Option<usize>,
}

impl<'a> SplitViews<'a> {
    /// Split views named `label` with `layout`, the dividers at `at`. A mixed split shows no dividers. Pane 0 is
    /// active until [`active`](Self::active) says otherwise.
    pub fn new(label: impl Into<Label>, layout: SplitLayout, at: Bind<'a, SplitAt>) -> Self {
        let props = ContainerProps::new().arena(true).sizing(Sizing::Fill).pad(Pad::None);
        SplitViews { common: Common::new(), props, layout, at, active: 0, activated: None }.label(label)
    }

    /// Copies the main view's centre and zoom into the linked panes, each keeping its own rect and rotation.
    pub fn follow(main: &CanvasView, panes: &mut [SplitPane]) {
        for pane in panes.iter_mut().filter(|pane| pane.linked) {
            pane.view.center = main.center;
            pane.view.zoom = main.zoom;
        }
    }

    fn active_pane(&self) -> usize {
        self.active.min(self.layout.panes() - 1)
    }
}

/// Dragging a divider moves its boundary (one gesture from the drag's start to its release), the arrow keys move the
/// focused divider by a step, Shift or Page by a jump, and assistive technology sets it to a value.
fn resize(cx: &mut Cx<'_, SplitViews<'_>>, at: SplitAt, dividers: &[Divider]) {
    let rect = cx.rect;
    for (index, divider) in dividers.iter().enumerate() {
        match divider::pointer(cx, DIVIDER, index as u64, divider) {
            Some(Drag::Start) => cx.begin(),
            Some(Drag::To(coord)) => cx.propose(moved(at, divider, coord, rect)),
            Some(Drag::Release) => cx.commit(),
            None => {}
        }
    }
    let sizes = cx.style.sizes;
    let keyed: Vec<(u64, Divider)> = dividers.iter().enumerate().map(|(index, divider)| (index as u64, *divider)).collect();
    if let Some((index, coord)) = divider::keys(cx, DIVIDER, &keyed, sizes.step, sizes.jump)
        && let Some(divider) = dividers.get(index as usize)
    {
        let next = moved(at, divider, coord, rect);
        if next != at {
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
        let at = cx.value().copied().unwrap_or_default();
        let active = self.active_pane();
        let flow = Flow::new(rect, cx.flow.is_rtl());
        for (pane, pane_rect) in self.layout.placed(at, rect, flow.is_rtl()).into_iter().enumerate() {
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
        for (index, divider) in dividers(self.layout, at, rect, &sizes).iter().enumerate() {
            divider::declare(cx, rect, DIVIDER, index as u64, divider);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(at) = cx.value().copied() {
            let sizes = cx.style.sizes;
            let dividers = dividers(self.layout, at, cx.rect, &sizes);
            resize(cx, at, &dividers);
        }
        let active = self.active_pane();
        // A press chooses the pane, as in Photoshop; Enter, Space and assistive technology activate it too.
        let chosen = |pane: &usize| {
            let part = PANE.at(*pane as u64);
            cx.part_if_declared(part).is_some_and(|result| matches!(result.press.phase, craft_behavior::PressPhase::Down { .. }))
                || cx.part_activated(part).is_some()
        };
        self.activated = (0..self.layout.panes()).filter(|&pane| pane != active).find(chosen);
    }

    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let at = cx.value().copied().unwrap_or_default();
        if cx.value().is_some() {
            for (index, divider) in dividers(self.layout, at, cx.rect, &cx.style.sizes).iter().enumerate() {
                ui.face(DIVIDER.at(index as u64), Splitter::new(divider.axis));
            }
        }
        if self.layout.panes() > 1
            && let Some(rect) = self.layout.placed(at, cx.rect, cx.flow.is_rtl()).get(self.active_pane())
        {
            ui.draw().stroke(&PaintShape::Rect(*rect, CornerRadius::ZERO), StrokeWidth::Thin, cx.style.active_edge);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<SplitWords>();
        for (number, pane) in self.layout.reading_order().into_iter().enumerate() {
            let number = cx.format().number((number + 1) as f64, 0);
            let name = words.view.fill(&[("n", number.as_str())]);
            access.node(PANE.at(pane as u64), Role::GROUP).name = Name::Text(name);
        }
        let at = cx.value().copied().unwrap_or_default();
        let shown = if cx.value().is_some() { dividers(self.layout, at, cx.rect, &cx.style.sizes) } else { Vec::new() };
        for (index, divider) in shown.iter().enumerate() {
            divider::describe(access, DIVIDER, index as u64, divider, words.resize.as_str(), cx.style.sizes.step);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> SplitViewsOutput {
        SplitViewsOutput { edit: cx.edit_response(), activated: self.activated }
    }
}

impl Container for SplitViews<'_> {}
