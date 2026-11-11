//! `MarginHandles`: the page margins as draggable edges on the rulers (Word, Excel's Page Layout view). The grey part of
//! each ruler is the margin, the white the text area; the pointer over an edge shows a resize cursor and the margin's
//! name, and a drag moves the margin as one edit gesture (one undo step).

#[cfg(feature = "screen-testing")]
mod stories;

use std::ops::RangeInclusive;

use craft_behavior::kurbo::Point;
use craft_behavior::{Axis, Metrics, PartKey, PressPhase};
use craft_ui::canvas::{RulerMargins, Rulers, readout};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Outcome, PaintUi, PartsCx, Pointer as _, Props,
    Role, WordSetCx as _,
};
use craft_ui::docking::divider::{self, Divider, Drag};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::{Label, UnitKind, Units};
use craft_ui::runtime::focus::KeyUse;
use craft_ui::runtime::memory::Remember as _;
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{Pos2, Rect, Response, Vec2};

use crate::setup::Margins;
use crate::ui::view::PlacedPage;

/// The edge `EDGE.at(n)` with `n` the edge's index in [`MarginEdge::ALL`].
const EDGE: PartKey = PartKey::new("edge");

/// The least text width (or height) a margin drag leaves, points: one inch (Word). GridCraft leaves 36 pt; an app sets
/// its own with [`MarginHandles::min_text`].
pub const MIN_TEXT: f64 = 72.0;

/// The step a dragged margin snaps to, points: 1/16 inch (Word). Alt drags freely.
pub const SNAP: f64 = 4.5;

/// An arrow key's large step as a multiple of the snap step (Shift).
const JUMP: f64 = 10.0;

craft_ui::words! {
    /// The edges' names, shown as tooltips and AccessKit names.
    pub struct MarginHandlesWords {
        left: "marginHandles.left" = "Left Margin";
        right: "marginHandles.right" = "Right Margin";
        top: "marginHandles.top" = "Top Margin";
        bottom: "marginHandles.bottom" = "Bottom Margin";
        header: "marginHandles.header" = "Header Margin";
        footer: "marginHandles.footer" = "Footer Margin";
    }
}

/// An edge a margin drag moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MarginEdge {
    /// The left margin.
    Left,
    /// The right margin.
    Right,
    /// The top margin.
    Top,
    /// The bottom margin.
    Bottom,
    /// The header's distance from the top edge.
    Header,
    /// The footer's distance from the bottom edge.
    Footer,
}

impl MarginEdge {
    /// Every edge, in Tab order.
    pub const ALL: [MarginEdge; 6] =
        [MarginEdge::Left, MarginEdge::Right, MarginEdge::Top, MarginEdge::Bottom, MarginEdge::Header, MarginEdge::Footer];

    /// Whether the edge is on the horizontal ruler.
    fn horizontal(self) -> bool {
        matches!(self, MarginEdge::Left | MarginEdge::Right)
    }

    fn index(self) -> u64 {
        Self::ALL.iter().position(|edge| *edge == self).unwrap_or_default() as u64
    }

    fn name(self, words: &MarginHandlesWords) -> &Label {
        match self {
            MarginEdge::Left => &words.left,
            MarginEdge::Right => &words.right,
            MarginEdge::Top => &words.top,
            MarginEdge::Bottom => &words.bottom,
            MarginEdge::Header => &words.header,
            MarginEdge::Footer => &words.footer,
        }
    }
}

/// A margin's new value: the stored edge (on an even page of mirrored margins the visible left edge is the stored right
/// margin) and its points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct MarginChange {
    /// The margin that moved.
    pub edge: MarginEdge,
    /// Its new size in points.
    pub points: f64,
}

impl MarginChange {
    /// `edge` set to `points`.
    pub const fn new(edge: MarginEdge, points: f64) -> Self {
        MarginChange { edge, points }
    }
}

/// The distances the handles edit: the four margins and the header and footer distances, points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PageEdges {
    /// The page's margins.
    pub margins: Margins,
    /// The header's distance from the top edge; the top margin never drops below it.
    pub header: f64,
    /// The footer's distance from the bottom edge; the bottom margin never drops below it.
    pub footer: f64,
}

impl PageEdges {
    /// Edges with `margins` and no header or footer.
    pub const fn new(margins: Margins) -> Self {
        PageEdges { margins, header: 0.0, footer: 0.0 }
    }

    /// The header and footer distances.
    #[must_use]
    pub const fn header_footer(mut self, header: f64, footer: f64) -> Self {
        (self.header, self.footer) = (header, footer);
        self
    }

