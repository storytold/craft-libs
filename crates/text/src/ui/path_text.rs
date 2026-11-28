//! `PathTextMarkers`: the start, end and overflow marks of text laid on a path, a canvas control.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use craft_behavior::kurbo::{Point, Vec2 as PathVec};
use craft_behavior::{
    CanvasView, Cursor, CursorPriority, HitLevel, HitPriority, HitShape, Metrics, NavShape, PaintedCursor, PartKey, PartRef, PointerButton,
    PressPhase, Roving, Sense,
};
use craft_ui::canvas::{CanvasSpace, canvas_space};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{Handle, PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{FocusShape, KeyUse};
use craft_ui::style::{ComponentStyle, HandleLook, StyleColor, StyleCx};
use egui::{Pos2, Rect, Response, Vec2};

/// The part of the markers that is a mark: `MARK.at(0)` is the start, `MARK.at(1)` the end.
const MARK: PartKey = PartKey::new("mark");

/// The parts that stand for the text's body: `BODY.at(i)` is glyph box `i`.
const BODY: PartKey = PartKey::new("body");

/// The keyboard path of the glyph boxes after the first: the first one's node.
const BODY_KEYS_BY: &str = "the Text on path node";

/// The distance an arrow key moves a mark along the path, in document units; Shift moves it by `NUDGE_FAR`.
const NUDGE: f64 = 1.0;
const NUDGE_FAR: f64 = 10.0;

/// The hit radius of a mark, in standard handle sides (`size.handle.md`).
const HIT_IN_HANDLES: f32 = 1.5;

/// How far the pointer must be from the path, in screen points, on the other side than at the press to flip the text.
const FLIP_DISTANCE: f64 = 4.0;

/// Marks closer than this on the screen, in points, lie on top of each other.
const COINCIDENT: f32 = 1.0;

/// On a full lap with coinciding marks, a press this close to the marks along the path, in points, takes the body.
const LAP_CENTER: f32 = 1.5;

/// A range of at least a lap minus this, in document units, is a full lap.
const FULL_LAP_SLACK: f64 = 1e-3;

craft_ui::words! {
    /// The names of the markers' parts in the accessibility tree.
    pub struct PathTextWords {
        start: "pathText.start" = "Text start";
        end: "pathText.end" = "Text end";
        body: "pathText.body" = "Text on path";
    }
}

/// The path the text runs along, in its own direction, measured by arc length.
///
/// The markers map a flipped range onto it themselves (a range with `flip` set is measured on the reversed path); the
/// host never reverses the track.
pub trait PathTrack {
    /// The path's length in document units.
    fn length(&self) -> f64;

    /// Whether the path is closed, so a range may wrap past its end.
    fn closed(&self) -> bool;

    /// The point at arc length `s` and the direction of the path there, or `None` off the path.
    fn at(&self, s: f64) -> Option<(Point, PathVec)>;

    /// The arc length of the path's point nearest `p`; `near` prefers the candidate closest to that arc length.
    fn project(&self, p: Point, near: Option<f64>) -> Option<f64>;
}

/// The part of the path the text fills: arc lengths of its start and end, and whether it runs against the path.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PathRange {
    /// The arc length of the text's start.
    pub start: f64,
    /// The arc length of the text's end.
    pub end: f64,
    /// The text sits on the other side of the path and runs backwards.
    pub flip: bool,
}

impl PathRange {
    /// The range from arc length `start` to `end`, running against the path when `flip`.
    pub const fn new(start: f64, end: f64, flip: bool) -> Self {
        Self { start, end, flip }
    }
}

/// The part of the markers under the pointer or being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PathGrab {
    /// The start mark.
    Start,
    /// The end mark.
    End,
    /// The path between the marks.
    Body,
}

/// The style of the markers.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PathTextMarkersStyle {
    #[style(metrics)]
    metrics: Metrics,
    handle: HandleLook,
    line: StyleColor,
    halo: StyleColor,
    /// The hit radius of a mark.
    hit: f32,
}

