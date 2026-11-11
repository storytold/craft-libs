//! `TimelineView`: the one owner of the time axis. It binds the app's [`TimeView`] once, publishes the shown view to its
//! body as the [`TimeAxis`] app value (the ruler, the lanes, the marker strip and the graph read it, so every part maps
//! time the same way), takes the wheel's sideways and zoom share, the pinch, the time area's keys and the follow of the
//! playhead, and shows the horizontal scroll bar as a scroll part over the view (a thumb end drag zooms).

use super::ruler::{SHIFT_STEP_FRAMES, STEP_FRAMES};
use super::stack::TrackSizes;
use crate::clock::{Follow, FollowMode};
use crate::time::{Tick, TimeRange, TimeView, VIEW_EXTENT_PAST_END};
use crate::timecode::TimeFormat;
use craft_behavior::scroll::{self, ScrollSpan};
use craft_behavior::{Axis, ColumnWidth, EditEvent, Granularity, Intent, Metrics, Mixed, NavMove, PartKey, Sense, Wheel};
use craft_ui::contract::{
    Access, Bind, BindMut, CommandSet, Commands as _, Common, Component, Container, ContainerProps, Cx, Edit as _, EditResponse, Focus as _, Name,
    Outcome, PaintUi, PartsCx, Pointer as _, Props, Remember as _, Role, Sizing, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::access::Numeric;
use craft_ui::runtime::commands::CommandState;
use craft_ui::runtime::env::{AppValues, EnvValue, EnvView, Label};
use craft_ui::runtime::focus::KeyUse;
use craft_ui::runtime::motion::{Animate as _, spec};
use craft_ui::style::{ComponentStyle, RuleKind, ScrollBarLook, StyleColor, StyleCx, SurfaceKind};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};
use std::cell::Cell;
use std::ops::Deref;
use std::sync::Arc;

craft_ui::words! {
    /// The panel's names and tooltips, English by default (FilmCraft's).
    #[non_exhaustive]
    pub struct TimelineWords {
        panel: "timeline.panel" = "Timeline";
        snap: "timeline.snap" = "Snap in Timeline (S)";
        linked_selection: "timeline.linkedSelection" = "Linked Selection";
        add_marker: "timeline.addMarker" = "Add Marker (M)";
        settings: "timeline.settings" = "Timeline Display Settings";
        track_color: "timeline.trackColor" = "Track Color";
        lock: "timeline.lock" = "Toggle Track Lock";
        target: "timeline.target" = "Toggle track targeting";
        sync_lock: "timeline.syncLock" = "Toggle Sync Lock";
        output: "timeline.output" = "Toggle Track Output";
        mute: "timeline.mute" = "Mute Track";
        solo: "timeline.solo" = "Solo Track";
        no_label: "timeline.noLabel" = "None";
        empty: "timeline.empty" = "Drop media here to create sequence.";
        overview: "timeline.overview" = "Overview";
        mix: "timeline.mix" = "Mix";
        scroll_time: "timeline.scrollTime" = "Time scroll bar";
        move_playhead: "timeline.movePlayhead" = "Move Playhead";
        track_height: template "timeline.trackHeight" = "Drag to change the height of {track}";
        track_height_top: template "timeline.trackHeightTop" = "Drag the top edge to change the height of {track}";
        track_expand: template "timeline.trackExpand" = "Toggle the height of {track}";
        header_width: "timeline.headerWidth" = "Drag to change the width of the track headers";
        tracks: "timeline.tracks" = "Video and audio tracks";
        divider: "timeline.divider" = "Drag to change how the room is shared between video and audio tracks";
        label_violet: "timeline.labelViolet" = "Violet";
        label_iris: "timeline.labelIris" = "Iris";
        label_caribbean: "timeline.labelCaribbean" = "Caribbean";
        label_lavender: "timeline.labelLavender" = "Lavender";
        label_cerulean: "timeline.labelCerulean" = "Cerulean";
        label_forest: "timeline.labelForest" = "Forest";
        label_rose: "timeline.labelRose" = "Rose";
        label_mango: "timeline.labelMango" = "Mango";
        label_purple: "timeline.labelPurple" = "Purple";
        label_blue: "timeline.labelBlue" = "Blue";
        label_teal: "timeline.labelTeal" = "Teal";
        label_magenta: "timeline.labelMagenta" = "Magenta";
        label_tan: "timeline.labelTan" = "Tan";
        label_green: "timeline.labelGreen" = "Green";
        label_brown: "timeline.labelBrown" = "Brown";
        label_yellow: "timeline.labelYellow" = "Yellow";
    }
}

