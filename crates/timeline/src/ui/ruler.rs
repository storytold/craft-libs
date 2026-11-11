//! `TimelineRuler`: the time ruler across the top of a timeline (N1.01), a leaf that reads the `TimeAxis` of the
//! `TimelineView` around it.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::{Axis, CursorIcon, HitLevel, HitPriority, Metrics, NavMove, PartKey, PointerButton, PressPhase, RulerUnits, Sense, Ticks};
use craft_ui::canvas::{Rulers, StepLadder, TickRule};
use craft_ui::contract::Remember as _;
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Name, Numeric, Outcome, PaintUi, PartsCx,
    Pointer as _, Props, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth, TextRun, Tick as TickMark, TickEdge, TickRank};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{ActivateKeys, KeyUse};
use craft_ui::style::{ComponentStyle, FocusLook, Stateful, StyleColor, StyleCx, TextLook, TypeRole, UiTokens};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};
use std::ops::Deref;

use super::DragEvent;
use super::view::{TimeAxis, time_axis};
use crate::clips::SnapTargets;
use crate::time::{Tick, TimeRange};
use crate::timecode::format_time;

/// Frames one arrow key steps, without and with Shift.
pub(crate) const STEP_FRAMES: i64 = 1;
pub(crate) const SHIFT_STEP_FRAMES: i64 = 5;

/// The scrub strip: the whole ruler.
pub const SCRUB: PartKey = PartKey::new("scrub");
/// The work-area bar's middle, which moves the work area.
pub const WORK_BAR: PartKey = PartKey::new("workBar");
/// The work area's start end.
pub const WORK_START: PartKey = PartKey::new("workStart");
/// The work area's end end.
pub const WORK_END: PartKey = PartKey::new("workEnd");
/// A stretch of the render bar: `RENDER.at(i)` for the `i`-th span.
pub const RENDER: PartKey = PartKey::new("render");
/// A split-edit mark: `SPLIT.at(i)` counted video in, video out, audio in, audio out.
const SPLIT: PartKey = PartKey::new("split");

/// The type role of the tick labels.
const LABEL_ROLE: TypeRole = TypeRole::Caption;

/// The arm of a split-edit bracket as a share of its height, pointing into the span.
const SPLIT_ARM_SHARE: f32 = 0.6;
/// Where a split-edit tag sits from its bracket, in arms.
const SPLIT_TAG_ARMS: f32 = 2.0;
/// Labelled ticks are at least this many label heights apart.
const LABEL_GAP_LINES: f32 = 7.0;
/// Ticks are at least this many label heights apart.
const TICK_GAP_LINES: f32 = 0.75;

craft_ui::words! {
    /// The ruler's parts, English by default.
    pub struct TimelineRulerWords {
        work_area: "timelineRuler.workArea" = "Work Area";
        work_area_start: "timelineRuler.workAreaStart" = "Work Area Start";
        work_area_end: "timelineRuler.workAreaEnd" = "Work Area End";
        video_tag: "timelineRuler.videoTag" = "V";
        audio_tag: "timelineRuler.audioTag" = "A";
        split_video_in: "timelineRuler.splitVideoIn" = "Split V In";
        split_video_out: "timelineRuler.splitVideoOut" = "Split V Out";
        split_audio_in: "timelineRuler.splitAudioIn" = "Split A In";
        split_audio_out: "timelineRuler.splitAudioOut" = "Split A Out";
    }
}

/// One ruler tick.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TimeTick {
    /// The tick's time.
    pub at: Tick,
    /// The label of a major tick ("00:00:10:00"); minor ticks have none.
    pub label: Option<String>,
    /// The tick's rank: a major tick is the longest and may carry a label.
    pub rank: TickRank,
}

/// The step ladder of a ruler whose labels are `line` points high, over `base` frames to the timecode second.
pub fn ruler_ladder(base: i64, line: f32) -> StepLadder {
    StepLadder::new(base, line * LABEL_GAP_LINES, line * TICK_GAP_LINES)
}

