//! The "Control:" row of a brush dynamic ([`DynamicControlRow`]).

use craft_behavior::{IconId, Mixed, Value};
use craft_ui::buttons::ToggleButton;
use craft_ui::contract::{Bind, Entry, Outcome, Show as _, ShowContainer as _};
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::text::NumberField;
use egui::{Response, Ui};

use super::words::BrushSettingsWords;

/// Fade's step count: whole steps, 1 to 9999.
const FADE_STEPS: (f64, f64) = (1.0, 9999.0);

/// One entry of a dynamic's Control menu: the app's value (`C`, its control enum) and its name,
/// and what the row shows beside it when it is current.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DynamicControl<C> {
    /// The app's value.
    pub value: C,
    /// The name in the menu.
    pub label: String,
    /// This is Fade: the row shows the Fade steps field while it is current.
    pub fade: bool,
    /// This control has a 0..1 input its response curve shapes (Fade, Pen Pressure, Pen Tilt,
    /// Stylus Wheel, Rotation, outside Angle Jitter): the row shows the curve toggle while it is
    /// current.
    pub curve: bool,
}

impl<C> DynamicControl<C> {
    /// An entry with neither the Fade field nor the curve toggle (Off, the direction controls).
    pub fn new(value: C, label: impl Into<String>) -> Self {
        Self { value, label: label.into(), fade: false, curve: false }
    }

    /// Marks the entry as Fade.
    pub fn fade(mut self) -> Self {
        self.fade = true;
        self
    }

    /// Marks the entry as one a response curve shapes.
    pub fn curve(mut self) -> Self {
        self.curve = true;
        self
    }
}

/// What [`DynamicControlRow::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host shows its curve editor when the toggle was pressed"]
pub struct DynamicControlRowOutput {
    /// The row.
    pub response: Response,
    /// The curve toggle was pressed: the app opens or closes its curve editor.
    pub curve_toggled: bool,
}

impl Outcome for DynamicControlRowOutput {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Photoshop's `Control: [Off ▾] [25] [~]` row under a jitter slider: the control `Select`, the
/// Fade steps field (1 to 9999, only while the current control is [`fade`](DynamicControl::fade))
/// and the response curve toggle (only while it takes a [`curve`](DynamicControl::curve); lit while
/// the curve is open or not linear). The app lists the entries it offers (the direction controls
/// only for Angle Jitter) and shows its curve editor in its own slot under the row when
/// [`DynamicControlRowOutput::curve_toggled`] opens it.
///
/// - **Keys.** Three Tab stops (select, field, toggle).
/// - **AccessKit.** The select named "Control:", the field a `SpinButton` "Fade steps", the toggle
///   a `Button` "Response curve" with `toggled` while lit.
#[must_use = "show the row with `show`"]
#[derive(craft_ui::contract::Pattern)]
#[pattern(probe(DynamicControlRow<'static, u8>))]
pub struct DynamicControlRow<'a, C: Value> {
    controls: &'a [DynamicControl<C>],
    current: Bind<'a, C>,
    fade_steps: Bind<'a, f64>,
    curve_open: bool,
    curve_shaped: bool,
}

impl<'a, C: Value> DynamicControlRow<'a, C> {
    /// The row for the `current` control among `controls`, in menu order, with the Fade step count
    /// in `fade_steps`.
    pub fn new(controls: &'a [DynamicControl<C>], current: Bind<'a, C>, fade_steps: Bind<'a, f64>) -> Self {
        Self { controls, current, fade_steps, curve_open: false, curve_shaped: false }
    }

    /// Whether the app shows the curve editor (`open`) and whether the curve is not linear
    /// (`shaped`): either lights the toggle.
    pub fn curve(mut self, open: bool, shaped: bool) -> Self {
        self.curve_open = open;
        self.curve_shaped = shaped;
        self
    }

    /// Shows the row.
    pub fn show(self, ui: &mut Ui) -> DynamicControlRowOutput {
        let DynamicControlRow { controls, mut current, mut fade_steps, curve_open, curve_shaped } = self;
        let words = Env::of(ui).words::<BrushSettingsWords>();
        let entries: Vec<Entry<C>> = controls.iter().map(|entry| Entry::new(entry.value.clone(), Label::from(entry.label.clone()))).collect();
        let mut curve_toggled = false;
        let shown = Field::new(words.control.clone()).show(ui, |ui, _| {
            Row::new().show(ui, |ui, cx| {
                let held = match current.reborrow().read(cx) {
                    Some(Mixed::Same(held)) => Some(held),
                    _ => None,
                };
                let entry = held.and_then(|held| controls.iter().find(|entry| entry.value == held));
                let (fade, curve) = entry.map_or((false, false), |entry| (entry.fade, entry.curve));
                let _ = Select::new(words.control.clone(), current.reborrowed(), &entries).show(ui);
                if fade {
                    let steps = match fade_steps.reborrow().read(cx) {
                        Some(Mixed::Same(steps)) => steps.clamp(FADE_STEPS.0, FADE_STEPS.1),
                        _ => FADE_STEPS.0,
                    };
                    let field = NumberField::new(words.fade_steps.clone(), Bind::given(Some(steps)))
                        .range(FADE_STEPS.0..=FADE_STEPS.1)
                        .step(1.0)
                        .snap_typed(true)
                        .commit_decimals(0)
                        .show(ui);
                    for event in field.edit.events.iter().cloned() {
                        let event = event.map(|value| value.unwrap_or(steps).clamp(FADE_STEPS.0, FADE_STEPS.1));
                        fade_steps.reborrow().write(cx, &event);
                    }
                }
                if curve {
                    let lit = curve_open || curve_shaped;
                    let toggle = ToggleButton::new(words.response_curve.clone(), Bind::given(lit))
                        .icon(IconSpec::svg(IconId::new("spline")))
                        .icon_only()
                        .show(ui);
                    curve_toggled = !toggle.events.is_empty();
                }
            })
        });
        DynamicControlRowOutput { response: shown.output, curve_toggled }
    }
}
