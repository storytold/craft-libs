//! The clips of one track row, with Premiere's trims and moves (N1.05).
//!
//! [`ClipLane`] is the lane helper of `TrackRow`: the row registers the lane's parts, runs the lane's pointer rules in
//! its `update`, and paints the lane in its `paint`. The row's value is the [`ClipEdit`] the lane's drags edit, so a
//! drag is the row's one gesture.

use std::rc::Rc;

use craft_behavior::{CursorIcon, HitLevel, HitPriority, Intent, MarqueeSelect, PartKey, PartRef, PointerButton, PressPhase, SelectionChange, Sense};
use craft_ui::contract::{
    Access, Component, ContentPainter, Cx, Edit as _, Focus as _, Name, Outcome, PaintUi, PartsCx, Pointer as _, Remember as _, Styled as _,
};
use craft_ui::primitives::{ContentPainterCx, PaintShape, StrokeWidth, TextRun};
use craft_ui::style::{DataColor, Stateful, States, StyleColor, StyleCx, TypeRole};
use egui::{CornerRadius, Pos2, Rect, pos2};

use super::{TimeAxis, label_palette};
use crate::clips::{ClipEdge, ClipEdit, EdgeZones, SnapTargets, TrimKind, edge_at, edge_zone, selection_trim, snap_time};
use crate::markers::MarkerColor;
use crate::model::{ClipId, ClipInfo};
use crate::time::Tick;
use crate::timecode::format_time;

/// How strongly a track's colour label tints its lane (FilmCraft `timeline.rs:255` `TRACK_LANE_TINT`, used at `:335`).
const LANE_LABEL_TINT: f32 = 0.10;
/// The locked hatch's line spacing, as a share of the control height.
const HATCH_STEP_SHARE: f32 = 0.25;
/// Within this share of the edge zone of a cut, the Selection tool with its no-modifier trims rolls.
const ROLL_SHARE: f32 = 0.5;

/// The clip body of `CLIP.at(id)`; its start and end edges are `CLIP_START.at(id)` and `CLIP_END.at(id)`, which rank
/// above the body.
/// The empty space of the lane: it takes the drags that sweep a marquee and the click that clears the clip selection
/// (Premiere; Shift or Cmd/Ctrl keeps it).
const BACKDROP: PartKey = PartKey::new("laneBackdrop");
/// A clip's body wins over the backdrop behind it, and the backdrop wins over the row's own root shape.
const CLIP_PRIORITY: HitPriority = HitPriority::new(HitLevel::Body, 2);
const BACKDROP_PRIORITY: HitPriority = HitPriority::new(HitLevel::Body, 1);
/// The clip's body.
pub const CLIP: PartKey = PartKey::new("clip");
/// The start edge of the clip.
pub const CLIP_START: PartKey = PartKey::new("clipStart");
/// The end edge of the clip.
pub const CLIP_END: PartKey = PartKey::new("clipEnd");

/// The timeline tool that decides what a press on a clip does (Premiere's Tools panel; the app
/// keeps the current one).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TimelineTool {
    /// Select, move, and trim at edges ([`selection_trim`](crate::clips::selection_trim)).
    #[default]
    Selection,
    /// Ripple Edit: trims move later clips along.
    Ripple,
    /// Rolling Edit: moves the cut between two touching clips.
    Rolling,
    /// Rate Stretch: changes a clip's speed to fill its new length.
    RateStretch,
    /// Razor: cuts a clip at the pointer.
    Razor,
    /// Slip: moves a clip's media inside the clip.
    Slip,
    /// Slide: moves a clip between its neighbours.
    Slide,
}

/// How a video clip's thumbnails are laid out (Premiere's Video Thumbnails display setting).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ThumbnailMode {
    /// None.
    Off,
    /// One at the head.
    Head,
    /// One at the head and one at the tail.
    #[default]
    HeadAndTail,
    /// Tiles along the clip.
    Continuous,
}

/// At most this many thumbnail tiles per clip (FilmCraft `clips.rs:128`).
pub const MAX_THUMBNAIL_TILES: usize = 256;

