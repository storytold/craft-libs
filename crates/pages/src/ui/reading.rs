//! `ReadingView`: Word's Read Mode: the document reflowed into full-height columns side by side, turned one column at
//! a time with the arrows, the wheel, the page-turn buttons or a swipe, as a book.
//!
//! The app lays its text out into pages of [`reading_size`] and gives them to a [`ReadingView`]; this module has the
//! geometry and the turning. [`ReadingNav`] is the row of page-turn buttons under the columns.

use craft_behavior::{IconId, Metrics, NavMove, PartKey, PressPhase, Sense};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{
    Access, Animate as _, Common, Component, Cx, Focus as _, Input, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _,
    Remember as _, Role, Show as _, ShowContainer as _, WordSetCx as _, clicked, spec,
};
use craft_ui::layout_components::Row;
use craft_ui::primitives::textures::{Priority, TextureStoreOptions, ThumbPaint, ThumbRequest, ThumbnailOptions, Thumbnails, texture_side};
use craft_ui::primitives::{PaintShape, ReadingMeasure};
use craft_ui::runtime::commands::FireSource;
use craft_ui::runtime::env::Env;
use craft_ui::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use craft_ui::style::{ComponentStyle, DataColor, FocusLook, StyleCx, UiTokens};
use egui::{CornerRadius, Pos2, Rect, Response, Ui, Vec2, pos2, vec2};

use super::view::PageSource;
use crate::layout::SCREEN_POINTS_PER_POINT;

#[cfg(feature = "screen-testing")]
mod stories;

/// The page-turn parts: one over each gap between the columns and one over each side margin, indexed from the left.
/// A click on one turns toward its side and a swipe starting on one turns by the swipe's direction.
const TURN: PartKey = PartKey::new("turn");

/// How far a swipe along the columns must go to turn, as a share of a column's width.
const SWIPE_SHARE: f32 = 0.2;

/// The most turns one pass's wheel makes either way.
const MOST_TURNS: i32 = 5;

/// The points of wheel that turn one column: one notch.
const WHEEL_TURN: f32 = 40.0;

/// The width change within which the view counts as not resized, screen points.
const SAME_SIZE: f32 = 0.5;

/// The column widths Word's View > Column Width offers, page points (Narrow to Wide).
pub const REQUESTED_WIDTH: std::ops::RangeInclusive<f32> = 144.0..=612.0;

/// The narrowest column a chosen width gives way to on a narrow window, screen points (WordCraft `reading_size`:
/// `.max(144.0)` pixels, whatever the scale).
const NARROWEST_FIT: f32 = 144.0;

/// The most columns side by side (WordCraft).
pub const MAX_COLUMNS: usize = 5;

/// The reading zooms, as Word's View > Column Width offers them (WordCraft clamps 0.75 to 1.5).
pub const READING_ZOOM: std::ops::RangeInclusive<f32> = 0.75..=1.5;

craft_ui::words! {
    /// The view's words (Word's Read Mode).
    #[non_exhaustive]
    pub struct ReadingViewWords {
        previous: "readingView.previous" = "Previous Screen";
        next: "readingView.next" = "Next Screen";
    }
}

/// The sizes the reading layout works from, read from the tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ReadingMetrics {
    /// The gap between columns and around them, screen points.
    pub column_gap: f32,
    /// The height under the columns for the page-turn buttons, screen points.
    pub nav_bar: f32,
    /// One column per this much width, screen points.
    pub column_target: f32,
    /// The least column width, screen points.
    pub column_min: f32,
}

impl ReadingMetrics {
    /// Metrics with these gaps and bar height and the column widths of `measure`.
    pub const fn new(column_gap: f32, nav_bar: f32, measure: ReadingMeasure) -> Self {
        ReadingMetrics { column_gap, nav_bar, column_target: measure.target, column_min: measure.least }
    }

    /// The metrics under `tokens` with the column widths of `measure`, which the app takes from the body text it
    /// lays out: `ReadingMeasure::of(TypeRole::Body, |run| ..)` with the layout of the pass it measures in.
    pub fn of(tokens: &UiTokens, measure: ReadingMeasure) -> Self {
        ReadingMetrics::new(tokens.space.gap.section, tokens.size.panel_header, measure)
    }
}