/// The ticks over a view `width` points wide: one every ladder stride from the first one at or before the left edge,
/// labelled every label stride in the axis's format. The strides come from the one tick rule with the time ladder.
pub fn ruler_ticks(axis: &TimeAxis, width: f32, ladder: StepLadder) -> Vec<TimeTick> {
    let (rate, pps) = (axis.format.rate, axis.view.points_per_second);
    if !(pps > 0.0 && pps.is_finite() && width.is_finite() && width > 0.0) {
        return Vec::new();
    }
    let first = rate.frame_at(axis.view.start).max(0);
    let last = rate.frame_at(axis.view.time_at(width, 0.0)).saturating_add(1);
    let frames = RulerUnits::new(1.0, Ticks::decimal(false));
    Rulers::ticks(&frames, TickRule::Steps(ladder), pps / rate.as_f64(), first as f64..=last as f64)
        .into_iter()
        .filter(|tick| tick.value >= 0.0)
        .map(|tick| {
            let at = rate.tick_of(tick.value as i64);
            TimeTick { at, label: matches!(tick.rank, TickRank::Major).then(|| format_time(at, &axis.format)), rank: tick.rank }
        })
        .collect()
}

/// What a stretch of the render bar says about its frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderBarState {
    /// A rendered preview (Premiere's green; After Effects' RAM cache).
    Rendered,
    /// Unrendered, but it should play in real time (yellow).
    RealTime,
    /// Unrendered, and it needs rendering to play in real time (red).
    NeedsRender,
    /// Frames on disk (After Effects' blue disk cache).
    OnDisk,
}

/// One stretch of the render bar along the ruler's bottom edge.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RenderSpan {
    /// The stretch of time.
    pub range: TimeRange,
    /// What the stretch says about its frames.
    pub state: RenderBarState,
    /// What hovering the stretch says ("Rendered preview"); no tooltip when empty.
    pub tooltip: String,
}

impl RenderSpan {
    /// A span over `range` with `tooltip` shown on hover.
    pub fn new(range: TimeRange, state: RenderBarState, tooltip: impl Into<String>) -> Self {
        Self { range, state, tooltip: tooltip.into() }
    }
}

/// The split-edit points of a sequence (Premiere's L and J cuts): where the video and the audio of a clip start and
/// end apart, drawn as brackets tagged V and A.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SplitMarks {
    /// Where the video starts, if it starts apart from the audio.
    pub video_in: Option<Tick>,
    /// Where the video ends, if it ends apart from the audio.
    pub video_out: Option<Tick>,
    /// Where the audio starts, if it starts apart from the video.
    pub audio_in: Option<Tick>,
    /// Where the audio ends, if it ends apart from the video.
    pub audio_out: Option<Tick>,
}

/// One split-edit bracket: its time, whether it starts a span, and whether it is the audio's.
#[derive(Clone, Copy)]
struct Bracket {
    time: Tick,
    is_in: bool,
    audio: bool,
}

impl SplitMarks {
    /// The marks in part order: video in, video out, audio in, audio out.
    fn listed(&self) -> [Option<Bracket>; 4] {
        let mark = |time: Option<Tick>, is_in: bool, audio: bool| time.map(|time| Bracket { time, is_in, audio });
        [mark(self.video_in, true, false), mark(self.video_out, false, false), mark(self.audio_in, true, true), mark(self.audio_out, false, true)]
    }
}

/// Which part of the work-area bar a press took.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum WorkGrab {
    #[default]
    Move,
    Start,
    End,
}

/// A work-area drag under way: what was grabbed, the area then and where the pointer went down.
#[derive(Clone, Copy, Debug, Default)]
struct WorkHold {
    grab: WorkGrab,
    anchor: TimeRange,
    press_x: f32,
}

