//! [`CylinderWarp`]: the editor of Edit > Transform > Warp's Cylinder (#4112), the preset that wraps the contents round
//! a horizontal cylinder. The warp is seven values, not a mesh: the left and right silhouettes, the top and bottom
//! edges, the depth of the upper and lower ellipse (in units of the half axis) and how far the contents wrap round.
//! Five handles set them and the app rebuilds the mesh from them.

use craft_behavior::kurbo::{Point, Vec2 as DocVec};
use craft_behavior::{
    CancelReason, CursorIcon, HitLevel, HitPriority, HitShape, Metrics, NavShape, PartKey, PointerButton, PressPhase, PressState, Roving, Sense,
    resize_cursor,
};
use craft_ui::canvas::{CanvasSpace, NUDGE, NUDGE_FAR, canvas_space};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, HandleLook, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};

use super::WarpMesh;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The bend of both arcs the Cylinder starts with, in units of the half axis: about a quarter of the width.
const DEFAULT_DEPTH: f64 = 0.5;

/// How far the wrap handle starts along its travel from the centre: about 15 %.
const DEFAULT_WRAP_POS: f64 = 0.15;

/// The silhouettes never cross, nor do the edges: the moved one stops this far (document units) short of the other.
const MIN_SIDE: f64 = 1.0;

/// An arc is at most as deep as the half axis, either way up.
const DEPTH_LIMIT: f64 = 1.0;

/// Curve samples along one traced line of the cylinder.
const CURVE_STEPS: usize = 32;

/// A handle of the five: the part of `HANDLE.at(index)` with the handle's place in [`HANDLES`].
const HANDLE: PartKey = PartKey::new("handle");

/// The surface of the cylinder, below the handles.
const BODY: PartKey = PartKey::new("body");

/// The handles with a part each, in keyboard order.
const HANDLES: [CylinderHandle; 5] =
    [CylinderHandle::TopRight, CylinderHandle::BottomLeft, CylinderHandle::Bend, CylinderHandle::Bottom, CylinderHandle::Wrap];

craft_ui::words! {
    /// The accessible names of the cylinder editor's handles.
    pub struct CylinderWords {
        top_right: "cylinderWarp.topRight" = "Top right corner";
        bottom_left: "cylinderWarp.bottomLeft" = "Bottom left corner";
        bend: "cylinderWarp.bend" = "Bend";
        bottom: "cylinderWarp.bottom" = "Bottom edge";
        wrap: "cylinderWarp.wrap" = "Wrap";
    }
}

/// The cylinder's values: the box, the depths of its top and bottom edges and the wrap.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CylinderValues {
    /// The left edge of the box.
    pub left: f64,
    /// The bottom edge of the box.
    pub bottom: f64,
    /// The right edge of the box.
    pub right: f64,
    /// The top edge of the box.
    pub top: f64,
    /// How far the top edge curves in depth.
    pub top_depth: f64,
    /// How far the bottom edge curves in depth.
    pub bottom_depth: f64,
    /// How far the surface wraps around the cylinder.
    pub wrap: f64,
}

impl CylinderValues {
    /// The values of the cylinder the Warp menu starts with over `rect` (left, top, right, bottom): its corners on the
    /// box, both arcs half an axis deep and the wrap handle 15 % along its travel.
    pub fn for_box(rect: [f64; 4]) -> Self {
        let [left, top, right, bottom] = rect;
        Self { left, bottom, right, top, top_depth: DEFAULT_DEPTH, bottom_depth: DEFAULT_DEPTH, wrap: wrap_of_pos(DEFAULT_WRAP_POS) }
    }

    /// The values from `[left, bottom, right, top, top_depth, bottom_depth, wrap]`.
    pub fn from_array(values: [f64; 7]) -> Self {
        let [left, bottom, right, top, top_depth, bottom_depth, wrap] = values;
        Self { left, bottom, right, top, top_depth, bottom_depth, wrap }
    }

    /// The values as `[left, bottom, right, top, top_depth, bottom_depth, wrap]`.
    pub fn to_array(self) -> [f64; 7] {
        [self.left, self.bottom, self.right, self.top, self.top_depth, self.bottom_depth, self.wrap]
    }

    /// Where the five handles are, in [`HANDLES`] order: the top-right and bottom-left corners, the middle of each arc
    /// (where the mesh puts its front) and the wrap handle, on the centre line midway between the arcs' fronts.
    fn handle_points(&self) -> [Point; 5] {
        let a = (self.right - self.left) / 2.0;
        let cx = (self.left + self.right) / 2.0;
        let (top_front, bottom_front) = (self.top + self.top_depth * a, self.bottom + self.bottom_depth * a);
        [
            Point::new(self.right, self.top),
            Point::new(self.left, self.bottom),
            Point::new(cx, top_front),
            Point::new(cx, bottom_front),
            Point::new(cx + pos_of_wrap(self.wrap) * a, (top_front + bottom_front) / 2.0),
        ]
    }