/// Where the thumbnails of a clip from `x0` to `x1` go, `tile_width` wide, and the clip time each
/// shows: `(x, time)`. Only tiles within `visible` (an x range) are listed; `Continuous` tiles the
/// clip edge to edge, capped at [`MAX_THUMBNAIL_TILES`]; `frame` is one frame's duration (the
/// tail shows the last frame).
// filmcraft: crates/ui-egui/src/panels/timeline/clips.rs:135 `thumbnail_tiles`.
pub fn thumbnail_tiles(mode: ThumbnailMode, x0: f32, x1: f32, tile_width: f32, visible: (f32, f32), duration: Tick, frame: Tick) -> Vec<(f32, Tick)> {
    let width = x1 - x0;
    if !(x0.is_finite() && width.is_finite() && width > 0.0 && tile_width.is_finite() && tile_width >= 1.0) {
        return Vec::new();
    }
    let last = (duration - frame).max(Tick::ZERO);
    let offset_at = |x: f32| Tick(((x - x0) as f64 / width as f64 * duration.0 as f64).round() as i64).clamp(Tick::ZERO, last);
    let shown = |x: f32| x < visible.1 && x + tile_width > visible.0;
    let head = (x0, Tick::ZERO);
    let tiles = match mode {
        ThumbnailMode::Off => Vec::new(),
        ThumbnailMode::Head => vec![head],
        ThumbnailMode::HeadAndTail if width >= 2.0 * tile_width => vec![head, (x1 - tile_width, last)],
        ThumbnailMode::HeadAndTail => vec![head],
        ThumbnailMode::Continuous => {
            // A grid anchored at the clip's head: scrolling reveals tiles without changing the
            // others' frames.
            let first = ((visible.0 - x0).max(0.0) / tile_width).floor() as usize;
            let end = ((visible.1.min(x1) - x0) / tile_width).ceil().max(1.0) as usize;
            (first..end).take(MAX_THUMBNAIL_TILES).map(|k| x0 + k as f32 * tile_width).map(|x| (x, offset_at(x))).collect()
        }
    };
    tiles.into_iter().filter(|(x, _)| shown(*x)).collect()
}

type ClipContentFn<'a> = Rc<dyn Fn(&ContentPainterCx<'_>, Rect, Rect, &ClipInfo<'_>) + 'a>;

/// Paints a clip's content (thumbnails, waveform, fades): called with the painter, the clip's whole body, the part of
/// the body inside the lane (clip the painter to it) and the clip.
#[derive(Clone)]
pub struct ClipContent<'a>(ClipContentFn<'a>);

impl<'a> ClipContent<'a> {
    /// The content that `paint` draws.
    pub fn new(paint: impl Fn(&ContentPainterCx<'_>, Rect, Rect, &ClipInfo<'_>) + 'a) -> Self {
        ClipContent(Rc::new(paint))
    }
}

/// The fills, strokes and sizes of a clip lane, built in the style hook of the row that hosts the lane.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ClipLook {
    /// The outline of a selected clip.
    outline: StyleColor,
    /// The line at a clip's start.
    separator: StyleColor,
    /// The stripes of a locked track.
    hatch: StyleColor,
    /// The clip's name; the disabled state is a disabled clip's.
    name: Stateful<StyleColor>,
    /// The hit sizes of the edges.
    zones: EdgeZones,
    /// How near a snap target a dragged time must be to snap, in points.
    snap_reach: f32,
    /// The room around the name.
    pad: f32,
    /// The spacing of the locked stripes.
    hatch_step: f32,
}

impl ClipLook {
    /// The look the tokens give.
    pub fn of(cx: &StyleCx<'_>) -> Self {
        let tokens = cx.tokens;
        let edge = tokens.size.hit.target;
        let control = cx.control();
        ClipLook {
            outline: cx.color(tokens.color.accent.solid),
            separator: cx.color(tokens.color.surface.canvas),
            hatch: cx.color(tokens.color.text.faint),
            name: {
                let on_strong = cx.color(tokens.color.text.on_strong);
                let disabled = cx.color(tokens.color.text.disabled);
                Stateful::new(on_strong).off(States::new(on_strong, on_strong, on_strong, disabled))
            },
            zones: EdgeZones { edge, roll: edge * ROLL_SHARE },
            snap_reach: edge,
            pad: control.pad_x,
            hatch_step: control.height * HATCH_STEP_SHARE,
        }
    }
}

/// What the lane reports in a pass. The drags are the host's gesture (its `ClipEdit` value).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ClipLaneEvents {
    /// A click's, a keyboard activation's or a drag start's selection change.
    pub select: Option<SelectionChange<ClipId>>,
    /// Cmd/Ctrl left the linked partners out of the pick.
    pub unlinked: bool,
    /// The clip that was double-clicked.
    pub open: Option<ClipId>,
    /// The clip that asked for its menu, a rect at the place to open it, and the pointer's position when the menu came
    /// from a secondary click (none for a key).
    pub context: Option<(ClipId, Rect, Option<Pos2>)>,
    /// Where a drag snapped this pass.
    pub snapped_at: Option<Tick>,
    /// The pointer's y while a move drag runs: the panel that holds the rows works out the track change from it.
    pub pointer_y: Option<f32>,
    /// The marquee a drag on empty lane space sweeps, with the selection it started from. The clips it touches can lie
    /// on other tracks, so the panel that holds the rows works out the selection.
    pub marquee: Option<MarqueeSelect<ClipId>>,
    /// The marquee ended in this pass (released): its last position is in [`marquee`](Self::marquee).
    pub marquee_ended: bool,
}

