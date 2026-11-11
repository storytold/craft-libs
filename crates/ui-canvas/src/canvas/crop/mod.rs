//! `CropOverlay`: the crop frame of a document, a canvas control (20.6). The frame, its corners and its edges are
//! parts in the canvas's arena; the outside of the frame turns it and, while Straighten is armed, the whole canvas draws
//! the straighten line. The value is a [`CropBox`] in document units.

mod gesture;
mod paint;
mod shield;
#[cfg(feature = "testing")]
mod stories;

use super::words::CanvasWords;
use super::{CanvasSpace, NUDGE, NUDGE_FAR, canvas_space, readout};
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, WordSetCx as _,
};
use crate::runtime::env::Label;
use crate::runtime::focus::KeyUse;
use crate::style::{ComponentStyle, StyleColor, StyleCx};
use craft_behavior::geometry::{BoxPart, CropBox, CropGuide, GuideShow, SnapLine, SnapTargets};
use craft_behavior::kurbo::{Point, Rect as DocRect, Vec2 as DocVec};
use craft_behavior::{
    Axis, CancelReason, CanvasView, CursorIcon, CursorPriority, HitLevel, HitPriority, HitShape, Intent, Metrics, NavShape, PaintedCursor, PartKey,
    PointerButton, PressPhase, Roving, Sense, resize_cursor,
};
use craft_keymap::{Chord, Key, Mods};
use egui::{Pos2, Rect, Response, Vec2};
use gesture::{Drag, Kind, Rules};

pub use shield::{CropShield, ShieldColor};

/// A part of the frame: `FRAME.at(part.index())` with the whole frame at 0, the corners at 1 to 4 and the edges at 5
/// to 8, so the keyboard order is the whole frame and then the corners and the edges clockwise.
pub(super) const FRAME: PartKey = PartKey::new("frame");
/// The outside of the frame: a press there turns it.
const TURN: PartKey = PartKey::new("turn");
/// The whole canvas while Straighten is armed.
const STRAIGHTEN: PartKey = PartKey::new("straighten");

/// The shortest Straighten line, in screen points.
const STRAIGHTEN_MIN: f64 = 3.0;
/// The step of a Shift turn, in degrees.
const TURN_STEP: f64 = 15.0;
/// X swaps portrait and landscape.
const SWAP: Chord = Chord::plain(Key::X);

/// The style of the crop overlay.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CropStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The frame line, corner brackets and edge bars.
    line: StyleColor,
    /// Under the line, the brackets and the Straighten line.
    halo: StyleColor,
    /// The composition guide lines.
    guide: StyleColor,
    /// The canvas colour the shield matches.
    canvas: StyleColor,
    /// The line of a frame the binding refuses.
    invalid: StyleColor,
    /// The focus line round a picked part.
    focus: StyleColor,
    /// Reach of the corners and edges, in screen points.
    reach: f32,
    /// A corner bracket's arm, in screen points.
    arm: f32,
    /// How far the focus line sits outside the frame.
    focus_gap: f32,
    /// How far the Grid guide's cells are apart.
    grid_step: f32,
}

/// The Grid guide's most lines per axis.
const GRID_MAX: usize = 96;
/// The share of a side an edge bar covers at most: a quarter.
const BAR_SHARE: f32 = 4.0;
/// How far the guide lines fade towards the canvas colour.
const GUIDE_FADE: f32 = 0.55;

/// What the overlay reports beyond the edit.
#[derive(Clone, Debug, Default)]
struct Report {
    apply: bool,
    discard: bool,
    straightened: Option<f64>,
    view: Option<CanvasView>,
    snapped: Vec<SnapLine>,
    hovered: Option<BoxPart>,
    dragging: bool,
    turning: Option<f64>,
    sizing: Option<(f64, f64)>,
}

/// What a pass leaves for the next and for paint.
#[derive(Clone, Default)]
struct Live {
    drag: Option<Drag>,
}

