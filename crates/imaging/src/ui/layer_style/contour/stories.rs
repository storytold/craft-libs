//! Stories of the contour field: the built-in contours and a user's, under the host's "Contour" caption.

use craft_ui::contract::ShowContainer as _;
use craft_ui::form::Field;
use craft_ui::patterns::presets::PresetState;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{ContourField, ContourKey, contour_source};
use crate::layer_style::{Contour, ContourPoint};

fn user_contours() -> Vec<Contour> {
    let point = |input, output| ContourPoint { input, output, corner: false };
    vec![Contour { name: String::from("Soft Step"), points: vec![point(0.0, 0.0), point(0.4, 0.1), point(1.0, 1.0)] }]
}

/// The field over `start` with its Anti-aliased check box, in the host's Field, and the contour it holds as the
/// readout.
fn field(ui: &mut Ui, data: &mut StoryData, start: Contour, anti_alias: bool) -> StoryOutput {
    let user = user_contours();
    let presets = Contour::presets();
    let rows = contour_source(&presets, &user, None);
    let ((contour, state, anti_alias), field_at) = data.value_at("field", (start, PresetState::<ContourKey>::new(), Some(anti_alias)));
    let shown = Field::new("Contour").show(ui, |ui, _| {
        ContourField::new("Contour", field_at.bind(&mut *contour), &rows, field_at.bind(&mut *state))
            .user(&user)
            .anti_alias(field_at.bind(&mut *anti_alias))
            .show(ui)
    });
    let output = shown.inner.map(StoryOutput::from).unwrap_or_default();
    output.readout(format!("Contour: {}, {} points.", contour.name, contour.points.len()))
}

#[story(ContourField<'static>, covers = [Default], drive = Drive::Never)]
fn contour_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let cone = Contour::preset("Cone").unwrap_or_else(Contour::linear);
    field(ui, data, cone, false)
}

#[story(ContourField<'static>, covers = [], drive = Drive::Never)]
fn contour_field_with_user_contour(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    field(ui, data, user_contours().remove(0), true)
}
