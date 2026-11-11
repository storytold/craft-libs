//! Stories of `ColorDisc`: three markers with one selected.

use super::{ColorDisc, DiscMove, MARKER};
use craft_theme::Srgba;
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::Ui;

const MARKERS: [Srgba; 3] = [Srgba::rgb(0.9, 0.2, 0.2), Srgba::rgb(0.2, 0.7, 0.4), Srgba::rgb(0.3, 0.4, 0.9)];

#[story(ColorDisc<'static>, covers = [Default], drive = Drive::Drag(Grip::part(MARKER.at(1))))]
fn color_disc(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let moved = data.value("move", DiscMove::new(0, 0.0, 0.7));
    ColorDisc::new("Harmony", &MARKERS, Bind::bound(moved)).chosen(1).show(ui).into()
}

#[story(ColorDisc<'static>, covers = [Invalid], drive = Drive::Never)]
fn color_disc_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_pale = |moved: &DiscMove| if moved.saturation < 0.5 { Validity::Refuse(Message::new("Too pale")) } else { Validity::Ok };
    let moved = Bind::bound(data.value("pale", DiscMove::new(1, 0.0, 0.2))).check(refuse_pale);
    ColorDisc::new("Harmony", &MARKERS, moved).chosen(1).show(ui).into()
}
