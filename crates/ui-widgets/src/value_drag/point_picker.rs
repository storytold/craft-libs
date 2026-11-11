//! `PointPicker`: a point on a well, over an image or a checkerboard. A press on the well jumps the point there, a drag
//! on the point or the well follows the pointer, and the arrow keys and the wheel move it along one axis each. The X
//! and Y fields are `NumberField`s the caller shows beside it, bound to the same value.

use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Numeric, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, WordSetCx as _,
};
use crate::primitives::{ContentPainterCx, PaintShape};
use crate::runtime::access::Name;
use crate::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use crate::style::{ComponentStyle, FocusLook, StyleCx};
use craft_behavior::nav::{LARGE_STEPS, STEP_SHARE, StepResult, ValueAxes, ValueSteps, snap_to_step, step_value};
use craft_behavior::{
    AccessAction, Axis, HitLevel, HitPriority, Metrics, Mixed, MoveMods, NavMove, PartKey, PressPhase, PressState, Sense, ValueDrag, Wheel,
    decimals_for_step, wheel_step,
};
use craft_keymap::Mods;
use egui::{Pos2, Rect, Vec2, pos2};
use std::ops::RangeInclusive;

#[cfg(feature = "testing")]
mod stories;

/// The point: a drag on it follows the pointer.
const THUMB: PartKey = PartKey::new("thumb");

/// The accessible sub-node of the x value.
const AXIS_X: PartKey = PartKey::new("axis_x");
/// The accessible sub-node of the y value.
const AXIS_Y: PartKey = PartKey::new("axis_y");
const AXES: [PartKey; 2] = [AXIS_X, AXIS_Y];

/// The value of a picker whose point is not shown: every axis mixed.
const ALL_MIXED: [Mixed<f64>; 2] = [Mixed::Mixed, Mixed::Mixed];

craft_ui::words! {
    /// The words of point pickers.
    pub struct PointPickerWords {
        axis_x: "pointPicker.axisX" = "X";
        axis_y: "pointPicker.axisY" = "Y";
        named_axis_x: template "pointPicker.namedAxisX" = "{name} X";
        named_axis_y: template "pointPicker.namedAxisY" = "{name} Y";
        value: template "pointPicker.value" = "{x}, {y}";
    }
}

/// The style of a point picker.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PointPickerStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The length of the cross at the point.
    thumb: f32,
    /// The side of the square that grabs the point.
    hit: f32,
}

/// The point at the start of the running drag.
#[derive(Clone, Default)]
struct DragStart([f64; 2]);

/// Paints the image under the point into the plot rect through the content painter: `PointPicker::backdrop`.
pub struct Backdrop<'a>(BackdropFn<'a>);

type BackdropFn<'a> = Box<dyn FnOnce(&ContentPainterCx, Rect) + 'a>;

impl<'a> Backdrop<'a> {
    /// The backdrop that runs `paint`.
    pub fn new(paint: impl FnOnce(&ContentPainterCx, Rect) + 'a) -> Self {
        Backdrop(Box::new(paint))
    }
}

/// A point `[x, y]` picked on a well: x grows to the right and y downwards, as image coordinates do, in either
/// reading direction. Each axis can be mixed on its own: an arrow key or a wheel notch on a mixed axis starts from
/// where the point sits on it (the middle of its range) and leaves the other axis as it is.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(content_painter)]
pub struct PointPicker<'a> {
    #[common]
    common: Common,
    value: Bind<'a, [Mixed<f64>; 2]>,
    /// Each axis's range with its smaller bound first.
    x: RangeInclusive<f64>,
    y: RangeInclusive<f64>,
    /// Whether the axis runs from its larger bound at the start edge (left, top) to its smaller one.
    flipped: [bool; 2],
    /// The step the point snaps to and one arrow key moves it on each axis; the default is a hundredth of the range
    /// with no snapping.
    #[prop(quantity = "step")]
    step: Option<f64>,
    /// The step of each axis, for axes that step differently (a colour area over chroma and hue); it replaces `step`.
    #[prop(quantity = "step")]
    steps: Option<[f64; 2]>,
    /// The width over the height of the image the well shows; the plot is the image fitted in the well.
    #[prop(quantity = "aspect ratio")]
    aspect: Option<f32>,
    /// Paints the image under the point; without one the well shows a checkerboard.
    #[prop(func = "FnOnce(&ContentPainterCx, Rect)")]
    backdrop: Option<Backdrop<'a>>,
}