/// The work area after dragging `grab` of it by `moved` from `anchor`; no end gets closer to the other than `frame`,
/// and `settle` snaps a moving time.
fn dragged_range(grab: WorkGrab, anchor: TimeRange, moved: Tick, frame: Tick, settle: impl Fn(Tick) -> Tick) -> TimeRange {
    match grab {
        WorkGrab::Move => TimeRange::new(settle((anchor.start + moved).max(Tick::ZERO)), anchor.duration),
        WorkGrab::Start => {
            let start = settle((anchor.start + moved).clamp(Tick::ZERO, (anchor.end() - frame).max(Tick::ZERO)));
            TimeRange::from_bounds(start, anchor.end())
        }
        WorkGrab::End => TimeRange::from_bounds(anchor.start, settle((anchor.end() + moved).max(anchor.start + frame))),
    }
}

/// The style of the ruler.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TimelineRulerStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    fill: StyleColor,
    shade: StyleColor,
    bar: StyleColor,
    cap: StyleColor,
    bracket: StyleColor,
    head: StyleColor,
    rendered: StyleColor,
    real_time: StyleColor,
    needs_render: StyleColor,
    on_disk: StyleColor,
    label: TextLook,
    /// The tick row and its labels.
    body: f32,
    /// The work-area band above the body.
    band: f32,
    /// The render bar under the body.
    render: f32,
    /// The playhead head's side.
    head_side: f32,
    /// The width of a work-area cap.
    cap_width: f32,
    /// The width of an end's grab zone.
    grab: f32,
    /// Between a tick and its label.
    pad: f32,
    /// How far from a snap target a scrub still snaps to it.
    reach: f32,
    /// The label row's height.
    line: f32,
    /// The height of a split-edit bracket.
    bracket_height: f32,
}

impl TimelineRulerStyle {
    fn height(&self) -> f32 {
        self.band + self.body + self.render
    }
}

/// The tick body: as thick as its labels need, and no thinner than a small icon.
fn body_height(tokens: &UiTokens) -> f32 {
    tokens.size.icon.sm.max(tokens.typography.get(LABEL_ROLE).line_height)
}

/// The rows of the ruler inside `rect`: the work-area band, the tick body and the render bar.
fn rows(rect: Rect, style: &TimelineRulerStyle) -> (Rect, Rect, Rect) {
    let band = Rect::from_min_size(rect.min, Vec2::new(rect.width(), style.band));
    let body = Rect::from_min_size(band.left_bottom(), Vec2::new(rect.width(), style.body));
    let render = Rect::from_min_size(body.left_bottom(), Vec2::new(rect.width(), style.render));
    (band, body, render)
}

/// The ruler across the top of a timeline: ticks and labels, the playhead's head, In and Out shading (inside the
/// span), the work-area bar, the render bar and split-edit marks.
///
/// - Press and drag scrubs the playhead (snapped to [`snap`](Self::snap) targets when given; Shift snaps when no
///   targets are given, as After Effects), reported as the playhead's edit; Esc during a scrub cancels it back to
///   where it began.
/// - The work-area bar drags by its middle (moves) or an end (moves that end), Shift snaps, reported as a
///   [`DragEvent<TimeRange>`] in [`TimelineRulerResponse::work_area`]; double-clicking it sets it to the visible span.
///   The three zones are pointer-only: the Work Area In and Out point commands set it by keyboard.
/// - Keyboard: one Tab stop. Left and Right step the playhead one frame (Shift: five), Home and End go to the start
///   and the end; each a `Set`.
/// - A secondary click reports the time under it in [`TimelineRulerResponse::context`].
/// - AccessKit: a slider named by the label, value the playhead's text, numeric value the playhead in seconds over
///   `0..=end`.
#[derive(Component, Props)]
#[component(content_painter)]
pub struct TimelineRuler<'a> {
    #[common]
    common: Common,
    playhead: Bind<'a, Tick>,
    /// The Mark In and Mark Out times: the span between them is shaded.
    #[prop]
    in_out: (Option<Tick>, Option<Tick>),
    /// The work area, drawn as a bar.
    #[prop]
    work_area: Option<TimeRange>,
    /// The render bar along the bottom edge: Premiere's per-stretch green, yellow and red with a tooltip each; After
    /// Effects' RAM and disk caches are `Rendered` and `OnDisk` stretches. Later stretches paint over earlier ones.
    #[prop]
    render_bar: &'a [RenderSpan],
    /// The split-edit brackets.
    #[prop]
    split_marks: SplitMarks,
    /// Scrubbing snaps to these targets (Premiere with Snap on).
    #[prop]
    snap: Option<&'a SnapTargets>,
    work: Option<DragEvent<TimeRange>>,
    snapped: Option<Tick>,
    context: Option<Tick>,
}