/// The press the markers are dragging with, kept from the press to its release.
///
/// Arc lengths are in the frame of the range at the press (`at_press.flip`); `moved` and `mirrored` work there. The
/// track is the same one for the whole gesture.
#[derive(Clone, Copy, Debug)]
struct Grip {
    /// What the drag moves; `None` while `either` waits for the first motion.
    grab: Option<PathGrab>,
    /// The press took coinciding marks that are not a full lap: the first motion picks the start (backward) or the end (forward).
    either: bool,
    /// The range at the press.
    at_press: PathRange,
    /// The arc length of the pointer's last projection.
    previous: f64,
    /// The pointer's motion along the path since the press.
    delta: f64,
    /// Whether the press was on the positive side of the path (to the left of its direction on the screen).
    side: bool,
    /// The pointer is on the other side of the path than at the press.
    flipped: bool,
    began: bool,
}

impl Default for Grip {
    fn default() -> Self {
        Grip {
            grab: None,
            either: false,
            at_press: PathRange { start: 0.0, end: 0.0, flip: false },
            previous: 0.0,
            delta: 0.0,
            side: true,
            flipped: false,
            began: false,
        }
    }
}

impl Grip {
    /// The range the drag so far proposes.
    fn proposal(&self, anchor: f64, length: f64, closed: bool) -> PathRange {
        let range = moved(self.at_press, self.grab, self.delta, anchor, length, closed);
        if self.flipped { mirrored(range, length, closed) } else { range }
    }
}

/// `v` limited to `lo..=hi`; a reversed pair gives `hi`, where `f64::clamp` would panic.
fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

/// Arc length `s` seen from the other end of the path.
fn reversed(s: f64, length: f64, closed: bool) -> f64 {
    let s = length - s;
    if closed { s.rem_euclid(length) } else { s }
}

/// `range` on the reversed path, flip toggled: the same span of the path from its other end.
fn mirrored(range: PathRange, length: f64, closed: bool) -> PathRange {
    let start = reversed(range.end, length, closed);
    PathRange { start, end: start + range.end - range.start, flip: !range.flip }
}

/// `range` after `grab` moves by `delta` along the path. No motion leaves the range as it is.
///
/// A mark clamps to the path (open) or to one lap from the other mark (closed). The body of a closed path shifts both
/// marks; the body of an open path keeps the span's `anchor` point (0, ½ or 1 of the span) at the clamped place when
/// the span reaches an end of the path, so a span ending at the path's end can still move.
fn moved(range: PathRange, grab: Option<PathGrab>, delta: f64, anchor: f64, length: f64, closed: bool) -> PathRange {
    if delta == 0.0 {
        return range;
    }
    let PathRange { start, end, flip } = range;
    let (start, end) = match grab {
        Some(PathGrab::Start) => (clamp(start + delta, if closed { end - length } else { 0.0 }, end), end),
        Some(PathGrab::End) => (start, clamp(end + delta, start, if closed { start + length } else { length })),
        _ if closed => (start + delta, end + delta),
        _ => {
            let span = end - start;
            let at = clamp(start + span * anchor + delta, 0.0, length);
            if anchor == 0.0 {
                (at, (at + span).min(length))
            } else if anchor == 1.0 {
                ((at - span).max(0.0), at)
            } else {
                let half = (span / 2.0).min(at).min(length - at);
                (at - half, at + half)
            }
        }
    };
    PathRange { start, end, flip }
}

/// `v` as a unit vector, or `None` for a vector too short to give a direction.
fn unit(v: Vec2) -> Option<Vec2> {
    let length = v.length();
    (length > 1e-6).then(|| v / length)
}

/// The pointer for `grab`: the I-beam with a triangle toward each side the grab moves.
fn cursor_of(grab: Option<PathGrab>) -> Cursor {
    let (start, end) = match grab {
        Some(PathGrab::Start) => (true, false),
        Some(PathGrab::End) => (false, true),
        _ => (true, true),
    };
    PaintedCursor::path_text(start, end).into()
}