/// The crop frame of a document, edited by dragging it as Photoshop's Classic crop does (or moving and turning the image
/// behind it in image mode) and by the keyboard.
///
/// - Pointer: a corner or an edge resizes along the frame's own axes (Shift keeps the ratio, Alt resizes about the
///   centre); inside moves; outside turns the frame about its centre (Shift in steps). Inside the
///   [default frame](CropOverlay::default_frame) a drag draws a new frame (Shift square, Alt from the centre, Space moves
///   it while it is drawn). A [ratio](CropOverlay::ratio) is held by every gesture. A drag begins on the first move; a
///   press released without moving changes nothing, and a drag that leaves a side under one document unit is cancelled.
/// - Keys on a focused part: Enter applies; Escape cancels a drag, else discards; the arrows nudge the picked part
///   (Shift by ten); X swaps portrait and landscape. The frame, its corners and its edges are one Tab stop; Page Down
///   and Page Up pick the next and previous part.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(canvas_control)]
pub struct CropOverlay<'a> {
    #[common]
    common: Common,
    crop: Bind<'a, CropBox>,
    /// The width over height every gesture holds.
    #[prop(quantity = "aspect ratio")]
    ratio: Option<f64>,
    /// The frame is the untouched default (the canvas or the selection's bounds): a drag inside it draws a new frame and
    /// a double click inside does not apply.
    #[prop(flag)]
    default_frame: bool,
    /// The step of a Shift turn, in degrees.
    #[prop(quantity = "degrees")]
    turn_step: f64,
    /// The composition guide and its orientation.
    #[prop]
    guide: Option<(CropGuide, u8)>,
    /// When the guide shows.
    #[prop]
    guide_show: GuideShow,
    /// The shield outside the frame.
    #[prop]
    shield: CropShield,
    /// Straighten is armed: a drag on the canvas draws a line instead of a frame gesture, and its release reports the
    /// angle that levels it.
    #[prop(flag)]
    straighten: bool,
    /// The area the frame may not leave, in document units.
    #[prop]
    bounds: Option<DocRect>,
    /// Image mode: the frame stays upright on screen and a move or turn moves or turns the view instead.
    #[prop(flag)]
    image_mode: bool,
    /// Auto Center Preview in image mode: a resized frame keeps its centre on screen and a drawn frame moves to the
    /// middle of the canvas.
    #[prop(flag)]
    auto_center: bool,
    /// The snap targets a dragged part snaps to.
    #[prop]
    snap: Option<&'a SnapTargets>,
    report: Report,
}

impl<'a> CropOverlay<'a> {
    /// An overlay named `label` over `crop`. A mixed value shows the whole document.
    pub fn new(label: impl Into<Label>, crop: Bind<'a, CropBox>) -> Self {
        CropOverlay {
            common: Common::new(),
            crop,
            ratio: None,
            default_frame: false,
            turn_step: TURN_STEP,
            guide: None,
            guide_show: GuideShow::Auto,
            shield: CropShield::default(),
            straighten: false,
            bounds: None,
            image_mode: false,
            auto_center: false,
            snap: None,
            report: Report::default(),
        }
        .label(label)
    }

    fn rules(&self) -> Rules<'a> {
        Rules {
            ratio: self.ratio.filter(|k| k.is_finite() && *k > 0.0),
            bounds: self.bounds,
            turn_step: self.turn_step,
            image: self.image_mode,
            auto_center: self.auto_center,
            snap: self.snap,
        }
    }

    /// The frame to show: the value, or the whole document while the value is mixed.
    fn shown(&self, value: Option<&CropBox>, view: &CanvasView) -> CropBox {
        value.copied().unwrap_or_else(|| CropBox::upright(self.bounds.unwrap_or_else(|| view.visible())))
    }
}

/// How the overlay maps the document: the canvas's space, or, in a canvas with no view, the shown rect at one to one
/// from its top left.
fn space_of(env: &crate::runtime::env::EnvView<'_>, rect: Rect) -> CanvasSpace {
    canvas_space(env).unwrap_or_else(|| {
        let center = Point::new(f64::from(rect.width()) / 2.0, f64::from(rect.height()) / 2.0);
        let view = CanvasView::new(rect, center, 1.0);
        CanvasSpace::new(view, 1.0)
    })
}

/// The corners of `frame` on screen, clockwise from the top left of the upright frame.
fn screen_corners(view: &CanvasView, frame: &CropBox) -> [Pos2; 4] {
    frame.corners().map(|p| view.to_screen(p))
}

