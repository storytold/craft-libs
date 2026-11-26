//! [`PerspectiveWarp`]: Perspective Warp's planes on the canvas, drawn, linked, snapped and dragged by pointer and
//! keyboard. Each corner is a part in the canvas's arena; the picture below them takes the drag that draws a plane.

use craft_behavior::kurbo::{Point, Vec2 as DocVec};
use craft_behavior::{
    CancelReason, CanvasView, CursorIcon, Granularity, HitLevel, HitPriority, HitShape, Intent, Metrics, NavShape, PartKey, PointerButton,
    PressPhase, Roving, Sense,
};
use craft_ui::canvas::{CanvasSpace, NUDGE, NUDGE_FAR, canvas_space};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Numeric, Outcome, PaintUi, PartsCx,
    Pointer as _, Props as _, Remember as _, Role, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};

use super::{PLANE_MINIMUM, PerspectiveEdit, PerspectiveSession, Plane, PlaneCorner, PlaneMode, linked_corners};
use crate::ui::warp::pin::point_text;

craft_ui::words! {
    /// The accessible words of [`PerspectiveWarp`].
    pub struct PerspectiveWarpWords {
        corner: template "perspectiveWarp.corner" = "Plane {plane}, corner {corner}";
        linked: "perspectiveWarp.linked" = "linked";
        position: template "perspectiveWarp.position" = "{x}, {y}";
    }
}

/// The part of one corner: `CORNER.at(plane * 4 + corner)`.
pub(super) const CORNER: PartKey = PartKey::new("corner");
/// The part of the picture below the corners: a drag on it in Layout draws a plane.
const PICTURE: PartKey = PartKey::new("picture");

/// The commands of Perspective Warp (11). They are app commands: the app runs them with
/// [`PerspectiveCommands::apply`] while the session is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui::contract::CommandSet)]
#[command(prefix = "perspectiveWarp")]
#[non_exhaustive]
pub enum PerspectiveCommands {
    /// Add a plane over the middle half of the layer (Layout only).
    #[command(label = "Add Plane", default = "Insert")]
    AddPlane,
}

impl PerspectiveCommands {
    /// Runs the command on `session`: `AddPlane` adds [`PerspectiveSession::inserted_plane`] as one step in Layout.
    pub fn apply(self, session: &mut PerspectiveSession) {
        match self {
            PerspectiveCommands::AddPlane => {
                if session.mode() != PlaneMode::Layout || session.dragging() {
                    return;
                }
                let mut grown = session.planes().to_vec();
                grown.push(session.inserted_plane());
                let edit = PerspectiveEdit::replace(session.planes(), grown, "Add Plane");
                session.apply(edit);
            }
        }
    }
}

/// The look of the planes.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PerspectiveWarpStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// A plane's outline, its thirds and the rubber band of a new plane.
    line: StyleColor,
    /// A corner's fill.
    corner_fill: StyleColor,
    /// A corner's diameter.
    corner: f32,
    /// How far from a corner a press takes it, and the distance within which a dragged corner snaps to another
    /// plane's corner, in points.
    reach: f32,
}

/// The rubber band of a plane being drawn: where it started and the pointer, kept for the paint step.
#[derive(Clone, Copy, Debug, Default)]
struct Band(Option<(Point, Pos2)>);

/// What the planes report beyond the edit of the plane list.
#[derive(Clone, Debug, Default)]
struct Report {
    apply: bool,
    discard: bool,
    hovered: Option<PlaneCorner>,
}

