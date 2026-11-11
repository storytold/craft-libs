//! `RotateView`: the compass of a turned view (a canvas control), and [`ViewTurn`], the maths of a Rotate View drag.
//!
//! The compass shows the document's up as the view draws it. Dragging it, or moving it with the arrow keys, turns the
//! view; Shift snaps to [`RotateView::SNAP_DEG`]; a double click resets the angle to 0.

use super::words::CanvasWords;
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Role, WordSetCx as _,
};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::runtime::env::{Env, Label};
use crate::runtime::focus::KeyUse;
use crate::style::{ComponentStyle, StyleColor, StyleCx};
use craft_behavior::geometry::crop::{normalized_angle, snap_angle};
use craft_behavior::{CancelReason, CanvasView, CursorIcon, HitLevel, HitPriority, HitShape, Metrics, PartKey, PressPhase, Sense};
use egui::{Align2, Pos2, Rect, Response, Ui, Vec2};

pub(super) const RIM: PartKey = PartKey::new("rim");

/// The compass shows while the view is turned by more than this many degrees.
const VISIBLE_DEG: f64 = 0.05;

/// Share of the compass' radius the needle reaches.
const NEEDLE_REACH: f32 = 0.64;

/// The style of the compass.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct RotateViewStyle {
    #[style(metrics)]
    metrics: Metrics,
    side: f32,
    face: StyleColor,
    rim: StyleColor,
    needle: StyleColor,
    focus: StyleColor,
}

/// What the compass did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct RotateViewOutput {
    /// The angle's edits and the compass' response.
    pub rotation: EditResponse<f64>,
    /// A double click set the angle back to 0.
    pub reset: bool,
}

impl Outcome for RotateViewOutput {
    type Value = f64;

    fn response(&self) -> &Response {
        &self.rotation.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.rotation.story_facts()
    }
}

/// The compass of a turned view: a dial whose needle points where the document's up lies on screen. A canvas control:
/// show it with `show_at` at [`compass_rect`](Self::compass_rect) inside the canvas body, only while
/// [`visible`](Self::visible).
#[derive(crate::contract::Component)]
#[component(canvas_control)]
pub struct RotateView<'a> {
    #[common]
    common: Common,
    rotation: Bind<'a, f64>,
    flip_x: bool,
    reset: bool,
}

impl<'a> RotateView<'a> {
    /// Degrees Shift snaps to.
    pub const SNAP_DEG: f64 = 15.0;

    /// A compass named `label` over the view's `rotation`, in degrees clockwise. `flip_x` is the view's mirror, which
    /// turns the dial the other way on screen.
    pub fn new(label: impl Into<Label>, rotation: Bind<'a, f64>, flip_x: bool) -> Self {
        RotateView { common: Common::new(), rotation, flip_x, reset: false }.label(label)
    }

    /// Whether a view turned by `rotation` degrees shows the compass.
    pub fn visible(rotation: f64) -> bool {
        rotation.is_finite() && rotation.abs() > VISIBLE_DEG
    }

    /// The compass' rect in the top corner of `canvas` where the reading direction ends, at the size the theme gives
    /// it, inset from the canvas edges by the stack gap.
    pub fn compass_rect(ui: &Ui, canvas: Rect) -> Rect {
        let env = Env::of(ui);
        let corner = if env.direction.is_rtl() { Align2::LEFT_TOP } else { Align2::RIGHT_TOP };
        corner.align_size_within_rect(Vec2::splat(env.tokens.size.canvas.compass), canvas.shrink(env.tokens.space.gap.stack))
    }
}

/// The angle of the pointer about `center` in degrees clockwise from screen up; 0 at the center.
fn pointer_deg(pointer: Pos2, center: Pos2) -> f64 {
    let d = pointer - center;
    if !(d.x.is_finite() && d.y.is_finite()) || d.length_sq() < 1e-8 {
        return 0.0;
    }
    f64::from(d.x).atan2(f64::from(-d.y)).to_degrees()
}