fn degenerate(q: &[Pos2; 4]) -> bool {
    let (lu, lv) = ((q[1] - q[0]).length(), (q[3] - q[0]).length());
    !(lu.is_finite() && lv.is_finite()) || lu < 1e-3 || lv < 1e-3
}

/// The part keyed by `index` of `FRAME`.
fn frame_part(index: u64) -> Option<BoxPart> {
    BoxPart::from_index(index).filter(|part| *part != BoxPart::Pivot)
}

/// What a press lands on, as the overlay tells the parts apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grab {
    Part(BoxPart),
    Turn,
    Line,
}

impl Grab {
    fn key(self) -> (PartKey, Option<u64>) {
        match self {
            Grab::Part(part) => (FRAME, Some(part.index())),
            Grab::Turn => (TURN, None),
            Grab::Line => (STRAIGHTEN, None),
        }
    }
}

/// The units of the document's x and y rulers as text.
fn size_row(cx: &Cx<'_, CropOverlay<'_>>, space: &CanvasSpace, (w, h): (f64, f64)) -> Vec<(Label, String)> {
    let words = cx.words::<CanvasWords>();
    let units = cx.env.units();
    let (x, _) = space.rulers().across(Axis::Y);
    let (y, _) = space.rulers().across(Axis::X);
    let text = |ruler: crate::canvas::RulerText, v: f64| ruler.format(&units, v, 0.0, 0..=*ruler.kind.decimals().end());
    vec![(Label::from(words.crop_width.as_str().to_owned()), text(x, w)), (Label::from(words.crop_height.as_str().to_owned()), text(y, h))]
}

fn angle_row(cx: &Cx<'_, CropOverlay<'_>>, degrees: f64) -> Vec<(Label, String)> {
    let words = cx.words::<CanvasWords>();
    let number = cx.env.units().number_ungrouped(degrees, 1..=1);
    vec![(Label::from(words.crop_angle.as_str().to_owned()), format!("{number}°"))]
}