impl TimelineWords {
    /// The name of the colour label `label` (Premiere's sixteen; a number past them wraps to the first).
    pub fn label_name(&self, label: crate::markers::MarkerColor) -> &str {
        let names = [
            &self.label_violet,
            &self.label_iris,
            &self.label_caribbean,
            &self.label_lavender,
            &self.label_cerulean,
            &self.label_forest,
            &self.label_rose,
            &self.label_mango,
            &self.label_purple,
            &self.label_blue,
            &self.label_teal,
            &self.label_magenta,
            &self.label_tan,
            &self.label_green,
            &self.label_brown,
            &self.label_yellow,
        ];
        names[usize::from(label.0) % names.len()].as_str()
    }
}

/// The time mapping every part of a timeline reads: the shown view, how times are written and where the content ends.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TimeAxis {
    /// The view being shown: eased while a zoom runs.
    pub view: TimeView,
    /// How times are shown and typed.
    pub format: TimeFormat,
    /// The end of the content.
    pub end: Tick,
}

impl TimeAxis {
    /// The screen x of `t` in a time area whose left edge is at `left`.
    pub fn x_of(&self, t: Tick, left: f32) -> f32 {
        self.view.x_of(t, left)
    }

    /// The time under screen x `x`, not snapped.
    pub fn tick_at(&self, x: f32, left: f32) -> Tick {
        self.view.time_at(x, left)
    }

    /// One frame of the format's rate.
    pub fn frame(&self) -> Tick {
        self.format.rate.tick_of(1)
    }
}

/// The env value that carries the axis of the innermost [`TimelineView`]; `None` outside one.
#[derive(Clone, Copy, Debug, PartialEq)]
struct InTimeline(Option<TimeAxis>);

impl EnvValue for InTimeline {
    fn root() -> Self {
        InTimeline(None)
    }
}

/// The axis of the `TimelineView` around the component, or `None` outside one.
pub fn time_axis(env: &EnvView<'_>) -> Option<TimeAxis> {
    env.get::<InTimeline>().0
}

/// The factor one wheel reading zooms by in Alt+wheel (FilmCraft `interact.rs`, Premiere): `1 + dy * 0.01` within half
/// to double.
pub(crate) fn wheel_zoom_factor(dy: f32) -> f64 {
    (1.0 + f64::from(dy) * WHEEL_ZOOM_PER_POINT).clamp(0.5, 2.0)
}

const WHEEL_ZOOM_PER_POINT: f64 = 0.01;
/// The factor one `=` or `-` press zooms by, about the playhead (FilmCraft `menus.rs:421`).
const KEY_ZOOM: f64 = 1.6;
/// How far a pinch's factor differs from 1 before it counts (FilmCraft `interact.rs:233`).
const PINCH_EPSILON: f64 = 1e-4;
/// A zoom within this share of its target has arrived.
const ZOOM_SETTLE: f64 = 0.0005;
/// The room the fit leaves at the sequence's end, as a share of the width (the legacy `fit_margin`).
const FIT_MARGIN_SHARE: f32 = 0.02;

/// The time area's zoom commands: the keyboard path of the wheel and the bar's zoom. A key reaches the timeline that
/// holds key focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "timelineView", scope = "timelineView")]
#[non_exhaustive]
pub enum TimeCommand {
    /// Zoom in about the playhead.
    #[command(label = "Zoom In", default = "=", default = "+")]
    ZoomIn,
    /// Zoom out about the playhead.
    #[command(label = "Zoom Out", default = "-")]
    ZoomOut,
    /// Fit the whole sequence.
    #[command(label = "Zoom to Sequence", default = "\\")]
    Fit,
}