    /// The values after `handle` is dragged by `by` from the press, where they were `self`. A corner takes its
    /// silhouette and its edge along (the opposite corner stays); Bend moves both arcs' depth by the same step; Bottom
    /// moves the lower edge; Wrap follows the pointer along its travel; the surface moves the whole cylinder. The
    /// silhouettes and the edges keep their order (a moved one stops 1 unit short of the other) and a depth stays
    /// within the half axis, or within its own reach when it already lies past that (a cylinder read from a file). A
    /// drag by nothing changes nothing.
    fn moved(self, handle: CylinderHandle, by: DocVec) -> Self {
        let mut v = self;
        let a = ((self.right - self.left) / 2.0).max(MIN_SIDE / 2.0);
        match handle {
            CylinderHandle::TopRight => {
                v.right += by.x;
                v.top += by.y;
            }
            CylinderHandle::BottomLeft => {
                v.left += by.x;
                v.bottom += by.y;
            }
            CylinderHandle::Bend => {
                let step = by.y / a;
                v.top_depth = depth_dragged(self.top_depth, step);
                v.bottom_depth = depth_dragged(self.bottom_depth, step);
            }
            CylinderHandle::Bottom => v.bottom += by.y,
            CylinderHandle::Wrap => v.wrap = wrap_of_pos(pos_of_wrap(self.wrap) + by.x / a),
            CylinderHandle::Surface => {
                v.left += by.x;
                v.right += by.x;
                v.bottom += by.y;
                v.top += by.y;
                // A move keeps the shape as it is, however narrow.
                return v;
            }
        }
        if v.right - v.left < MIN_SIDE {
            if handle == CylinderHandle::TopRight {
                v.right = v.left + MIN_SIDE;
            } else {
                v.left = v.right - MIN_SIDE;
            }
        }
        if v.bottom - v.top < MIN_SIDE {
            if handle == CylinderHandle::TopRight {
                v.top = v.bottom - MIN_SIDE;
            } else {
                v.bottom = v.top + MIN_SIDE;
            }
        }
        v
    }
}

/// The wrap handle's place along its travel (0 at the centre, 1 at the right silhouette) for the stored wrap `g`. At
/// `g` = 0 the model puts a sixth of the picture round the back on each side with the middle stretched most, at 1
/// almost none: the handle slides right as the middle is stretched, so its travel is the wrap reversed.
fn pos_of_wrap(g: f64) -> f64 {
    1.0 - g.clamp(0.0, 1.0)
}

/// The stored wrap for the handle's place along its travel ([`pos_of_wrap`] inverted).
fn wrap_of_pos(p: f64) -> f64 {
    1.0 - p.clamp(0.0, 1.0)
}

/// `depth0` moved by `step`, kept within the half axis, or within its own reach when it already lies past that.
fn depth_dragged(depth0: f64, step: f64) -> f64 {
    let limit = DEPTH_LIMIT.max(depth0.abs());
    (depth0 + step).clamp(-limit, limit)
}

/// A handle of the cylinder editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CylinderHandle {
    /// The top right corner.
    TopRight,
    /// The bottom left corner.
    BottomLeft,
    /// The bend of the top edge.
    Bend,
    /// The bottom edge.
    Bottom,
    /// The wrap.
    Wrap,
    /// The surface.
    Surface,
}

/// The look of the cylinder editor: its lines in `overlay.line` over `overlay.halo`, the corners as handle squares and
/// the other handles as small handle circles.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CylinderWarpStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The cylinder's outline and its grid.
    line: StyleColor,
    /// Under the lines, so they read over any picture.
    halo: StyleColor,
    /// The two corners.
    anchor: HandleLook,
    /// Bend, Bottom and Wrap.
    end: HandleLook,
    /// How far from a handle a press takes it.
    reach: f32,
}

/// A press the editor owns: the handle, where it began and the values then, which every move is applied to.
#[derive(Clone, Debug)]
struct Press {
    handle: CylinderHandle,
    origin: Point,
    values: CylinderValues,
    began: bool,
}

impl Default for Press {
    fn default() -> Self {
        Press { handle: CylinderHandle::Surface, origin: Point::ZERO, values: CylinderValues::for_box([0.0; 4]), began: false }
    }
}

#[derive(Default)]
struct Report {
    hovered: Option<CylinderHandle>,
}

