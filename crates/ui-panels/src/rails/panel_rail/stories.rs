//! Stories of `PanelRail`.

use super::PanelRail;
use crate::contract::PaneEdge;
use crate::rails::tool::stories::tools;
use crate::testing::{Drive, StoryData, StoryOutput, in_edge_pane, story};
use egui::Ui;

#[story(PanelRail<'static, u8>, covers = [Default], drive = Drive::Never)]
fn panel_rail(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let panels = tools(&[0, 1, 2, 3]);
    // Docked at the window's end edge, as an app shows it: its divider runs along the canvas side.
    in_edge_pane(ui, PaneEdge::End, |ui| PanelRail::new("Panels", &panels).show_rail(ui, |_| {})).into()
}