    /// The stored value of `edge`, points.
    pub fn get(&self, edge: MarginEdge) -> f64 {
        match edge {
            MarginEdge::Left => self.margins.left,
            MarginEdge::Right => self.margins.right,
            MarginEdge::Top => self.margins.top,
            MarginEdge::Bottom => self.margins.bottom,
            MarginEdge::Header => self.header,
            MarginEdge::Footer => self.footer,
        }
    }

    /// The edges after `change`.
    #[must_use]
    pub fn with(mut self, change: MarginChange) -> Self {
        match change.edge {
            MarginEdge::Left => self.margins.left = change.points,
            MarginEdge::Right => self.margins.right = change.points,
            MarginEdge::Top => self.margins.top = change.points,
            MarginEdge::Bottom => self.margins.bottom = change.points,
            MarginEdge::Header => self.header = change.points,
            MarginEdge::Footer => self.footer = change.points,
        }
        self
    }
}

/// What a margin drag works against: the page, its margins and the drag's rules.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct MarginRules {
    /// Page width and height, points.
    pub page: (f64, f64),
    /// The margins and the header and footer distances.
    pub edges: PageEdges,
    /// Binding room added to the inside margin, points.
    pub gutter: f64,
    /// An even page of mirrored margins.
    pub mirrored_even: bool,
    /// The snap step, points; 0 for none.
    pub snap: f64,
    /// The least text width or height left, points.
    pub min_text: f64,
}

impl MarginRules {
    /// The rules for a page `page` points across and down with `edges`: no gutter, margins not mirrored, snapping to
    /// [`SNAP`] and leaving [`MIN_TEXT`].
    pub const fn new(page: (f64, f64), edges: PageEdges) -> Self {
        MarginRules { page, edges, gutter: 0.0, mirrored_even: false, snap: SNAP, min_text: MIN_TEXT }
    }

    /// The gutter, points.
    #[must_use]
    pub const fn gutter(mut self, gutter: f64) -> Self {
        self.gutter = gutter;
        self
    }

    /// The page is an even page of mirrored margins.
    #[must_use]
    pub const fn mirrored_even(mut self, on: bool) -> Self {
        self.mirrored_even = on;
        self
    }

    /// The snap step, points; 0 for none.
    #[must_use]
    pub const fn snap(mut self, points: f64) -> Self {
        self.snap = points;
        self
    }

    /// The least text width or height left, points.
    #[must_use]
    pub const fn min_text(mut self, points: f64) -> Self {
        self.min_text = points;
        self
    }

    /// The stored edge a visible one edits.
    fn stored(&self, visible: MarginEdge) -> MarginEdge {
        match visible {
            MarginEdge::Left if self.mirrored_even => MarginEdge::Right,
            MarginEdge::Right if self.mirrored_even => MarginEdge::Left,
            edge => edge,
        }
    }

    /// Where the visible `edge` lies for a stored value of `points`: points from the page's left (horizontal edges) or
    /// top (vertical ones).
    fn position(&self, visible: MarginEdge, points: f64) -> f64 {
        let (width, height) = self.page;
        match visible {
            MarginEdge::Left if self.mirrored_even => points,
            MarginEdge::Left => points + self.gutter,
            MarginEdge::Right if self.mirrored_even => width - (points + self.gutter),
            MarginEdge::Right => width - points,
            MarginEdge::Top | MarginEdge::Header => points,
            MarginEdge::Bottom | MarginEdge::Footer => height - points,
        }
    }
}

/// The values the stored `edge` can take under `rules`, points: from 0 (or, for the top and bottom margins, the header
/// and footer distances) to the largest that leaves `rules.min_text` of text and the gutter; a header or footer stays
/// within its page margin.
pub fn margin_range(edge: MarginEdge, rules: &MarginRules) -> RangeInclusive<f64> {
    let (width, height) = rules.page;
    let (m, e) = (&rules.edges.margins, &rules.edges);
    let (least, most) = match edge {
        MarginEdge::Left => (0.0, width - m.right - rules.gutter - rules.min_text),
        MarginEdge::Right => (0.0, width - m.left - rules.gutter - rules.min_text),
        MarginEdge::Top => (e.header, height - m.bottom - rules.min_text),
        MarginEdge::Bottom => (e.footer, height - m.top - rules.min_text),
        MarginEdge::Header => (0.0, m.top),
        MarginEdge::Footer => (0.0, m.bottom),
    };
    let least = least.max(0.0);
    least..=most.max(least)
}