/// A marquee under way on empty lane space, kept from the drag start to the release.
#[derive(Clone)]
struct Sweep(MarqueeSelect<ClipId>);

impl Default for Sweep {
    fn default() -> Self {
        Sweep(MarqueeSelect::begin(Pos2::ZERO, craft_keymap::Mods::NONE, Vec::new()))
    }
}

/// What a press grabbed, worked out on the press pass (FilmCraft `Pressed`, #259).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    /// A clip edge and the trim it starts; `partner` is the clip touching the cut on the other side.
    Edge {
        clip: ClipId,
        edge: ClipEdge,
        kind: TrimKind,
        partner: Option<ClipId>,
    },
    Body(ClipId),
    Empty,
}

/// A primary press under way: kept from the press to the release.
#[derive(Clone, Debug)]
struct Pressed {
    grab: Grab,
    /// The time under the pointer at the press.
    from: Tick,
    /// The clips a body drag moves: the selection when it holds the pressed clip, else the clip.
    moving: Vec<ClipId>,
    /// Linked partners trim along (false when Alt was held at the press).
    linked: bool,
}

impl Default for Pressed {
    fn default() -> Self {
        Pressed { grab: Grab::Empty, from: Tick::ZERO, moving: Vec::new(), linked: true }
    }
}

/// The clips of one track row.
///
/// - Draws each [`ClipInfo`] as a block: label colour, name, a selected outline, dimmed when disabled; the app paints
///   the content through [`content`](Self::content) before the outline.
/// - A press with the [`TimelineTool`] on an edge ([`edge_at`] with the look's edge and roll zones) starts a trim of the
///   kind the tool and modifiers give; on a body it moves (Selection), slips or slides; the Razor cuts at the
///   pointer (snapped to frames). The drag is the host's gesture of [`ClipEdit`]; moves snap to the
///   [`snap`](Self::snap) targets within the snap reach ([`snap_time`]). Esc during the drag cancels it, and the
///   release after an Esc does nothing (FilmCraft #580).
/// - A press acts on what it pressed, worked out on the press pass, not when the drag threshold passes (FilmCraft
///   `Pressed`, #259).
/// - A click picks a clip: a click replaces, Shift toggles, Cmd/Ctrl toggles without linked partners (Premiere's),
///   reported as a [`SelectionChange`]; a double-click and a secondary click are reported. A click on empty space
///   clears the selection (Shift or Cmd/Ctrl keeps it); a drag from it sweeps a marquee.
/// - A locked track draws stripes over its clips and starts no gesture.
/// - Each clip is a focusable part: Enter and Space pick it. AccessKit: a button per clip, named by its name, valued by
///   its start and duration.
#[non_exhaustive]
pub struct ClipLane<'a> {
    clips: &'a [ClipInfo<'a>],
    tool: TimelineTool,
    no_modifier_trims: bool,
    snap: Option<&'a SnapTargets>,
    locked: bool,
    selection: &'a [ClipId],
    track_label: Option<MarkerColor>,
    content: Option<ClipContent<'a>>,
}