/// Perspective Warp's planes over the picture (the warped picture is drawn by the transform preview through the
/// app's map).
///
/// Pointer (Photoshop):
/// - Layout: a drag on the picture away from corners draws a plane (a rubber band), added on release when at least
///   [`PLANE_MINIMUM`] each way, its corners snapped to other planes' corners within reach; a drag on a corner moves
///   it and every corner linked to it, snapping to another plane's corner, which links them.
/// - Warp: a drag on a corner moves its target and the linked corners'; the picture follows; the layout is kept.
/// - A drag holds its corners by value: when the session's [generation](PerspectiveSession::generation) changes during
///   it, the drag ends (cancelled).
/// - Escape during a drag cancels it.
///
/// Keys (one Tab stop): Page Down and Page Up pick the next and previous corner (plane by plane, clockwise); arrows move
/// the picked corner and its linked corners (Shift by the large step); in Layout Delete or Backspace removes the
/// picked corner's plane; Enter reports [`apply`](PerspectiveWarpOutput::apply) and Escape with no drag reports
/// [`discard`](PerspectiveWarpOutput::discard). [`PerspectiveCommands::AddPlane`] adds a plane in Layout.
///
/// Each corner is a graphics object "Plane 2, corner 3" valued "x, y" in the canvas's units, with "linked" when
/// linked. Nothing mirrors in a right-to-left interface.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct PerspectiveWarp<'a> {
    #[common]
    common: Common,
    planes: Bind<'a, Vec<Plane>>,
    mode: PlaneMode,
    generation: u64,
    report: Report,
}

impl<'a> PerspectiveWarp<'a> {
    /// The planes named `label` over `planes`, in `mode`, of the session at `generation`
    /// ([`PerspectiveSession::generation`]).
    pub fn new(label: impl Into<Label>, planes: Bind<'a, Vec<Plane>>, mode: PlaneMode, generation: u64) -> Self {
        PerspectiveWarp { common: Common::new(), planes, mode, generation, report: Report::default() }.label(label)
    }

    fn warp(&self) -> bool {
        self.mode == PlaneMode::Warp
    }
}

/// What a press does.
#[derive(Clone, Debug)]
enum Kind {
    /// Moves these linked corners, keeping the pointer's offset from the corner.
    Corner { group: Vec<PlaneCorner>, offset: DocVec },
    /// Draws a plane from this point (snapped) to the pointer.
    Plane { start: Point },
}

/// A press the planes own, kept for its length.
#[derive(Clone, Debug)]
struct Drag {
    kind: Kind,
    /// The session's generation at the press.
    generation: u64,
    /// The planes the movement is added to: the session's at the press.
    base: Vec<Plane>,
    began: bool,
}

impl Default for Drag {
    fn default() -> Self {
        Drag { kind: Kind::Plane { start: Point::ZERO }, generation: 0, base: Vec::new(), began: false }
    }
}

/// The part index of a corner: planes in order, four corners each.
fn index_of(corner: PlaneCorner) -> u64 {
    (corner.plane * 4 + corner.corner) as u64
}

fn corner_of(index: u64) -> PlaneCorner {
    PlaneCorner::new((index / 4) as usize, (index % 4) as usize)
}

/// The corners a mode shows and drags: the sources in Layout, the targets in Warp.
fn corners(plane: &Plane, warp: bool) -> &[Point; 4] {
    if warp { &plane.target } else { &plane.source }
}

/// Moves `group` to `at`: the target, and in Layout the source too.
fn place(planes: &mut [Plane], group: &[PlaneCorner], at: Point, layout: bool) {
    for c in group {
        if let Some(plane) = planes.get_mut(c.plane) {
            plane.target[c.corner] = at;
            if layout {
                plane.source[c.corner] = at;
            }
        }
    }
}

/// The corners linked to `corner`, itself included.
fn group_of(planes: &[Plane], corner: PlaneCorner) -> Vec<PlaneCorner> {
    linked_corners(planes).into_iter().find(|g| g.contains(&corner)).unwrap_or_else(|| vec![corner])
}

