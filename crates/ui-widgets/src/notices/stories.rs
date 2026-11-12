//! Stories of the notices family.

use super::{
    Notice, NoticeAction, NoticeCenter, NoticeIcon, NoticeLayout, PostedNotice, SaveResult, SettingsLoad, SettingsNoticeState, SettingsNotices,
    ToastStack,
};
use crate::contract::{Politeness, Show as _};
use crate::display::{Shade, Text};
use crate::primitives::IconSpec;
use crate::runtime::toast;
use crate::testing::{Drive, StoryData, StoryOutput, story};
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

#[story(NoticeCenter, covers = [], drive = Drive::Never)]
fn notice_center(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let ctx = ui.ctx().clone();
    if toast::cards(&ctx).is_empty() {
        let notice = PostedNotice::new("Exported").lines(vec!["poster.png".to_owned()]).tone(Tone::Good);
        NoticeCenter::new().post(&ctx, &notice, &[]);
    }
    NoticeCenter::new().show(ui);
    Text::new("Notices").show(ui).into()
}

#[story(ToastStack, covers = [], drive = Drive::Never)]
fn toast_stack(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let ctx = ui.ctx().clone();
    if toast::cards(&ctx).is_empty() {
        toast::post(&ctx, toast::Toast::new("poster.png was saved to Pictures.").tone(Tone::Good).title("Exported"));
    }
    ToastStack::show(ui);
    Text::new("Toasts").show(ui).into()
}