/// The cylinder editor over `values`, drawn over `mesh` (the mesh the app builds from the values).
///
/// - Look: the cylinder's outline and a 3 × 2 grid (two vertical curves and the centre line across, four horizontal
///   curves) traced on the mesh; squares at the top-right and bottom-left corners, circles at the middle of each arc,
///   and one for the wrap.
/// - Pointer: a handle within the hit reach takes the press; elsewhere on the cylinder a press moves the whole of it;
///   off it nothing. One drag is one gesture (`Begin` on the first move, `Commit` on release); Esc cancels it. A press
///   that does not move changes nothing.
/// - Keyboard: one Tab stop over the five handles; Page Down and Page Up pick the next and previous, the arrows move
///   the picked handle one unit (Shift ten) as one `Set`.
/// - Each pass applies its own input only: the press phases and the nudge are read once from the frame's input, so a
///   second pass of the frame moves nothing again, and a drag proposes from the values at the press.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct CylinderWarp<'a> {
    #[common]
    common: Common,
    values: Bind<'a, CylinderValues>,
    mesh: &'a WarpMesh,
    report: Report,
}

impl<'a> CylinderWarp<'a> {
    /// The cylinder editor named `label` over `values` and `mesh`.
    pub fn new(label: impl Into<Label>, values: Bind<'a, CylinderValues>, mesh: &'a WarpMesh) -> Self {
        CylinderWarp { common: Common::new(), values, mesh, report: Report::default() }.label(label)
    }

    /// One press phase of `handle`.
    fn run_press(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, handle: CylinderHandle, press: PressState) {
        let view = &space.view;
        match press.phase {
            PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                let origin = view.to_doc(start.origin);
                let Some(values) = cx.value().copied() else { return };
                // A press off the cylinder grabs nothing.
                if handle == CylinderHandle::Surface && self.mesh.surface_at(origin).is_none() {
                    return;
                }
                *cx.press_slot::<Press>() = Some(Press { handle, origin, values, began: false });
            }
            PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. } if start.button == PointerButton::Primary => {
                let Some(mut held) = cx.press_slot::<Press>().clone() else { return };
                if !held.began {
                    cx.begin();
                    held.began = true;
                    *cx.press_slot::<Press>() = Some(held.clone());
                }
                cx.propose(held.values.moved(held.handle, view.to_doc(pos) - held.origin));
            }
            PressPhase::Release { .. } => {
                if cx.press_slot::<Press>().take().is_some_and(|held| held.began) {
                    cx.commit();
                }
            }
            PressPhase::Cancel { .. } if cx.press_slot::<Press>().take().is_some_and(|held| held.began) => {
                cx.cancel(CancelReason::Escape);
            }
            _ => {}
        }
    }

    /// The arrows move the picked handle by one unit (Shift ten) as one `Set`. The nudge is taken from the intents, so
    /// a second pass finds none.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, values: CylinderValues) {
        let focused =
            cx.active_part().filter(|part| part.key == HANDLE).and_then(|part| part.index).and_then(|index| HANDLES.get(index as usize).copied());
        let Some(handle) = focused else { return };
        if let Some((direction, mods)) = cx.intents().take_nudge() {
            let step = if mods.shift { NUDGE_FAR } else { NUDGE };
            cx.set(values.moved(handle, space.view.unmap_vec(direction) * step));
        }
    }
}

/// What the cylinder editor reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct CylinderWarpOutput {
    /// The edit of the values.
    pub values: EditResponse<CylinderValues>,
    /// The handle under the pointer.
    pub hovered: Option<CylinderHandle>,
}

impl Outcome for CylinderWarpOutput {
    type Value = CylinderValues;

    fn response(&self) -> &Response {
        &self.values.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.values.story_facts()
    }
}

