//! [`PerspectiveCropQuad`]: the Perspective Crop tool's quad, a canvas control. Its corners and its inside are parts in
//! the canvas's arena; the perspective grid shows while the quad can be committed.

use craft_behavior::geometry::{Homography, screen_step};
use craft_behavior::kurbo::{Point, Rect as DocRect, Vec2 as DocVec};
use craft_behavior::{
    CanvasView, CursorIcon, HitLevel, HitPriority, HitShape, Intent, Metrics, NavShape, PaintedCursor, PartKey, PartRef, PointerButton, PressPhase,
    Roving, Sense,
};
use craft_keymap::Mods;
use craft_theme::variant::Tone;
use craft_ui::canvas::{CanvasSpace, CropShield, NUDGE, NUDGE_FAR, canvas_space, paint_outside};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::{EnvView, Label};
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, DataColor, HandleLook, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};

use super::words::CropWords;

/// The quad of a perspective crop in document units, its corners in the order of the crop's top left, top right,
/// bottom right, bottom left; `None` before one is drawn.
pub type PerspectiveQuad = Option<[Point; 4]>;

/// Fewest and most grid steps along a side.
const GRID_STEPS: (usize, usize) = (2, 12);
/// The grid's target cell side, in handle sides.
const GRID_CELL: f32 = 4.0;
/// How far the grid lines fade towards the canvas colour.
const GRID_FADE: f32 = 0.6;

/// The whole quad as a part: a press inside moves it.
const INSIDE: PartKey = PartKey::new("quad");
/// A corner of the quad: `CORNER.at(i)` with `i` counted clockwise from the top left.
const CORNER: PartKey = PartKey::new("corner");

const CORNERS: u64 = 4;

/// The style of the perspective crop quad.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PerspectiveCropStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The corner squares.
    handle: HandleLook,
    /// The outline of a quad that can be committed.
    line: StyleColor,
    /// Under the outline.
    halo: StyleColor,
    /// The outline of a quad that cannot be committed.
    invalid: StyleColor,
    /// A perspective grid line: the white of the handles on the document, faded towards the canvas.
    grid: StyleColor,
    /// Reach of a corner in screen points.
    reach: f32,
    /// Screen points of the longest side per grid step.
    grid_step: f32,
    /// The canvas colour the shield may match.
    canvas: StyleColor,
}

/// Screen points of the longest side per grid step: the one scale of the quad's grid and the tool's preview grid.
pub(super) fn grid_step(cx: &StyleCx<'_>) -> f32 {
    cx.tokens.size.handle.md * GRID_CELL
}

/// A perspective grid line: the white of the handles on the document, faded towards the canvas.
pub(super) fn grid_color(cx: &StyleCx<'_>) -> StyleColor {
    let tokens = cx.tokens;
    craft_theme::Mix::mix(cx.color(tokens.color.overlay.handle_fill), cx.color(tokens.color.surface.canvas), GRID_FADE)
}

/// What a drag does to the quad.
#[derive(Clone, Copy, Debug)]
enum Grab {
    /// Moves corner `index`; `offset` is the press's distance from it.
    Corner { index: usize, offset: DocVec },
    /// Moves the whole quad.
    Move,
}

/// A drag in progress, kept between passes.
#[derive(Clone, Copy, Debug)]
struct Drag {
    grab: Grab,
    /// The quad at the press.
    at_press: [Point; 4],
    /// The press in document space.
    from: Point,
}

/// What a pass leaves for the next.
#[derive(Clone, Default)]
struct Live {
    drag: Option<Drag>,
}

/// What the quad reports beyond the edit.
#[derive(Clone, Debug, Default)]
struct Report {
    apply: bool,
    discard: bool,
    /// A corner or the whole quad is being dragged: Auto Adjust Opacity lightens the shield.
    dragging: bool,
}