const TIME_COMMANDS: [TimeCommand; 3] = [TimeCommand::ZoomIn, TimeCommand::ZoomOut, TimeCommand::Fit];

const BAR: PartKey = PartKey::new("bar");

/// What the view keeps between passes: the time held under one x while a zoom eases, and the follow's state.
#[derive(Clone, Copy, Debug, Default)]
struct Anim {
    anchor: Option<(Tick, f32)>,
    follow: Follow,
    seen: Option<Tick>,
}

/// The style of the view: its scroll bar.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TimelineViewStyle {
    #[style(metrics)]
    metrics: Metrics,
    look: ScrollBarLook,
    /// Between the bar's edge and its thumb.
    inset: f32,
    /// The bar's track.
    track: StyleColor,
    /// The sizes of the headers left of the time area.
    sizes: TrackSizes,
    /// The playhead's line over the panes.
    playhead: StyleColor,
    /// The line at the time a drag snapped to.
    snap: StyleColor,
    /// The fill and the edge of a marquee.
    marquee_fill: StyleColor,
    marquee_edge: StyleColor,
}

/// What the panel works out from its panes in a pass and the view draws over them in the same pass, after the body.
#[derive(Debug, Default)]
pub struct Overlay {
    /// The rect the panes fill and the left edge of their lanes.
    panes: Cell<Option<(Rect, f32)>>,
    /// The time a drag snapped to.
    snap: Cell<Option<Tick>>,
    /// The rectangle of a marquee under way.
    marquee: Cell<Option<Rect>>,
}

impl Overlay {
    /// The panes fill `rect`, their lanes beginning at `lane_left`.
    pub fn panes(&self, rect: Rect, lane_left: f32) {
        self.panes.set(Some((rect, lane_left)));
    }

    /// A drag snapped to `at`.
    pub fn snapped(&self, at: Option<Tick>) {
        self.snap.set(at);
    }

    /// A marquee under way covers `rect`.
    pub fn marquee(&self, rect: Option<Rect>) {
        self.marquee.set(rect);
    }
}

/// The container around every pane of a timeline. It binds the [`TimeView`] (a proposal of the view is never an undo
/// step), publishes the [`TimeAxis`] to its body and shows the horizontal bar over the time area. The time area is one
/// Tab stop and an F6 pane; its keys are Premiere's, each an exact chord: Left and Right step the playhead a frame
/// (Shift: five), Home and End go to the start and the end, Up and Down to the previous and next edit point, and the
/// [`TimeCommand`]s zoom and fit. The Menu key and Shift+F10 report [`TimelineViewOutput::context_menu`].
#[derive(Component, Props)]
#[component(container)]
pub struct TimelineView<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    view: Bind<'a, TimeView>,
    format: TimeFormat,
    end: Tick,
    /// The name column's width of the headers left of the time area; the headers take the default until the user drags
    /// it.
    #[prop(into)]
    header_width: Option<ColumnWidth>,
    /// Whether track headers sit left of the time area; the time area spans the whole width without them.
    #[prop]
    headers: bool,
    follow: FollowMode,
    playing: bool,
    playhead: Tick,
    /// The edit points Up and Down move the playhead to, in time order.
    #[prop]
    edit_points: &'a [Tick],
    playhead_edit: Option<EditEvent<Tick>>,
    context_menu: bool,
    anim: Anim,
    /// What the body reports for the view to draw over it, in the pass the body reports it: the playhead's line, the
    /// snap line and the marquee.
    #[prop(into)]
    overlay: Option<&'a Overlay>,
    /// Whether the horizontal bar shows; a [`strip`](Self::strip) view has none.
    bar: bool,
}

