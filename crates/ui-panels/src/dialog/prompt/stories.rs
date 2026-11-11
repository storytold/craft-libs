//! Stories of the prompts.

use super::super::{Alert, Prompt, RecoveredDocument, RecoveryDialog, StandardPrompt, UnsavedChangesDialog};
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _};
use crate::sections::{DialogRole, FooterButton};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_quit::CloseScope;
use craft_theme::variant::Tone;
use egui::Ui;
use std::time::Duration;

/// How long before the recovery folder was listed the document was autosaved, from a few seconds to days.
const AGES: [(&str, Duration); 4] = [
    ("Seconds", Duration::from_secs(20)),
    ("Minutes", Duration::from_secs(5 * 60)),
    ("Hours", Duration::from_secs(3 * 3_600)),
    ("Days", Duration::from_secs(2 * 86_400)),
];

fn answers() -> [FooterButton; 3] {
    [
        FooterButton::new("Save", DialogRole::Accept).letter('S'),
        FooterButton::new("Don't Save", DialogRole::Other).letter('D'),
        FooterButton::new("Cancel", DialogRole::Cancel).letter('C'),
    ]
}

#[story(Prompt<'static>, covers = [], drive = Drive::Never)]
fn prompt(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Prompt::new("Unsaved changes", "Do you want to save the changes you made to “poster.psd”?")
        .answers(answers())
        .opened(Bind::bound(data.value("open", true)))
        .show(ui)
        .into()
}

#[story(Prompt<'static>, covers = [], drive = Drive::Never)]
fn prompt_with_extras(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut apply_to_all = *data.value("apply", false);
    let shown = Prompt::new("Unsaved changes", "Do you want to save the changes you made to “poster.psd”?")
        .answers(answers())
        .tone(Tone::Warn, "Two more documents have unsaved changes.")
        .lines(["Layer 3 has a missing font".into(), "Layer 7 has a missing profile".into()])
        .details("Details", "The file was last saved by version 2.1.")
        .extras(|ui| {
            Checkbox::new(Bind::bound(&mut apply_to_all)).label("Apply to All").show(ui);
        })
        .opened(Bind::bound(data.value("open", true)))
        .show(ui)
        .into();
    *data.value("apply", false) = apply_to_all;
    shown
}

#[story(Alert<'static>, covers = [], drive = Drive::Never)]
fn alert(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Alert::new("About", "Version 1.0").tone(Tone::Info, "A newer version is available.").opened(Bind::bound(data.value("open", true))).show(ui).into()
}

#[story(StandardPrompt, covers = [], drive = Drive::Never)]
fn standard_prompt(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let prompt = if *data.value("several_documents", true) {
        StandardPrompt::confirm_delete(["Background", "Layer 1"], 2)
    } else {
        StandardPrompt::confirm_delete(["Background"], 1)
    };
    prompt.dont_show_again().show(ui).into()
}

#[story(StandardPrompt, covers = [], drive = Drive::Never)]
fn standard_prompt_without_apply(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let alternate_form = *data.value("alternate_form", true);
    StandardPrompt::trash_mask().can_apply(!alternate_form).show(ui).into()
}

#[story(UnsavedChangesDialog, covers = [], drive = Drive::Never)]
fn unsaved_changes_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let remaining = if *data.value("several_documents", true) { 3 } else { 1 };
    UnsavedChangesDialog::new(CloseScope::Close, "poster.psd").remaining(remaining).show(ui).into()
}

#[story(RecoveryDialog, covers = [], drive = Drive::Never)]
fn recovery_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let age = *data.choice("age", AGES[1].1, &AGES);
    let blocked = *data.value("recover_blocked", true);
    let document = RecoveredDocument::new("poster", "poster.psd", age);
    RecoveryDialog::new(if blocked { document.blocked("Stop recording first") } else { document }).show(ui).into()
}
