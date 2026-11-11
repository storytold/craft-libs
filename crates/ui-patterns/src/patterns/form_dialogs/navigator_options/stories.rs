//! Stories of Panel Options.

use super::NavigatorOptionsDialog;
use crate::canvas_data::{NavigatorOptions, ProxyColor};
use crate::contract::Bind;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(NavigatorOptionsDialog<'static>, covers = [], drive = Drive::Never)]
fn navigator_options_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let options = data.value("options", NavigatorOptions::new(ProxyColor::Blue));
    NavigatorOptionsDialog::new(Bind::bound(options)).opened(Bind::given(true)).show(ui).into()
}
