//! `SettingsNotices` (20.3): what the user is told when settings can't be saved or couldn't be read. The count of
//! failed saves and the messages are a value the app owns (`SettingsNoticeState`); the pattern shows the notices
//! that value calls for.

use super::notice::Notice;
use super::words::NoticeWords;
use crate::contract::{NoValue, Outcome, Politeness, Props as _, Show as _};
use crate::runtime::env::{Env, Label};
use crate::runtime::frame;
use craft_theme::variant::Tone;
use egui::{Context, Response, Ui};

/// Failed saves in a row before the "can't be saved" notice shows beside the status line.
pub const SAVE_NOTICE_AFTER: u32 = 3;

/// What the settings store reported when it loaded.
#[derive(Clone, Debug, Default, PartialEq, crate::contract::Props)]
#[non_exhaustive]
pub struct SettingsLoad {
    /// The settings whose stored values did not fit their type and were reset, by path.
    #[prop(into)]
    reset: Vec<String>,
    /// Where an unreadable file was moved.
    #[prop(into)]
    moved_aside: Option<String>,
    /// Why the store will not save: a newer version's file, a failed migration.
    #[prop(into)]
    read_only: Option<String>,
}

impl SettingsLoad {
    /// A load that went without problems.
    pub fn new() -> Self {
        SettingsLoad::default()
    }
}

/// The outcome of one save attempt that did something.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SaveResult {
    /// The save failed; the text is the cause.
    Failed(String),
    /// The save wrote the settings.
    Saved,
}

/// A message for the status bar, with its tone.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SettingsStatus {
    /// A save failed; the text is the cause.
    SaveFailed(String),
    /// A save worked after failures.
    Saved,
    /// The unreadable file was kept at this path.
    MovedAside(String),
    /// The store will not save, for this reason.
    ReadOnly(String),
}

impl SettingsStatus {
    /// How the message is drawn: good, warning or bad.
    pub fn tone(&self) -> Tone {
        match self {
            SettingsStatus::SaveFailed(_) => Tone::Bad,
            SettingsStatus::Saved => Tone::Good,
            SettingsStatus::MovedAside(_) | SettingsStatus::ReadOnly(_) => Tone::Warn,
        }
    }

    /// The message in `words`' language.
    pub fn text(&self, words: &NoticeWords) -> String {
        match self {
            SettingsStatus::SaveFailed(error) => words.save_failed.fill(&[("error", error.as_str())]),
            SettingsStatus::Saved => words.saved.as_str().to_owned(),
            SettingsStatus::MovedAside(path) => words.moved_aside.fill(&[("path", path.as_str())]),
            SettingsStatus::ReadOnly(reason) => words.read_only.fill(&[("reason", reason.as_str())]),
        }
    }
}

/// The settings messages of one app run, owned by the app.
///
/// - **Load** (`loaded`): settings that did not fit their type were reset, and a notice names them. A file that
///   did not parse was moved aside, and a read-only store says so, both in the status line.
/// - **Save** (`ticked`, with every save attempt that did something): the first failure puts a message in the
///   status line; the `SAVE_NOTICE_AFTER`th failure in a row adds the "can't be saved" notice; the next success
///   says "saved" and removes the notices.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SettingsNoticeState {
    failures: u32,
    status: Option<SettingsStatus>,
    cant_save: Option<String>,
    reset: Vec<String>,
}

impl SettingsNoticeState {
    /// No failures and no messages.
    pub fn new() -> Self {
        SettingsNoticeState::default()
    }

    /// Reads the store's load report once at startup and asks `ctx` for a pass to show what it found.
    pub fn loaded(&mut self, ctx: &Context, load: &SettingsLoad) {
        frame::request_repaint(ctx);
        if !load.reset.is_empty() {
            self.reset.clone_from(&load.reset);
        }
        if let Some(path) = &load.moved_aside {
            self.status = Some(SettingsStatus::MovedAside(path.clone()));
        }
        if let Some(reason) = &load.read_only {
            self.status = Some(SettingsStatus::ReadOnly(reason.clone()));
        }
    }

    /// Counts failed saves and clears them on success, and asks `ctx` for a pass to show the change.
    pub fn ticked(&mut self, ctx: &Context, result: &SaveResult) {
        frame::request_repaint(ctx);
        match result {
            SaveResult::Failed(error) => {
                self.failures = self.failures.saturating_add(1);
                if self.failures == 1 {
                    self.status = Some(SettingsStatus::SaveFailed(error.clone()));
                } else if self.failures == SAVE_NOTICE_AFTER {
                    self.cant_save = Some(error.clone());
                }
            }
            SaveResult::Saved => {
                if self.failures > 0 {
                    self.status = Some(SettingsStatus::Saved);
                }
                self.failures = 0;
                self.cant_save = None;
                self.reset.clear();
            }
        }
    }

    /// The status message to show, once.
    pub fn take_status(&mut self) -> Option<SettingsStatus> {
        self.status.take()
    }

    /// Whether a notice is up.
    pub fn has_notice(&self) -> bool {
        self.cant_save.is_some() || !self.reset.is_empty()
    }
}

/// The notices a `SettingsNoticeState` calls for, newest problem first: an error notice when saving keeps failing
/// and a warning when stored settings were reset. Nothing shows when there are none. The notices cannot be
/// dismissed; they go when the problem does.
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct SettingsNotices {
    /// The app's name for the messages; "The app" without one.
    #[prop(into)]
    app: Label,
    cant_save: Option<String>,
    reset: Vec<String>,
}

impl SettingsNotices {
    /// The notices for `state`.
    pub fn new(state: &SettingsNoticeState) -> Self {
        SettingsNotices { app: Label::Empty, cant_save: state.cant_save.clone(), reset: state.reset.clone() }
    }

    /// Shows the notices in the flow of `ui`.
    pub fn show(self, ui: &mut Ui) -> SettingsNoticesOutput {
        let env = Env::of(ui);
        let words = env.words::<NoticeWords>();
        let app = match &self.app {
            Label::Empty => words.app_fallback.as_str().to_owned(),
            label => env.view().resolve(label).as_str().to_owned(),
        };
        if let Some(error) = &self.cant_save {
            let body = words.cant_save_body.fill(&[("app", app.as_str())]);
            Notice::new(Tone::Bad, format!("{error}\n{body}"))
                .title(words.cant_save_title.clone())
                .announce(Politeness::Assertive)
                .id_salt("settings-cant-save")
                .show(ui);
        }
        if !self.reset.is_empty() {
            let paths = self.reset.join(", ");
            let body = words.reset_body.fill(&[("app", app.as_str()), ("paths", paths.as_str())]);
            Notice::new(Tone::Warn, body).title(words.reset_title.clone()).announce(Politeness::Polite).id_salt("settings-reset").show(ui);
        }
        SettingsNoticesOutput { response: ui.response() }
    }
}

/// What `SettingsNotices::show` reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct SettingsNoticesOutput {
    /// The response of the `Ui` the notices were shown in.
    pub response: Response,
}

impl Outcome for SettingsNoticesOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}