impl<'a> ClipLane<'a> {
    /// A lane for the `clips` of one track, those that overlap the visible time.
    pub fn new(clips: &'a [ClipInfo<'a>]) -> Self {
        Self {
            clips,
            tool: TimelineTool::Selection,
            no_modifier_trims: false,
            snap: None,
            locked: false,
            selection: &[],
            track_label: None,
            content: None,
        }
    }

    /// The tool that decides what a press does; [`TimelineTool::Selection`] by default.
    pub fn tool(mut self, tool: TimelineTool) -> Self {
        self.tool = tool;
        self
    }

    /// Premiere's Trim setting "Allow Selection tool to choose Roll and Ripple trims without modifier key".
    pub fn no_modifier_trims(mut self, on: bool) -> Self {
        self.no_modifier_trims = on;
        self
    }

    /// The snap targets for drags; `None` turns snapping off.
    pub fn snap(mut self, targets: Option<&'a SnapTargets>) -> Self {
        self.snap = targets;
        self
    }

    /// Whether the track is locked: clips draw hatched and ignore presses.
    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    /// The selected clips; none selected without it.
    pub fn selection(mut self, selection: &'a [ClipId]) -> Self {
        self.selection = selection;
        self
    }

    /// The track's colour label: it tints the lane.
    pub fn track_label(mut self, label: Option<MarkerColor>) -> Self {
        self.track_label = label;
        self
    }

    /// Paints a clip's content into its body.
    pub fn content(mut self, content: ClipContent<'a>) -> Self {
        self.content = Some(content);
        self
    }

    fn geometry<'g>(&'g self, axis: &'g TimeAxis, lane: Rect, look: &ClipLook) -> Lane<'g> {
        Lane {
            clips: self.clips,
            axis,
            lane,
            zones: look.zones,
            no_modifier_trims: self.no_modifier_trims,
            tool: self.tool,
            snap: self.snap,
            tolerance: look.snap_reach,
        }
    }

    /// Registers the clips as parts of the lane cell: each visible body, and its two edges above it when the tool
    /// trims. A locked track registers hover only.
    pub fn parts<C: Component>(&self, lane: Rect, axis: &TimeAxis, look: &ClipLook, cx: &mut PartsCx<'_, C>) {
        let geometry = self.geometry(axis, lane, look);
        let sense = if self.locked { Sense::HOVER } else { Sense::CLICK | Sense::DRAG | Sense::CONTEXT_CLICK | Sense::TAKE_FOCUS };
        if !self.locked {
            cx.part(BACKDROP, lane)
                .sense(Sense::CLICK | Sense::DRAG)
                .priority(BACKDROP_PRIORITY)
                .pointer_only("the clips' own keys and the selection commands pick without the pointer");
        }
        let trims = matches!(self.tool, TimelineTool::Selection | TimelineTool::Ripple | TimelineTool::Rolling | TimelineTool::RateStretch);
        for clip in self.clips {
            let Some(body) = geometry.body(clip) else { continue };
            let part = cx.part(CLIP.at(clip.id.0), body).sense(sense).priority(CLIP_PRIORITY).nudges().inactive(!clip.enabled);
            let part = if self.locked { part } else { part.focusable() };
            let part = match self.tool {
                TimelineTool::Razor => part.cursor(CursorIcon::Crosshair),
                TimelineTool::Slip | TimelineTool::Slide => part.cursor(CursorIcon::ResizeHorizontal),
                _ => part,
            };
            if self.selection.contains(&clip.id) {
                part.selected();
            }
            if !trims || self.locked {
                continue;
            }
            let Some(zone) = edge_zone(body.width(), &look.zones) else { continue };
            let x0 = axis.x_of(clip.range.start, lane.left());
            let x1 = axis.x_of(clip.range.end(), lane.left());
            for (key, at) in [(CLIP_START, x0), (CLIP_END, x1)] {
                let area = Rect::from_min_max(pos2(at - zone, lane.top()), pos2(at + zone, lane.bottom())).intersect(lane);
                if area.is_positive() {
                    cx.part(key.at(clip.id.0), area)
                        .sense(Sense::CLICK | Sense::DRAG | Sense::CONTEXT_CLICK)
                        .priority(HitPriority::new(HitLevel::Edge, 0))
                        .cursor(CursorIcon::ResizeHorizontal)
                        .pointer_only("the host's trim-to-playhead commands trim an edge without the pointer");
                }
            }
        }
    }

    /// One AccessKit button per visible clip.
    pub fn access(&self, access: &mut Access, lane: Rect, axis: &TimeAxis, look: &ClipLook) {
        let geometry = self.geometry(axis, lane, look);
        for clip in self.clips.iter().filter(|clip| geometry.body(clip).is_some()) {
            let node = access.node(CLIP.at(clip.id.0), craft_ui::contract::Role::BUTTON);
            node.name = Name::Text(clip.name.to_owned());
            node.value(format!("{} {}", format_time(clip.range.start, &axis.format), format_time(clip.range.duration, &axis.format)));
        }
    }

    /// Turns the pass's presses on the clips into the host's gesture and the lane's events.
    pub fn update<C>(&mut self, lane: Rect, axis: &TimeAxis, look: &ClipLook, cx: &mut Cx<'_, C>) -> ClipLaneEvents
    where
        C: Component,
        C::Output: Outcome<Value = ClipEdit>,
    {
        let mut events = ClipLaneEvents::default();
        if self.locked {
            *cx.press_slot::<Pressed>() = None;
            return events;
        }
        let geometry = self.geometry(axis, lane, look);
        let mut pressing = None;
        for clip in self.clips {
            for key in [CLIP, CLIP_START, CLIP_END] {
                let Some(result) = cx.part_if_declared(key.at(clip.id.0)) else { continue };
                if result.press.phase.is_press() {
                    pressing = Some((clip.id, result.press));
                }
                if result.press.double_pressed() {
                    events.open = Some(clip.id);
                }
            }
        }
        if let Some((clip, press)) = pressing {
            self.run_press(cx, &geometry, clip, press.phase, press.mods, &mut events);
        } else {
            *cx.press_slot::<Pressed>() = None;
        }
        self.run_picks(cx, &geometry, &mut events);
        self.run_marquee(cx, &geometry, &mut events);
        for clip in self.clips {
            let Some(body) = geometry.body(clip) else { continue };
            if let Some((at, by_pointer)) = context_menu(cx, CLIP.at(clip.id.0)) {
                events.context = Some((clip.id, body, by_pointer.then_some(at)));
            }
        }
        events
    }

    /// The phases of the press on `clip`'s parts: the grab at the press, the gesture along the drag.
    fn run_press<C>(
        &self,
        cx: &mut Cx<'_, C>,
        geometry: &Lane<'_>,
        clip: ClipId,
        phase: PressPhase,
        mods: craft_keymap::Mods,
        events: &mut ClipLaneEvents,
    ) where
        C: Component,
        C::Output: Outcome<Value = ClipEdit>,
    {
        let Some(start) = phase.start().filter(|start| start.button == PointerButton::Primary) else { return };
        let at = match phase {
            PressPhase::Down { start, .. } => start.origin,
            PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } | PressPhase::Release { pos, .. } => pos,
            _ => return,
        };
        let x = at.x;
        if matches!(phase, PressPhase::Down { .. }) {
            if self.tool == TimelineTool::Razor {
                let (at, snapped) = geometry.snapped(geometry.rate_snap(geometry.axis.tick_at(start.origin.x, geometry.lane.left())));
                events.snapped_at = snapped;
                cx.set(ClipEdit::Cut { clip, at });
                return;
            }
            let grab = geometry.grab(start.origin.x, mods);
            let moving = match grab {
                Grab::Body(clip) if self.selection.contains(&clip) => self.selection.to_vec(),
                Grab::Body(clip) => vec![clip],
                _ => Vec::new(),
            };
            let from = geometry.axis.tick_at(start.origin.x, geometry.lane.left());
            *cx.press_slot::<Pressed>() = Some(Pressed { grab, from, moving, linked: !mods.alt });
        }
        let Some(pressed) = cx.press_slot::<Pressed>().clone() else { return };
        let Some(rest) = geometry.rest(&pressed) else { return };
        if matches!(rest, ClipEdit::Move { .. })
            && matches!(phase, PressPhase::DragStart { .. } | PressPhase::Drag { .. } | PressPhase::Release { click: false, .. })
        {
            events.pointer_y = Some(at.y);
        }
        match phase {
            PressPhase::DragStart { .. } => {
                if let Grab::Body(clip) = pressed.grab
                    && !self.selection.contains(&clip)
                {
                    events.select = Some(SelectionChange::Set(vec![clip]));
                }
                cx.propose(rest);
            }
            PressPhase::Drag { .. } => {
                if let Some((edit, snapped)) = geometry.edit(&pressed, x, mods) {
                    cx.propose(edit);
                    events.snapped_at = snapped;
                }
            }
            PressPhase::Release { click: false, .. } => cx.commit(),
            _ => {}
        }
    }

    /// A drag from empty lane space sweeps a marquee (Shift adds, Cmd/Ctrl toggles, Alt subtracts); Esc restores the
    /// selection it started from.
    fn run_marquee<C: Component>(&self, cx: &mut Cx<'_, C>, geometry: &Lane<'_>, events: &mut ClipLaneEvents) {
        let press = cx.part_if_declared(BACKDROP).map(|result| result.press);
        match press.map(|press| press.phase) {
            Some(PressPhase::DragStart { start, pos, .. })
                if start.button == PointerButton::Primary && self.tool != TimelineTool::Razor && geometry.clip_at(start.origin.x).is_none() =>
            {
                let mods = press.map_or(craft_keymap::Mods::NONE, |press| press.mods);
                let mut marquee = MarqueeSelect::begin(start.origin, mods, self.selection.to_vec());
                marquee.update(pos);
                *cx.press_slot::<Sweep>() = Some(Sweep(marquee));
            }
            Some(PressPhase::Drag { pos, .. }) => {
                if let Some(sweep) = cx.press_slot::<Sweep>().as_mut() {
                    sweep.0.update(pos);
                }
            }
            Some(PressPhase::Release { pos, click: false, .. }) => {
                if let Some(sweep) = cx.press_slot::<Sweep>().as_mut() {
                    sweep.0.update(pos);
                    events.marquee_ended = true;
                }
            }
            Some(PressPhase::Cancel { .. }) => {
                if let Some(Sweep(marquee)) = cx.press_slot::<Sweep>().take() {
                    let base = marquee.cancel();
                    events.select = Some(if base.is_empty() { SelectionChange::Clear } else { SelectionChange::Set(base) });
                }
            }
            _ => {}
        }
        events.marquee = cx.press_slot::<Sweep>().clone().map(|sweep| sweep.0);
        if events.marquee_ended {
            *cx.press_slot::<Sweep>() = None;
        }
    }

    /// Clicks, keyboard and AccessKit activations: the picks.
    fn run_picks<C: Component>(&self, cx: &mut Cx<'_, C>, geometry: &Lane<'_>, events: &mut ClipLaneEvents) {
        if events.select.is_some() {
            return;
        }
        // A click on the empty space clears the selection, unless the press held Shift or Cmd/Ctrl (they extend it).
        if let Some(press) = cx.part_if_declared(BACKDROP).map(|result| result.press)
            && press.phase.is_click()
            && press.phase.start().is_some_and(|start| start.button == PointerButton::Primary)
            && !press.mods.shift
            && !press.mods.cmd
            && !self.selection.is_empty()
        {
            events.select = Some(SelectionChange::Clear);
            return;
        }
        for clip in self.clips {
            let mods = [CLIP, CLIP_START, CLIP_END]
                .into_iter()
                .filter_map(|key| cx.part_if_declared(key.at(clip.id.0)))
                .find(|result| result.press.phase.is_click() && result.press.phase.start().is_some_and(|s| s.button == PointerButton::Primary))
                .map(|result| result.press.mods);
            let activated = cx.part_activated(CLIP.at(clip.id.0));
            let mods = match (mods, activated) {
                (Some(mods), _) => mods,
                (None, Some(craft_ui::runtime::commands::FireSource::Key | craft_ui::runtime::commands::FireSource::Access)) => {
                    craft_keymap::Mods::NONE
                }
                _ => continue,
            };
            let (change, unlinked) = geometry.pick(clip.id, mods);
            events.select = change;
            events.unlinked = unlinked;
            return;
        }
    }

    /// Paints the lane: the label tint, each clip, the stripes of a locked track.
    pub fn paint<C: ContentPainter>(&self, ui: &mut PaintUi<'_>, lane: Rect, axis: &TimeAxis, look: &ClipLook, cx: &Cx<'_, C>) {
        let palette = label_palette(&cx.env);
        let geometry = self.geometry(axis, lane, look);
        if let Some(label) = self.track_label {
            let tint = palette.0[usize::from(label.0) % palette.0.len()].gamma_multiply(LANE_LABEL_TINT);
            ui.draw().data_fill(&PaintShape::Rect(lane, CornerRadius::ZERO), DataColor::new(tint));
        }
        for clip in self.clips {
            let Some(body) = geometry.body(clip) else { continue };
            let base = palette.0[usize::from(clip.label.unwrap_or_default().0) % palette.0.len()];
            ui.draw_part(CLIP.at(clip.id.0)).data_fill(&PaintShape::Rect(body, CornerRadius::ZERO), DataColor::new(base));
        }
        if let Some(content) = &self.content {
            let painter = ui.content_painter(cx);
            for clip in self.clips {
                let Some(visible) = geometry.body(clip) else { continue };
                let whole = geometry.whole(clip);
                (content.0)(&painter, whole, visible, clip);
            }
        }
        let draw = ui.draw();
        for clip in self.clips {
            let Some(body) = geometry.body(clip) else { continue };
            let name_color = cx.resolve_part(CLIP.at(clip.id.0), &look.name);
            let room = Rect::from_min_max(pos2(body.left() + look.pad, body.top() + look.pad), pos2(body.right() - look.pad, body.bottom()));
            if room.width() > 0.0 {
                draw.text(room, &TextRun::new(clip.name).role(TypeRole::Caption).color(name_color).elide());
            }
            if geometry.axis.x_of(clip.range.start, lane.left()) >= lane.left() {
                draw.stroke(&PaintShape::Path(vec![body.left_top(), body.left_bottom()]), StrokeWidth::Hairline, look.separator);
            }
            if self.locked {
                stripes(&draw, body, look);
            }
            if self.selection.contains(&clip.id) {
                draw.stroke(&PaintShape::Rect(body, CornerRadius::ZERO), StrokeWidth::Medium, look.outline);
            }
        }
    }
}

