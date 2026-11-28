//! Optional scalar-row key interest. Geometry and real stops come from the ordinary component pass.

use super::{current, route};
use crate::runtime::egui::LayerId;
use crate::runtime::egui::areas;
use crate::runtime::focus::{self, StopRec};
use crate::runtime::pointer;
use craft_behavior::{KeyOwner, MoveMods, NavMove, OwnerClass};
use egui::{Context, Id, Rect};

/// Which real scalar row can take a vertical arrow, without moving keyboard focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScalarKeyTrigger {
    /// Only the row whose slider has keyboard focus.
    #[default]
    Focused,
    /// The first drawn eligible row: its slider has focus, or its clipped whole row is hovered.
    HoveredOrFocused,
}

/// Opt-in additive model-unit vertical-key and accessibility nudges.
///
/// Pointer, typed-value and wheel quantization keep their original steps. The widget snaps the result of the
/// addition to its original value grid, not a new grid based on `nudge`. A policy is optional: omitting it keeps
/// the existing slider behavior. Non-positive or non-finite amounts make the policy inactive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScalarKeyPolicy {
    nudge: f64,
    shift_factor: f64,
    trigger: ScalarKeyTrigger,
    model_step: Option<f64>,
}

impl ScalarKeyPolicy {
    /// A focused-row policy, with Shift multiplying the additive nudge by five.
    pub const fn new(nudge: f64) -> Self {
        Self { nudge, shift_factor: 5.0, trigger: ScalarKeyTrigger::Focused, model_step: None }
    }

    /// Chooses whether whole-row hover also makes a row eligible. This never changes focus.
    pub const fn trigger(mut self, trigger: ScalarKeyTrigger) -> Self {
        self.trigger = trigger;
        self
    }

    /// Sets the multiplier used by Shift. This does not change any global or wheel step multiplier.
    pub const fn shift_factor(mut self, factor: f64) -> Self {
        self.shift_factor = factor;
        self
    }

    /// Selects an explicit zero-origin model grid for scalar keys and accessibility actions only.
    /// The final value is rounded to a multiple of `step`, then clamped to the slider bounds, before any edit event.
    /// This does not change the additive nudge or pointer, wheel, horizontal-key, or numeric-editor quantization.
    /// By default no model grid is selected: the original track grid and its on-grid bit preservation stay unchanged.
    /// Non-positive or non-finite steps make this optional scalar policy inactive.
    pub const fn model_step(mut self, step: f64) -> Self {
        self.model_step = Some(step);
        self
    }

    /// The optional model grid used by the SDK component's scalar landing, not its input routing or other paths.
    #[doc(hidden)]
    pub const fn model_quantum(self) -> Option<f64> {
        self.model_step
    }

    /// Whether the amounts and any selected model step describe usable, finite positive changes.
    pub fn is_valid(self) -> bool {
        self.nudge.is_finite()
            && self.nudge > 0.0
            && self.shift_factor.is_finite()
            && self.shift_factor > 0.0
            && (self.nudge * self.shift_factor).is_finite()
            && self.nudge * self.shift_factor > 0.0
            && self.model_step.is_none_or(|step| step.is_finite() && step > 0.0)
    }

    /// The unmodified model-unit accessibility increment.
    pub fn amount(self) -> Option<f64> {
        self.is_valid().then_some(self.nudge)
    }

    /// An additive offset for vertical keys or accessibility increment/decrement (Forward/Back).
    pub fn offset(self, movement: NavMove, modifiers: MoveMods) -> Option<f64> {
        let direction = match movement {
            NavMove::Up | NavMove::Forward => 1.0,
            NavMove::Down | NavMove::Back => -1.0,
            _ => return None,
        };
        self.amount().map(|amount| direction * amount * if modifiers.extend { self.shift_factor } else { 1.0 })
    }
}

/// SDK component metadata for one real scalar row. It creates no widget or focus stop.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScalarRow {
    pub(super) owner: Id,
    pub(super) slider: Id,
    pub(super) editor: Option<Id>,
    rect: Rect,
    layer: LayerId,
    policy: ScalarKeyPolicy,
}