/// The start and end marks of type on a path, and the overflow mark when the text does not fit.
///
/// The value is the [`PathRange`]. A drag of a mark moves that end, a drag of the body slides the range, and a drag
/// across the path flips the text to its other side.
///
/// - A drag is one gesture (`Begin` on the first move, `Commit` on the release); Escape cancels it. A click with no
///   motion changes nothing.
/// - Marks on top of each other on a full closed lap: a press ahead of them along the path takes the start, behind
///   them the end, between them the body. Marks on top of each other on any other range wait for the first motion:
///   backward takes the start, forward the end.
/// - The marks and the body are parts with one node each (Tab stops of one roving group). The arrows move the
///   focused mark one document unit along the path, Shift ten; the body moves both.
/// - A pass applies only its own input: a second pass in the same frame sees the pointer where the first left it and
///   proposes the same range.
#[must_use = "show the markers with `show`"]
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct PathTextMarkers<'a> {
    #[common]
    common: Common,
    range: Bind<'a, PathRange>,
    track: &'a dyn PathTrack,
    /// Where the paragraph's anchor sits in the span: 0 the start, 0.5 the centre, 1 the end.
    #[prop(quantity = "fraction of the span")]
    anchor: f64,
    /// The text overflows the range: the end mark shows a plus.
    #[prop]
    overflow: bool,
    /// The text's body as quads (four corners each), for the grab of the whole range.
    #[prop]
    body: &'a [[Point; 4]],
    /// The markers show but take no input.
    #[prop]
    locked: bool,
    hovered: Option<PathGrab>,
}

impl<'a> PathTextMarkers<'a> {
    /// The markers named `label` over `range` on `track`.
    pub fn new(label: impl Into<Label>, range: Bind<'a, PathRange>, track: &'a dyn PathTrack) -> Self {
        PathTextMarkers { common: Common::new(), range, track, anchor: 0.0, overflow: false, body: &[], locked: false, hovered: None }.label(label)
    }

    /// The marks' sense: they take a drag unless the markers are locked.
    fn mark_sense(&self) -> Sense {
        if self.locked { Sense::TAKE_FOCUS } else { Sense::DRAG | Sense::TAKE_FOCUS }
    }

    /// Arc length `s` of a range with `flip` as an arc length of the track, and the other way round.
    fn arc(&self, s: f64, flip: bool) -> f64 {
        if flip { reversed(s, self.track.length(), self.track.closed()) } else { s }
    }

    /// The point at arc length `s` of a range with `flip` on the screen and the range's direction there on the screen.
    fn screen_at(&self, view: &CanvasView, s: f64, flip: bool) -> Option<(Pos2, Vec2)> {
        let (point, tangent) = self.track.at(self.arc(s, flip))?;
        let tangent = view.map_vec(tangent);
        Some((view.to_screen(point), if flip { -tangent } else { tangent }))
    }

    /// The parts that take a press, with the grab each stands for.
    fn pointer_parts(&self) -> Vec<(PartRef, PathGrab)> {
        let marks = [(MARK.at(0), PathGrab::Start), (MARK.at(1), PathGrab::End)];
        marks.into_iter().chain((0..self.body.len()).map(|i| (BODY.at(i as u64), PathGrab::Body))).collect()
    }

    /// What a press at `at` on `part` grabs, and whether the first motion decides between the marks.
    fn grab_for(&self, part: PathGrab, at: Pos2, range: &PathRange, view: &CanvasView) -> (Option<PathGrab>, bool) {
        if part == PathGrab::Body {
            return (Some(part), false);
        }
        let (Some((start, tangent)), Some((end, _))) = (self.screen_at(view, range.start, range.flip), self.screen_at(view, range.end, range.flip))
        else {
            return (Some(part), false);
        };
        if start.distance(end) >= COINCIDENT {
            return (Some(part), false);
        }
        let full_lap = self.track.closed() && range.end - range.start >= self.track.length() - FULL_LAP_SLACK;
        if !full_lap {
            return (None, true);
        }
        let Some(tangent) = unit(tangent) else { return (None, false) };
        let along = (at - start).dot(tangent);
        if along > LAP_CENTER {
            (Some(PathGrab::Start), false)
        } else if along < -LAP_CENTER {
            (Some(PathGrab::End), false)
        } else {
            (None, false)
        }
    }

    /// The signed distance in screen points of `pos` from the path at arc length `current` of a range with `flip`,
    /// positive to the left of the range's direction.
    fn across(&self, view: &CanvasView, current: f64, pos: Pos2, flip: bool) -> Option<f64> {
        let (at, tangent) = self.screen_at(view, current, flip)?;
        let tangent = unit(tangent)?;
        let offset = pos - at;
        Some(f64::from(tangent.x * offset.y - tangent.y * offset.x))
    }