/// The stored `edge` set to `points`: snapped to `rules.snap` (none when 0) and kept within [`margin_range`]. A value
/// that is not finite leaves the edge as it is.
pub fn clamp_margin(edge: MarginEdge, points: f64, rules: &MarginRules) -> MarginChange {
    let range = margin_range(edge, rules);
    if !points.is_finite() || !range.end().is_finite() {
        return MarginChange::new(edge, rules.edges.get(edge));
    }
    let snapped = if rules.snap > 0.0 { (points / rules.snap).round() * rules.snap } else { points };
    MarginChange::new(edge, snapped.clamp(*range.start(), *range.end()))
}

/// The margin a drag of the visible `edge` to `at` points from the page's left (or top) edge gives: snapped to
/// `rules.snap` (none when 0), never negative, leaving `rules.min_text` between it and the opposite margin (and the
/// gutter). On an even page of mirrored margins the pointer is reflected across the page and the edge swapped
/// (WordCraft #2914). A header or footer stays within its page margin, and the top and bottom margins stay beyond them.
pub fn dragged_margin(edge: MarginEdge, at: f64, rules: &MarginRules) -> MarginChange {
    let swapped = rules.mirrored_even && matches!(edge, MarginEdge::Left | MarginEdge::Right);
    let (edge, at) = match edge {
        MarginEdge::Left if swapped => (MarginEdge::Right, rules.page.0 - at),
        MarginEdge::Right if swapped => (MarginEdge::Left, rules.page.0 - at),
        edge => (edge, at),
    };
    let (width, height) = rules.page;
    let raw = match edge {
        MarginEdge::Left => at - rules.gutter,
        MarginEdge::Right => width - at,
        MarginEdge::Top | MarginEdge::Header => at,
        MarginEdge::Bottom | MarginEdge::Footer => height - at,
    };
    clamp_margin(edge, raw, rules)
}

/// The style of the margin handles.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct MarginHandlesStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// How far from an edge the pointer still grabs it.
    grab: f32,
    /// The ruler strips' thickness: the edges lie on the strips along the top and left of the canvas.
    strip: f32,
    active: StyleColor,
}

/// The press that began a drag: the edges, and the pointer's start.
#[derive(Clone, Copy, Default)]
struct Anchor(PageEdges);

/// The margin edges of one page on the horizontal and vertical ruler strips of the canvas.
///
/// - The edges lie on the rulers' strips (the canvas rect outside `Rulers::content_rect`): show the handles after the
///   rulers in the canvas body so a press on an edge reaches them before the strip. The rulers paint the margin shade
///   themselves: the app gives them [`spans`](MarginHandles::spans) through `Rulers::margins`.
/// - Each edge grabs within a slop of it on its strip; the cursor is a resize cursor; the tooltip names it, and while
///   dragging the value shows beside the pointer in the [`unit`](MarginHandles::unit).
/// - A drag is one edit gesture: Begin, Update per move, Commit on release with [`dragged_margin`]'s value; Escape
///   cancels. Alt drags without snapping.
/// - The edges are one Tab stop each (a splitter named by [`MarginHandlesWords`], valued in the unit, its range
///   [`margin_range`]); the arrow keys along the strip move the edge one [`snap`](MarginHandles::snap) step toward
///   the arrow as one `Set`, Shift ten steps. A screen reader's SetValue sets the margin.
#[derive(Component, Props)]
#[component(canvas_control)]
pub struct MarginHandles<'a> {
    #[common]
    common: Common,
    page: &'a PlacedPage,
    edges: Bind<'a, PageEdges>,
    /// Binding room added to the inside margin, points.
    #[prop(quantity = "document points")]
    gutter: f64,
    /// The page is an even page of mirrored margins.
    #[prop]
    mirrored_even: bool,
    /// The header and footer distances are two more edges on the vertical ruler (Excel).
    #[prop(flag)]
    header_footer: bool,
    /// The snap step in points; 0 turns snapping off.
    #[prop(quantity = "document points")]
    snap: f64,
    /// The least width of text a dragged margin leaves between it and the opposite margin, points.
    #[prop(quantity = "document points")]
    min_text: f64,
    /// The unit the drag readout and the AccessKit value show (the app's ruler unit).
    #[prop]
    unit: UnitKind,
}

impl<'a> MarginHandles<'a> {
    /// The handles named `label` of `page`, whose distances are `edges`.
    pub fn new(label: impl Into<Label>, page: &'a PlacedPage, edges: Bind<'a, PageEdges>) -> Self {
        MarginHandles {
            common: Common::new(),
            page,
            edges,
            gutter: 0.0,
            mirrored_even: false,
            header_footer: false,
            snap: SNAP,
            min_text: MIN_TEXT,
            unit: UnitKind::Inches,
        }
        .label(label)
        .ltr_axis()
    }

