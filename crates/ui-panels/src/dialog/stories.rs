//! Stories of the dialog shells.

use super::modal::ModalDragStrip;
use super::{BlockingOverlay, Dialog, DialogFrame};
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::layout_components::Stack;
use crate::runtime::focus::Initial;
use crate::sections::{DialogRole, FooterButton};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

/// What the story dialogs hold: two check boxes whose values the story keeps between passes.
#[doc(hidden)]
pub fn body(ui: &mut Ui, checks: &mut [bool; 2]) {
    let [snap, rulers] = checks;
    Stack::new().show(ui, |ui, _| {
        Checkbox::new(Bind::bound(snap)).label("Snap to grid").show(ui);
        Checkbox::new(Bind::bound(rulers)).label("Show rulers").show(ui);
    });
}

/// The drive of the shells with a close action: a click on it.
#[doc(hidden)]
pub fn close_box(fixture: &mut Fixture) {
    if let Some(close) = fixture.by_label("Close") {
        fixture.click(&close);
    }
}

#[story(Dialog<'static>, covers = [Default], drive = Drive::Custom(close_box))]
fn dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let shown: StoryOutput = Dialog::new("Grid options").opened(Bind::bound(data.value("open", true))).show(ui, |ui, _| body(ui, &mut checks)).into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(Dialog<'static>, covers = [], drive = Drive::Never)]
fn dialog_with_footer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let shown: StoryOutput = Dialog::new("Grid options")
        .opened(Bind::bound(data.value("open", true)))
        .footer(|ui, _| {
            Checkbox::new(Bind::given(false)).label("Don't ask again").show(ui);
        })
        .show(ui, |ui, _| body(ui, &mut checks))
        .into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(Dialog<'static>, covers = [], drive = Drive::Never)]
fn dialog_resizable_with_initial_focus(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let shown: StoryOutput = Dialog::new("Grid options")
        .opened(Bind::bound(data.value("open", true)))
        .initial_focus(Initial::First)
        .resizable(true)
        .show(ui, |ui, _| body(ui, &mut checks))
        .into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(Dialog<'static>, covers = [], drive = Drive::Never)]
fn dialog_prompt(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let shown: StoryOutput = Dialog::prompt("Discard changes?")
        .opened(Bind::bound(data.value("open", true)))
        .buttons([FooterButton::new("Discard", DialogRole::Destructive).letter('D'), FooterButton::new("Keep editing", DialogRole::Cancel)])
        .show(ui, |ui, _| body(ui, &mut checks))
        .into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(Dialog<'static>, covers = [], drive = Drive::Never)]
fn dialog_modeless(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let resizable = *data.value("resizable", false);
    let shown: StoryOutput = Dialog::new("Grid options")
        .opened(Bind::bound(data.value("open", true)))
        .modeless()
        .resizable(resizable)
        .show(ui, |ui, _| body(ui, &mut checks))
        .into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(DialogFrame<'static>, covers = [Default], drive = Drive::Custom(close_box))]
fn dialog_frame(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let closable = *data.value("close box", true);
    let roomy = *data.value("roomy", false);
    let shown: StoryOutput = DialogFrame::new("Grid options").closable(closable).roomy(roomy).show(ui, |ui, _| body(ui, &mut checks)).into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(DialogFrame<'static>, covers = [], drive = Drive::Never)]
fn dialog_frame_with_footer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let shown: StoryOutput = DialogFrame::new("Grid options")
        .roomy(true)
        .footer(|ui, _| {
            Checkbox::new(Bind::given(false)).label("Don't ask again").show(ui);
        })
        .show(ui, |ui, _| body(ui, &mut checks))
        .into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(BlockingOverlay<'static>, covers = [Default], drive = Drive::Never)]
fn blocking_overlay(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let scrim = *data.value("scrim", true);
    let card = *data.value("wait card", true);
    let mut checks = *data.value("checks", [true, false]);
    let shown: StoryOutput = BlockingOverlay::new("Exporting")
        .opened(Bind::bound(data.value("open", true)))
        .card(card)
        .scrim(scrim)
        .show(ui, |ui, _| body(ui, &mut checks))
        .into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(ModalDragStrip, covers = [Default], drive = Drive::Never)]
fn modal_drag_strip(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = ui.max_rect();
    ModalDragStrip::new(room).show(ui, |_, _| {}).into()
}
