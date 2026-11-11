//! [`ColorStrip`]: one component of a colour along a vertical strip, the others fixed.

use super::channels::{QUANTIZE_CELLS, channel_range, channel_step, primary, quantized, settled, snapped};
use crate::AreaMode;
use craft_behavior::nav::{LARGE_STEPS, StepResult, ValueAxes, ValueSteps, step_value};
use craft_behavior::{Axis, Metrics, NavMove, PartKey, PressPhase, Sense, ValueDrag};
use craft_theme::ModelValues;
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, Numeric, PaintUi, PartsCx, Pointer as _, Props as _,
    Remember as _, Role,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use craft_ui::style::{ComponentStyle, DataColor, FocusLook, StyleColor, StyleCx, UiTokens};
use egui::{CornerRadius, Mesh, Pos2, Rect, Vec2, pos2};

#[cfg(feature = "screen-testing")]
mod stories;

/// The strip itself: a press on it sets the value there.
const STRIP: PartKey = PartKey::new("strip");

/// The stops of the strip's colour ramp (PhotoCraft's 24 steps).
const RAMP_STEPS: usize = 24;

/// The style of a colour strip.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ColorStripStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The edge round the strip.
    edge: StyleColor,
    /// The pointers' fill.
    pointer: StyleColor,
    /// The side of the square a pointer triangle fills.
    pointer_size: f32,
}

/// The value at the start of the running drag.
#[derive(Clone, Default)]
struct DragStart(f64);

/// The component strip beside a colour picker's [`ColorArea`](super::ColorArea): the [`AreaMode`]'s strip component
/// from its lowest value at the bottom to its highest at the top, the others fixed (PhotoCraft's component strip; hue
/// runs from 360 degrees at the top to 0 at the bottom). The HSB hue strip shows pure hues; the others show the
/// colour's own values.
///
/// It edits the mode's model values and changes only its component:
///
/// - Pointer: a press sets the value under the pointer; a drag moves it (the pointer may leave the strip); Esc during
///   a drag cancels it. Values snap to the channel's step.
/// - Keys, while focused: Up and Right step the value up, Down and Left down; Shift steps ten; Page Up and Page Down
///   move a tenth of the range, Home and End go to its ends. The wheel scrolls the panel. Right to left the strip
///   keeps its directions.
/// - AccessKit: a `Slider` named `label` ("Hue") with its value, bounds and step, which takes increment, decrement
///   and set-value requests.
///
/// The pointer triangles on both sides are drawn outside its rect.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub struct ColorStrip<'a> {
    #[common]
    common: Common,
    mode: AreaMode,
    values: Bind<'a, ModelValues>,
    /// Puts the colour the strip reports on this many levels (at least two) per red, green and blue channel, whatever
    /// the mode: a web-safe strip has 6. The pointer takes the nearest such colour and a key steps to the next
    /// different one. The strip shows flat rows of the snapped colours.
    #[prop]
    quantize: Option<usize>,
}

impl<'a> ColorStrip<'a> {
    /// A strip named `label` ("Hue") showing `values`, values of `mode`'s model.
    pub fn new(label: impl Into<Label>, mode: AreaMode, values: Bind<'a, ModelValues>) -> Self {
        ColorStrip { common: Common::new(), mode, values, quantize: None }.label(label).ltr_axis()
    }

    fn levels(&self) -> Option<usize> {
        self.quantize.map(|levels| levels.max(2))
    }

    /// `values` with the strip's component set to `value`, on its step and the quantised colour.
    fn with(&self, values: &ModelValues, value: f64) -> ModelValues {
        let model = self.mode.model();
        let channel = self.mode.strip();
        let mut out = *values;
        out.values[channel] = settled(model, channel, value as f32);
        match self.levels() {
            Some(levels) => quantized(model, &out, levels),
            None => out,
        }
    }

    /// The value after the key `mv` held with `mods`, from `values`; a quantising strip steps again until the colour
    /// is a different one or the component can go no further.
    fn stepped(&self, values: &ModelValues, mv: NavMove, mods: craft_behavior::MoveMods) -> ModelValues {
        let model = self.mode.model();
        let channel = self.mode.strip();
        let range = channel_range(model, channel);
        let (min, max) = (*range.start(), *range.end());
        let step = channel_step(model, channel);
        let mv = match mv {
            NavMove::Forward => NavMove::Up,
            NavMove::Back => NavMove::Down,
            other => other,
        };
        let large = if matches!(mv, NavMove::PageUp | NavMove::PageDown) { (max - min) / LARGE_STEPS } else { step * LARGE_STEPS };
        let once = |from: f64| match step_value(&ValueSteps::new(from, min, max, step, large), ValueAxes::Vertical, mv, mods) {
            StepResult::Set(next) => next,
            _ => from,
        };
        let mut next = f64::from(values.values[channel]);
        let from = self.levels().map(|levels| quantized(model, values, levels));
        loop {
            let after = once(next);
            let moved = after != next;
            next = after;
            let unchanged = from.is_some_and(|from| quantized(model, &self.with(values, next), self.levels().unwrap_or(2)) == from);
            if !(moved && unchanged) {
                break;
            }
        }
        self.with(values, next)
    }

    /// The colour of the strip at `value` of its component.
    fn color_at(&self, base: &ModelValues, value: f32) -> craft_theme::Srgba {
        let model = self.mode.model();
        let mut values = *base;
        values.values[self.mode.strip()] = value;
        model.to_srgb(&values.values)
    }
}