impl<'a> TimelineView<'a> {
    /// A time area named `label` over `view`, for a sequence ending at `end` written in `format`.
    pub fn new(label: impl Into<Label>, view: Bind<'a, TimeView>, format: TimeFormat, end: Tick) -> Self {
        let props = ContainerProps::new().surface(SurfaceKind::Sunken).sizing(Sizing::Fill).command_scope(TimeCommand::SCOPE);
        TimelineView {
            common: Common::new(),
            props,
            view,
            format,
            end,
            header_width: None,
            headers: false,
            follow: FollowMode::Page,
            playing: false,
            playhead: Tick::ZERO,
            edit_points: &[],
            playhead_edit: None,
            context_menu: false,
            anim: Anim::default(),
            overlay: None,
            bar: true,
        }
        .label(label)
    }

    /// A view around one strip, a ruler or a lane on its own: as wide as its room and as tall as its content, with no
    /// surface and no bar. The axis, the wheel and the keys are those of any view.
    #[must_use]
    pub fn strip(mut self) -> Self {
        self.props.surface = None;
        self.props.sizing = Sizing::FillWidth;
        self.bar = false;
        self
    }

    /// How the view follows the playhead at `playhead` while `playing`.
    #[must_use]
    pub fn follow(mut self, mode: FollowMode, playing: bool, playhead: Tick) -> Self {
        (self.follow, self.playing, self.playhead) = (mode, playing, playhead);
        self
    }

    /// The width of the time area in a view `rect` wide.
    fn width(&self, rect: Rect, style: &TimelineViewStyle) -> f32 {
        (rect.width() - self.inset(style)).max(0.0)
    }

    /// The width of the headers, left of the time area, along the start edge.
    fn inset(&self, style: &TimelineViewStyle) -> f32 {
        if self.headers { style.sizes.header_extent(self.header_width) } else { 0.0 }
    }

    /// The playhead moved to `to`: one `Set` event, as a key gives it.
    fn move_playhead(&mut self, to: Tick, label: &str) {
        let to = to.max(Tick::ZERO);
        if to != self.playhead {
            let before = Mixed::Same(self.playhead);
            self.playhead_edit = Some(EditEvent::set(before, to, Arc::from(label), None));
        }
    }

    /// The view the user sees: the bound view with its zoom eased.
    fn shown(&self, cx: &Cx<'_, Self>) -> TimeView {
        let Some(target) = cx.value().copied() else { return TimeView::default() };
        if !(target.points_per_second > 0.0 && target.points_per_second.is_finite()) {
            return target;
        }
        let eased = f64::from(cx.anim("timeline.zoom", target.points_per_second.ln() as f32, spec::MOVE)).exp();
        let zooming = (eased - target.points_per_second).abs() / target.points_per_second > ZOOM_SETTLE;
        let mut view = TimeView { start: target.start, points_per_second: if zooming { eased } else { target.points_per_second } };
        if let (true, Some((time, x))) = (zooming, self.anim.anchor) {
            view.start = (time - Tick::from_seconds_f64(f64::from(x) / view.points_per_second)).max(Tick::ZERO);
        }
        view
    }

    /// The bar's span over `view` in seconds: the sequence and the room past its end, and the seconds in view.
    fn span(&self, view: &TimeView, width: f32) -> ScrollSpan {
        let pps = view.points_per_second;
        let visible = view.start.seconds()..view.start.seconds() + f64::from(width) / pps;
        let content = 0.0..(self.end.max(Tick::ZERO) + VIEW_EXTENT_PAST_END).seconds().max(visible.end);
        ScrollSpan::new(content, visible).resize_ends(true)
    }

    /// `view` zoomed by `factor` about `anchor`, which stays under the same x while the zoom eases.
    fn zoom(&mut self, view: &mut TimeView, factor: f64, anchor: Tick, width: f32) {
        let x = ((anchor - view.start).seconds() * view.points_per_second) as f32;
        let x = if (0.0..=width).contains(&x) { x } else { width / 2.0 };
        view.zoom_about(factor, anchor, width);
        self.anim.anchor = Some((anchor, x));
    }