/// The locked stripes over `body`: diagonal lines `look.hatch_step` apart, cut at the body's sides.
fn stripes(draw: &craft_ui::primitives::Draw<'_>, body: Rect, look: &ClipLook) {
    if look.hatch_step <= 0.0 {
        return;
    }
    let height = body.height();
    let mut x = body.left() - height;
    while x < body.right() {
        let (from, to) = (x.max(body.left()), (x + height).min(body.right()));
        if from < to {
            draw.stroke(
                &PaintShape::Path(vec![pos2(from, body.bottom() - (from - x)), pos2(to, body.bottom() - (to - x))]),
                StrokeWidth::Hairline,
                look.hatch,
            );
        }
        x += look.hatch_step;
    }
}

/// Where `part` of the component was asked for its context menu in this pass: a secondary click on it, or the Menu key
/// or Shift+F10 while it holds the focus (then its bottom start corner). The intent is taken, so the component's
/// generic context request is not recorded. The flag is true when a secondary click asked, and the position is then the
/// pointer's.
pub(crate) fn context_menu<C: Component>(cx: &mut Cx<'_, C>, part: impl Into<PartRef>) -> Option<(Pos2, bool)> {
    let part = part.into();
    let result = cx.part_if_declared(part)?;
    let (press, rect) = (result.press, result.rect);
    if let PressPhase::Release { start, pos, .. } = press.phase
        && start.button == PointerButton::Secondary
    {
        return Some((pos, true));
    }
    (cx.active_part() == Some(part) && cx.intents().take(Intent::ContextMenu)).then(|| (rect.left_bottom(), false))
}