/// The Perspective Crop tool's quad over the host's document, as Photoshop's Perspective Crop: the user marks four
/// corners on a skewed object, and on commit the host maps the quad onto an upright rectangle. The control draws the
/// quad and its perspective grid and takes the gestures on an existing quad; [`PerspectiveCropTool`](super::PerspectiveCropTool)
/// draws a new one. The mapping, the W x H fields and the document step stay in the app.
///
/// - The corner nearest the pointer within its reach moves alone, keeping the press offset; inside moves the quad. A
///   press elsewhere reaches the tool.
/// - Shift on a corner drag keeps the axis the pointer moved less along; Alt on a corner of an upright quad moves the
///   neighbouring corners too, so it stays a rectangle.
/// - With a `shield` the document outside a valid quad is dimmed, under the grid.
/// - With `bounds` a drag or an arrow key stops each corner it moves at the bounds' edge; a whole-quad move stops when
///   its farthest corner reaches the edge, so the quad keeps its shape. The shield then dims the bounds outside the
///   quad.
/// - The corners stay where the user puts them: a quad wound counter-clockwise (a box flipped on one axis) applies
///   mirrored, a box turned half way round applies upside down.
/// - A quad that is not convex draws in the invalid colour without the grid and cannot be committed.
/// - The quad and its corners are one Tab stop. Page Down and Page Up pick the whole quad or a corner; the arrows move
///   the picked corner (1 document unit, Shift 10), or the whole quad when it is picked. Enter applies a valid quad.
///   Esc cancels a drag, else discards.
/// - The quad holds the canvas's session (`KeyUse::session`): with the canvas focused or nothing focused, Enter, Esc
///   and the arrows reach it before the tool, and the arrows move the whole quad.
/// - Nothing mirrors in a right-to-left interface: the quad is the document's.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct PerspectiveCropQuad<'a> {
    #[common]
    common: Common,
    quad: Bind<'a, PerspectiveQuad>,
    /// Whether a valid quad shows its perspective grid (Show Grid).
    #[prop]
    show_grid: bool,
    /// The shield painted over the document outside a valid quad; `None` paints none.
    #[prop]
    shield: Option<CropShield>,
    /// The rectangle every corner stays inside while dragged or moved by keys, in document units; the shield covers it
    /// outside the quad. `None` leaves the corners unbounded and shields the whole document.
    #[prop]
    bounds: Option<DocRect>,
    report: Report,
}

impl<'a> PerspectiveCropQuad<'a> {
    /// A quad named `label` over `quad`, in document units.
    pub fn new(label: impl Into<Label>, quad: Bind<'a, PerspectiveQuad>) -> Self {
        PerspectiveCropQuad { common: Common::new(), quad, show_grid: true, shield: None, bounds: None, report: Report::default() }.label(label)
    }
}

/// What the quad reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PerspectiveCropOutput {
    /// A drag (`Begin` on the first move, `Commit` on release; Esc cancels it) or an arrow key's `Set`.
    pub edit: EditResponse<PerspectiveQuad>,
    /// Enter while the quad is convex ([`is_valid`]).
    pub apply: bool,
    /// Esc with no drag.
    pub discard: bool,
    /// The corner the keys act on, picked by the pointer or the Page keys; `None` for the whole quad.
    pub picked: Option<usize>,
}