impl Component for ColorStrip<'_> {
    const SINGLE_VALUE: bool = true;
    const ROLE: Role = Role::SLIDER;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither);
    type Style = ColorStripStyle;
    type Output = EditResponse<ModelValues>;

    fn style(cx: &StyleCx<'_>, _: ColorStripStyleKey) -> ColorStripStyle {
        let tokens = cx.tokens;
        ColorStripStyle {
            metrics: Metrics::new().min_size(Vec2::new(tokens.size.icon.lg, tokens.size.plot.height)),
            focus: cx.focus_look(),
            edge: cx.color(tokens.color.line.control_edge),
            pointer: cx.color(tokens.color.text.normal),
            pointer_size: tokens.size.handle.md,
        }
    }

    fn paint_outset(tokens: &UiTokens) -> f32 {
        tokens.size.handle.md
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(STRIP, rect).sense(Sense::CLICK | Sense::DRAG | Sense::TAKE_FOCUS).value_part().pointer_only("the arrow keys step the value");
    }

    fn bind(&mut self) -> Option<BindMut<'_, ModelValues>> {
        Some(self.values.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let model = self.mode.model();
        let channel = self.mode.strip();
        let drag = ValueDrag::absolute(cx.rect.bottom()..cx.rect.top(), channel_range(model, channel)).axis(Axis::Y);
        let start_values = ModelValues::new(model.id(), &[]);
        let current = |cx: &Cx<'_, Self>| cx.value().copied().unwrap_or(start_values);
        let press = cx.part(STRIP).press;
        match press.phase {
            PressPhase::DragStart { .. } if primary(&press) => {
                cx.memory::<DragStart>().0 = f64::from(current(&*cx).values[channel]);
                cx.begin();
            }
            PressPhase::Drag { .. } if primary(&press) => {
                let from = cx.memory::<DragStart>().0;
                if let Some(next) = drag.value(from, &press, false) {
                    cx.propose(self.with(&current(&*cx), next));
                }
            }
            PressPhase::Release { click, .. } if primary(&press) => {
                if cx.gesture().is_some() {
                    let from = cx.memory::<DragStart>().0;
                    if let Some(next) = drag.value(from, &press, false) {
                        cx.propose(self.with(&current(&*cx), next));
                    }
                    cx.commit();
                } else if click && let Some(next) = drag.value(0.0, &press.mods(craft_keymap::Mods::NONE), false) {
                    cx.set(self.with(&current(&*cx), next));
                }
            }
            _ => {}
        }
        // A running drag is one edit: keys and SetValue wait for it, and are still taken.
        let running = cx.gesture().is_some();
        while let Some((mv, mods)) = cx.intents().take_move() {
            if !running {
                let next = self.stepped(&current(&*cx), mv, mods);
                cx.set(next);
            }
        }
        if let Some(value) = cx.intents().take_set_value().filter(|value| value.is_finite() && !running) {
            cx.set(self.with(&current(&*cx), value));
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let model = self.mode.model();
        let channel = self.mode.strip();
        let range = channel_range(model, channel);
        let numeric = match cx.value() {
            Some(values) => Numeric::new(f64::from(values.values[channel]), range),
            None => Numeric::empty(range),
        };
        access.numeric(numeric.step(channel_step(model, channel)));
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let model = self.mode.model();
        let channel = self.mode.strip();
        let range = channel_range(model, channel);
        let (min, max) = (*range.start() as f32, *range.end() as f32);
        let rect = cx.rect;
        let shown = cx.value().copied().unwrap_or_else(|| ModelValues::new(model.id(), &[]));
        let base = self.mode.strip_base(&shown);
        let painter = ui.content_painter(cx);
        match self.levels() {
            Some(levels) => {
                let height = rect.height() / QUANTIZE_CELLS as f32;
                let mut mesh = Mesh::default();
                for row in 0..QUANTIZE_CELLS {
                    let t = (row as f32 + 0.5) / QUANTIZE_CELLS as f32;
                    let top = rect.top() + row as f32 * height;
                    let color = snapped(self.color_at(&base, egui::lerp(max..=min, t)), levels);
                    mesh.add_colored_rect(Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), top + height)), color.into());
                }
                painter.painter.add(mesh);
            }
            None => {
                let stops: Vec<(f32, DataColor)> = (0..=RAMP_STEPS)
                    .map(|k| {
                        let t = k as f32 / RAMP_STEPS as f32;
                        (t, DataColor::new(self.color_at(&base, egui::lerp(max..=min, t)).into()))
                    })
                    .collect();
                painter.ramp(rect, &stops, Axis::Y);
            }
        }
        let draw = ui.draw();
        draw.stroke(&PaintShape::Rect(rect, CornerRadius::ZERO), StrokeWidth::Thin, cx.style.edge);
        if cx.value().is_none() {
            return;
        }
        let span = max - min;
        let share = if span > 0.0 { ((shown.values[channel] - min) / span).clamp(0.0, 1.0) } else { 0.0 };
        let y = egui::lerp(rect.bottom()..=rect.top(), share);
        let (depth, half) = (cx.style.pointer_size / 2.0, cx.style.pointer_size / 2.0);
        for (tip, direction) in [(rect.left(), -1.0), (rect.right(), 1.0)] {
            let base_x = tip + direction * depth;
            let points: Vec<Pos2> = vec![pos2(tip, y), pos2(base_x, y - half), pos2(base_x, y + half)];
            let triangle = PaintShape::Polygon(points);
            draw.fill(&triangle, cx.style.pointer);
            draw.stroke(&triangle, StrokeWidth::Thin, cx.style.edge);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<ModelValues> {
        cx.edit_response()
    }
}