/// The lane's clips and the numbers its rules read.
struct Lane<'a> {
    clips: &'a [ClipInfo<'a>],
    axis: &'a TimeAxis,
    lane: Rect,
    zones: EdgeZones,
    no_modifier_trims: bool,
    tool: TimelineTool,
    snap: Option<&'a SnapTargets>,
    tolerance: f32,
}

impl Lane<'_> {
    fn info(&self, id: ClipId) -> Option<&ClipInfo<'_>> {
        self.clips.iter().find(|clip| clip.id == id)
    }

    fn rate_snap(&self, t: Tick) -> Tick {
        self.axis.format.rate.snap_nearest(t)
    }

    /// A clip's block over the lane's full height, whole.
    fn whole(&self, clip: &ClipInfo<'_>) -> Rect {
        let (x0, x1) = (self.axis.x_of(clip.range.start, self.lane.left()), self.axis.x_of(clip.range.end(), self.lane.left()));
        Rect::from_min_max(pos2(x0, self.lane.top()), pos2(x1, self.lane.bottom()))
    }

    /// The part of a clip's block inside the lane, `None` when it lies outside.
    fn body(&self, clip: &ClipInfo<'_>) -> Option<Rect> {
        let body = self.whole(clip).intersect(self.lane);
        (body.min.x <= body.max.x && body.max.x >= self.lane.left() && body.min.x <= self.lane.right()).then_some(body)
    }

    fn clip_at(&self, x: f32) -> Option<ClipId> {
        self.clips
            .iter()
            .find(|clip| (self.axis.x_of(clip.range.start, self.lane.left())..self.axis.x_of(clip.range.end(), self.lane.left())).contains(&x))
            .map(|clip| clip.id)
    }

    /// What a press of the tool at `x` grabs (FilmCraft `timeline_hit.rs:283` `grab_at`).
    fn grab(&self, x: f32, mods: craft_keymap::Mods) -> Grab {
        let body_at = || self.clip_at(x).map_or(Grab::Empty, Grab::Body);
        let ranges: Vec<_> = self.clips.iter().map(|clip| (clip.id, clip.range)).collect();
        let Some(hit) = edge_at(&ranges, &self.axis.view, self.lane.left(), x, &self.zones) else { return body_at() };
        let Some(info) = self.info(hit.clip) else { return body_at() };
        let cut = match hit.edge {
            ClipEdge::In => info.range.start,
            ClipEdge::Out => info.range.end(),
        };
        let partner = self
            .clips
            .iter()
            .find(|other| {
                other.id != hit.clip
                    && match hit.edge {
                        ClipEdge::In => other.range.end() == cut,
                        ClipEdge::Out => other.range.start == cut,
                    }
            })
            .map(|other| other.id);
        let kind = match self.tool {
            TimelineTool::Selection => selection_trim(mods.cmd, mods.shift, self.no_modifier_trims, hit.distance, partner.is_some(), &self.zones),
            TimelineTool::Ripple => TrimKind::Ripple,
            TimelineTool::Rolling => TrimKind::Roll,
            TimelineTool::RateStretch => TrimKind::Stretch,
            _ => return Grab::Body(hit.clip),
        };
        // Nothing to roll against: a regular trim.
        let kind = if kind == TrimKind::Roll && partner.is_none() { TrimKind::Trim } else { kind };
        Grab::Edge { clip: hit.clip, edge: hit.edge, kind, partner }
    }

    fn snapped(&self, t: Tick) -> (Tick, Option<Tick>) {
        match self.snap {
            Some(targets) => snap_time(targets, t, &self.axis.view, self.tolerance),
            None => (t, None),
        }
    }

    /// The selection change of a click on `clip` and whether the linked partners stay out: Shift and Cmd/Ctrl toggle,
    /// Cmd/Ctrl without partners, else replace.
    fn pick(&self, clip: ClipId, mods: craft_keymap::Mods) -> (Option<SelectionChange<ClipId>>, bool) {
        if mods.shift || mods.cmd { (Some(SelectionChange::Toggle(clip)), mods.cmd) } else { (Some(SelectionChange::Set(vec![clip])), false) }
    }

    /// The edit a drag reports before it moves: the gesture's start value.
    fn rest(&self, pressed: &Pressed) -> Option<ClipEdit> {
        match pressed.grab {
            Grab::Empty => None,
            Grab::Edge { clip, edge, kind: TrimKind::Roll, partner: Some(other) } => {
                let (left, right) = if edge == ClipEdge::Out { (clip, other) } else { (other, clip) };
                Some(ClipEdit::Roll { left, right, delta: Tick::ZERO })
            }
            Grab::Edge { clip, edge, kind, .. } => Some(ClipEdit::Trim { clip, edge, kind, delta: Tick::ZERO, linked: pressed.linked }),
            Grab::Body(clip) => match self.tool {
                TimelineTool::Razor => None,
                TimelineTool::Slip => Some(ClipEdit::Slip { clip, delta: Tick::ZERO }),
                TimelineTool::Slide => Some(ClipEdit::Slide { clip, delta: Tick::ZERO }),
                _ => Some(ClipEdit::Move { clips: pressed.moving.clone(), delta: Tick::ZERO, track_delta: 0, new_track: false, copy: false }),
            },
        }
    }

    /// The edit of the drag with the pointer at `x`, and the target it snapped to (FilmCraft `interact.rs:558`
    /// `drag_update`).
    fn edit(&self, pressed: &Pressed, x: f32, mods: craft_keymap::Mods) -> Option<(ClipEdit, Option<Tick>)> {
        let here = self.axis.tick_at(x, self.lane.left());
        let moved = here - pressed.from;
        let frame = self.axis.frame();
        match pressed.grab {
            Grab::Empty => None,
            Grab::Body(clip) => match self.tool {
                TimelineTool::Razor => None,
                TimelineTool::Slip => {
                    let info = self.info(clip)?;
                    let mut delta = -self.rate_snap(moved);
                    if let Some(source) = info.source {
                        delta = delta.clamp(info.range.end() - source.end(), info.range.start - source.start);
                    }
                    Some((ClipEdit::Slip { clip, delta }, None))
                }
                TimelineTool::Slide => Some((ClipEdit::Slide { clip, delta: self.rate_snap(moved) }, None)),
                _ => {
                    let dragged: Vec<_> = self.clips.iter().filter(|info| pressed.moving.contains(&info.id)).collect();
                    let first = dragged.iter().map(|info| info.range.start).min()?;
                    let last = dragged.iter().map(|info| info.range.end()).max()?;
                    let raw = self.rate_snap(moved);
                    let (start, start_hit) = self.snapped(first + raw);
                    let (offset, hit) = if start != first + raw {
                        (start - first, start_hit)
                    } else {
                        let (end, end_hit) = self.snapped(last + raw);
                        (end - last, end_hit)
                    };
                    let delta = offset.max(-first);
                    let edit = ClipEdit::Move { clips: pressed.moving.clone(), delta, track_delta: 0, new_track: false, copy: mods.alt };
                    Some((edit, hit))
                }
            },
            Grab::Edge { clip, edge, kind: TrimKind::Roll, partner: Some(other) } => {
                let (left, right) = if edge == ClipEdge::Out { (self.info(clip)?, self.info(other)?) } else { (self.info(other)?, self.info(clip)?) };
                let cut = left.range.end();
                let (target, hit) = self.snapped(self.rate_snap(cut + moved));
                let mut lowest = -(left.range.duration - frame);
                let mut highest = right.range.duration - frame;
                if let Some(source) = left.source {
                    highest = highest.min(source.end() - cut);
                }
                if let Some(source) = right.source {
                    lowest = lowest.max(source.start - right.range.start);
                }
                let delta = (target - cut).clamp(lowest, highest);
                let hit = hit.filter(|_| delta == target - cut);
                Some((ClipEdit::Roll { left: left.id, right: right.id, delta }, hit))
            }
            Grab::Edge { clip, edge, kind, .. } => {
                let info = self.info(clip)?;
                let base = match edge {
                    ClipEdge::In => info.range.start,
                    ClipEdge::Out => info.range.end(),
                };
                let (target, hit) = self.snapped(self.rate_snap(base + moved));
                // A clip keeps one frame, and a trim stops at the media's ends (a stretch changes the speed and has none).
                let shortest = info.range.duration - frame;
                let (mut lowest, mut highest) = match edge {
                    ClipEdge::In => (Tick::MIN, shortest),
                    ClipEdge::Out => (-shortest, Tick::MAX),
                };
                if kind != TrimKind::Stretch
                    && let Some(source) = info.source
                {
                    match edge {
                        ClipEdge::In => lowest = source.start - base,
                        ClipEdge::Out => highest = source.end() - base,
                    }
                }
                let delta = (target - base).clamp(lowest, highest);
                let hit = hit.filter(|_| delta == target - base);
                Some((ClipEdit::Trim { clip, edge, kind, delta, linked: pressed.linked }, hit))
            }
        }
    }
}