/// `at`, or the source corner of another plane nearest it and within `reach` screen points; the corners in `skip` are
/// not offered.
fn snap(view: &CanvasView, planes: &[Plane], at: Point, reach: f32, skip: &[PlaneCorner]) -> Point {
    let from = view.to_screen(at);
    planes
        .iter()
        .enumerate()
        .flat_map(|(plane, pl)| pl.source.iter().enumerate().map(move |(corner, &p)| (PlaneCorner::new(plane, corner), p)))
        .filter(|(c, _)| !skip.contains(c))
        .map(|(_, p)| (p, view.to_screen(p).distance(from)))
        .filter(|&(_, d)| d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(at, |(p, _)| p)
}

impl PerspectiveWarp<'_> {
    /// A press on corner `corner`: its linked corners move with it.
    fn press_corner(&self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, planes: &[Plane], corner: PlaneCorner, origin: Pos2) {
        let at = corners(&planes[corner.plane], self.warp())[corner.corner];
        let group = group_of(planes, corner);
        let offset = at - space.view.to_doc(origin);
        *cx.press_slot::<Drag>() =
            Some(Drag { kind: Kind::Corner { group, offset }, generation: self.generation, base: planes.to_vec(), began: false });
    }

    /// A press on the picture in Layout: a plane is drawn from the snapped press point.
    fn press_picture(&self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, planes: &[Plane], origin: Pos2) {
        let reach = cx.style.reach;
        let start = snap(&space.view, planes, space.view.to_doc(origin), reach, &[]);
        *cx.press_slot::<Drag>() = Some(Drag { kind: Kind::Plane { start }, generation: self.generation, base: planes.to_vec(), began: false });
    }

    /// Follows or ends the drag of a press.
    fn track(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, phase: PressPhase) {
        let reach = cx.style.reach;
        let layout = !self.warp();
        match phase {
            PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                let Some(mut drag) = cx.press_slot::<Drag>().clone() else { return };
                match &drag.kind {
                    Kind::Plane { start } => cx.memory::<Band>().0 = Some((*start, pos)),
                    Kind::Corner { group, offset } => {
                        if !drag.began {
                            cx.begin();
                            drag.began = true;
                        }
                        let mut at = space.view.to_doc(pos) + *offset;
                        if layout {
                            at = snap(&space.view, &drag.base, at, reach, group);
                        }
                        let mut moved = drag.base.clone();
                        place(&mut moved, group, at, layout);
                        cx.propose(moved);
                    }
                }
                *cx.press_slot::<Drag>() = Some(drag);
            }
            PressPhase::Release { pos, .. } => {
                let Some(drag) = cx.press_slot::<Drag>().take() else { return };
                match drag.kind {
                    Kind::Plane { start } => {
                        cx.memory::<Band>().0 = None;
                        let end = snap(&space.view, &drag.base, space.view.to_doc(pos), reach, &[]);
                        let (x0, y0, x1, y1) = (start.x.min(end.x), start.y.min(end.y), start.x.max(end.x), start.y.max(end.y));
                        if x1 - x0 >= PLANE_MINIMUM && y1 - y0 >= PLANE_MINIMUM {
                            let corners = [Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)];
                            let mut grown = drag.base.clone();
                            grown.push(Plane::identity(corners.map(|c| snap(&space.view, &drag.base, c, reach, &[]))));
                            cx.set(grown);
                        }
                    }
                    Kind::Corner { .. } => {
                        if drag.began {
                            cx.commit();
                        }
                    }
                }
            }
            PressPhase::Cancel { .. } => {
                cx.memory::<Band>().0 = None;
                if cx.press_slot::<Drag>().take().is_some_and(|drag| drag.began) {
                    cx.cancel(CancelReason::Escape);
                }
            }
            _ => {}
        }
    }

    fn run_pointer(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, planes: &[Plane]) {
        if let Some(drag) = cx.press_slot::<Drag>().clone()
            && drag.generation != self.generation
        {
            *cx.press_slot::<Drag>() = None;
            cx.memory::<Band>().0 = None;
            if drag.began {
                cx.cancel(CancelReason::HostChanged);
            }
            return;
        }
        for index in 0..planes.len() as u64 * 4 {
            let Some(result) = cx.part_if_declared(CORNER.at(index)) else { continue };
            let (press, hovered) = (result.press, result.hovered);
            if hovered {
                self.report.hovered = Some(corner_of(index));
            }
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    self.press_corner(cx, space, planes, corner_of(index), start.origin);
                }
                phase => self.track(cx, space, phase),
            }
        }
        let Some(result) = cx.part_if_declared(PICTURE) else { return };
        let press = result.press;
        match press.phase {
            PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                self.press_picture(cx, space, planes, start.origin);
            }
            phase => self.track(cx, space, phase),
        }
    }

    /// The keys of the focused corner: the arrows move it and its linked corners, Delete removes its plane in Layout,
    /// Enter applies and Escape discards.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, planes: &[Plane]) {
        if cx.intents().take_exact(&Intent::Submit) {
            self.report.apply = true;
        }
        if cx.intents().take_exact(&Intent::Cancel) {
            self.report.discard = true;
        }
        let focused = cx.active_part().filter(|part| part.key == CORNER).and_then(|part| part.index).map(corner_of);
        let Some(corner) = focused.filter(|c| c.plane < planes.len()) else { return };
        let layout = !self.warp();
        if let Some((direction, mods)) = cx.intents().take_nudge() {
            let step = if mods.shift { NUDGE_FAR } else { NUDGE };
            let toward = space.view.unmap_vec(direction);
            if toward.hypot() > 0.0 {
                let by = toward / toward.hypot() * step;
                let mut moved = planes.to_vec();
                for c in group_of(planes, corner) {
                    let at = corners(&moved[c.plane], !layout)[c.corner] + by;
                    place(&mut moved, &[c], at, layout);
                }
                cx.set(moved);
            }
        }
        let forward = cx.intents().take_exact(&Intent::delete(true, Granularity::Unit));
        let backward = cx.intents().take_exact(&Intent::delete(false, Granularity::Unit));
        if (forward || backward) && layout {
            let mut rest = planes.to_vec();
            rest.remove(corner.plane);
            cx.set(rest);
        }
    }
}

