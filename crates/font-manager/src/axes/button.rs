//! The axes behind one button: a `Button` that opens the axis sliders in a popover.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Pattern, PopupKind, Props, Remember as _, Show as _, ShowContainer as _, clicked};
use craft_ui::layout_components::Stack;
use craft_ui::popover::Popover;
use craft_ui::runtime::env::Label;
use egui::Ui;

use super::{AxisSliders, AxisSlidersOutput, AxisValues, FontAxis, NamedInstance};

/// Whether the popover of the sliders is open.
#[derive(Clone, Default)]
struct Open(bool);

/// A `Button` with the label that opens an anchored `Popover` holding `AxisSliders` over the same props. The open
/// state is the button's own memory. The output is the sliders' (`edit`, `instance`), with the button's response.
#[must_use = "show the button with `show`"]
#[derive(Pattern, Props)]
pub struct AxesButton<'a> {
    label: Label,
    axes: &'a [FontAxis],
    values: Bind<'a, AxisValues>,
    /// The named instances of the face, listed above the sliders.
    #[prop]
    instances: &'a [NamedInstance],
    /// The style's weight, which an unset `wght` axis shows (an unset axis shows its default without it).
    #[prop(quantity = "font weight, the `wght` coordinate")]
    weight: Option<f32>,
    /// Whether the style is italic: an unset `ital` axis then shows its maximum, once a weight is given.
    #[prop]
    italic: bool,
    /// The axes (by tag) the selection mixes: each shows `Mixed` and is left out of the edit's held values.
    #[prop]
    mixed: &'a [[u8; 4]],
}

impl<'a> AxesButton<'a> {
    /// A button named `label` over the `axes` of a face, editing the coordinates `values`.
    pub fn new(label: impl Into<Label>, axes: &'a [FontAxis], values: Bind<'a, AxisValues>) -> Self {
        AxesButton { label: label.into(), axes, values, instances: &[], weight: None, italic: false, mixed: &[] }
    }

    /// Draws the button and, while it is open, the sliders; reports the sliders' edit made this pass.
    pub fn show(self, ui: &mut Ui) -> AxisSlidersOutput {
        let AxesButton { label, axes, mut values, instances, weight, italic, mixed } = self;
        let mut edit = Default::default();
        let mut instance = None;
        let shown = Stack::new().show(ui, |ui, cx| {
            let mut open = cx.memory::<Open>().0;
            let button = Button::new(label.clone()).popup(PopupKind::Dialog).expanded(open).show(ui).response;
            if clicked(&button) {
                open = !open;
            }
            let _ = Popover::new(label.clone(), Bind::bound(&mut open)).opened_by(&button).show(ui, |ui, _| {
                let mut sliders = AxisSliders::new(label.clone(), axes, values.reborrowed()).instances(instances).italic(italic).mixed(mixed);
                if let Some(weight) = weight {
                    sliders = sliders.weight(weight);
                }
                let out = sliders.show(ui);
                edit = out.edit;
                instance = out.instance;
            });
            cx.memory::<Open>().0 = open;
            button
        });
        AxisSlidersOutput { response: shown.inner.unwrap_or(shown.output), edit, instance }
    }
}
