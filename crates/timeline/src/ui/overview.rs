//! The whole timeline in miniature (N1.13).

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::{Axis, Metrics, PointerButton, PressPhase, Sense};
use craft_ui::contract::{
    Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, Outcome, PaintUi, Pointer as _, Props as _, Role,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use egui::{Color32, CornerRadius, Pos2, Rangef, Rect, Response, Vec2, pos2};

use crate::time::{Tick, TimeRange, TimeView};

/// The overview shows this long past the content's end (SoundCraft `universe.rs:16`).
const TAIL_SECONDS: f64 = 10.0;
/// The room a run leaves above and below in its row, and the least width of a run, in points
/// (SoundCraft `universe.rs:25`, `:26`).
const RUN_INSET: f32 = 0.5;
const RUN_MIN_WIDTH: f32 = 1.0;
/// The overview's height, in control heights.
const HEIGHT_IN_CONTROLS: f32 = 1.5;

/// The time the overview spans: the content's end plus the tail.
fn overview_total(end: Tick) -> Tick {
    end + Tick::from_seconds_f64(TAIL_SECONDS)
}

/// A run of touching clips of one colour on one track, along the overview's width.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct OverviewRun {
    /// The track's row, from the top.
    pub track: usize,
    /// The run's start and end, in points from the overview's start edge.
    pub span: Rangef,
    /// The run's fill colour.
    pub color: Color32,
}

/// The runs of every track of a timeline, laid out along an overview of some width; [`TimelineOverview`] spreads the
/// tracks' rows over its own height.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct OverviewRuns {
    /// The number of tracks, each a row.
    pub tracks: usize,
    /// The width the runs were laid out for, in points.
    pub width: f32,
    /// The runs, track by track.
    pub runs: Vec<OverviewRun>,
}

/// The overview's runs for `tracks` (each a list of clip ranges and colours, in time order) along an overview `width`
/// points across, over the content's `end` plus the tail the overview adds. Each clip is at least one point wide, and
/// neighbours of one colour that touch or overlap merge into one run, which covers exactly the same pixels in the same
/// order (a drum edit holds tens of thousands of clips under a point wide). The app caches the result per document
/// revision and width.
// soundcraft: crates/ui-egui/src/edit_window/universe.rs:15 `universe_runs`.
pub fn overview_runs(tracks: &[Vec<(TimeRange, Color32)>], end: Tick, width: f32) -> OverviewRuns {
    let total = overview_total(end).0.max(1) as f32;
    let mut runs = Vec::new();
    for (track, clips) in tracks.iter().enumerate() {
        let block = |from: f32, to: f32, color: Color32| OverviewRun { track, span: Rangef::new(from, to), color };
        let mut run: Option<(f32, f32, Color32)> = None;
        for &(range, color) in clips {
            let from = range.start.0 as f32 / total * width;
            let to = (range.end().0 as f32 / total * width).max(from + RUN_MIN_WIDTH);
            run = match run {
                Some((a, b, held)) if held == color && from >= a && from <= b => Some((a, b.max(to), held)),
                Some((a, b, held)) => {
                    runs.push(block(a, b, held));
                    Some((from, to, color))
                }
                None => Some((from, to, color)),
            };
        }
        if let Some((a, b, held)) = run {
            runs.push(block(a, b, held));
        }
    }
    OverviewRuns { tracks: tracks.len(), width, runs }
}

/// The style of the overview.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct OverviewStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The outline of the box that marks the visible window.
    window: StyleColor,
    /// The playhead's line.
    playhead: StyleColor,
    /// The window box's corners.
    radius: CornerRadius,
    /// How far the window box reaches above and below the runs.
    overhang: f32,
    /// The overview's height.
    height: f32,
}

/// Pro Tools' Universe (SoundCraft): the runs, the visible window as a box, the playhead while playing; a click or
/// drag centres the window at the pointer, changing the view's start. Total time is the content end plus 10
/// seconds. Not a Tab stop (pointer-only; the time area scrolls from the keyboard); AccessKit: an image named by
/// the overview's label.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct TimelineOverview<'a> {
    #[common]
    common: Common,
    runs: &'a OverviewRuns,
    view: Bind<'a, TimeView>,
    end: Tick,
    /// The width of the timeline's time area, in points: the width of the window box.
    #[prop(quantity = "points")]
    shown_width: f32,
    /// The playhead to draw; none draws none.
    #[prop]
    playhead: Option<Tick>,
}

impl<'a> TimelineOverview<'a> {
    /// An overview named `label` of `runs` over `end`, that edits `view`.
    pub fn new(label: impl Into<Label>, runs: &'a OverviewRuns, view: Bind<'a, TimeView>, end: Tick) -> Self {
        TimelineOverview { common: Common::new(), runs, view, end, shown_width: 0.0, playhead: None }.label(label).ltr_axis()
    }

    /// The fraction of the overview's span the window covers.
    fn shown_fraction(&self, view: &TimeView) -> f64 {
        let seconds = f64::from(self.shown_width) / view.points_per_second.max(f64::EPSILON);
        Tick::from_seconds_f64(seconds).0 as f64 / overview_total(self.end).0.max(1) as f64
    }