/// The column size to lay out at, page points, for a viewport `avail` at `scale` screen points per point. With a
/// chosen column width (`requested`, within [`REQUESTED_WIDTH`] page points) that width, at most the window less two
/// gaps; otherwise about one column per `metrics.column_target` screen points, 1 to [`MAX_COLUMNS`], each at least
/// `metrics.column_min`. A chosen width gives way to a window narrower than it, but not below `NARROWEST_FIT` screen
/// points. The height leaves the gap and `metrics.nav_bar` (WordCraft's `READ_NAV_BAR`, #1772).
// wordcraft: crates/ui-egui/src/canvas/reading.rs:49 `reading_size`.
pub fn reading_size(avail: Vec2, scale: f32, metrics: &ReadingMetrics, requested: Option<f32>) -> (f32, f32) {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let width = if avail.x.is_finite() { avail.x.max(0.0) } else { 0.0 };
    let height = if avail.y.is_finite() { avail.y.max(0.0) } else { 0.0 };
    let gap = metrics.column_gap;
    let column = match requested.filter(|points| points.is_finite()) {
        Some(points) => {
            let wanted = points.clamp(*REQUESTED_WIDTH.start(), *REQUESTED_WIDTH.end()) * scale;
            wanted.min((width - 2.0 * gap).max(NARROWEST_FIT))
        }
        None => {
            let count = (width / metrics.column_target).round().clamp(1.0, MAX_COLUMNS as f32);
            ((width - (count + 1.0) * gap) / count).max(metrics.column_min)
        }
    };
    (column / scale, (height - gap - metrics.nav_bar).max(metrics.column_min) / scale)
}

/// The part of `full` the columns scroll in: as many whole columns as fit, centred, so none is cut at the right edge
/// (#1772); half a point of overrun allowed before a column is dropped.
// wordcraft: crates/ui-egui/src/canvas/reading.rs:70 `reading_viewport`.
pub fn reading_viewport(full: Rect, column: f32, gap: f32, scale: f32) -> Rect {
    if !column.is_finite() || column <= 0.0 || !full.width().is_finite() {
        return full;
    }
    let step = column + gap;
    let slack = if scale.is_finite() { (SAME_SIZE * scale).clamp(0.0, 4.0) } else { 0.0 };
    // A hair of tolerance: columns sized to fill the view exactly come back from points as 2.9999998 of them, and
    // flooring that would drop a whole column.
    let whole = ((full.width() - gap) / (step - slack) + 1e-3).floor().max(1.0);
    let width = (whole * step + gap).min(full.width());
    Rect::from_min_size(pos2(full.center().x - width / 2.0, full.min.y), vec2(width, full.height()))
}

/// The turns a wheel scroll of `scroll` points makes, keeping the part of a turn not yet made in `carry`: one notch is
/// one column, a touchpad's small steps add up; at most [`MOST_TURNS`] either way. Scrolling down turns forward.
pub fn wheel_turns(scroll: Vec2, carry: &mut f32) -> i32 {
    let along = scroll.x + scroll.y;
    if !along.is_finite() {
        return 0;
    }
    *carry -= along;
    let whole = (*carry / WHEEL_TURN).trunc();
    *carry -= whole * WHEEL_TURN;
    (whole as i32).clamp(-MOST_TURNS, MOST_TURNS)
}

/// The columns a swipe from `start` to `end` turns (positive: forward), if it ran along the columns for
/// [`SWIPE_SHARE`] of a column. `forward` is 1 (columns run to the right) or -1 (right to left): a swipe against the
/// columns' direction turns forward.
pub fn swipe_turn(start: Pos2, end: Pos2, column: f32, forward: f32) -> i32 {
    let across = end.x - start.x;
    if across.abs() >= column * SWIPE_SHARE && across.abs() > (end.y - start.y).abs() { if across * forward < 0.0 { 1 } else { -1 } } else { 0 }
}

/// The reading view's state, kept by the app.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ReadingState {
    /// The column at the leading edge.
    pub column: usize,
    /// The wheel scroll not yet made into a turn, in points.
    pub wheel: f32,
    /// The text at the start of the first column shown (app-defined position), kept across reflows instead of the
    /// editing caret.
    pub anchor: Option<u64>,
}