    /// The margin bands of the page as document ranges along the x axis and along the y axis, for `Rulers::margins`;
    /// `origin` is the page's top-left corner in the rulers' document space. The ranges follow the visible edges (an
    /// even page of mirrored margins swaps them).
    pub fn spans(rules: &MarginRules, origin: Point) -> RulerMargins {
        let (width, height) = rules.page;
        let at = |edge: MarginEdge| rules.position(edge, rules.edges.get(rules.stored(edge)));
        let (left, right) = (at(MarginEdge::Left), at(MarginEdge::Right));
        let (top, bottom) = (at(MarginEdge::Top), at(MarginEdge::Bottom));
        RulerMargins::new(
            [origin.x..origin.x + left, origin.x + right..origin.x + width],
            [origin.y..origin.y + top, origin.y + bottom..origin.y + height],
        )
    }

    /// The edges on the rulers, in Tab order.
    fn edges_shown(&self) -> &'static [MarginEdge] {
        if self.header_footer { &MarginEdge::ALL } else { &MarginEdge::ALL[..4] }
    }

    fn page_size(&self) -> (f64, f64) {
        let rect = self.page.rect;
        (f64::from(rect.width()) / self.page.scale, f64::from(rect.height()) / self.page.scale)
    }

    fn rules(&self, edges: PageEdges, snap: f64) -> MarginRules {
        MarginRules::new(self.page_size(), edges).gutter(self.gutter).mirrored_even(self.mirrored_even).snap(snap).min_text(self.min_text)
    }

    /// The screen coordinate along the visible edge's axis.
    fn screen(&self, rules: &MarginRules, visible: MarginEdge) -> f32 {
        self.screen_of(rules, visible, rules.edges.get(rules.stored(visible)))
    }

    /// The screen coordinate of the visible edge when its margin is `points`.
    fn screen_of(&self, rules: &MarginRules, visible: MarginEdge, points: f64) -> f32 {
        let origin = if visible.horizontal() { self.page.rect.min.x } else { self.page.rect.min.y };
        origin + (rules.position(visible, points) * self.page.scale) as f32
    }

    /// The divider of `visible` in screen coordinates: its bar on the ruler strip `(grab, strip)` thick of `rect`, moving
    /// between the edge's least and most position.
    fn divider(&self, rules: &MarginRules, visible: MarginEdge, rect: Rect, (grab, strip): (f32, f32)) -> Divider {
        let range = margin_range(rules.stored(visible), rules);
        let (a, b) = (self.screen_of(rules, visible, *range.start()), self.screen_of(rules, visible, *range.end()));
        let (axis, span) = if visible.horizontal() {
            (Axis::X, Rect::from_x_y_ranges(rect.x_range(), rect.top()..=rect.top() + strip))
        } else {
            (Axis::Y, Rect::from_x_y_ranges(rect.left()..=rect.left() + strip, rect.y_range()))
        };
        Divider::new(axis, self.screen(rules, visible), span, (a.min(b), a.max(b)), grab)
    }

    /// `points` in the display unit with its symbol.
    fn shown(&self, units: &Units, points: f64) -> String {
        let value = self.unit.from_pixels(points, 0.0, UnitKind::POINTS_PER_INCH, UnitKind::POINTS_PER_INCH).unwrap_or(points);
        units.show(self.unit, value)
    }
}