impl<'a> PointPicker<'a> {
    /// The well: a press on it moves the point there, and a drag from it follows the pointer.
    pub const WELL: PartKey = PartKey::new("well");

    /// A picker of `value` over `x` and `y`. A range written from its larger bound to its smaller (`100..=0`) puts the
    /// larger bound at the left or top edge: `0..=100` over `100..=0` is a plot with y up.
    pub fn new(value: Bind<'a, [Mixed<f64>; 2]>, x: RangeInclusive<f64>, y: RangeInclusive<f64>) -> Self {
        let order = |range: RangeInclusive<f64>| {
            let (start, end) = (*range.start(), *range.end());
            if start > end { (end..=start, true) } else { (range, false) }
        };
        let ((x, flip_x), (y, flip_y)) = (order(x), order(y));
        PointPicker { common: Common::new(), value, x, y, flipped: [flip_x, flip_y], step: None, steps: None, aspect: None, backdrop: None }
            .ltr_axis()
    }

    fn ranges(&self) -> [&RangeInclusive<f64>; 2] {
        [&self.x, &self.y]
    }

    fn middle(&self) -> [f64; 2] {
        self.ranges().map(|range| (range.start() + range.end()) / 2.0)
    }

    /// The step set for `axis`: its own, else the picker's.
    fn set_step(&self, axis: usize) -> Option<f64> {
        self.steps.map(|steps| steps[axis]).or(self.step)
    }

    /// The step of one arrow press on `axis`.
    fn small(&self, axis: usize) -> f64 {
        let range = self.ranges()[axis];
        self.set_step(axis).unwrap_or((range.end() - range.start()) * STEP_SHARE)
    }

    /// The steps of `axis` from `value`.
    fn value_steps(&self, axis: usize, value: f64) -> ValueSteps {
        let range = self.ranges()[axis];
        let small = self.small(axis);
        ValueSteps::new(value, *range.start(), *range.end(), small, small * LARGE_STEPS)
    }

    /// The grid values snap to on `axis`: its step, or a tenth of it for a fine move; none without a step.
    fn grid(&self, axis: usize, fine: bool) -> Option<f64> {
        self.set_step(axis).filter(|step| *step > 0.0).map(|step| if fine { step / LARGE_STEPS } else { step })
    }

    /// `value` on `axis` in its range, and on `grid`.
    fn settle(&self, axis: usize, value: f64, grid: Option<f64>) -> f64 {
        let (min, max) = (*self.ranges()[axis].start(), *self.ranges()[axis].end());
        grid.map_or(value.clamp(min, max), |step| snap_to_step(value, step, min, min..=max))
    }

    /// Where the point stands on each axis: a mixed axis stands in the middle of its range.
    fn placed(&self, value: &[Mixed<f64>; 2]) -> [f64; 2] {
        let middle = self.middle();
        [0, 1].map(|axis| *value[axis].or(&middle[axis]))
    }

    /// The range of `axis` as the drag reads it: from the value at the start edge to the value at the end edge.
    fn along(&self, axis: usize) -> RangeInclusive<f64> {
        let range = self.ranges()[axis];
        if self.flipped[axis] { *range.end()..=*range.start() } else { range.clone() }
    }

    /// The value of `axis` after an arrow key that goes `toward` the end edge (1) or the start edge (-1) on screen.
    fn stepped(&self, axis: usize, from: f64, toward: f64, mods: MoveMods) -> f64 {
        let raises = (toward > 0.0) != self.flipped[axis];
        let (axes, mv) = match (axis, raises) {
            (0, true) => (ValueAxes::Horizontal, NavMove::Forward),
            (0, false) => (ValueAxes::Horizontal, NavMove::Back),
            (_, true) => (ValueAxes::Vertical, NavMove::Up),
            (_, false) => (ValueAxes::Vertical, NavMove::Down),
        };
        match step_value(&self.value_steps(axis, from), axes, mv, mods) {
            StepResult::Set(next) => self.settle(axis, next, self.grid(axis, mods.fine && !mods.extend)),
            _ => from,
        }
    }