impl Outcome for PerspectiveCropOutput {
    type Value = PerspectiveQuad;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// How the quad maps the document: the canvas's space, or, in a canvas with no view, the shown rect at one to one from
/// its top left.
fn space_of(env: &EnvView<'_>, rect: Rect) -> CanvasSpace {
    canvas_space(env).unwrap_or_else(|| {
        let center = Point::new(f64::from(rect.width()) / 2.0, f64::from(rect.height()) / 2.0);
        CanvasSpace::new(CanvasView::new(rect, center, 1.0), 1.0)
    })
}

/// The quad to show: the value, or none while it is mixed or not finite.
fn shown(value: Option<&PerspectiveQuad>) -> Option<[Point; 4]> {
    value.copied().flatten().filter(finite)
}

/// Whether `q` is an upright rectangle in quad order: top left, top right, bottom right, bottom left.
fn upright(q: &[Point; 4]) -> bool {
    let same = |a: f64, b: f64| (a - b).abs() < 1e-6;
    same(q[0].y, q[1].y) && same(q[1].x, q[2].x) && same(q[2].y, q[3].y) && same(q[3].x, q[0].x)
}

/// The quad a drag gives with the pointer at document point `to` and `mods` held at this step; it reads only the
/// step's own modifiers, so a released key puts the quad back as the press had it.
///
/// - Shift keeps the press position's coordinate of the corner on the axis the drag moved less along.
/// - Alt on a corner of an upright quad moves the two neighbouring corners one coordinate each, so it stays a
///   rectangle. The corners stay as they lie when the corner passes the opposite one, so the box flips. A skewed
///   quad's corner moves alone.
fn dragged(drag: &Drag, to: Point, mods: Mods) -> [Point; 4] {
    match drag.grab {
        Grab::Corner { index, offset } => {
            let from = drag.at_press[index];
            let mut target = to - offset;
            if mods.shift {
                if (target.x - from.x).abs() >= (target.y - from.y).abs() {
                    target.y = from.y;
                } else {
                    target.x = from.x;
                }
            }
            let mut quad = drag.at_press;
            quad[index] = target;
            if mods.alt && upright(&drag.at_press) {
                let (next, prev) = ((index + 1) % 4, (index + 3) % 4);
                // The corner shares its row with one neighbour and its column with the other.
                let (in_row, in_column) = if index % 2 == 0 { (next, prev) } else { (prev, next) };
                quad[in_row].y = target.y;
                quad[in_column].x = target.x;
            }
            quad
        }
        Grab::Move => {
            let by = to - drag.from;
            drag.at_press.map(|p| p + by)
        }
    }
}

/// The quad `after` that `grab` made of `before`, kept inside `bounds`.
///
/// - A corner grab clamps each coordinate it moved. A coordinate nothing moved stays as it lies, even outside the
///   bounds: that corner is the caller's. So a click with no drag changes nothing, and Alt's neighbours, clamped from
///   the same coordinate as the corner, keep the rectangle.
/// - A move shifts the whole quad back by the overshoot of its farthest corner on each axis, so it keeps its shape.
/// - Nothing is pushed further out than `before` lay: the allowed range on each axis takes in what `before` covered.
fn clamped(before: &[Point; 4], after: [Point; 4], grab: Grab, bounds: Option<DocRect>) -> [Point; 4] {
    let Some(b) = bounds.map(|b| b.abs()) else { return after };
    match grab {
        Grab::Corner { .. } => {
            // `max` then `min` rather than `f64::clamp`, which panics on reversed or NaN limits.
            let keep = |was: f64, to: f64, lo: f64, hi: f64| to.max(lo.min(was)).min(hi.max(was));
            std::array::from_fn(|i| Point::new(keep(before[i].x, after[i].x, b.x0, b.x1), keep(before[i].y, after[i].y, b.y0, b.y1)))
        }
        Grab::Move => {
            let span = |q: &[Point; 4], axis: fn(&Point) -> f64| {
                q.iter().map(axis).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)))
            };
            let x = kept_in(span(before, |p| p.x), span(&after, |p| p.x), b.x0, b.x1);
            let y = kept_in(span(before, |p| p.y), span(&after, |p| p.y), b.y0, b.y1);
            after.map(|p| p + DocVec::new(x, y))
        }
    }
}

/// The shift along one axis that brings the span `after` (min, max) inside `lo..hi`, widened to the span `before`
/// covered: the max side is pulled back first, then the min side while the span fits.
fn kept_in(before: (f64, f64), after: (f64, f64), lo: f64, hi: f64) -> f64 {
    let (lo, hi) = (lo.min(before.0), hi.max(before.1));
    let mut shift = (hi - after.1).min(0.0);
    if after.1 - after.0 <= hi - lo {
        shift += (lo - (after.0 + shift)).max(0.0);
    }
    shift
}

