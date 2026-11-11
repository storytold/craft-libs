//! Stories of `ParamDialog`.

use super::{ParamDialog, ParamLayout};
use crate::contract::Bind;
use crate::patterns::form_dialogs::Param;
use crate::primitives::TextureCache;
use crate::runtime::env::Label;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::load::SizedTexture;
use egui::{Color32, ColorImage, Ui, Vec2};

const SPEC: &str = r#"{"radius":0.1..1000=1,"method":"spin|zoom","smooth":bool}"#;

/// The picture's size in points at zoom 1.
const PICTURE: [usize; 2] = [360, 240];

fn picture(ui: &Ui) -> SizedTexture {
    let id = TextureCache::texture(ui.ctx(), "form-dialogs-dialog-story", || ColorImage::filled(PICTURE, Color32::from_rgb(0x8a, 0x6f, 0x4d)));
    SizedTexture::new(id, Vec2::new(PICTURE[0] as f32, PICTURE[1] as f32))
}

const LAYOUTS: [(&str, ParamLayout); 2] = [("Beside", ParamLayout::Beside), ("Below", ParamLayout::Below)];

/// A blur dialog with the `preview` and `layout` values of the story.
fn blur_story(ui: &mut Ui, data: &mut StoryData, preview: bool, layout: ParamLayout) -> StoryOutput {
    let preview = *data.value("preview", preview);
    let layout = *data.choice("layout", layout, &LAYOUTS);
    let dialog = ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).opened(Bind::bound(data.value("open", true)));
    let dialog = if preview { dialog.preview(picture(ui)).layout(layout) } else { dialog };
    dialog.show(ui).into()
}

#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    blur_story(ui, data, false, ParamLayout::Beside)
}

#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_with_preview(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    blur_story(ui, data, true, ParamLayout::Beside)
}

#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_with_preview_below(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    blur_story(ui, data, true, ParamLayout::Below)
}

#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn param_dialog_close_without_reset(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ParamDialog::new("blur", "Blur", Param::parse_list(SPEC))
        .opened(Bind::bound(data.value("open", true)))
        .cancel_label(Label::from("Close"))
        .alt_resets(false)
        .show(ui)
        .into()
}

const COMMAND_SPEC: &str = r#"{"name":text,"size":1..100=10,"method":"fit|fill","keep":bool}"#;

#[story(ParamDialog<'static>, covers = [], drive = Drive::Never)]
fn command_form(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ParamDialog::command("fit-image", "Fit Image", Param::parse_list(COMMAND_SPEC)).opened(Bind::bound(data.value("open", true))).show(ui).into()
}
