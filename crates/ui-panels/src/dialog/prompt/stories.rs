//! Stories of the prompts.

use super::super::stories::{hosted, opener};
use super::super::{
    Alert, DialogHost, HostedAlert, Prompt, PromptChoice, RecoveredDocument, RecoveryDialog, StackDialog as _, StandardPrompt, UnsavedChangesDialog,
};
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _};
use crate::runtime::env::Env;
use crate::sections::{DialogRole, FooterButton};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use crate::text::NumberField;
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
        FooterButton::new("Don't Save", DialogRole::Alternate).letter('D'),
        FooterButton::new("Cancel", DialogRole::Cancel).letter('C'),
    ]
}

#[story(Prompt<'static>, covers = [], drive = Drive::Never)]
fn prompt(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Prompt::new("Unsaved changes", "Do you want to save the changes you made to “poster.psd”?")
        .answers(answers())
        .opened(data.bind("open", true))
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
        .opened(data.bind("open", true))
        .show(ui)
        .into();
    *data.value("apply", false) = apply_to_all;
    shown
}

/// The answer that takes no input until the prompt has been shown for half a second, behind the button that opens it.
#[story(Prompt<'static>, covers = [], drive = Drive::Never)]
fn prompt_with_an_armed_answer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Prompt::new("Install plug-in", "Do you want to install “Sharpen Pro”?")
        .paragraph("Plug-ins run with the same access as the app.")
        .answers([FooterButton::new("OK", DialogRole::Other).armed_after(0.5), FooterButton::new("Cancel", DialogRole::Cancel)])
        .opened(Bind::bound(opener(ui, data, "Install Plug-in…")))
        .show(ui)
        .into()
}

#[story(Alert<'static>, covers = [], drive = Drive::Never)]
fn alert(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || HostedAlert::new(Alert::new("About", "Version 1.0").tone(Tone::Info, "A newer version is available.")))
}

/// A size field that holds typed values to 1–100 %: a value typed past the range commits the nearest end and opens
/// the hosted alert that says so.
#[story(Alert<'static>, covers = [], drive = Drive::Never)]
fn range_alert(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    // `RangeAlert` is `#[non_exhaustive]` with no constructor outside craft-ui-widgets, so the story raises one
    // through a real field commit instead of building it.
    let mut size = *data.value("size", Some(50.0));
    let field = NumberField::new("Size", Bind::bound(&mut size)).range(1.0..=100.0).unit("%").show(ui);
    *data.value("size", Some(50.0)) = size;
    let host = data.note("host", DialogHost::new());
    if let Some(alert) = &field.out_of_range {
        host.open(HostedAlert::new(Alert::range("Size", alert, &Env::of(ui))));
    }
    host.run(ui);
    match host.get::<HostedAlert>() {
        Some(alert) => alert.story_facts(),
        None => field.into(),
    }
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

/// A link prompt whose safe answer is the default: Enter cancels.
#[story(StandardPrompt, covers = [], drive = Drive::Never)]
fn standard_prompt_default_cancel(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    StandardPrompt::custom("Open the link “https://example.com/report”?")
        .title("Open Link")
        .button(PromptChoice::Proceed, "Open")
        .button(PromptChoice::Cancel, "Cancel")
        .default_choice(PromptChoice::Cancel)
        .show(ui)
        .into()
}

#[story(StandardPrompt, covers = [], drive = Drive::Never)]
fn standard_prompt_changed_on_disk(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    StandardPrompt::changed_on_disk("poster.pdf").show(ui).into()
}

#[story(UnsavedChangesDialog, covers = [], drive = Drive::Never)]
fn unsaved_changes_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let remaining = if *data.value("several_documents", true) { 3 } else { 1 };
    let mut dialog = UnsavedChangesDialog::new(CloseScope::Close, "poster.psd").remaining(remaining);
    if *data.value("save_failed", false) {
        dialog = dialog.reason("Couldn't save “poster.psd”: the disk is full.");
    }
    dialog.show(ui).into()
}

#[story(RecoveryDialog, covers = [], drive = Drive::Never)]
fn recovery_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let age = *data.choice("age", AGES[1].1, &AGES);
    let blocked = *data.value("recover_blocked", false);
    let remaining = if *data.value("several_documents", false) { 3 } else { 1 };
    let protected = *data.value("protected", false);
    let document = RecoveredDocument::new("poster", "poster.psd", age);
    let document = if blocked { document.blocked("Stop recording first") } else { document };
    let document = if protected { document.protected() } else { document };
    RecoveryDialog::new(document).remaining(remaining).show(ui).into()
}
