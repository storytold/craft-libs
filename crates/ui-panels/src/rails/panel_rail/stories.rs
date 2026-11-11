//! Stories of `PanelRail`.

use super::PanelRail;
use crate::rails::tool::stories::tools;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(PanelRail<'static, u8>, covers = [Default], drive = Drive::Never)]
fn panel_rail(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let panels = tools(&[0, 1, 2, 3]);
    PanelRail::new("Panels", &panels).show_rail(ui, |_| {}).into()
}
