//! Stories of the notices family.

use super::{
    Notice, NoticeAction, NoticeCenter, NoticeIcon, NoticeLayout, PostedNotice, SaveResult, SettingsLoad, SettingsNoticeState, SettingsNotices,
    ToastStack,
};
use crate::contract::{Politeness, Show as _};
use crate::display::{Shade, Text};
use crate::primitives::IconSpec;
use crate::runtime::env::{Label, WindowKey};
use crate::runtime::toast;
use crate::testing::{Drive, StoryData, StoryOutput, stage, story};
use craft_behavior::IconId;
use craft_theme::variant::Tone;
use egui::Ui;

#[story(Notice<'static>, covers = [Default])]
fn notice(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Notice::new(Tone::Info, "The file will be saved in the new format the next time you save.")
        .title("Format changed")
        .action(NoticeAction::new("Undo"))
        .dismiss()
        .announce(Politeness::Polite)
        .show(ui)
        .into()
}

#[story(Notice<'static>, covers = [], drive = Drive::Never)]
fn notice_warning(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Notice::new(Tone::Warn, "ICO is limited to 256 by 256 pixels.").show(ui).into()
}

#[story(Notice<'static>, covers = [], drive = Drive::Never)]
fn notice_error(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Notice::new(Tone::Bad, "The export failed because the disk is full.").announce(Politeness::Assertive).show(ui).into()
}

#[story(Notice<'static>, covers = [], drive = Drive::Never)]
fn notice_neutral(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Notice::new(Tone::Neutral, "Applies at the next launch.").title("Colour management").show(ui).into()
}

#[story(Notice<'static>, covers = [], drive = Drive::Never)]
fn notice_card(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Notice::new(Tone::Good, "poster.png was saved to Pictures.")
        .layout(NoticeLayout::Card)
        .title("Exported")
        .action(NoticeAction::new("Show in folder"))
        .dismiss()
        .show(ui)
        .into()
}

/// The no-document screen: icon, title, description, two actions and a drop hint under them.
#[story(Notice<'static>, covers = [], drive = Drive::Never)]
fn notice_centred(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Notice::new(Tone::Neutral, "Create a new document or open an existing file.")
        .layout(NoticeLayout::Centred)
        .icon(NoticeIcon::Custom(IconSpec::svg(IconId::new("image"))))
        .title("No document")
        .action(NoticeAction::new("New Document\u{2026}"))
        .action(NoticeAction::new("Open\u{2026}"))
        .content(Text::new("Drop an image here to open it").shade(Shade::Muted), |_| {})
        .show(ui)
        .into()
}

/// An app's start screen: the brand title with its byline, a description, two actions, a drop hint and a footer.
#[story(Notice<'static>, covers = [], drive = Drive::Never)]
fn notice_brand(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Notice::new(Tone::Neutral, "Create a new document or open an existing file.")
        .layout(NoticeLayout::Centred)
        .icon(NoticeIcon::None)
        .title("Sample App")
        .brand("open source")
        .action(NoticeAction::new("New Document\u{2026}"))
        .action(NoticeAction::new("Open\u{2026}"))
        .content(Text::new("Drop an image here to open it").shade(Shade::Muted), |_| {})
        .content(Text::new("No recent files").shade(Shade::Muted), |_| {})
        .show(ui)
        .into()
}

#[story(SettingsNotices, covers = [], drive = Drive::Never)]
fn settings_notices(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut state = SettingsNoticeState::new();
    state.loaded(ui.ctx(), &SettingsLoad::new().reset(vec!["interface.uiScale".to_owned(), "tools.brushSize".to_owned()]));
    for _ in 0..3 {
        state.ticked(ui.ctx(), &SaveResult::Failed("disk full".to_owned()));
    }
    SettingsNotices::new(&state).app("Studio").show(ui);
    Text::new("Preferences").show(ui).into()
}

/// Three notices in the bottom corner of the story's stage: a neutral one, a warning with Don't Show Again and an
/// error. They stay until closed, and come back once all are closed.
#[story(NoticeCenter, covers = [], drive = Drive::Never)]
fn notice_center(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let ctx = ui.ctx().clone();
    let window = WindowKey::new("notice-center-story");
    if toast::cards(&ctx, &window).is_empty() {
        let center = NoticeCenter::new().window(window.clone());
        let _ = center.post(&ctx, &PostedNotice::new("Pasted from the clipboard").sticky(), &[]);
        let warning = PostedNotice::new("Profile mismatch").lines(vec!["The document uses its own profile.".to_owned()]).tone(Tone::Warn);
        let _ = center.post(&ctx, &warning.dismiss_key("profile-mismatch".to_owned()).sticky(), &[]);
        let error = PostedNotice::new("Couldn't save the document").lines(vec!["The disk is full.".to_owned()]).tone(Tone::Bad);
        let _ = center.post(&ctx, &error.sticky(), &[]);
    }
    let area = stage(ui);
    ToastStack::show(ui, &window, area);
    NoticeCenter::new().window(window).show(ui).into()
}

/// A neutral toast, a warning with ten lines and an error in the bottom corner of the story's stage.
#[story(ToastStack, covers = [], drive = Drive::Never)]
fn toast_stack(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let ctx = ui.ctx().clone();
    let window = WindowKey::new("toast-stack-story");
    if toast::cards(&ctx, &window).is_empty() {
        let lines = (1..=10).map(|n| format!("Warning {n}"));
        toast::post(&ctx, &window, toast::Toast::new(Label::Empty).title("Pasted from the clipboard").timeout(None));
        toast::post(&ctx, &window, toast::Toast::new(Label::Empty).title("Opened with 10 warnings").lines(lines).tone(Tone::Warn).timeout(None));
        let error = toast::Toast::new(Label::Empty).title("Couldn't save the document").lines(["The disk is full."]).tone(Tone::Bad);
        toast::post(&ctx, &window, error.timeout(None));
    }
    let area = stage(ui);
    ToastStack::show(ui, &window, area);
    Text::new("Toasts").show(ui).into()
}
