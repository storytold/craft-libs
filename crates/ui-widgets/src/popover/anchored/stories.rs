//! Stories of `Popover`.

use super::Popover;
use crate::buttons::Button;
use crate::contract::{Bind, Show as _, ShowContainer as _};
use crate::layout_components::Stack;
use crate::runtime::focus::Initial;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// What the story popovers hold: two buttons; whether one was clicked.
fn body(ui: &mut Ui) -> bool {
    Stack::new()
        .show(ui, |ui, _| {
            let snap = Button::new("Snap to grid").show(ui);
            let rulers = Button::new("Show rulers").show(ui);
            crate::contract::clicked(&snap) || crate::contract::clicked(&rulers)
        })
        .inner
        .unwrap_or(false)
}

/// Shows `story` in a row as wide as the room: the opener sits at the start edge and the popup hangs from it, so the
/// popup's place mirrors with the direction.
fn in_row(ui: &mut Ui, mut story: impl FnMut(&mut Ui) -> StoryOutput) -> StoryOutput {
    let shown = Stack::new().fill().show(ui, |ui, _| story(ui));
    match shown.inner {
        Some(output) => output,
        None => shown.output.into(),
    }
}

fn popover_with(ui: &mut Ui, data: &mut StoryData, build: impl Fn(Popover<'_>) -> Popover<'_>) -> StoryOutput {
    in_row(ui, |ui| {
        let opener = Button::new("Grid").show(ui);
        let mut clicked = false;
        let shown = build(Popover::new("Grid options", Bind::bound(data.value("open", true))).opened_by(&opener)).show(ui, |ui, _| {
            clicked = body(ui);
        });
        let mut output = StoryOutput::from(shown.output);
        output.clicked |= clicked;
        output
    })
}

#[story(Popover<'static>, covers = [Default], drive = Drive::Never)]
fn popover(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let resizable = *data.value("resizable", false);
    let enter = *data.value("enter", false);
    popover_with(ui, data, |popover| popover.resizable(resizable).close_on_enter(enter))
}

#[story(Popover<'static>, covers = [], drive = Drive::Never)]
fn popover_resizable_closing_on_enter(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    popover_with(ui, data, |popover| popover.resizable(true).close_on_enter(true).initial_focus(Initial::First))
}