/// What the planes report: the edit of the plane list and what they ask the app to do.
#[derive(Debug)]
#[non_exhaustive]
pub struct PerspectiveWarpOutput {
    /// The edit of the plane list: a corner drag is one `Begin` to `Commit`; a new plane, a removal or a nudge is a
    /// `Set`.
    pub planes: EditResponse<Vec<Plane>>,
    /// Enter: the app commits ([`PerspectiveSession::result`]).
    pub apply: bool,
    /// Escape with no drag running: the app discards the session.
    pub discard: bool,
    /// The corner under the pointer.
    pub hovered: Option<PlaneCorner>,
}

impl PerspectiveWarpOutput {
    /// The edits for [`PerspectiveSession::apply`].
    pub fn edits(&self) -> Vec<PerspectiveEdit> {
        self.planes.events.iter().map(|event| PerspectiveEdit::Planes(event.clone())).collect()
    }
}

impl Outcome for PerspectiveWarpOutput {
    type Value = Vec<Plane>;

    fn response(&self) -> &Response {
        &self.planes.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.planes.story_facts()
    }
}

impl Component for PerspectiveWarp<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_moves().with_deletes().with_submits();
    type Style = PerspectiveWarpStyle;
    type Output = PerspectiveWarpOutput;

    fn style(cx: &StyleCx<'_>, _: PerspectiveWarpStyleKey) -> PerspectiveWarpStyle {
        let tokens = cx.tokens;
        let overlay = &tokens.color.overlay;
        PerspectiveWarpStyle {
            metrics: Metrics::new(),
            line: cx.color(overlay.line),
            corner_fill: cx.color(overlay.handle_fill),
            corner: tokens.size.handle.md,
            reach: tokens.size.hit.target,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, Vec<Plane>>> {
        Some(self.planes.reborrow())
    }

    /// The corners as handle-level drag parts in one roving group, plane by plane, and in Layout the picture below
    /// them.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let (Some(space), Some(planes)) = (canvas_space(&cx.env), cx.value()) else { return };
        let radius = cx.style.reach;
        let warp = self.warp();
        for (p, plane) in planes.iter().enumerate() {
            for (c, &at) in corners(plane, warp).iter().enumerate() {
                cx.part(CORNER.at(index_of(PlaneCorner::new(p, c))), HitShape::circle(space.view.to_screen(at), radius))
                    .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                    .priority(HitPriority::new(HitLevel::Handle, 0))
                    .cursor(CursorIcon::Move)
                    .focusable();
            }
        }
        if !planes.is_empty() {
            cx.roving(Roving::new(NavShape::Flat).wrap(true).page(Some(1)));
        }
        if !warp {
            cx.part(PICTURE, HitShape::from(rect))
                .sense(Sense::DRAG)
                .priority(HitPriority::new(HitLevel::Body, 0))
                .cursor(CursorIcon::Crosshair)
                .pointer_only("PerspectiveCommands::AddPlane adds a plane over the picture");
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let planes = cx.value().cloned().unwrap_or_default();
        self.run_pointer(cx, &space, &planes);
        self.run_keys(cx, &space, &planes);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(space), Some(planes)) = (canvas_space(&cx.env), cx.value()) else { return };
        let draw = ui.draw();
        let (style, warp) = (&cx.style, self.warp());
        for plane in planes {
            for k in 1..3 {
                let t = f64::from(k) / 3.0;
                for (a, b) in [((t, 0.0), (t, 1.0)), ((0.0, t), (1.0, t))] {
                    let line = vec![space.view.to_screen(plane.point(a.0, a.1, warp)), space.view.to_screen(plane.point(b.0, b.1, warp))];
                    draw.stroke(&PaintShape::Path(line), StrokeWidth::Hairline, style.line);
                }
            }
            let mut outline: Vec<Pos2> = corners(plane, warp).iter().map(|&c| space.view.to_screen(c)).collect();
            outline.push(outline[0]);
            draw.stroke(&PaintShape::Path(outline.clone()), StrokeWidth::Thin, style.line);
            for &at in &outline[..4] {
                let disc = PaintShape::Circle(at, style.corner / 2.0);
                draw.fill(&disc, style.corner_fill);
                draw.stroke(&disc, StrokeWidth::Thin, style.line);
            }
        }
        if let Some((start, to)) = cx.recall::<Band>().0 {
            let band = Rect::from_two_pos(space.view.to_screen(start), to);
            draw.stroke(&PaintShape::Rect(band, CornerRadius::ZERO), StrokeWidth::Thin, style.line);
        }
    }

    /// One graphics object per corner, named by its plane and place and valued by its position.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let (Some(space), Some(planes)) = (canvas_space(&cx.env), cx.value()) else { return };
        let words = cx.words::<PerspectiveWarpWords>();
        let units = cx.env.units();
        let warp = self.warp();
        let groups = linked_corners(planes);
        for (p, plane) in planes.iter().enumerate() {
            for (c, &at) in corners(plane, warp).iter().enumerate() {
                let here = PlaneCorner::new(p, c);
                let name = words.corner.fill(&[("plane", &(p + 1).to_string()), ("corner", &(c + 1).to_string())]);
                let mut value = point_text(&space, &units, &words.position, at);
                if groups.iter().any(|g| g.len() > 1 && g.contains(&here)) {
                    value = format!("{value}, {}", words.linked.as_str());
                }
                let node = access.node(CORNER.at(index_of(here)), Role::GRAPHICS_OBJECT);
                node.name = Name::Text(name);
                node.numeric = Some(Numeric::empty(0.0..=0.0).text(value));
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> PerspectiveWarpOutput {
        let Report { apply, discard, hovered } = self.report;
        PerspectiveWarpOutput { planes: cx.edit_response(), apply, discard, hovered }
    }
}