    /// Reads this pass's wheel into `next`.
    fn wheel(&mut self, wheel: Wheel, next: &mut TimeView, at: Tick, width: f32) {
        let pinched = (f64::from(wheel.zoom) - 1.0).abs() > PINCH_EPSILON;
        if pinched || (wheel.mods.alt && wheel.scroll.y != 0.0) {
            let factor = if pinched { f64::from(wheel.zoom) } else { wheel_zoom_factor(wheel.scroll.y) };
            if factor.is_finite() {
                self.zoom(next, factor, at, width);
            }
            return;
        }
        let sideways = if wheel.scroll.x.abs() > wheel.scroll.y.abs() {
            Some(wheel.scroll.x)
        } else if wheel.mods.cmd || wheel.mods.ctrl || wheel.mods.shift {
            Some(wheel.scroll.y)
        } else {
            None
        };
        if let Some(points) = sideways {
            let moved = Tick::from_seconds_f64(f64::from(points) / next.points_per_second);
            next.start = (next.start - moved).max(Tick::ZERO);
            self.anim.anchor = None;
        }
    }

    /// Runs the time area's keys and zoom commands on `next`.
    fn keys(&mut self, cx: &mut Cx<'_, Self>, next: &mut TimeView, width: f32) {
        let rate = self.format.rate;
        let from = rate.frame_at(self.playhead);
        let words = cx.words::<TimelineWords>();
        let label = words.move_playhead.as_str().to_owned();
        while let Some((step, mods)) = cx.intents().take_move() {
            // Alt+arrow is a nudge of a clip, and Cmd+arrow is not Premiere's: only the bare and Shift chords.
            let arrow = matches!(step, NavMove::Back | NavMove::Forward | NavMove::Up | NavMove::Down);
            if mods.fine || mods.lead || (arrow && mods.by != Granularity::Unit) {
                continue;
            }
            let frames = if mods.extend { SHIFT_STEP_FRAMES } else { STEP_FRAMES };
            let to = match step {
                NavMove::Back => Some(rate.tick_of(from.saturating_sub(frames))),
                NavMove::Forward => Some(rate.tick_of(from.saturating_add(frames))),
                NavMove::Start => Some(Tick::ZERO),
                NavMove::End => Some(self.end),
                NavMove::Up => self.edit_points.iter().copied().filter(|p| *p < self.playhead).max(),
                NavMove::Down => self.edit_points.iter().copied().filter(|p| *p > self.playhead).min(),
                _ => None,
            };
            if let Some(to) = to {
                self.move_playhead(to, &label);
                if matches!(step, NavMove::Start | NavMove::End) {
                    // Explicit navigation reveals the playhead even when its time did not change.
                    if let Some(start) = next.scroll_to_reveal(to, width) {
                        next.start = start;
                    }
                }
            }
        }
        if cx.intents().take(Intent::ContextMenu) {
            self.context_menu = true;
        }
        for command in TIME_COMMANDS {
            cx.publish(command, CommandState::new().enabled(true));
        }
        while let Some((command, _)) = cx.take_command::<TimeCommand>() {
            match command {
                TimeCommand::ZoomIn => self.zoom(next, KEY_ZOOM, self.playhead, width),
                TimeCommand::ZoomOut => self.zoom(next, 1.0 / KEY_ZOOM, self.playhead, width),
                TimeCommand::Fit => {
                    let margin = width * FIT_MARGIN_SHARE;
                    *next = TimeView::fit(TimeRange::new(Tick::ZERO, self.end), width, margin);
                    self.anim.anchor = None;
                }
            }
        }
    }
}

