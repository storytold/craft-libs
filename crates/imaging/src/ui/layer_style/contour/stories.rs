//! Stories of the contour field: the built-in contours and a user's.

use craft_ui::contract::Bind;
use craft_ui::patterns::presets::PresetState;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{ContourField, ContourKey, ContourWords, contour_rows};
use crate::layer_style::{Contour, ContourPoint};

fn user_contours() -> Vec<Contour> {
    let point = |input, output| ContourPoint { input, output, corner: false };
    vec![Contour { name: String::from("Soft Step"), points: vec![point(0.0, 0.0), point(0.4, 0.1), point(1.0, 1.0)] }]
}

#[story(ContourField<'static>, covers = [Default], drive = Drive::Never)]
fn contour_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<ContourWords>();
    let user = user_contours();
    let rows = contour_rows(&Contour::presets(), &user, &words, None);
    let (contour, state) = data.value("field", (Contour::linear(), PresetState::<ContourKey>::new()));
    ContourField::new("Contour", Bind::bound(contour), &rows, Bind::bound(state)).user(&user).show(ui).into()
}

#[story(ContourField<'static>, covers = [], drive = Drive::Never)]
fn contour_field_with_user_contour(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<ContourWords>();
    let user = user_contours();
    let rows = contour_rows(&Contour::presets(), &user, &words, None);
    let (contour, state, anti_alias) = data.value("field", (user[0].clone(), PresetState::<ContourKey>::new(), Some(true)));
    ContourField::new("Contour", Bind::bound(contour), &rows, Bind::bound(state)).user(&user).anti_alias(Bind::bound(anti_alias)).show(ui).into()
}