impl Component for MarginHandles<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::SPLITTER;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = MarginHandlesStyle;
    type Output = MarginHandlesOutput;

    fn style(cx: &StyleCx<'_>, _: MarginHandlesStyleKey) -> MarginHandlesStyle {
        MarginHandlesStyle {
            metrics: Metrics::new(),
            grab: cx.tokens.size.hit.target,
            strip: Rulers::content_rect(cx, Rect::ZERO).min.x,
            active: cx.color(cx.tokens.color.accent.solid),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, PageEdges>> {
        Some(self.edges.reborrow())
    }

    /// The edges: drag parts on the strips at edge priority, each its own Tab stop that the arrow keys move.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(edges) = cx.value().copied() else { return };
        let rules = self.rules(edges, 0.0);
        let (grab, strip) = (cx.style.grab, cx.style.strip);
        for &visible in self.edges_shown() {
            divider::declare(cx, rect, EDGE, visible.index(), &self.divider(&rules, visible, rect, (grab, strip)));
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(edges) = cx.value().copied() else { return };
        let units = cx.env.units();
        let words = cx.words::<MarginHandlesWords>();
        let (rect, scale) = (cx.rect, self.page.scale);
        let (grab, strip) = (cx.style.grab, cx.style.strip);
        let rules = self.rules(edges, self.snap);
        let mut dividers = Vec::new();
        for &visible in self.edges_shown() {
            let stored = rules.stored(visible);
            let divider = self.divider(&rules, visible, rect, (grab, strip));
            dividers.push((visible.index(), divider));
            let press = cx.part(EDGE.at(visible.index())).press;
            let snap = if press.mods.alt { 0.0 } else { self.snap };
            let origin = if visible.horizontal() { self.page.rect.min.x } else { self.page.rect.min.y };
            match divider::pointer(cx, EDGE, visible.index(), &divider) {
                Some(Drag::Start) => {
                    *cx.press_slot::<Anchor>() = Some(Anchor(edges));
                    cx.begin();
                }
                Some(Drag::To(coord)) => {
                    let anchor = cx.press_slot::<Anchor>().map_or(edges, |anchor| anchor.0);
                    let change = dragged_margin(visible, f64::from(coord - origin) / scale, &self.rules(anchor, snap));
                    cx.propose(anchor.with(change));
                    if let PressPhase::Drag { pos, .. } = press.phase {
                        let name = visible.name(&words).clone();
                        readout(cx, pos, &[(name, self.shown(&units, change.points))]);
                    }
                }
                Some(Drag::Release) => cx.commit(),
                _ => {}
            }
            if let Some(value) = cx.intents().take_set_value_at(EDGE.at(visible.index())) {
                cx.set(edges.with(clamp_margin(stored, value, &self.rules(edges, 0.0))));
            }
        }
        let step = if self.snap > 0.0 { self.snap } else { SNAP } * scale;
        if let Some((index, coord)) = divider::keys(cx, EDGE, &dividers, step as f32, (step * JUMP) as f32)
            && let Some(&visible) = self.edges_shown().iter().find(|visible| visible.index() == index)
        {
            let origin = if visible.horizontal() { self.page.rect.min.x } else { self.page.rect.min.y };
            let change = dragged_margin(visible, f64::from(coord - origin) / scale, &rules);
            if change.points != edges.get(rules.stored(visible)) {
                cx.set(edges.with(change));
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(edges) = cx.value().copied() else { return };
        let rules = self.rules(edges, 0.0);
        let rect = cx.rect;
        let focused = cx.focus_visible().then(|| cx.active_part()).flatten().filter(|part| part.key == EDGE).and_then(|part| part.index);
        let draw = ui.draw();
        for &visible in self.edges_shown() {
            let part = cx.part(EDGE.at(visible.index()));
            if !(part.hovered || part.press.phase.is_press() || focused == Some(visible.index())) {
                continue;
            }
            let at = self.screen(&rules, visible);
            let line = if visible.horizontal() {
                [Pos2::new(at, rect.top()), Pos2::new(at, rect.top() + cx.style.strip)]
            } else {
                [Pos2::new(rect.left(), at), Pos2::new(rect.left() + cx.style.strip, at)]
            };
            draw.stroke(&PaintShape::Path(line.to_vec()), StrokeWidth::Medium, cx.style.active);
        }
    }

    /// One splitter per edge, named by the margin and valued by it in the unit.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(edges) = cx.value().copied() else { return };
        let words = cx.words::<MarginHandlesWords>();
        let rules = self.rules(edges, 0.0);
        for &visible in self.edges_shown() {
            let stored = rules.stored(visible);
            let range = margin_range(stored, &rules);
            let mut divider = self.divider(&rules, visible, cx.rect, (cx.style.grab, cx.style.strip));
            (divider.at, divider.lo, divider.hi) = (edges.get(stored) as f32, *range.start() as f32, *range.end() as f32);
            divider::describe(access, EDGE, visible.index(), &divider, visible.name(&words).as_str(), self.snap as f32);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> MarginHandlesOutput {
        MarginHandlesOutput { edit: cx.edit_response() }
    }
}

/// What the margin handles report.
#[derive(Debug)]
#[non_exhaustive]
pub struct MarginHandlesOutput {
    /// The edit of the edges: a drag is one `Begin` to `Commit`; an arrow press is a `Set`.
    pub edit: EditResponse<PageEdges>,
}

impl Outcome for MarginHandlesOutput {
    type Value = PageEdges;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}