impl ReadingState {
    /// Turns `by` columns, staying within the `count` columns.
    pub fn turn(&mut self, by: i32, count: usize) {
        let last = count.saturating_sub(1);
        self.column = (self.column as i64 + i64::from(by)).clamp(0, last as i64) as usize;
    }
}

/// The style of a reading view.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ReadingViewStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The gap between columns.
    gap: f32,
    /// The least width of a side margin's page-turn part.
    reach: f32,
}

/// What the view did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct ReadingViewOutput {
    /// The view's response.
    pub response: Response,
    /// The view's size changed since the last pass: a window resize is running. The columns stay as laid out while it
    /// does and the pictures are not asked for sharp; the app reflows to a new [`reading_size`] once it stops.
    pub resizing: bool,
}

impl Outcome for ReadingViewOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The size the view had in the last pass.
#[derive(Clone, Copy, Debug, Default)]
struct LastSize(Option<Vec2>);

/// The columns, shown over pages the app laid out at [`reading_size`], with [`ReadingMetrics`] gaps between them.
///
/// - Columns in a single row, scrolled horizontally inside [`reading_viewport`]. Left and right arrows, Page Up and
///   Page Down and Space turn a column, Home and End go to the ends (one Tab stop: the view; [`ReadingNav`]'s buttons
///   are their own). The wheel turns a column a notch; a swipe along the columns turns one. The view does not zoom on
///   Ctrl and the wheel: the reading size is the app's.
/// - In a right-to-left interface the row mirrors: the first column is at the right and a swipe toward the left turns
///   back, so an arrow or swipe always moves toward the side it points at.
/// - A click on a gap between the columns or on a side margin turns toward that side (a screen reader's Click on
///   such a part turns forward), and a swipe that starts there turns; Space turns forward. A click or drag inside a
///   column is the app's, so the text stays selectable; the app's editing caret doesn't move the columns.
/// - Every page is painted on `paper` while its picture loads.
/// - Show it over the reading area with `show_at`: it takes the rect it is given.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub struct ReadingView<'a> {
    #[common]
    common: Common,
    source: &'a dyn PageSource,
    state: &'a mut ReadingState,
    /// The page zoom; 1.0 is the default size.
    #[prop(quantity = "zoom factor")]
    zoom: f32,
    resizing: bool,
    paper: DataColor,
}

impl<'a> ReadingView<'a> {
    /// A view named `label` (the document's name) of the pages of `source` on `paper`, with `state` kept by the app
    /// between passes.
    pub fn new(label: impl Into<craft_ui::runtime::env::Label>, source: &'a dyn PageSource, state: &'a mut ReadingState, paper: DataColor) -> Self {
        ReadingView { common: Common::new(), source, state, zoom: 1.0, resizing: false, paper }.label(label)
    }

    /// Screen points per page point at `zoom`.
    pub fn screen_scale(zoom: f32) -> f32 {
        zoom * SCREEN_POINTS_PER_POINT as f32
    }

    fn column_width(&self, scale: f32) -> f32 {
        self.source.page_size(0).width as f32 * scale
    }

    /// The page-turn parts' rects, from the leading edge: the leading margin with the first gap, the gaps between the
    /// columns of `viewport`, the last gap with the trailing margin. A margin part is at least `reach` wide.
    fn turn_rects(full: Rect, viewport: Rect, column: f32, gap: f32, reach: f32, rtl: bool) -> Vec<Rect> {
        let step = column + gap;
        let columns = (((viewport.width() - gap) / step).round().max(1.0)) as usize;
        (0..=columns)
            .map(|at| {
                let (left, right) = (viewport.left() + at as f32 * step, viewport.left() + at as f32 * step + gap);
                let (left, right) = match at {
                    0 => (full.left(), right.max(full.left() + reach)),
                    _ if at == columns => (left.min(full.right() - reach), full.right()),
                    _ => (left, right),
                };
                let (left, right) = if rtl { (full.left() + full.right() - right, full.left() + full.right() - left) } else { (left, right) };
                Rect::from_min_max(pos2(left, full.top()), pos2(right, full.bottom()))
            })
            .collect()
    }

