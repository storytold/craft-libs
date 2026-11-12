//! The story of the full-window dialog shell.

use super::FullWindowDialog;
use crate::contract::{Bind, Show as _, ShowContainer as _};
use crate::dialog::{DialogWords, FullWindowSide};
use crate::display::Text;
use crate::form::SliderField;
use crate::runtime::env::Env;
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

/// The drive of the workspace: a click on Cancel.
fn cancel(fixture: &mut Fixture) {
    if let Some(cancel) = fixture.by_label("Cancel") {
        fixture.click(&cancel);
    }
}

/// A workspace as Liquify shows one: the view in the body, a side column with the brush size, and the footer among
/// the side columns, with Apply when the story asks for it.
#[story(FullWindowDialog<'static>, covers = [Default], drive = Drive::Custom(cancel))]
fn full_window_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let apply = *data.value("apply", false);
    let mut brush = *data.value("brush size", 50.0_f64);
    let env = Env::of(ui);
    let common = env.words::<CommonWords>();
    let mut buttons = vec![FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];
    if apply {
        buttons.push(FooterButton::new(env.words::<DialogWords>().apply.clone(), DialogRole::Other).stays_open(true));
    }
    let shown: StoryOutput = FullWindowDialog::new("Liquify")
        .opened(Bind::bound(data.value("open", true)))
        .buttons(buttons)
        .sides(|ui, _| {
            FullWindowSide::new("Properties").show(ui, |ui, _| {
                SliderField::new("Brush size", Bind::bound(&mut brush), 1.0..=100.0).step(1.0).show(ui);
            });
        })
        .show(ui, |ui, _| {
            Text::new("The view fills the rest of the window.").show(ui);
        })
        .into();
    *data.value("brush size", 50.0_f64) = brush;
    shown
}
