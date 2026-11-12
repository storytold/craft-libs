//! Stories of `Rating`.

use super::Rating;
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use craft_theme::ControlSize;
use egui::Ui;

fn pick_four(fixture: &mut Fixture) {
    if let Some(star) = fixture.by_label("4 stars") {
        fixture.click(&star);
    }
}

#[story(Rating<'static>, covers = [Default], drive = Drive::Custom(pick_four), group = "Sizes", title = "Medium stars")]
fn rating(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let disabled = *data.value("disabled", false);
    Rating::new(Bind::bound(data.value("stars", 3u8)), 5).label("Medium stars").size(ControlSize::Md).enabled(!disabled).show(ui).into()
}

#[story(Rating<'static>, covers = [], drive = Drive::Custom(pick_four), group = "Sizes", title = "Small stars")]
fn rating_small(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let disabled = *data.value("disabled", false);
    Rating::new(Bind::bound(data.value("stars", 0u8)), 5).label("Small stars").size(ControlSize::Sm).enabled(!disabled).show(ui).into()
}

/// The compact form a tile's badge bar shows: only the lit stars.
#[story(Rating<'static>, covers = [], drive = Drive::Never, group = "Sizes", title = "Compact stars")]
fn rating_compact(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Rating::new(Bind::bound(data.value("stars", 3u8)), 5).label("Compact stars").compact(true).show(ui).into()
}

#[story(Rating<'static>, covers = [Mixed], drive = Drive::Never)]
fn rating_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Rating::new(Bind::bound_mixed(data.value("mixed", Mixed::Mixed)), 5).label("Rating").show(ui).into()
}

#[story(Rating<'static>, covers = [Invalid], drive = Drive::Never)]
fn rating_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_top = |stars: &u8| if *stars == 5 { Validity::Refuse(Message::new("Five stars needs a reason")) } else { Validity::Ok };
    Rating::new(Bind::bound(data.value("top", 5u8)).check(refuse_top), 5).label("Rating").show(ui).into()
}

#[story(Rating<'static>, covers = [], disabled_text = "Open the photo first")]
fn rating_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tooltip = data.tooltip("The photo's rating");
    Rating::new(Bind::bound(data.value("stars", 0u8)), 5).label("Rating").tooltip(tooltip).show(ui).into()
}
