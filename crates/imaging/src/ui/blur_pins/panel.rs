//! [`BlurPinsPanel`]: the selected pin's rows in the Blur Gallery dialog.

use craft_behavior::EditEvents;
use craft_ui::contract::{Bind, Outcome as _, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::{Shade, Text};
use craft_ui::form::SliderField;
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::style::TypeRole;
use egui::{Response, Ui};

use super::model::{BlurKind, BlurPins};
use super::words::BlurPinsWords;

/// What [`BlurPinsPanel::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct BlurPinsPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// The amount slider's gesture, with the pins after it.
    pub edit: EditEvents<BlurPins>,
}

/// The rows the Blur Gallery's filter dialog shows for its pins (in the dialog's body, above its Preview check
/// box): the selected pin's Blur (0..=500 px), or for Spin Blur Angle (0..=360°), a `SliderField`; "No pin selected"
/// in muted text without one; then the hints of the kind in the caption role, muted, wrapped.
#[must_use = "show the panel with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct BlurPinsPanel<'a> {
    pins: &'a mut BlurPins,
}

impl<'a> BlurPinsPanel<'a> {
    /// The panel over `pins`, which it edits in place.
    pub fn new(pins: &'a mut BlurPins) -> Self {
        Self { pins }
    }

    /// Draws the rows and returns the response and any amount edit.
    pub fn show(self, ui: &mut Ui) -> BlurPinsPanelOutput {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:398 `body` (the slider, the hints).
        let Self { pins } = self;
        let words = Env::of(ui).words::<BlurPinsWords>();
        let kind = pins.kind;
        let mut edit = EditEvents::new();
        let group = Stack::new().label(words.pins.clone()).show(ui, |ui, _| {
            edit = EditEvents::new();
            match pins.selected.and_then(|index| pins.pins.get(index).map(|pin| (index, pin.amount))) {
                Some((index, amount)) => {
                    let (label, unit) = if kind == BlurKind::Spin { (&words.blur_angle, UnitKind::Degrees) } else { (&words.blur, UnitKind::Pixels) };
                    let row = SliderField::new(label.clone(), Bind::given(amount), 0.0..=kind.max()).step(1.0).unit_kind(unit).show(ui);
                    for event in row.events {
                        if let Some(&value) = event.value() {
                            pins.pins[index].amount = value.clamp(0.0, kind.max());
                        }
                        edit.push(event.map(|_| pins.clone()));
                    }
                }
                None => {
                    Text::new(words.no_pin.clone()).role(TypeRole::Label).shade(Shade::Muted).show(ui);
                }
            }
            let hint = |ui: &mut Ui, text: &Label| {
                Text::new(text.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            };
            hint(ui, if kind == BlurKind::TiltShift { &words.hint_tilt } else { &words.hint_pins });
            match kind {
                BlurKind::Iris => hint(ui, &words.hint_iris),
                BlurKind::Spin => hint(ui, &words.hint_spin),
                _ => {}
            }
        });
        BlurPinsPanelOutput { response: group.response().clone(), edit }
    }
}
