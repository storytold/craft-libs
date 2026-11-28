//! Stories of `ColorDisc`: three markers with one selected.

use super::{ColorDisc, DiscLook, DiscMove, DiscRadius, MARKER};
use craft_theme::Srgba;
use craft_ui::contract::{Props as _, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::Ui;

const MARKERS: [Srgba; 3] = [Srgba::rgb(0.9, 0.2, 0.2), Srgba::rgb(0.2, 0.7, 0.4), Srgba::rgb(0.3, 0.4, 0.9)];

#[story(ColorDisc<'static>, covers = [Default], drive = Drive::Drag(Grip::part(MARKER.at(1))))]
fn color_disc(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (moved, moved_at) = data.value_at("move", DiscMove::new(0, 0.0, 0.7));
    ColorDisc::new("Harmony", &MARKERS, moved_at.bind(moved)).base(1).chosen(1).show(ui).into()
}

#[story(ColorDisc<'static>, covers = [], drive = Drive::Never, title = "Segmented")]
fn color_disc_segmented(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let moved = data.bind("move", DiscMove::new(0, 0.0, 0.7));
    ColorDisc::new("Harmony", &MARKERS, moved).look(DiscLook::Segmented).base(1).chosen(1).show(ui).into()
}

#[story(ColorDisc<'static>, covers = [], drive = Drive::Never, title = "Brightness radius")]
fn color_disc_brightness(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let moved = data.bind("move", DiscMove::new(0, 0.0, 0.7));
    ColorDisc::new("Harmony", &MARKERS, moved).radius(DiscRadius::Brightness).level(0.8).base(1).chosen(1).show(ui).into()
}

#[story(ColorDisc<'static>, covers = [], drive = Drive::Never, title = "Base marker")]
fn color_disc_base(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    // The base is large; the chosen marker, another one, wears the selection ring.
    let moved = data.bind("move", DiscMove::new(0, 0.0, 0.7));
    ColorDisc::new("Harmony", &MARKERS, moved).base(0).chosen(2).show(ui).into()
}

#[story(ColorDisc<'static>, covers = [Invalid], drive = Drive::Never)]
fn color_disc_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_pale = |moved: &DiscMove| if moved.radial < 0.5 { Validity::Refuse(Message::new("Too pale")) } else { Validity::Ok };
    let moved = data.bind("pale", DiscMove::new(1, 0.0, 0.2)).check(refuse_pale);
    ColorDisc::new("Harmony", &MARKERS, moved).base(1).chosen(1).show(ui).into()
}

#[story(ColorDisc<'static>, covers = [], drive = Drive::Never, title = "Disabled")]
fn color_disc_disabled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (moved, moved_at) = data.value_at("move", DiscMove::new(0, 0.0, 0.7));
    ColorDisc::new("Harmony", &MARKERS, moved_at.bind(moved)).base(1).chosen(1).enabled(false).show(ui).into()
}