impl Component for TimelineView<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::SPLITTER;
    const OVERLAYS: bool = false;
    const SINGLE_VALUE: bool = true;
    type Style = TimelineViewStyle;
    type Output = TimelineViewOutput;

    fn style(cx: &StyleCx<'_>, _: TimelineViewStyleKey) -> TimelineViewStyle {
        TimelineViewStyle {
            metrics: Metrics::new(),
            look: cx.scroll_bar(),
            inset: cx.tokens.space.gap.tight,
            track: cx.color(cx.tokens.color.surface.chrome),
            sizes: TrackSizes::of(cx.tokens),
            playhead: cx.color(cx.tokens.color.accent.solid),
            snap: cx.color(cx.tokens.color.overlay.line),
            marquee_fill: cx.color(cx.tokens.color.accent.soft),
            marquee_edge: cx.color(cx.tokens.color.accent.border),
        }
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS | Sense::TAKE_FOCUS | Sense::WHEEL
    }

    fn bind(&mut self) -> Option<BindMut<'_, TimeView>> {
        Some(self.view.reborrow())
    }

    /// The bar along the bottom edge, from the end of the headers to the end edge.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        if !self.bar {
            return;
        }
        let Some(view) = cx.value().copied() else { return };
        let width = self.width(rect, cx.style);
        if !(view.points_per_second > 0.0 && view.points_per_second.is_finite() && width > 0.0) {
            return;
        }
        let bar = self.bar(rect, cx.flow, cx.style);
        cx.scroll(BAR, bar, Axis::X, self.span(&view, width));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.anim = *cx.memory::<Anim>();
        let Some(target) = cx.value().copied() else { return };
        let width = self.width(cx.rect, cx.style);
        let valid = target.points_per_second > 0.0 && target.points_per_second.is_finite() && width > 0.0;
        if !valid {
            return;
        }
        let mut next = target;
        let pointer = cx.response.hover_pos().unwrap_or_else(|| cx.rect.center());
        let left = cx.flow.span(self.inset(cx.style), width).left();
        if let Some(wheel) = cx.take_wheel() {
            let at = next.time_at(pointer.x.max(left), left);
            self.wheel(wheel, &mut next, at, width);
        }
        self.keys(cx, &mut next, width);
        if let Some(range) = cx.scroll_moved(BAR) {
            let seconds = range.end - range.start;
            if seconds > 0.0 {
                let shown = self.span(&target, width).visible;
                let zoomed = ((shown.end - shown.start) - seconds).abs() > f64::EPSILON;
                if zoomed {
                    next.points_per_second = (f64::from(width) / seconds).clamp(TimeView::MIN_ZOOM, TimeView::MAX_ZOOM);
                }
                next.start = Tick::from_seconds_f64(range.start.max(0.0));
                self.anim.anchor = None;
            }
        }
        // Follow: while playing the view follows the playhead; a jump while stopped reveals it.
        let mut moved = self.anim.follow.update(self.follow, self.playing, &next, width, self.playhead);
        if moved.is_none() && !self.playing && self.anim.seen.is_some_and(|seen| seen != self.playhead) {
            moved = next.scroll_to_reveal(self.playhead, width);
        }
        if let Some(start) = moved {
            next.start = start;
            self.anim.anchor = None;
        }
        self.anim.seen = Some(self.playhead);
        next.start = next.start.clamp(Tick::ZERO, next.max_start(self.end, width));
        if next != target {
            cx.set(next);
        }
        *cx.memory::<Anim>() = self.anim;
    }

    fn content_env(&self, cx: &Cx<'_, Self>, values: &mut AppValues) {
        values.set(InTimeline(Some(TimeAxis { view: self.shown(cx), format: self.format, end: self.end })));
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if !self.bar {
            return;
        }
        let Some(view) = cx.value().copied() else { return };
        let width = self.width(cx.rect, cx.style);
        if !(view.points_per_second > 0.0 && view.points_per_second.is_finite() && width > 0.0) {
            return;
        }
        let style = cx.style;
        let track = self.bar(cx.rect, cx.flow, style);
        let draw = ui.draw();
        draw.fill(&PaintShape::Rect(track, CornerRadius::ZERO), style.track);
        draw.rule(RuleKind::Edge, track.left_top(), track.right_top());
        let span = self.span(&view, width);
        let (at, len) = scroll::thumb(&span, track.width(), style.look.min_thumb);
        let thumb = if cx.flow.is_rtl() {
            Rect::from_min_size(Pos2::new(track.max.x - at - len, track.min.y), Vec2::new(len, track.height()))
        } else {
            Rect::from_min_size(Pos2::new(track.min.x + at, track.min.y), Vec2::new(len, track.height()))
        }
        .shrink(style.inset);
        let radius = (thumb.width().min(thumb.height()) / 2.0).clamp(0.0, f32::from(u8::MAX)) as u8;
        draw.fill(&PaintShape::Rect(thumb, CornerRadius::same(radius)), cx.resolve_part(BAR.at(0), &style.look.thumb));
    }

    /// The lines and the marquee over the panes, after the body has reported them.
    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(overlay) = self.overlay else { return };
        let Some((panes, left)) = overlay.panes.get() else { return };
        let axis = TimeAxis { view: self.shown(cx), format: self.format, end: self.end };
        let style = cx.style;
        let draw = ui.draw();
        for (at, color) in [(overlay.snap.get(), style.snap), (Some(self.playhead), style.playhead)] {
            let Some(at) = at else { continue };
            let x = axis.x_of(at, left);
            if x >= left && x <= panes.right() {
                draw.stroke(&PaintShape::Path(vec![Pos2::new(x, panes.top()), Pos2::new(x, panes.bottom())]), StrokeWidth::Thin, color);
            }
        }
        if let Some(swept) = overlay.marquee.get().map(|swept| swept.intersect(panes)).filter(|swept| swept.is_positive()) {
            let area = PaintShape::Rect(swept, CornerRadius::ZERO);
            draw.fill(&area, style.marquee_fill);
            draw.stroke(&area, StrokeWidth::Thin, style.marquee_edge);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(view) = cx.value().copied() else { return };
        let width = self.width(cx.rect, cx.style);
        if !(view.points_per_second > 0.0 && view.points_per_second.is_finite() && width > 0.0) {
            return;
        }
        let span = self.span(&view, width);
        let room = (span.content.end - span.content.start - (span.visible.end - span.visible.start)).max(0.0);
        let position = (span.visible.start - span.content.start).clamp(0.0, room);
        let node = access.node(BAR, Role::SCROLL_BAR);
        node.name = Name::Text(cx.words::<TimelineWords>().scroll_time.as_str().to_owned());
        node.numeric = Some(Numeric::new(position, 0.0..=room));
    }

    fn finish(self, cx: Cx<'_, Self>) -> TimelineViewOutput {
        TimelineViewOutput { view: cx.edit_response(), playhead: self.playhead_edit, context_menu: self.context_menu }
    }
}