impl Component for RotateView<'_> {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::CANVAS;
    type Style = RotateViewStyle;
    type Output = RotateViewOutput;

    fn style(cx: &StyleCx<'_>, _: RotateViewStyleKey) -> RotateViewStyle {
        let (colors, side) = (&cx.tokens.color, cx.tokens.size.canvas.compass);
        RotateViewStyle {
            metrics: Metrics::new().min_size(Vec2::splat(side)),
            side,
            face: cx.color(colors.overlay.handle_fill),
            rim: cx.color(colors.overlay.handle_stroke),
            needle: cx.color(colors.overlay.line),
            focus: cx.color(colors.line.focus),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.side)
    }

    fn bind(&mut self) -> Option<BindMut<'_, f64>> {
        Some(self.rotation.reborrow())
    }

    /// The rim: one part that drags and takes the keyboard.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(RIM, HitShape::circle(rect.center(), rect.width() / 2.0))
            .sense(Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS)
            .priority(HitPriority::new(HitLevel::Handle, 0))
            .cursor(CursorIcon::Grab)
            .focusable();
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let center = cx.rect.center();
        let shown = cx.value().copied().unwrap_or(0.0);
        let flip = self.flip_x;
        let angle = |pointer: Pos2, snap: bool| {
            let raw = if flip { -pointer_deg(pointer, center) } else { pointer_deg(pointer, center) };
            if snap { snap_angle(raw, Self::SNAP_DEG) } else { normalized_angle(raw) }
        };
        let press = cx.part(RIM).press;
        let snap = press.mods.shift;
        if press.double_pressed() {
            self.reset = true;
            cx.set(0.0);
            return;
        }
        match press.phase {
            PressPhase::DragStart { pos, .. } => {
                cx.begin();
                cx.propose(angle(pos, snap));
            }
            PressPhase::Drag { pos, .. } => cx.propose(angle(pos, snap)),
            PressPhase::Release { click: false, .. } => cx.commit(),
            PressPhase::Cancel { .. } => cx.cancel(CancelReason::Escape),
            _ => {}
        }
        if cx.active_part().is_some_and(|part| part.key == RIM)
            && let Some((direction, mods)) = cx.intents().take_nudge()
        {
            let step = if mods.shift { Self::SNAP_DEG } else { 1.0 };
            let sign = if direction.x > 0.0 || direction.y < 0.0 { 1.0 } else { -1.0 };
            let next = normalized_angle(shown + sign * step * if flip { -1.0 } else { 1.0 });
            cx.set(next);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let angle = cx.value().copied().unwrap_or(0.0);
        let angle = if self.flip_x { -angle } else { angle }.to_radians() as f32;
        let (center, radius) = (cx.rect.center(), cx.rect.width() / 2.0);
        let draw = ui.draw();
        let disc = PaintShape::Circle(center, radius);
        draw.fill(&disc, cx.style.face);
        draw.stroke(&disc, StrokeWidth::Thin, cx.style.rim);
        if cx.focus_visible() {
            draw.stroke(&disc, StrokeWidth::Medium, cx.style.focus);
        }
        let tip = center + Vec2::new(angle.sin(), -angle.cos()) * (radius * NEEDLE_REACH);
        draw.stroke(&PaintShape::Path(vec![center, tip]), StrokeWidth::Medium, cx.style.needle);
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        access.node(RIM, Role::BUTTON).name = Name::Text(cx.words::<CanvasWords>().rotate_compass.as_str().to_owned());
    }

    fn finish(self, cx: Cx<'_, Self>) -> RotateViewOutput {
        RotateViewOutput { rotation: cx.edit_response(), reset: self.reset }
    }
}

/// The Rotate View drag: the image turns about the view's centre following the pointer (clockwise increases the angle;
/// mirrored in a flipped view so it stays under the pointer). Shift snaps to [`RotateView::SNAP_DEG`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewTurn {
    pivot: Pos2,
    start_pointer: f64,
    start_rotation: f64,
}

impl ViewTurn {
    /// A drag pressed at `screen` on `view`.
    pub fn begin(view: &CanvasView, screen: Pos2) -> Self {
        let pivot = view.rect.center();
        let d = screen - pivot;
        let start_pointer = if d.length_sq() < 1e-8 { 0.0 } else { f64::from(d.y).atan2(f64::from(d.x)).to_degrees() };
        Self { pivot, start_pointer, start_rotation: view.rotation }
    }

    /// The rotation for the pointer at `pointer` (Shift: `snap`), in `(-180, 180]`.
    pub fn drag(&self, pointer: Pos2, snap: bool, flip_x: bool) -> f64 {
        let d = pointer - self.pivot;
        let now = if d.length_sq() < 1e-8 { 0.0 } else { f64::from(d.y).atan2(f64::from(d.x)).to_degrees() };
        let mut angle = self.start_rotation + Self::screen_delta(now - self.start_pointer, flip_x);
        if !angle.is_finite() {
            angle = self.start_rotation;
        }
        if snap { snap_angle(angle, RotateView::SNAP_DEG) } else { normalized_angle(angle) }
    }

    /// The change of the view's rotation for a pointer turn of `delta` screen degrees (clockwise positive). A
    /// mirrored view turns the other way on screen for the same angle, so the sign flips to keep the image under the
    /// pointer.
    pub fn screen_delta(delta: f64, flip_x: bool) -> f64 {
        let d = normalized_angle(delta);
        if flip_x { normalized_angle(-d) } else { d }
    }
}