impl<'a> TimelineRuler<'a> {
    /// A ruler named `label` that edits `playhead`. The time axis comes from the `TimelineView` around it.
    pub fn new(label: impl Into<Label>, playhead: Bind<'a, Tick>) -> Self {
        TimelineRuler {
            common: Common::new(),
            playhead,
            in_out: (None, None),
            work_area: None,
            render_bar: &[],
            split_marks: SplitMarks::default(),
            snap: None,
            work: None,
            snapped: None,
            context: None,
        }
        .label(label)
        .ltr_axis()
    }

    /// The times a drag snaps to: the app's targets, else with Shift what the ruler itself shows.
    fn targets(&self, shift: bool, playhead: Tick, skip_work_area: bool) -> Option<Vec<Tick>> {
        if let Some(targets) = self.snap {
            return Some(targets.times().to_vec());
        }
        shift.then(|| {
            let mut times: Vec<Tick> = [self.in_out.0, self.in_out.1].into_iter().flatten().collect();
            if !skip_work_area {
                times.extend(self.work_area.into_iter().flat_map(|area| [area.start, area.end()]));
            }
            times.push(playhead);
            times
        })
    }
}

/// The nearest of `targets` within `tolerance` of `t`.
fn nearest(targets: &[Tick], t: Tick, tolerance: Tick) -> Option<Tick> {
    targets.iter().copied().filter(|c| (*c - t).abs() <= tolerance).min_by_key(|c| (*c - t).abs())
}

/// The time of the horizontal position `x` in a ruler whose left edge is `left`, not before the start.
fn time_at(axis: &TimeAxis, x: f32, left: f32) -> Tick {
    axis.tick_at(x, left).max(Tick::ZERO)
}

/// The centre of a bracket's tag and the top of its stem.
fn bracket_points(axis: &TimeAxis, rect: Rect, style: &TimelineRulerStyle, bracket: Bracket) -> (Pos2, Pos2) {
    let at = if bracket.is_in { bracket.time } else { bracket.time + axis.frame() };
    let x = axis.x_of(at, rect.left());
    let (_, body, _) = rows(rect, style);
    let top = if bracket.audio { body.bottom() - style.bracket_height } else { body.top() + style.line };
    let arm = style.bracket_height * SPLIT_ARM_SHARE * if bracket.is_in { 1.0 } else { -1.0 };
    (Pos2::new(x + SPLIT_TAG_ARMS * arm, top + style.bracket_height / 2.0), Pos2::new(x, top))
}