impl PerspectiveCropQuad<'_> {
    /// Starts the drag of the part pressed at screen point `origin`.
    fn begin(cx: &mut Cx<'_, Self>, view: &CanvasView, quad: [Point; 4], corner: Option<usize>, origin: Pos2) -> Drag {
        let from = view.to_doc(origin);
        let grab = match corner {
            Some(index) => Grab::Corner { index, offset: from - quad[index] },
            None => Grab::Move,
        };
        cx.begin();
        Drag { grab, at_press: quad, from }
    }

    /// Follows the pointer at screen point `pos`, inside `bounds`.
    fn step(cx: &mut Cx<'_, Self>, view: &CanvasView, drag: &Drag, pos: Pos2, mods: Mods, bounds: Option<DocRect>) {
        let next = clamped(&drag.at_press, dragged(drag, view.to_doc(pos), mods), drag.grab, bounds);
        if finite(&next) {
            cx.propose(Some(next));
        }
    }

    /// The keys: Enter, Escape and the arrows, on the focused part or, with no part focused, the whole quad.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, view: &CanvasView, quad: [Point; 4]) {
        let corner = cx.active_part().filter(|part| part.key == CORNER).and_then(|part| part.index);
        if cx.intents().take(Intent::Submit) {
            self.report.apply = is_valid(&quad);
        }
        if cx.intents().take(Intent::Cancel) {
            self.report.discard = true;
        }
        let Some((direction, mods)) = cx.intents().take_nudge() else { return };
        let distance = if mods.shift { NUDGE_FAR } else { NUDGE };
        let by = screen_step(view, DocVec::new(f64::from(direction.x), f64::from(direction.y)) * distance);
        let (next, grab) = match corner {
            Some(index) => {
                let index = index as usize;
                let mut moved = quad;
                moved[index] += by;
                (moved, Grab::Corner { index, offset: DocVec::ZERO })
            }
            None => (quad.map(|p| p + by), Grab::Move),
        };
        let next = clamped(&quad, next, grab, self.bounds);
        // A nudge into the bounds' edge leaves the quad as it is and sets nothing.
        if finite(&next) && next != quad {
            cx.set(Some(next));
        }
    }
}