    /// The axis a wheel turn steps and its new value: vertical scroll steps y and horizontal scroll x. Scrolling up
    /// moves the point up, as the Up key does, and right moves it right.
    fn wheeled(&self, wheel: &Wheel, placed: [f64; 2]) -> Option<(usize, f64)> {
        let (axis, scroll) = if wheel.scroll.y != 0.0 {
            let up = if self.flipped[1] { 1.0 } else { -1.0 };
            (1, Vec2::new(0.0, wheel.scroll.y * up))
        } else {
            let right = if self.flipped[0] { -1.0 } else { 1.0 };
            (0, Vec2::new(wheel.scroll.x * right, 0.0))
        };
        let mut turned = *wheel;
        turned.scroll = scroll;
        match wheel_step(&self.value_steps(axis, placed[axis]), ValueAxes::Both, &turned, false) {
            StepResult::Set(next) => Some((axis, self.settle(axis, next, self.grid(axis, wheel.mods.alt && !wheel.mods.shift)))),
            _ => None,
        }
    }

    /// The image fitted in `well`, else the well.
    fn plot(&self, well: Rect) -> Rect {
        match self.aspect.filter(|aspect| aspect.is_finite() && *aspect > 0.0) {
            Some(aspect) => {
                let width = well.width().min(well.height() * aspect);
                Rect::from_center_size(well.center(), Vec2::new(width, width / aspect))
            }
            None => well,
        }
    }

    /// Where `value` stands in `plot`.
    fn position(&self, value: [f64; 2], plot: Rect) -> Pos2 {
        let at = |axis: usize| {
            let range = self.ranges()[axis];
            let span = range.end() - range.start();
            let share = if span > 0.0 { ((value[axis] - range.start()) / span).clamp(0.0, 1.0) as f32 } else { 0.0 };
            if self.flipped[axis] { 1.0 - share } else { share }
        };
        pos2(plot.left() + at(0) * plot.width(), plot.top() + at(1) * plot.height())
    }

    /// The point `press` gives, from the point at the start of the drag.
    fn moved_by(&self, drags: &[ValueDrag; 2], start: [f64; 2], press: &PressState) -> Option<[f64; 2]> {
        let fine = press.mods.alt && !press.mods.shift;
        Some([
            self.settle(0, drags[0].value(start[0], press, false)?, self.grid(0, fine)),
            self.settle(1, drags[1].value(start[1], press, false)?, self.grid(1, fine)),
        ])
    }
}

/// The axis an arrow key moves and the way on screen: 1 towards the end edge (right, bottom), -1 towards the start
/// edge. The picker keeps its axis order (`ltr_axis`), so its moves are the physical keys.
fn arrow(mv: NavMove) -> Option<(usize, f64)> {
    match mv {
        NavMove::Forward => Some((0, 1.0)),
        NavMove::Back => Some((0, -1.0)),
        NavMove::Down => Some((1, 1.0)),
        NavMove::Up => Some((1, -1.0)),
        _ => None,
    }
}

/// `value` with `axis` set to `to`; the other axis stays as it is, mixed or not.
fn with_axis(value: &[Mixed<f64>; 2], axis: usize, to: f64) -> [Mixed<f64>; 2] {
    let mut next = *value;
    next[axis] = Mixed::Same(to);
    next
}