    /// Takes the pointer at `pos` into `grip` and returns the range it proposes.
    fn drag_to(&self, grip: &mut Grip, space: &CanvasSpace, pos: Pos2) -> Option<PathRange> {
        let (length, closed) = (self.track.length(), self.track.closed());
        let flip = grip.at_press.flip;
        let current = self.track.project(space.view.to_doc(pos), Some(self.arc(grip.previous, flip)))?;
        let arc = self.arc(current, flip);
        if let Some(across) = self.across(&space.view, arc, pos, flip)
            && across.abs() > FLIP_DISTANCE
        {
            grip.flipped = (across > 0.0) != grip.side;
        }
        let mut delta = arc - grip.previous;
        if closed {
            delta = (delta + length / 2.0).rem_euclid(length) - length / 2.0;
        }
        grip.delta += delta;
        grip.previous = arc;
        if grip.either && grip.delta != 0.0 {
            grip.grab = Some(if grip.delta < 0.0 { PathGrab::Start } else { PathGrab::End });
            grip.either = false;
        }
        Some(grip.proposal(self.anchor, length, closed))
    }

    /// The pointer on the parts: presses, drags, releases and the cursor.
    fn run_pointer(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, shown: &PathRange) {
        let mut held: Option<Grip> = *cx.press_slot::<Grip>();
        for (reference, part) in self.pointer_parts() {
            let Some(result) = cx.part_if_declared(reference) else { continue };
            let (press, hover) = (result.press, cx.hover_pos(reference));
            if let Some(pos) = hover
                && held.is_none()
            {
                let (grab, _) = self.grab_for(part, pos, shown, &space.view);
                self.hovered = Some(grab.unwrap_or(PathGrab::Body));
                cx.cursor(cursor_of(grab), CursorPriority::Part);
            }
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    let (grab, either) = self.grab_for(part, start.origin, shown, &space.view);
                    let near = match part {
                        PathGrab::Start => Some(shown.start),
                        PathGrab::End => Some(shown.end),
                        _ => None,
                    };
                    let near = near.map(|s| self.arc(s, shown.flip));
                    let Some(on_track) = self.track.project(space.view.to_doc(start.origin), near) else { continue };
                    let arc = self.arc(on_track, shown.flip);
                    let side = self.across(&space.view, arc, start.origin, shown.flip).is_none_or(|across| across >= 0.0);
                    held = Some(Grip { grab, either, at_press: *shown, previous: arc, delta: 0.0, side, flipped: false, began: false });
                }
                PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                    let Some(mut grip) = held else { continue };
                    if !grip.began {
                        cx.begin();
                        grip.began = true;
                    }
                    let next = self.drag_to(&mut grip, space, pos);
                    held = Some(grip);
                    let Some(next) = next else { continue };
                    cx.propose(next);
                    cx.cursor(cursor_of(grip.grab), CursorPriority::Drag);
                }
                PressPhase::Release { .. } => {
                    if held.take().is_some_and(|grip| grip.began) {
                        cx.commit();
                    }
                }
                PressPhase::Cancel { .. } => held = None,
                _ => {}
            }
        }
        *cx.press_slot::<Grip>() = held;
    }

    /// The arrows move the focused part along the path: a mark by one document unit (Shift ten), the body both marks.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, shown: &PathRange) {
        let grab = cx.active_part().and_then(|part| match (part.key == MARK, part.key == BODY, part.index) {
            (true, _, Some(0)) => Some(PathGrab::Start),
            (true, _, Some(1)) => Some(PathGrab::End),
            (_, true, _) => Some(PathGrab::Body),
            _ => None,
        });
        let Some(grab) = grab else { return };
        let Some((direction, mods)) = cx.intents().take_nudge() else { return };
        let at = if grab == PathGrab::End { shown.end } else { shown.start };
        let Some((_, tangent)) = self.screen_at(&space.view, at, shown.flip) else { return };
        let along = f64::from(direction.x * tangent.x + direction.y * tangent.y);
        if along.abs() < 1e-9 {
            return;
        }
        let step = if mods.shift { NUDGE_FAR } else { NUDGE };
        let next = moved(*shown, Some(grab), step * along.signum(), self.anchor, self.track.length(), self.track.closed());
        if next != *shown {
            cx.set(next);
        }
    }
}