impl Component for PerspectiveCropQuad<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::CANVAS.with_session();
    type Style = PerspectiveCropStyle;
    type Output = PerspectiveCropOutput;

    fn style(cx: &StyleCx<'_>, _: PerspectiveCropQuadStyleKey) -> PerspectiveCropStyle {
        let tokens = cx.tokens;
        let handle = cx.handle();
        let (canvas, line) = (cx.color(tokens.color.surface.canvas), cx.color(tokens.color.overlay.line));
        PerspectiveCropStyle {
            metrics: Metrics::new().min_size(Vec2::splat(handle.side)),
            handle,
            line,
            halo: cx.color(tokens.color.overlay.halo),
            invalid: cx.tone(Tone::Bad),
            grid: grid_color(cx),
            reach: tokens.size.hit.target,
            grid_step: grid_step(cx),
            canvas,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.reach)
    }

    fn bind(&mut self) -> Option<BindMut<'_, PerspectiveQuad>> {
        Some(self.quad.reborrow())
    }

    /// The quad and its four corners as one roving group, the whole quad first.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let space = space_of(&cx.env, rect);
        let Some(quad) = shown(cx.value()) else { return };
        let screen = quad.map(|p| space.view.to_screen(p));
        cx.part(INSIDE, HitShape::Polygon(screen.to_vec()))
            .sense(Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS)
            .priority(HitPriority::BODY)
            .cursor(CursorIcon::Move)
            .focusable()
            .selected();
        for (at, corner) in screen.into_iter().enumerate() {
            cx.part(CORNER.at(at as u64), HitShape::circle(corner, cx.style.reach))
                .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .cursor(PaintedCursor::arrow(false))
                .focusable();
        }
        cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let space = space_of(&cx.env, cx.rect);
        let Some(quad) = shown(cx.value()) else { return };
        let view = space.view;
        let bounds = self.bounds;
        let mut live = cx.memory::<Live>().clone();
        let grabs = std::iter::once((PartRef::from(INSIDE), None)).chain((0..CORNERS).map(|at| (CORNER.at(at), Some(at as usize))));
        for (part, corner) in grabs {
            let Some(result) = cx.part_if_declared(part) else { continue };
            let press = result.press;
            // Only this pass's own modifiers apply: a second pass of the frame reads the same press state.
            let mods = press.mods;
            match press.phase {
                PressPhase::DragStart { start, pos, .. } if start.button == PointerButton::Primary => {
                    let drag = Self::begin(cx, &view, quad, corner, start.origin);
                    Self::step(cx, &view, &drag, pos, mods, bounds);
                    live.drag = Some(drag);
                }
                PressPhase::Drag { pos, .. } => {
                    if let Some(drag) = live.drag {
                        Self::step(cx, &view, &drag, pos, mods, bounds);
                    }
                }
                PressPhase::Release { start, pos, click: false, .. } => {
                    // A press that moved and ended before a pass saw it start still drags from where it was pressed.
                    let drag = match live.drag.take() {
                        Some(drag) => Some(drag),
                        None if start.button == PointerButton::Primary => Some(Self::begin(cx, &view, quad, corner, start.origin)),
                        None => None,
                    };
                    if let Some(drag) = drag {
                        Self::step(cx, &view, &drag, pos, mods, bounds);
                        cx.commit();
                    }
                }
                PressPhase::Cancel { .. } => live.drag = None,
                _ => {}
            }
        }
        self.report.dragging = live.drag.is_some();
        self.run_keys(cx, &view, quad);
        *cx.memory::<Live>() = live;
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(quad) = shown(cx.value()) else { return };
        let space = space_of(&cx.env, cx.rect);
        let view = space.view;
        let valid = is_valid(&quad);
        let style = &cx.style;
        let draw = ui.draw();
        let screen: Vec<Pos2> = quad.iter().map(|p| view.to_screen(*p)).collect();
        // The shield dims the bounds, or the whole document, outside a quad that can be committed, before the grid.
        if valid && let (Some(shield), Some(document)) = (&self.shield, space.document()) {
            let area = self.bounds.unwrap_or_else(|| DocRect::new(0.0, 0.0, document.width, document.height));
            let canvas = [Point::new(area.x0, area.y0), Point::new(area.x1, area.y0), Point::new(area.x1, area.y1), Point::new(area.x0, area.y1)]
                .map(|p| view.to_screen(p));
            if let Some(fill) = shield.fill_on(&space, style.canvas.egui(), self.report.dragging) {
                paint_outside(&draw, canvas, &screen, DataColor::new(fill));
            }
        }
        if valid && self.show_grid {
            for [a, b] in grid_lines(&view, &quad, style.grid_step) {
                draw.stroke(&PaintShape::Path(vec![a, b]), StrokeWidth::Thin, style.grid);
            }
        }
        let outline = PaintShape::Polygon(screen.clone());
        draw.stroke(&outline, StrokeWidth::Thick, style.halo);
        draw.stroke(&outline, StrokeWidth::Medium, if valid { style.line } else { style.invalid });
        let look = &style.handle;
        for (at, corner) in screen.into_iter().enumerate() {
            let square = PaintShape::Rect(Rect::from_center_size(corner, Vec2::splat(look.side)), CornerRadius::ZERO);
            draw.fill(&square, cx.resolve_part(CORNER.at(at as u64), &look.fill));
            draw.stroke(&square, StrokeWidth::Thin, look.stroke);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<CropWords>();
        access.node(INSIDE, Role::BUTTON).name = Name::Text(words.perspective_whole.as_str().to_owned());
        let corners = [&words.perspective_top_left, &words.perspective_top_right, &words.perspective_bottom_right, &words.perspective_bottom_left];
        for (at, name) in corners.into_iter().enumerate() {
            access.node(CORNER.at(at as u64), Role::BUTTON).name = Name::Text(name.as_str().to_owned());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> PerspectiveCropOutput {
        let picked = cx.active_part().filter(|part| part.key == CORNER).and_then(|part| part.index).map(|index| index as usize);
        PerspectiveCropOutput { edit: cx.edit_response(), apply: self.report.apply, discard: self.report.discard, picked }
    }
}

/// Four points in quad order: clockwise on screen about their centre, starting at the smallest x + y (the top left of
/// an upright box).
pub fn ordered(points: [Point; 4]) -> [Point; 4] {
    let c = Point::new(points.iter().map(|p| p.x).sum::<f64>() / 4.0, points.iter().map(|p| p.y).sum::<f64>() / 4.0);
    let mut v = points;
    // y grows downwards, so an increasing angle runs clockwise on screen.
    v.sort_by(|a, b| (a.y - c.y).atan2(a.x - c.x).total_cmp(&(b.y - c.y).atan2(b.x - c.x)));
    let first = (0..4).min_by(|&i, &j| (v[i].x + v[i].y).total_cmp(&(v[j].x + v[j].y))).unwrap_or(0);
    v.rotate_left(first);
    v
}

/// Whether `q` (top left, top right, bottom right, bottom left of the crop it maps onto) is a usable perspective crop
/// quad: finite, strictly convex in either winding, every corner angle above about 0.06 degrees and at least one
/// unit^2 in area. A quad wound counter-clockwise on screen (a box flipped on one axis) applies mirrored.
pub fn is_valid(q: &[Point; 4]) -> bool {
    wound_clockwise(q) || wound_clockwise(&[q[0], q[3], q[2], q[1]])
}

/// [`is_valid`] for a quad wound clockwise on screen (y down).
fn wound_clockwise(q: &[Point; 4]) -> bool {
    if !finite(q) {
        return false;
    }
    let mut area2 = 0.0;
    for i in 0..4 {
        let (a, b, c) = (q[i], q[(i + 1) % 4], q[(i + 2) % 4]);
        let (e1, e2) = (b - a, c - b);
        // sin(turn) = cross / (|e1| |e2|): positive (clockwise on screen) and not a straight angle.
        let cross = e1.cross(e2);
        if cross.is_nan() || cross <= 1e-3 * e1.hypot() * e2.hypot() {
            return false;
        }
        area2 += a.x * b.y - b.x * a.y;
    }
    area2.is_finite() && area2 >= 2.0
}

fn finite(q: &[Point; 4]) -> bool {
    q.iter().all(|p| p.x.is_finite() && p.y.is_finite())
}

/// Grid steps along a side that is about `side` screen points: one per `step`, from 2 to 12.
pub fn grid_steps(side: f32, step: f32) -> usize {
    ((side / step).round() as usize).clamp(GRID_STEPS.0, GRID_STEPS.1)
}

/// The grid of the valid quad `q` on screen: its inner lines at equal steps of the unit square mapped through the
/// quad's homography, so they converge like the object's own lines. The step count follows the longest of the first
/// three sides.
pub fn grid_lines(view: &CanvasView, q: &[Point; 4], step: f32) -> Vec<[Pos2; 2]> {
    let Some(h) = Homography::unit_to_quad(*q) else { return Vec::new() };
    let pts = q.map(|c| view.to_screen(c));
    let side = pts.iter().zip(pts.iter().skip(1)).map(|(a, b)| a.distance(*b)).fold(0.0, f32::max);
    let n = grid_steps(side, step);
    let at = |u: f64, v: f64| view.to_screen(h.apply(Point::new(u, v)));
    let mut out = Vec::with_capacity(2 * (n - 1));
    for k in 1..n {
        let t = k as f64 / n as f64;
        out.push([at(t, 0.0), at(t, 1.0)]);
        out.push([at(0.0, t), at(1.0, t)]);
    }
    out
}

#[cfg(feature = "suite")]
pub(crate) mod stories;
