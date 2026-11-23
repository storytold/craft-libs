//! Stories of Panel Options.

use super::NavigatorOptionsDialog;
use crate::canvas_data::{NavigatorOptions, ProxyColor};
use crate::contract::Bind;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(NavigatorOptionsDialog<'static>, covers = [], drive = Drive::Never)]
fn navigator_options_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (options, options_at) = data.value_at("options", NavigatorOptions::new(ProxyColor::Blue));
    NavigatorOptionsDialog::new(options_at.bind(options)).opened(Bind::given(true)).show(ui).into()
}
