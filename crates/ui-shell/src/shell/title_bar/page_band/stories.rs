//! Stories of `PageBand`: the band over a shown page, at the top of the room a shell gives it.

use super::PageBand;
use crate::contract::{Props as _, ShowContainer as _};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Rect, Ui, vec2};

/// The band with the window's ends, as a shell shows it over a page.
#[story(PageBand, covers = [Default], drive = Drive::Never)]
fn page_band(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = ui.available_rect_before_wrap();
    let rect = Rect::from_min_size(room.min, vec2(room.width(), ui.spacing().interact_size.y));
    PageBand::new().label("Window").show_at(ui, rect, |_, _| {}).into()
}