impl Component for CylinderWarp<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS;
    type Style = CylinderWarpStyle;
    type Output = CylinderWarpOutput;

    fn style(cx: &StyleCx<'_>, _: CylinderWarpStyleKey) -> CylinderWarpStyle {
        let overlay = &cx.tokens.color.overlay;
        let mut end = cx.handle();
        end.side = cx.tokens.size.handle.sm;
        CylinderWarpStyle {
            metrics: Metrics::new(),
            line: cx.color(overlay.line),
            halo: cx.color(overlay.halo),
            anchor: cx.handle(),
            end,
            reach: cx.tokens.size.hit.target,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, CylinderValues>> {
        Some(self.values.reborrow())
    }

    /// The five handles in one roving group, and the surface under them.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let (Some(space), Some(values)) = (canvas_space(&cx.env), cx.value().copied()) else { return };
        let view = &space.view;
        let diagonal = DocVec::new(values.right - values.left, values.top - values.bottom);
        for (index, (handle, at)) in HANDLES.into_iter().zip(values.handle_points()).enumerate() {
            let cursor = match handle {
                CylinderHandle::TopRight | CylinderHandle::BottomLeft => resize_cursor(view, diagonal),
                CylinderHandle::Bend | CylinderHandle::Bottom => resize_cursor(view, DocVec::new(0.0, 1.0)),
                _ => resize_cursor(view, DocVec::new(1.0, 0.0)),
            };
            cx.part(HANDLE.at(index as u64), HitShape::circle(view.to_screen(at), cx.style.reach))
                .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .cursor(cursor)
                .focusable();
        }
        cx.part(BODY, HitShape::Rect(cx.content_rect))
            .sense(Sense::DRAG | Sense::TAKE_FOCUS)
            .priority(HitPriority::new(HitLevel::Body, 0))
            .keyboard_via(HANDLE);
        cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let Some(values) = cx.value().copied() else { return };
        let mut hovered = None;
        let mut results = Vec::new();
        for (index, handle) in HANDLES.into_iter().enumerate() {
            if let Some(result) = cx.part_if_declared(HANDLE.at(index as u64)) {
                if result.hovered && hovered.is_none() {
                    hovered = Some(handle);
                }
                results.push((handle, result.press));
            }
        }
        if let Some((hover_pos, press)) = cx.part_if_declared(BODY).map(|result| (cx.hover_pos(BODY), result.press)) {
            let on_surface = hover_pos.is_some_and(|at| self.mesh.surface_at(space.view.to_doc(at)).is_some());
            if on_surface {
                cx.cursor(CursorIcon::Move, craft_behavior::CursorPriority::Part);
                hovered = hovered.or(Some(CylinderHandle::Surface));
            }
            results.push((CylinderHandle::Surface, press));
        }
        for (handle, press) in results {
            self.run_press(cx, &space, handle, press);
        }
        self.run_keys(cx, &space, values);
        self.report.hovered = hovered;
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(space), Some(values)) = (canvas_space(&cx.env), cx.value().copied()) else { return };
        let (draw, style, view) = (ui.draw(), &cx.style, &space.view);
        if self.mesh.is_valid() {
            let curve =
                |at: &dyn Fn(f64) -> Point| -> Vec<Pos2> { (0..=CURVE_STEPS).map(|i| view.to_screen(at(i as f64 / CURVE_STEPS as f64))).collect() };
            let mut lines = Vec::new();
            for s in [0.0, 0.5, 1.0] {
                lines.push(curve(&|t| self.mesh.eval(s, t)));
            }
            for t in [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0] {
                lines.push(curve(&|s| self.mesh.eval(s, t)));
            }
            for line in lines {
                draw.stroke(&PaintShape::Path(line.clone()), StrokeWidth::Thick, style.halo);
                draw.stroke(&PaintShape::Path(line), StrokeWidth::Thin, style.line);
            }
        }
        for (index, (handle, at)) in HANDLES.into_iter().zip(values.handle_points()).enumerate() {
            let center = view.to_screen(at);
            let part = HANDLE.at(index as u64);
            let corner = matches!(handle, CylinderHandle::TopRight | CylinderHandle::BottomLeft);
            let look = if corner { &style.anchor } else { &style.end };
            // An edit of this pass can have changed the values since `parts` declared the handles; those draw at rest.
            let fill = if cx.part_if_declared(part).is_some() { cx.resolve_part(part, &look.fill) } else { cx.resolve(&look.fill) };
            if corner {
                let square = PaintShape::Rect(Rect::from_center_size(center, Vec2::splat(look.side)), CornerRadius::ZERO);
                draw.fill(&square, fill);
                draw.stroke(&square, StrokeWidth::Thin, look.stroke);
            } else {
                let circle = PaintShape::Circle(center, look.side / 2.0);
                draw.fill(&circle, fill);
                draw.stroke(&circle, StrokeWidth::Thin, look.stroke);
            }
        }
    }

    /// One node per handle, named by its words and valued by its position in the canvas's units.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let (Some(space), Some(values)) = (canvas_space(&cx.env), cx.value().copied()) else { return };
        let words = cx.words::<CylinderWords>();
        let (rulers, units) = (space.rulers(), cx.env.units());
        for (index, (handle, at)) in HANDLES.into_iter().zip(values.handle_points()).enumerate() {
            let name = match handle {
                CylinderHandle::TopRight => &words.top_right,
                CylinderHandle::BottomLeft => &words.bottom_left,
                CylinderHandle::Bend => &words.bend,
                CylinderHandle::Bottom => &words.bottom,
                _ => &words.wrap,
            };
            let value = format!("{}, {}", rulers.x.format_value(&units, at.x, rulers.zero.x), rulers.y.format_value(&units, at.y, rulers.zero.y));
            let node = access.node(HANDLE.at(index as u64), Role::GRAPHICS_OBJECT);
            node.name = Name::Text(name.as_str().to_owned());
            node.description = Some(value);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> CylinderWarpOutput {
        CylinderWarpOutput { values: cx.edit_response(), hovered: self.report.hovered }
    }
}
