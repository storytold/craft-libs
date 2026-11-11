//! Stories of `ColorChoice`: named colours with the custom row.

use craft_theme::Srgba;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{ChosenColor, ColorChoice, NamedColor};

#[story(ColorChoice<'static>, covers = [Default], drive = Drive::Never)]
fn color_choice(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let chosen = data.value("chosen", ChosenColor::Named("black".into())).clone();
    let named = [
        NamedColor::new("default", "Default", None),
        NamedColor::new("black", "Black", Some(Srgba::rgb(0.0, 0.0, 0.0))),
        NamedColor::new("gray", "Medium Gray", Some(Srgba::rgb(0.5, 0.5, 0.5))),
    ];
    let out = ColorChoice::new("Pasteboard color", &chosen).named(named).custom(Some(Srgba::rgb(0.8, 0.3, 0.1))).show(ui);
    if let Some(next) = out.chosen.clone() {
        *data.value("chosen", ChosenColor::Named("black".into())) = next;
    }
    out.into()
}