    /// The view that centres the window at `x`, on an overview in `rect`.
    fn centred(&self, view: &TimeView, rect: Rect, x: f32) -> TimeView {
        let at = f64::from((x - rect.min.x) / rect.width().max(f32::EPSILON));
        let first = (at - self.shown_fraction(view) / 2.0).clamp(0.0, 1.0);
        TimeView { start: Tick((first * overview_total(self.end).0.max(1) as f64).round() as i64), ..*view }
    }

    /// Where `run` sits on an overview in `rect`: its track's row of the band the window box overhangs, inset by
    /// [`RUN_INSET`] above and below, and its span stretched from the width the runs were laid out for.
    fn run_rect(&self, run: &OverviewRun, rect: Rect, style: &OverviewStyle) -> Rect {
        let band = rect.shrink2(Vec2::new(0.0, style.overhang));
        let row = band.height() / self.runs.tracks.max(1) as f32;
        let stretch = if self.runs.width > 0.0 { rect.width() / self.runs.width } else { 0.0 };
        let top = band.min.y + run.track as f32 * row;
        Rect::from_min_max(
            pos2(rect.min.x + run.span.min * stretch, top + RUN_INSET),
            pos2(rect.min.x + run.span.max * stretch, top + row - RUN_INSET),
        )
    }

    /// The window box of `view`, in `rect`.
    fn window(&self, view: &TimeView, rect: Rect) -> Rect {
        let total = overview_total(self.end).0.max(1) as f64;
        let first = (view.start.0 as f64 / total) as f32;
        let last = first + self.shown_fraction(view) as f32;
        Rect::from_min_max(pos2(rect.min.x + first * rect.width(), rect.min.y), pos2(rect.min.x + last.min(1.0) * rect.width(), rect.max.y))
    }
}

impl Component for TimelineOverview<'_> {
    const ROLE: Role = Role::IMAGE;
    const OVERLAYS: bool = false;
    const SINGLE_VALUE: bool = true;
    type Style = OverviewStyle;
    type Output = TimelineOverviewOutput;

    fn style(cx: &StyleCx<'_>, _: TimelineOverviewStyleKey) -> OverviewStyle {
        let tokens = cx.tokens;
        OverviewStyle {
            metrics: Metrics::new(),
            window: cx.color(tokens.color.overlay.line),
            playhead: cx.color(tokens.color.accent.solid),
            radius: cx.field().radius,
            overhang: tokens.space.gap.tight,
            height: cx.control().height * HEIGHT_IN_CONTROLS,
        }
    }

    fn sense(&self) -> Sense {
        Sense::DRAG
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(0.0, cx.style.height)
    }

    fn bind(&mut self) -> Option<BindMut<'_, TimeView>> {
        Some(self.view.reborrow())
    }

    /// A press, a drag and the release centre the window at the pointer: one gesture from press to release.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(view) = cx.value().copied() else { return };
        let phase = cx.press().phase;
        let (at, last): (Option<Pos2>, bool) = match phase {
            PressPhase::Down { start, .. } if start.button == PointerButton::Primary => (Some(start.origin), false),
            PressPhase::DragStart { pos, start, .. } | PressPhase::Drag { pos, start, .. } if start.button == PointerButton::Primary => {
                (Some(pos), false)
            }
            PressPhase::Release { pos, start, .. } if start.button == PointerButton::Primary => (Some(pos), true),
            _ => (None, false),
        };
        let Some(at) = at else { return };
        if matches!(phase, PressPhase::Down { .. }) {
            cx.begin();
        }
        cx.propose(self.centred(&view, cx.rect, at.x));
        if last {
            cx.commit();
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(view) = cx.value().copied() else { return };
        let (rect, style) = (cx.rect, cx.style);
        let draw = ui.draw();
        for run in &self.runs.runs {
            draw.data_fill(&PaintShape::Rect(self.run_rect(run, rect, style), CornerRadius::ZERO), DataColor::new(run.color));
        }
        draw.stroke(&PaintShape::Rect(self.window(&view, rect), style.radius), StrokeWidth::Medium, style.window);
        if let Some(playhead) = self.playhead {
            let total = overview_total(self.end).0.max(1) as f64;
            let x = rect.min.x + (playhead.0 as f64 / total) as f32 * rect.width();
            draw.stroke(&PaintShape::Path(vec![pos2(x, rect.min.y), pos2(x, rect.max.y)]), StrokeWidth::Thin, style.playhead);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> TimelineOverviewOutput {
        TimelineOverviewOutput { view: cx.edit_response() }
    }
}

/// What the overview reports: the view its window was moved to.
#[derive(Debug)]
#[non_exhaustive]
pub struct TimelineOverviewOutput {
    /// The view: a click or drag centres the window, one gesture from press to release.
    pub view: EditResponse<TimeView>,
}

impl Outcome for TimelineOverviewOutput {
    type Value = TimeView;

    fn response(&self) -> &Response {
        &self.view.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.view.story_facts()
    }
}