    /// Whether a turn from a click or Click at `x` goes forward: toward the right in a left-to-right interface.
    fn forward_at(x: f32, centre: f32, rtl: bool) -> bool {
        (x > centre) != rtl
    }
}

impl Component for ReadingView<'_> {
    const ROLE: Role = Role::DOCUMENT;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::InlineArrows).with_activate(ActivateKeys::Space).with_moves();
    type Style = ReadingViewStyle;
    type Output = ReadingViewOutput;

    fn style(cx: &StyleCx<'_>, _: ReadingViewStyleKey) -> ReadingViewStyle {
        ReadingViewStyle {
            metrics: Metrics::new().min_size(Vec2::new(cx.tokens.size.plot.width, cx.tokens.size.plot.height)),
            focus: cx.focus_look(),
            gap: cx.tokens.space.gap.section,
            reach: cx.tokens.size.hit.target,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS | Sense::WHEEL_X | Sense::WHEEL_Y
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        if self.source.page_count() == 0 {
            return;
        }
        let scale = Self::screen_scale(self.zoom);
        let (column, gap, reach) = (self.column_width(scale), cx.style.gap, cx.style.reach);
        let viewport = reading_viewport(rect, column, gap, scale);
        for (at, shape) in Self::turn_rects(rect, viewport, column, gap, reach, cx.flow.is_rtl()).into_iter().enumerate() {
            cx.part(TURN.at(at as u64), shape).sense(Sense::CLICK | Sense::DRAG).pointer_only("the arrow keys, Page keys and Space turn the screen");
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        access.pans(false, true);
        if self.source.page_count() == 0 {
            return;
        }
        let words = cx.words::<ReadingViewWords>();
        let scale = Self::screen_scale(self.zoom);
        let (column, gap, reach) = (self.column_width(scale), cx.style.gap, cx.style.reach);
        let viewport = reading_viewport(cx.rect, column, gap, scale);
        let rtl = cx.flow.is_rtl();
        for (at, shape) in Self::turn_rects(cx.rect, viewport, column, gap, reach, rtl).into_iter().enumerate() {
            let forward = Self::forward_at(shape.center().x, viewport.center().x, rtl);
            let word = if forward { &words.next } else { &words.previous };
            let node = access.node(TURN.at(at as u64), Role::BUTTON);
            node.name = Name::Text(cx.env.resolve(word).as_str().to_owned());
            node.bounds(shape);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let size = cx.rect.size();
        let last = cx.memory::<LastSize>().0.replace(size);
        self.resizing = last.is_some_and(|last| (last - size).abs().max_elem() > SAME_SIZE);
        let count = self.source.page_count();
        if count == 0 {
            return;
        }
        let last_column = count - 1;
        let column = self.column_width(Self::screen_scale(self.zoom));
        let rtl = cx.flow.is_rtl();
        let mut turn = 0;
        let mut goal = None;
        if let Some(wheel) = cx.take_wheel() {
            turn += wheel_turns(wheel.scroll, &mut self.state.wheel);
        }
        let viewport = reading_viewport(cx.rect, column, cx.style.gap, Self::screen_scale(self.zoom));
        let parts = Self::turn_rects(cx.rect, viewport, column, cx.style.gap, cx.style.reach, rtl).len();
        for at in 0..parts {
            let part = TURN.at(at as u64);
            match (cx.part_activated(part), cx.part(part).press.phase) {
                (Some(FireSource::Pointer), PressPhase::Release { pos, .. }) => {
                    turn += if Self::forward_at(pos.x, viewport.center().x, rtl) { 1 } else { -1 };
                }
                (Some(FireSource::Access), _) => {
                    turn += if Self::forward_at(cx.part(part).rect.center().x, viewport.center().x, rtl) { 1 } else { -1 };
                }
                (_, PressPhase::Release { start, pos, click: false, .. }) => {
                    turn += swipe_turn(start.origin, pos, column, if rtl { -1.0 } else { 1.0 });
                }
                _ => {}
            }
        }
        while let Some((mv, _)) = cx.intents().take_move() {
            match mv {
                NavMove::Back | NavMove::PageUp => turn -= 1,
                NavMove::Forward | NavMove::PageDown => turn += 1,
                NavMove::Start => goal = Some(0),
                NavMove::End => goal = Some(last_column),
                _ => {}
            }
        }
        if cx.activation().is_some_and(|activation| activation.by_key) {
            turn += 1;
        }
        self.state.column = self.state.column.min(last_column);
        if let Some(goal) = goal {
            self.state.column = goal;
        }
        self.state.turn(turn, count);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let count = self.source.page_count();
        if count == 0 {
            return;
        }
        let scale = Self::screen_scale(self.zoom);
        let (column, gap) = (self.column_width(scale), cx.style.gap);
        let step = column + gap;
        let viewport = reading_viewport(cx.rect, column, gap, scale);
        let offset = cx.anim("offset", self.state.column.min(count - 1) as f32 * step, spec::MOVE);
        let mut painter = ui.content_painter(cx);
        painter.painter = painter.painter.with_clip_rect(viewport.intersect(painter.painter.clip_rect()));
        let ctx = painter.painter.ctx().clone();
        let thumbs = Thumbnails::<usize>::in_context(
            &ctx,
            (cx.response.id, "pictures"),
            ThumbnailOptions::new().max_side(texture_side(&ctx)).store(TextureStoreOptions::pages()),
        );
        thumbs.hold(self.resizing);
        let ppp = ctx.pixels_per_point();
        let rtl = cx.flow.is_rtl();
        let top = viewport.top() + gap;
        for page in 0..count {
            let size = self.source.page_size(page);
            let size = vec2(size.width as f32, size.height as f32) * scale;
            let along = gap + page as f32 * step - offset;
            let x = if rtl { viewport.right() - along - size.x } else { viewport.left() + along };
            let at = Rect::from_min_size(pos2(x, top), size);
            let shown = at.intersects(viewport);
            let near = !shown && [-step, step].into_iter().any(|by| at.translate(vec2(by, 0.0)).intersects(viewport));
            if !shown && !near {
                continue;
            }
            let priority = if shown { Priority::VISIBLE } else { Priority::NEAR };
            let request = ThumbRequest::for_cell(size.x.max(size.y), ppp);
            let picture = thumbs.state(&ctx, self.source, &page, &request, priority);
            if shown {
                // The paper shows under a page that has no picture yet; under a picture the thumbnail paints it.
                ui.draw().clipped(viewport, |draw| draw.data_fill(&PaintShape::Rect(at, CornerRadius::ZERO), self.paper));
                thumbs.paint(&painter, at, &picture, &ThumbPaint::default().paper(self.paper));
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ReadingViewOutput {
        ReadingViewOutput { response: cx.response.clone(), resizing: self.resizing }
    }
}

/// What the page-turn row reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ReadingNavOutput {
    /// The row's response.
    pub response: Response,
    /// The state's column changed: a button turned the screen.
    pub turned: bool,
}

impl Outcome for ReadingNavOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The page-turn buttons under the columns: previous and next, each disabled at its end. A button turns the
/// [`ReadingState`] by one column.
#[must_use = "show the row with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct ReadingNav<'a> {
    state: &'a mut ReadingState,
    count: usize,
}

impl<'a> ReadingNav<'a> {
    /// The buttons for a view of `count` columns in `state`.
    pub fn new(state: &'a mut ReadingState, count: usize) -> Self {
        ReadingNav { state, count }
    }

    /// Shows the row.
    pub fn show(self, ui: &mut Ui) -> ReadingNavOutput {
        let words = Env::of(ui).words::<ReadingViewWords>();
        let (column, count) = (self.state.column, self.count);
        let mut by = 0;
        let row = Row::new().show(ui, |ui, _| {
            let previous = IconButton::new(IconId::new("chevron_left")).label(words.previous.clone()).enabled(column > 0).show(ui);
            let next = IconButton::new(IconId::new("chevron_right")).label(words.next.clone()).enabled(column + 1 < count).show(ui);
            by = i32::from(clicked(&next)) - i32::from(clicked(&previous));
        });
        self.state.turn(by, count);
        ReadingNavOutput { response: row.output, turned: by != 0 }
    }
}