impl crate::runtime::Shift for ScalarRow {
    fn shifted(&self, by: &egui::RecordShift) -> Self {
        ScalarRow { rect: by.rect(self.rect), ..*self }
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        ScalarRow { rect: self.rect.snapped(&to.rect, within), ..*self }
    }
}

impl ScalarRow {
    /// Uses the component's clipped allocated rect and the actual slider stop's owner.
    pub fn new(owner: Id, slider: Id, policy: ScalarKeyPolicy, rect: Rect, layer: LayerId) -> Self {
        Self { owner, slider, editor: None, rect, layer, policy }
    }

    /// Associates the real numeric child for accessibility increment/decrement; focused typing keeps its keys.
    pub fn editor(mut self, editor: Id) -> Self {
        self.editor = Some(editor);
        self
    }
}

/// Records opt-in interest in component draw order, exactly like the ordinary stop/node records.
#[doc(hidden)]
pub fn register_scalar(ctx: &Context, row: ScalarRow) {
    if crate::runtime::measure::active(ctx) || !row.policy.is_valid() || !row.rect.is_positive() {
        return;
    }
    crate::runtime::replay::journal(ctx, || row, |ctx, row| register_scalar(ctx, *row));
    current(ctx, |stage| stage.scalar_rows.push(row));
}

/// A captured gesture wins even over a different row; no raw pointer/key path bypasses that ownership.
pub(super) fn captured(ctx: &Context, press_captured: bool) -> bool {
    press_captured || pointer::with_pass_pointer(ctx, |pointer| pointer.primary_down)
}

/// Only rows whose real stop is allowed by the canonical popup/modal owner can act.
pub(super) fn allowed(ctx: &Context, row: &ScalarRow) -> bool {
    focus::stop_rec(ctx, (row.slider, None)).is_some_and(|record| {
        matches!(
            route::owner_at(ctx, Some(&record), &focus::scope_chain(ctx, &record), None),
            KeyOwner::Class(OwnerClass::Widget | OwnerClass::Dialog)
        )
    })
}

/// The focused row wins over a hovered one. Hover takes the arrows only where the focus leaves them, and source order
/// resolves the remaining rows.
pub(super) fn choose(ctx: &Context, rows: &[ScalarRow], focused: Option<&StopRec>, press_captured: bool) -> Option<ScalarRow> {
    if rows.is_empty() || captured(ctx, press_captured)
        // A real native editor/widget without a framework stop still owns its focused editing keys.
        || (focused.is_none() && crate::runtime::egui::focus::focused(ctx).is_some())
        || focused.is_some_and(|record| {
            record.stop.text || !matches!(route::owner_at(ctx, Some(record), &focus::scope_chain(ctx, record), None),
                KeyOwner::Class(OwnerClass::Widget | OwnerClass::Dialog | OwnerClass::App))
                || crate::runtime::ime::composing(ctx, record.stop.root)
        })
    {
        return None;
    }
    let is_focused = |row: &ScalarRow| focused.is_some_and(|record| record.stop.id == row.slider || record.stop.root == row.slider);
    if let Some(row) = rows.iter().find(|row| is_focused(row) && allowed(ctx, row)).copied() {
        return Some(row);
    }
    // A hovered row never takes Up and Down from a focused component that acts on the arrow moves itself: a
    // collection declares them in its key role, a select or a menu in its key use, and the stop's role holds both.
    if focused.is_some_and(|record| record.stop.role.moves) {
        return None;
    }
    let pointer = pointer::with_pass_pointer(ctx, |pointer| pointer.hover);
    rows.iter()
        .find(|row| {
            row.policy.trigger == ScalarKeyTrigger::HoveredOrFocused
                && pointer.is_some_and(|point| {
                    let local = areas::layer_map(ctx, row.layer).map_or(point, |map| map.to_layer(point));
                    row.rect.contains(local)
                        && pointer::with_hover(ctx, crate::runtime::egui::pass::current_viewport(ctx), |under, _| {
                            under.is_some_and(|under| under.contains(row.owner))
                        })
                })
                && allowed(ctx, row)
        })
        .copied()
}