impl TimelineView<'_> {
    /// The bar's rect: the bottom edge of `rect`, from the end of the headers to the end edge.
    fn bar(&self, rect: Rect, flow: craft_behavior::Flow, style: &TimelineViewStyle) -> Rect {
        let (inset, thickness) = (self.inset(style), style.look.thickness);
        let span = flow.span(inset, (rect.width() - inset).max(0.0));
        Rect::from_min_max(Pos2::new(span.left(), rect.bottom() - thickness), Pos2::new(span.right(), rect.bottom()))
    }
}

impl Container for TimelineView<'_> {}

/// What the time area reports in a pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct TimelineViewOutput {
    /// The view the wheel, the bar, the zoom commands and the follow asked for.
    pub view: EditResponse<TimeView>,
    /// The playhead a key moved, as a one-step `Set`.
    pub playhead: Option<EditEvent<Tick>>,
    /// The Menu key or Shift+F10 asked for the context menu.
    pub context_menu: bool,
}

impl Deref for TimelineViewOutput {
    type Target = EditResponse<TimeView>;

    fn deref(&self) -> &Self::Target {
        &self.view
    }
}

impl Outcome for TimelineViewOutput {
    type Value = TimeView;

    fn response(&self) -> &Response {
        &self.view.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.view.story_facts()
    }
}

#[cfg(feature = "screen-testing")]
pub(crate) mod stories;