impl CropOverlay<'_> {
    /// Starts the drag of `grab` pressed at screen point `origin`.
    fn begin(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, frame: &CropBox, grab: Grab, origin: Pos2) -> Drag {
        let view = space.view;
        let from = view.to_doc(origin);
        let kind = match grab {
            Grab::Line => {
                let from = self.rules().snap_screen(&view, origin);
                Kind::Line { from, to: from }
            }
            Grab::Turn => Kind::Turn { grab: gesture::grab(frame, from) },
            Grab::Part(BoxPart::Inside) if !self.default_frame => Kind::Move,
            Grab::Part(BoxPart::Inside) => Kind::Draw { anchor: from, cur: from, last: from },
            Grab::Part(part) => Kind::Resize(part),
        };
        if !matches!(kind, Kind::Line { .. }) {
            cx.begin();
        }
        Drag { kind, at_press: *frame, from, press_view: self.image_mode.then_some(view), last: *frame }
    }

    /// One step of the drag with the pointer at screen point `pos`.
    fn step(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, drag: &mut Drag, pos: Pos2, mods: Mods) {
        let view = space.view;
        let rules = self.rules();
        if let Kind::Line { from, to } = &mut drag.kind {
            *to = rules.snap_screen(&view, pos);
            self.report.dragging = true;
            if let Some(degrees) = gesture::line_degrees(*from, *to) {
                readout(cx, pos, &angle_row(cx, degrees));
            }
            return;
        }
        let held_space = crate::runtime::keys::down(&cx.ctx).contains(&Key::Space);
        let to = drag.press_view.as_ref().unwrap_or(&view).to_doc(pos);
        let (to, lines) = rules.snapped(drag, &view, to, mods, held_space);
        self.report.snapped = lines;
        if let Some(next) = rules.proposal(drag, to, mods, held_space) {
            drag.last = next;
            // A frame with no extent on screen declares no parts, which would end the press with the edit still open: it
            // is remembered (the release cancels the edit) but not proposed.
            if !degenerate(&screen_corners(&view, &next)) {
                cx.propose(next);
                if let Some(moved) = gesture::followed(drag, self.auto_center, &next) {
                    self.report.view = Some(moved);
                }
            }
        }
        self.report.dragging = true;
        match drag.kind {
            Kind::Turn { .. } => self.report.turning = Some(drag.last.angle),
            Kind::Draw { .. } | Kind::Resize(_) => self.report.sizing = drag.last.crop_size(),
            _ => {}
        }
        let rows = match drag.kind {
            Kind::Turn { .. } => angle_row(cx, gesture::turn_degrees(&drag.last, self.image_mode)),
            Kind::Draw { .. } | Kind::Resize(_) => drag.last.crop_size().map(|size| size_row(cx, space, size)).unwrap_or_default(),
            _ => Vec::new(),
        };
        readout(cx, pos, &rows);
    }

    /// The button released after a drag at screen point `pos`.
    fn finish(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, drag: Drag, pos: Pos2) {
        if let Kind::Line { from, to } = drag.kind {
            if f64::from((to - from).length()) >= STRAIGHTEN_MIN {
                self.report.straightened = CropBox::level_angle(space.view.to_doc(from), space.view.to_doc(to));
            }
            return;
        }
        let _ = pos;
        if drag.last.is_finite() && drag.last.size_ok() {
            if let Some(view) = gesture::centred(&drag, self.auto_center, &space.view, &drag.last) {
                self.report.view = Some(view);
            }
            cx.commit();
        } else {
            self.report.view = drag.press_view;
            cx.cancel(CancelReason::Dismissed);
        }
    }

    /// The keys of the focused part: Enter, Escape, the arrows and X.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, frame: &CropBox) {
        let Some(part) = cx.active_part().filter(|part| part.key == FRAME).and_then(|part| part.index).and_then(frame_part) else { return };
        cx.claim(SWAP);
        if cx.intents().take(Intent::Submit) {
            self.report.apply = true;
        }
        if cx.intents().take(Intent::Cancel) {
            self.report.discard = true;
        }
        let rules = self.rules();
        let mut next = None;
        if let Some((direction, mods)) = cx.intents().take_nudge() {
            let step = if mods.shift { NUDGE_FAR } else { NUDGE };
            let by = DocVec::new(f64::from(direction.x), f64::from(direction.y)) * step;
            let (moved, view) = gesture::nudged(frame, part, by, &space.view, &rules);
            self.report.view = view;
            next = Some(moved);
        } else if cx.take_claimed(SWAP) {
            let swapped = frame.swapped();
            if self.image_mode && self.auto_center {
                let mut view = craft_behavior::geometry::crop_view(&swapped, &space.view);
                view.center = swapped.center();
                self.report.view = Some(view);
            }
            next = Some(swapped);
        }
        let Some(next) = next.filter(CropBox::is_finite) else { return };
        let next = match self.bounds {
            Some(bounds) => next.within(bounds, next.rect.size() != frame.rect.size(), self.ratio.is_some()).unwrap_or(*frame),
            None => next,
        };
        if next != *frame {
            cx.set(next);
        }
    }
}

/// What the overlay reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct CropOverlayOutput {
    /// The edit of the frame: a drag is one `Begin` to `Commit` (a `Cancel` on Escape or when a side ends under one
    /// document unit); a key is a `Set`.
    pub edit: EditResponse<CropBox>,
    /// Apply the crop: Enter on a focused part, or a double click inside a drawn frame (not a handle, not the default
    /// frame).
    pub apply: bool,
    /// Close the frame without applying it: Escape with no drag, on a focused part.
    pub discard: bool,
    /// A released Straighten line's frame angle; the line itself sends no edit.
    pub straightened: Option<f64>,
    /// Image mode: the view the gesture moved or turned, or put back after a cancel. The tool hands it to the canvas.
    pub view: Option<CanvasView>,
    /// The snap lines of this pass, for the app's smart guides.
    pub snapped: Vec<SnapLine>,
    /// The part the keys act on: picked by the pointer or the Page keys.
    pub picked: Option<BoxPart>,
    /// What a press at the pointer would take.
    pub hovered: Option<BoxPart>,
    /// Whether a drag is running: the input for the guide's visibility and the shield's editing opacity.
    pub dragging: bool,
    /// The frame's angle while it is turned.
    pub turning: Option<f64>,
    /// The size Enter would crop to while the frame is drawn or resized.
    pub sizing: Option<(f64, f64)>,
}

impl Outcome for CropOverlayOutput {
    type Value = CropBox;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}