impl Component for PointPicker<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither);
    type Style = PointPickerStyle;
    type Output = EditResponse<[Mixed<f64>; 2]>;

    fn style(cx: &StyleCx<'_>, _: PointPickerStyleKey) -> PointPickerStyle {
        PointPickerStyle {
            metrics: Metrics::new().min_size(Vec2::new(cx.tokens.size.plot.width, cx.tokens.size.plot.height)),
            focus: cx.focus_look(),
            thumb: cx.tokens.size.handle.md,
            hit: cx.tokens.size.hit.target,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let placed = self.placed(cx.value().unwrap_or(&ALL_MIXED));
        let thumb = Rect::from_center_size(self.position(placed, self.plot(rect)), Vec2::splat(cx.style.hit));
        cx.part(Self::WELL, rect).sense(Sense::CLICK | Sense::DRAG | Sense::TAKE_FOCUS).value_part().pointer_only("the arrow keys move the point");
        cx.part(THUMB, thumb)
            .sense(Sense::DRAG | Sense::TAKE_FOCUS)
            .priority(HitPriority::new(HitLevel::Handle, 0))
            .value_part()
            .pointer_only("the arrow keys move the point");
    }

    fn bind(&mut self) -> Option<BindMut<'_, [Mixed<f64>; 2]>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let plot = self.plot(cx.rect);
        let drags = [
            ValueDrag::absolute(plot.left()..plot.right(), self.along(0)),
            ValueDrag::absolute(plot.top()..plot.bottom(), self.along(1)).axis(Axis::Y),
        ];
        for key in [THUMB, Self::WELL] {
            let press = cx.part(key).press;
            let start = cx.memory::<DragStart>().0;
            match press.phase {
                PressPhase::DragStart { .. } => {
                    let from = self.placed(cx.value().unwrap_or(&ALL_MIXED));
                    cx.memory::<DragStart>().0 = from;
                    cx.begin();
                }
                PressPhase::Drag { .. } => {
                    if let Some(next) = self.moved_by(&drags, start, &press) {
                        cx.propose(Mixed::each(next));
                    }
                }
                PressPhase::Release { click, .. } => {
                    if cx.gesture().is_some() {
                        if let Some(next) = self.moved_by(&drags, start, &press) {
                            cx.propose(Mixed::each(next));
                        }
                        cx.commit();
                    } else if click {
                        // A click jumps to the pointer whatever is held: it starts from the shown point and the fine and
                        // coarse factors of a drag do not apply.
                        let from = self.placed(cx.value().unwrap_or(&ALL_MIXED));
                        if let Some(next) = self.moved_by(&drags, from, &press.mods(Mods::NONE)) {
                            cx.set(Mixed::each(next));
                        }
                    }
                }
                _ => {}
            }
        }
        let running = cx.gesture().is_some();
        let mut now = cx.value().cloned().unwrap_or(ALL_MIXED);
        // While a drag runs, keys, the wheel and screen-reader requests change nothing; they are still taken.
        while let Some((mv, mods)) = cx.intents().take_move() {
            let Some((axis, toward)) = arrow(mv).filter(|_| !running) else { continue };
            let from = self.placed(&now)[axis];
            now = with_axis(&now, axis, self.stepped(axis, from, toward, mods));
            cx.set(now);
        }
        if let Some(wheel) = cx.take_wheel()
            && !running
            && let Some((axis, next)) = self.wheeled(&wheel, self.placed(&now))
        {
            now = with_axis(&now, axis, next);
            cx.set(now);
        }
        for (axis, key) in AXES.into_iter().enumerate() {
            if let Some(next) = cx.intents().take_set_value_at(key).filter(|next| next.is_finite() && !running) {
                now = with_axis(&now, axis, self.settle(axis, next, self.grid(axis, false)));
                cx.set(now);
            }
        }
        while let Some(request) = cx.intents().take_access() {
            let Some(axis) = AXES.iter().position(|key| request.target == Some((*key).into())) else { continue };
            let steps = match request.action {
                AccessAction::Increment => 1.0,
                AccessAction::Decrement => -1.0,
                _ => continue,
            };
            if !running {
                let from = self.placed(&now)[axis];
                now = with_axis(&now, axis, self.settle(axis, from + steps * self.small(axis), self.grid(axis, false)));
                cx.set(now);
            }
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<PointPickerWords>();
        let value = cx.value().unwrap_or(&ALL_MIXED);
        let number = |axis: usize| value[axis].same().map(|n| cx.format().number(*n, decimals_for_step(self.small(axis))));
        if let (Some(x), Some(y)) = (number(0), number(1)) {
            access.value(words.value.fill(&[("x", x.as_str()), ("y", y.as_str())]));
        }
        let label = cx.resolved.label;
        for (axis, key) in AXES.into_iter().enumerate() {
            let range = self.ranges()[axis].clone();
            let numeric = match (value[axis].same(), number(axis)) {
                (Some(n), Some(text)) => Numeric::new(*n, range).text(text),
                _ => Numeric::empty(range),
            };
            let (plain, named) = if axis == 0 { (&words.axis_x, &words.named_axis_x) } else { (&words.axis_y, &words.named_axis_y) };
            let node = access.node(key, Role::SLIDER);
            node.name = Name::Text(if label.is_empty() { plain.as_str().to_owned() } else { named.fill(&[("name", label)]) });
            node.numeric = Some(numeric.step(self.small(axis)));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let painter = ui.content_painter(cx);
        let inside = painter.plot_frame(cx.rect);
        let plot = self.plot(cx.rect);
        match self.backdrop.take() {
            Some(Backdrop(paint)) => paint(&painter, plot),
            None => painter.checker(plot.intersect(inside)),
        }
        if let Some(value) = cx.value() {
            let (center, arm) = (self.position(self.placed(value), plot), cx.style.thumb / 2.0);
            for (from, to) in [(Vec2::new(arm, 0.0), Vec2::new(-arm, 0.0)), (Vec2::new(0.0, arm), Vec2::new(0.0, -arm))] {
                painter.outline(&PaintShape::Path(vec![center + from, center + to]));
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<[Mixed<f64>; 2]> {
        cx.edit_response()
    }
}