impl Component for TimelineRuler<'_> {
    const ROLE: Role = Role::SLIDER;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_activate(ActivateKeys::Neither).with_moves();
    type Style = TimelineRulerStyle;
    type Output = TimelineRulerResponse;

    fn style(cx: &StyleCx<'_>, _: TimelineRulerStyleKey) -> TimelineRulerStyle {
        let c = &cx.tokens.color;
        let handle = cx.tokens.size.handle.sm;
        TimelineRulerStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            fill: cx.color(c.surface.chrome),
            shade: cx.color(c.state.selected),
            bar: cx.color(c.text.muted),
            cap: cx.color(c.accent.solid),
            bracket: cx.color(c.text.muted),
            head: cx.color(c.accent.solid),
            rendered: cx.color(c.data.green),
            real_time: cx.color(c.data.yellow),
            needs_render: cx.color(c.data.red),
            on_disk: cx.color(c.data.blue),
            label: TextLook::new(LABEL_ROLE, Stateful::new(cx.color(c.text.muted))),
            body: body_height(cx.tokens),
            band: handle,
            render: cx.tokens.size.groove,
            head_side: handle,
            cap_width: cx.tokens.stroke.thick,
            grab: cx.tokens.size.hit.target,
            pad: cx.tokens.space.gap.tight,
            reach: cx.tokens.size.hit.slop,
            line: cx.tokens.typography.get(LABEL_ROLE).line_height,
            bracket_height: handle,
        }
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(0.0, cx.style.height())
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS
    }

    fn bind(&mut self) -> Option<BindMut<'_, Tick>> {
        Some(self.playhead.reborrow())
    }

    /// The scrub strip under the work-area, render and split parts, which sit above it at handle priority.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(axis) = time_axis(&cx.env) else { return };
        let (band, _, strip) = rows(rect, cx.style);
        let handle = HitPriority::new(HitLevel::Handle, 0);
        let (left, grab) = (rect.left(), cx.style.grab);
        cx.part(SCRUB, rect)
            .sense(Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS | Sense::CONTEXT_CLICK)
            .value_part()
            .pointer_only("the arrow keys step the playhead");
        if let Some(area) = self.work_area.filter(|area| !area.is_empty()) {
            let (x0, x1) = (axis.x_of(area.start, left), axis.x_of(area.end(), left));
            let bar = Rect::from_min_max(Pos2::new(x0, band.top()), Pos2::new(x1, band.bottom())).intersect(rect);
            let reason = "the Work Area In and Out point commands set it";
            if bar.width() > 0.0 {
                cx.part(WORK_BAR, bar)
                    .sense(Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS)
                    .priority(handle)
                    .cursor(CursorIcon::Grab)
                    .pointer_only(reason);
            }
            for (key, x) in [(WORK_START, x0), (WORK_END, x1)] {
                let zone = Rect::from_min_max(Pos2::new(x - grab / 2.0, band.top()), Pos2::new(x + grab / 2.0, band.bottom())).intersect(rect);
                if zone.width() > 0.0 {
                    cx.part(key, zone)
                        .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                        .priority(HitPriority::new(HitLevel::Handle, 1))
                        .cursor(CursorIcon::ResizeHorizontal)
                        .pointer_only(reason);
                }
            }
        }
        for (at, span) in self.render_bar.iter().enumerate().filter(|(_, span)| !span.range.is_empty() && !span.tooltip.is_empty()) {
            let bar = Rect::from_min_max(
                Pos2::new(axis.x_of(span.range.start, left), strip.top()),
                Pos2::new(axis.x_of(span.range.end(), left), strip.bottom()),
            )
            .intersect(rect);
            if bar.width() > 0.0 {
                cx.part(RENDER.at(at as u64), bar).sense(Sense::HOVER).priority(handle).tooltip(Label::from(span.tooltip.clone()));
            }
        }
        for (at, bracket) in self.split_marks.listed().into_iter().enumerate() {
            let Some(bracket) = bracket else { continue };
            let (centre, _) = bracket_points(&axis, rect, cx.style, bracket);
            let spot = Rect::from_center_size(
                centre,
                Vec2::new(cx.style.bracket_height * SPLIT_ARM_SHARE * (SPLIT_TAG_ARMS + 1.0), cx.style.bracket_height),
            )
            .intersect(rect);
            cx.part(SPLIT.at(at as u64), spot).sense(Sense::HOVER).priority(handle);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(axis) = time_axis(&cx.env) else { return };
        let rect = cx.rect;
        let left = rect.left();
        let playhead = cx.value().copied().unwrap_or(Tick::ZERO);
        let rate = axis.format.rate;
        let tolerance = Tick::from_seconds_f64(f64::from(cx.style.reach) / axis.view.points_per_second.max(f64::EPSILON));

        // The playhead: a press or drag scrubs, a release commits.
        let press = cx.part(SCRUB).press;
        if let Some(start) = press.phase.start().filter(|start| start.button == PointerButton::Primary) {
            let at = match press.phase {
                PressPhase::Down { .. } => Some(start.origin.x),
                PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } | PressPhase::Release { pos, .. } => Some(pos.x),
                _ => None,
            };
            if let Some(x) = at {
                let t = time_at(&axis, x, left);
                let hit = self.targets(press.mods.shift, playhead, false).and_then(|targets| nearest(&targets, t, tolerance));
                self.snapped = hit;
                cx.propose(hit.unwrap_or_else(|| rate.snap_nearest(t)));
            }
            if matches!(press.phase, PressPhase::Release { .. }) {
                cx.commit();
            }
        }
        if let PressPhase::Release { start, pos, .. } = press.phase
            && start.button == PointerButton::Secondary
        {
            self.context = Some(time_at(&axis, pos.x, left));
        }

        // The work area: the bar moves, an end resizes.
        if let Some(area) = self.work_area {
            for (grab, key) in [(WorkGrab::Move, WORK_BAR), (WorkGrab::Start, WORK_START), (WorkGrab::End, WORK_END)] {
                if cx.part_if_declared(key).is_none() {
                    continue;
                }
                let press = cx.part(key).press;
                let Some(start) = press.phase.start().filter(|start| start.button == PointerButton::Primary) else { continue };
                // The second press of a double click sets the visible span; its drag does nothing.
                if grab == WorkGrab::Move && press.double_pressed() {
                    self.work = Some(DragEvent::Set(TimeRange::from_bounds(axis.view.start, axis.tick_at(rect.right(), left))));
                    continue;
                }
                if start.clicks >= 2 {
                    continue;
                }
                let held = *cx.press_slot::<WorkHold>().get_or_insert(WorkHold { grab, anchor: area, press_x: start.origin.x });
                let targets = self.targets(press.mods.shift, playhead, true);
                let next = |x: f32| {
                    let moved = Tick::from_seconds_f64(f64::from(x - held.press_x) / axis.view.points_per_second);
                    let settle = |t: Tick| nearest(targets.as_deref().unwrap_or(&[]), t, tolerance).unwrap_or_else(|| rate.snap_nearest(t));
                    dragged_range(held.grab, held.anchor, moved, rate.frame_duration(), settle)
                };
                let event = match press.phase {
                    PressPhase::DragStart { pos, .. } => Some(DragEvent::Begin(next(pos.x))),
                    PressPhase::Drag { pos, .. } => Some(DragEvent::Update(next(pos.x))),
                    PressPhase::Release { pos, click: false, .. } => Some(DragEvent::Commit(next(pos.x))),
                    PressPhase::Cancel { .. } => Some(DragEvent::Cancel),
                    _ => None,
                };
                if event.is_some() {
                    self.work = event;
                }
            }
        }

        // The keys: a frame (Shift: five) either way, or the start or the end.
        if cx.gesture().is_none() {
            let rtl = cx.env.direction().is_rtl();
            while let Some((mv, mods)) = cx.intents().take_move() {
                let frames = if mods.extend { SHIFT_STEP_FRAMES } else { STEP_FRAMES };
                let from = rate.frame_at(cx.value().copied().unwrap_or(Tick::ZERO));
                let to = match mv {
                    NavMove::Back | NavMove::Forward => {
                        let later = (mv == NavMove::Forward) != rtl;
                        Some(rate.tick_of(if later { from.saturating_add(frames) } else { from.saturating_sub(frames) }))
                    }
                    NavMove::Start => Some(Tick::ZERO),
                    NavMove::End => Some(axis.end),
                    _ => None,
                };
                if let Some(to) = to {
                    cx.set(to.max(Tick::ZERO));
                }
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let rect = cx.rect;
        let (band, body, strip) = rows(rect, style);
        let draw = ui.draw();
        let shape = |rect: Rect| PaintShape::Rect(rect, CornerRadius::ZERO);
        draw.fill(&shape(rect), style.fill);
        let Some(axis) = time_axis(&cx.env) else { return };
        if !(axis.view.points_per_second > 0.0 && axis.view.points_per_second.is_finite() && rect.width() > 0.0) {
            return;
        }
        let left = rect.left();
        let rate = axis.format.rate;
        let shown = cx.value().copied().unwrap_or(Tick::ZERO);

        if self.in_out.0.is_some() || self.in_out.1.is_some() {
            let from = self.in_out.0.unwrap_or(Tick::ZERO);
            let to = self.in_out.1.map_or(axis.end, |out| out + rate.frame_duration());
            let span =
                Rect::from_min_max(Pos2::new(axis.x_of(from, left), body.top()), Pos2::new(axis.x_of(to, left), body.bottom())).intersect(rect);
            draw.fill(&shape(span), style.shade);
        }
        for span in self.render_bar.iter().filter(|span| !span.range.is_empty()) {
            let fill = match span.state {
                RenderBarState::Rendered => style.rendered,
                RenderBarState::RealTime => style.real_time,
                RenderBarState::NeedsRender => style.needs_render,
                RenderBarState::OnDisk => style.on_disk,
            };
            let bar = Rect::from_min_max(
                Pos2::new(axis.x_of(span.range.start, left), strip.top()),
                Pos2::new(axis.x_of(span.range.end(), left), strip.bottom()),
            )
            .intersect(rect);
            draw.fill(&shape(bar), fill);
        }
        let area = match self.work {
            Some(DragEvent::Begin(range) | DragEvent::Update(range) | DragEvent::Commit(range) | DragEvent::Set(range)) => Some(range),
            _ => self.work_area,
        };
        if let Some(area) = area.filter(|area| !area.is_empty()) {
            let bar = Rect::from_min_max(Pos2::new(axis.x_of(area.start, left), band.top()), Pos2::new(axis.x_of(area.end(), left), band.bottom()));
            draw.fill(&shape(bar.intersect(rect)), style.bar);
            for x in [bar.left(), bar.right()] {
                let cap = Rect::from_center_size(Pos2::new(x, bar.center().y), Vec2::new(style.cap_width, bar.height())).intersect(rect);
                draw.fill(&shape(cap), style.cap);
            }
        }

        let ticks = ruler_ticks(&axis, rect.width(), ruler_ladder(rate.timecode_base(), style.line));
        let marks: Vec<TickMark> = ticks.iter().map(|tick| TickMark::new(axis.x_of(tick.at, left) - left, tick.rank)).collect();
        ui.content_painter(cx).ticks(body, TickEdge::Bottom, &marks);
        let color = cx.resolve(&style.label.color);
        for (tick, text) in ticks.iter().filter_map(|tick| tick.label.as_ref().map(|text| (tick, text))) {
            let layout = draw.layout(&TextRun::new(text.as_str()).role(style.label.role).color(color), f32::INFINITY);
            let at = Pos2::new(axis.x_of(tick.at, left) + style.pad, body.top());
            let text_rect = Rect::from_min_size(at, layout.size());
            if body.intersects(text_rect) && rect.contains_rect(text_rect) {
                draw.galley(at, &layout);
            }
        }

        let words = cx.words::<TimelineRulerWords>();
        for bracket in self.split_marks.listed().into_iter().flatten() {
            let (centre, top) = bracket_points(&axis, rect, style, bracket);
            let arm = style.bracket_height * SPLIT_ARM_SHARE * if bracket.is_in { 1.0 } else { -1.0 };
            draw.stroke(&PaintShape::Path(vec![top, Pos2::new(top.x, top.y + style.bracket_height)]), StrokeWidth::Medium, style.bracket);
            draw.stroke(&PaintShape::Path(vec![top, Pos2::new(top.x + arm, top.y)]), StrokeWidth::Medium, style.bracket);
            let tag = if bracket.audio { words.audio_tag.as_str() } else { words.video_tag.as_str() };
            let layout = draw.layout(&TextRun::new(tag).role(style.label.role).color(color), f32::INFINITY);
            draw.galley(centre - layout.size() / 2.0, &layout);
        }

        let x = axis.x_of(shown, left);
        draw.stroke(&PaintShape::Path(vec![Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())]), StrokeWidth::Thin, style.head);
        let half = style.head_side / 2.0;
        draw.fill(
            &PaintShape::Polygon(vec![Pos2::new(x - half, rect.top()), Pos2::new(x + half, rect.top()), Pos2::new(x, rect.top() + style.head_side)]),
            style.head,
        );
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(axis) = time_axis(&cx.env) else {
            // No time axis around the ruler: it reads nothing, but its slider still has a range.
            access.numeric(Numeric::empty(0.0..=0.0));
            return;
        };
        let shown = cx.value().copied().unwrap_or(Tick::ZERO);
        access.numeric(Numeric::new(shown.seconds(), 0.0..=axis.end.max(Tick::ZERO).seconds()).text(format_time(shown, &axis.format)));
        let words = cx.words::<TimelineRulerWords>();
        if let Some(area) = self.work_area.filter(|area| !area.is_empty()) {
            let at = |t: Tick| format_time(t, &axis.format);
            let end = axis.end.max(Tick::ZERO).seconds();
            let (start, stop) = (area.start.seconds(), area.end().seconds());
            // The bar slides as a whole, so its position runs up to the latest start that keeps it inside the sequence.
            let latest = (end - (stop - start)).max(start);
            let parts = [
                (WORK_BAR, &words.work_area, start, latest, format!("{} \u{2013} {}", at(area.start), at(area.end()))),
                (WORK_START, &words.work_area_start, start, end.max(start), at(area.start)),
                (WORK_END, &words.work_area_end, stop, end.max(stop), at(area.end())),
            ];
            for (part, name, value, highest, text) in parts {
                let node = access.node(part, Role::SLIDER);
                node.name = Name::Text(name.as_str().to_owned());
                node.numeric = Some(Numeric::new(value, 0.0..=highest).text(text));
            }
        }
        let names = [&words.split_video_in, &words.split_video_out, &words.split_audio_in, &words.split_audio_out];
        for (at, name) in names.into_iter().enumerate() {
            if self.split_marks.listed()[at].is_some() {
                access.node(SPLIT.at(at as u64), Role::LABEL).name = Name::Text(name.as_str().to_owned());
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> TimelineRulerResponse {
        TimelineRulerResponse { playhead: cx.edit_response(), work_area: self.work, snapped_at: self.snapped, context: self.context }
    }
}

/// What the ruler did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct TimelineRulerResponse {
    /// The playhead: a scrub's gesture or a key's `Set`.
    pub playhead: EditResponse<Tick>,
    /// The work-area drag, one event per pass: its gesture, or the visible span a double click sets.
    pub work_area: Option<DragEvent<TimeRange>>,
    /// Where scrubbing snapped this pass (for a snap line across the tracks).
    pub snapped_at: Option<Tick>,
    /// The time under a secondary click, for a context menu.
    pub context: Option<Tick>,
}

impl Deref for TimelineRulerResponse {
    type Target = EditResponse<Tick>;

    fn deref(&self) -> &Self::Target {
        &self.playhead
    }
}

impl Outcome for TimelineRulerResponse {
    type Value = Tick;

    fn response(&self) -> &Response {
        &self.playhead.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.playhead.story_facts()
    }
}