impl Component for CropOverlay<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::CANVAS;
    type Style = CropStyle;
    type Output = CropOverlayOutput;

    fn style(cx: &StyleCx<'_>, _: CropOverlayStyleKey) -> CropStyle {
        let tokens = cx.tokens;
        let (canvas, line) = (cx.color(tokens.color.surface.canvas), cx.color(tokens.color.overlay.line));
        let reach = tokens.size.hit.target;
        CropStyle {
            metrics: Metrics::new().min_size(Vec2::splat(reach)),
            line,
            halo: cx.color(tokens.color.overlay.halo),
            guide: craft_theme::Mix::mix(line, canvas, GUIDE_FADE),
            canvas,
            invalid: cx.tone(craft_theme::variant::Tone::Bad),
            focus: cx.color(tokens.color.line.focus),
            reach,
            arm: tokens.size.icon.sm,
            focus_gap: tokens.stroke.thick,
            grid_step: tokens.size.handle.md * 2.0,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.reach)
    }

    fn bind(&mut self) -> Option<BindMut<'_, CropBox>> {
        Some(self.crop.reborrow())
    }

    /// The frame, its corners and its edges as one roving group in keyboard order, the outside that turns it, and the
    /// Straighten surface while that is armed.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let space = space_of(&cx.env, rect);
        let view = space.view;
        let frame = self.shown(cx.value(), &view);
        if self.straighten {
            cx.part(STRAIGHTEN, HitShape::Rect(rect))
                .sense(Sense::DRAG)
                .priority(HitPriority::new(HitLevel::Handle, u8::MAX))
                .cursor(CursorIcon::Crosshair)
                .pointer_only("the options bar's angle field sets the straightened angle");
        }
        let q = screen_corners(&view, &frame);
        if !frame.is_finite() || degenerate(&q) {
            return;
        }
        let (lu, lv) = ((q[1] - q[0]).length(), (q[3] - q[0]).length());
        // A small frame keeps a movable middle: the reach inside is at most a quarter of the shortest side.
        let reach = cx.style.reach.min(lu.min(lv) / 4.0);
        let body = if self.default_frame { CursorIcon::Crosshair } else { CursorIcon::Move };
        cx.part(FRAME.at(BoxPart::Inside.index()), HitShape::Polygon(q.to_vec()))
            .sense(Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS)
            .priority(HitPriority::BODY)
            .cursor(body)
            .focusable()
            .selected();
        for i in 0..4u8 {
            let part = BoxPart::Corner(i);
            let mut declared = cx
                .part(FRAME.at(part.index()), HitShape::circle(q[usize::from(i)], reach))
                .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .focusable();
            if let Some(direction) = frame.part_direction(part) {
                declared = declared.cursor(resize_cursor(&view, direction));
            }
            let _ = declared;
        }
        for i in 0..4u8 {
            let part = BoxPart::Edge(i);
            let (a, b) = (q[usize::from(i)], q[(usize::from(i) + 1) % 4]);
            let across = (b - a).rot90().normalized() * reach;
            let strip = vec![a + across, b + across, b - across, a - across];
            let mut declared = cx
                .part(FRAME.at(part.index()), HitShape::Polygon(strip))
                .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Edge, 0))
                .focusable();
            if let Some(direction) = frame.part_direction(part) {
                declared = declared.cursor(resize_cursor(&view, direction));
            }
            let _ = declared;
        }
        cx.part(TURN, HitShape::Rect(rect))
            .sense(Sense::DRAG)
            .priority(HitPriority::new(HitLevel::Background, u8::MAX))
            .pointer_only("the options bar's angle field sets the frame's angle");
        cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let space = space_of(&cx.env, cx.rect);
        let view = space.view;
        let frame = self.shown(cx.value(), &view);
        if !frame.is_finite() {
            return;
        }
        let mut live = cx.memory::<Live>().clone();
        let grabs = [Grab::Line, Grab::Turn].into_iter().chain((0..=8).filter_map(frame_part).map(Grab::Part)).collect::<Vec<_>>();
        for grab in grabs {
            let (key, index) = grab.key();
            let part = match index {
                Some(index) => craft_behavior::PartRef { key, index: Some(index) },
                None => craft_behavior::PartRef { key, index: None },
            };
            let Some(result) = cx.part_if_declared(part) else { continue };
            let (press, hover) = (result.press, result.hover_pos);
            let mods = press.mods;
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    if grab == Grab::Part(BoxPart::Inside) && press.double_pressed() && !self.default_frame {
                        self.report.apply = true;
                    }
                }
                PressPhase::DragStart { start, pos, .. } if start.button == PointerButton::Primary => {
                    let mut drag = self.begin(cx, &space, &frame, grab, start.origin);
                    self.step(cx, &space, &mut drag, pos, mods);
                    live.drag = Some(drag);
                }
                PressPhase::Drag { pos, .. } => {
                    if let Some(mut drag) = live.drag.take() {
                        self.step(cx, &space, &mut drag, pos, mods);
                        live.drag = Some(drag);
                    }
                }
                PressPhase::Release { pos, click: false, .. } => {
                    if let Some(drag) = live.drag.take() {
                        self.finish(cx, &space, drag, pos);
                    }
                }
                PressPhase::Cancel { .. } => {
                    if let Some(drag) = live.drag.take() {
                        self.report.view = drag.press_view;
                    }
                }
                _ => {}
            }
            if grab == Grab::Turn
                && let Some(at) = hover.or_else(|| live.drag.as_ref().filter(|drag| matches!(drag.kind, Kind::Turn { .. })).map(|_| cx.rect.center()))
                && !self.straighten
            {
                let toward = view.map_vec(frame.quadrant_diagonal(view.to_doc(at)));
                cx.cursor(PaintedCursor::turn(toward), CursorPriority::Part);
            }
        }
        self.report.hovered = (0..=8)
            .filter_map(frame_part)
            .find(|part| cx.hover_pos(craft_behavior::PartRef { key: FRAME, index: Some(part.index()) }).is_some())
            .or_else(|| cx.hover_pos(TURN).map(|_| BoxPart::Outside));
        self.run_keys(cx, &space, &frame);
        *cx.memory::<Live>() = live;
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let space = space_of(&cx.env, cx.rect);
        let live = cx.recall::<Live>();
        let frame = self.shown(cx.value(), &space.view);
        let picked =
            cx.focus_visible().then(|| cx.active_part()).flatten().filter(|part| part.key == FRAME).and_then(|part| part.index).and_then(frame_part);
        let refused = matches!(cx.validity(), crate::contract::Validity::Refuse(_));
        paint::paint(ui, cx, &paint::Scene { space: &space, frame, drag: live.drag.as_ref(), picked, refused, overlay: self });
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let space = space_of(&cx.env, cx.rect);
        let frame = self.shown(cx.value(), &space.view);
        let words = cx.words::<CanvasWords>();
        let units = cx.env.units();
        let (x, _) = space.rulers().across(Axis::Y);
        let (y, _) = space.rulers().across(Axis::X);
        let (w, h) = (frame.rect.width(), frame.rect.height());
        let angle = units.number_ungrouped(gesture::turn_degrees(&frame, false), 1..=1);
        access.value(format!("{} × {}, {angle}°", x.format_value(&units, w, 0.0), y.format_value(&units, h, 0.0)));
        let names = [
            (BoxPart::Inside, &words.crop_whole),
            (BoxPart::Corner(0), &words.crop_top_left),
            (BoxPart::Corner(1), &words.crop_top_right),
            (BoxPart::Corner(2), &words.crop_bottom_right),
            (BoxPart::Corner(3), &words.crop_bottom_left),
            (BoxPart::Edge(0), &words.crop_top_edge),
            (BoxPart::Edge(1), &words.crop_right_edge),
            (BoxPart::Edge(2), &words.crop_bottom_edge),
            (BoxPart::Edge(3), &words.crop_left_edge),
        ];
        for (part, name) in names {
            access.node(FRAME.at(part.index()), Role::BUTTON).name = Name::Text(name.as_str().to_owned());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> CropOverlayOutput {
        let picked = cx.active_part().filter(|part| part.key == FRAME).and_then(|part| part.index).and_then(frame_part);
        let Report { apply, discard, straightened, view, snapped, hovered, dragging, turning, sizing } = self.report;
        CropOverlayOutput { edit: cx.edit_response(), apply, discard, straightened, view, snapped, picked, hovered, dragging, turning, sizing }
    }
}