impl Component for PathTextMarkers<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_moves();
    type Style = PathTextMarkersStyle;
    type Output = PathTextMarkersOutput;

    fn style(cx: &StyleCx<'_>, _: PathTextMarkersStyleKey) -> PathTextMarkersStyle {
        let overlay = &cx.tokens.color.overlay;
        PathTextMarkersStyle {
            metrics: Metrics::new(),
            handle: cx.handle(),
            line: cx.color(overlay.line),
            halo: cx.color(overlay.halo),
            hit: cx.tokens.size.handle.md * HIT_IN_HANDLES,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, PathRange>> {
        Some(self.range.reborrow())
    }

    /// The start and end marks, then the glyph boxes of the body. A mark outranks the body where they overlap. The
    /// body of locked markers takes no part, so the tool under it keeps its presses.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let (Some(space), Some(range)) = (canvas_space(&cx.env), cx.value().copied()) else { return };
        let hit = cx.style.hit;
        let mut any = false;
        for (index, s) in [range.start, range.end].into_iter().enumerate() {
            let Some((at, _)) = self.screen_at(&space.view, s, range.flip) else { continue };
            cx.part(MARK.at(index as u64), HitShape::circle(at, hit))
                .sense(self.mark_sense())
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .focus_shape(FocusShape::Circle)
                .focusable();
            any = true;
        }
        if !self.locked {
            for (index, quad) in self.body.iter().enumerate() {
                let shape = HitShape::Polygon(quad.iter().map(|&corner| space.view.to_screen(corner)).collect());
                let sense = Sense::DRAG | Sense::TAKE_FOCUS;
                let priority = HitPriority::new(HitLevel::Body, 0);
                if index == 0 {
                    cx.part(BODY.at(0), shape).sense(sense).priority(priority).focus_shape(FocusShape::Rect).focusable();
                    any = true;
                } else {
                    cx.part(BODY.at(index as u64), shape).sense(sense).priority(priority).pointer_only(BODY_KEYS_BY);
                }
            }
        }
        if any {
            cx.roving(Roving::new(NavShape::Flat));
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (Some(space), Some(shown)) = (canvas_space(&cx.env), cx.value().copied()) else { return };
        let length = self.track.length();
        if self.locked || length.is_nan() || length <= 0.0 {
            return;
        }
        self.run_pointer(cx, &space, &shown);
        self.run_keys(cx, &space, &shown);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(space), Some(range)) = (canvas_space(&cx.env), cx.value().copied()) else { return };
        let style = cx.style;
        let look = &style.handle;
        let radius = look.side / 2.0;
        let draw = ui.draw();
        if let Some((at, _)) = self.screen_at(&space.view, range.start, range.flip) {
            let arm = |x: f32, y: f32| PaintShape::Path(vec![at - Vec2::new(x, y) * radius, at + Vec2::new(x, y) * radius]);
            for cross in [arm(1.0, 1.0), arm(1.0, -1.0)] {
                draw.stroke(&cross, StrokeWidth::Thick, style.halo);
                draw.stroke(&cross, StrokeWidth::Medium, style.line);
            }
        }
        if let Some((at, _)) = self.screen_at(&space.view, range.end, range.flip) {
            let circle = PaintShape::Circle(at, radius);
            draw.fill(&circle, cx.resolve_part(MARK.at(1), &look.fill));
            draw.stroke(&circle, StrokeWidth::Thin, look.stroke);
            if self.overflow {
                Handle::plus(&draw, look, at, look.side);
            }
        }
    }

    /// One node per mark and one for the body.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(range) = cx.value().copied() else { return };
        let words = cx.words::<PathTextWords>();
        for (index, (s, name)) in [(range.start, words.start.as_str()), (range.end, words.end.as_str())].into_iter().enumerate() {
            if self.track.at(self.arc(s, range.flip)).is_some() {
                let node = access.node(MARK.at(index as u64), Role::BUTTON);
                node.name = Name::Text(name.to_owned());
            }
        }
        if !self.locked && !self.body.is_empty() {
            let node = access.node(BODY.at(0), Role::BUTTON);
            node.name = Name::Text(words.body.as_str().to_owned());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> PathTextMarkersOutput {
        PathTextMarkersOutput { range: cx.edit_response(), hovered: self.hovered }
    }
}

/// What the markers report.
#[derive(Debug)]
#[non_exhaustive]
pub struct PathTextMarkersOutput {
    /// The range as the user drags it (`Begin`, `Update`, `Commit`) or a key sets it.
    pub range: EditResponse<PathRange>,
    /// The part under the pointer.
    pub hovered: Option<PathGrab>,
}

impl Outcome for PathTextMarkersOutput {
    type Value = PathRange;

    fn response(&self) -> &Response {
        &self.range.response
    }
}
