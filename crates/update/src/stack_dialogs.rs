//! The update dialogs on craft-ui's dialog host: [`UpdatePromptDialog`] (a release is available)
//! and [`UpdateCheckDialog`] (the result of a manual check), on [`Dialog`], with the app's name from the
//! host's `AppInfo` and the words from [`UpdateWords`].

use std::hash::{Hash as _, Hasher as _};
use std::sync::OnceLock;

use craft_theme::variant::Tone;
use craft_ui::buttons::Link;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::dialog::{Dialog, DialogId, DialogOutput, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::display::{Picture, PictureSize, Shade, Spinner, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::notices::Notice;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::focus::Initial;
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use egui::{ColorImage, Context, Ui};

use crate::app::App;
use crate::release::{Release, safe_web_link};
use crate::self_update;
use crate::updater::Status;

#[cfg(feature = "screen-testing")]
mod stories;

const PROMPT_KEY: &str = "updateDialog.prompt";
const CHECK_KEY: &str = "updateDialog.check";

/// The largest icon the dialogs decode, a side; app icons are 128 px.
const ICON_MAX_SIDE: u32 = 1024;

craft_ui::words! {
    /// The update dialogs' words, English by default.
    #[non_exhaustive]
    pub struct UpdateWords {
        available_title: "updateDialog.availableTitle" = "Update available";
        available_message: template "updateDialog.availableMessage" = "{app} {version} is available. You have {running}.";
        update_and_restart: "updateDialog.updateAndRestart" = "Update and Restart";
        /// The prompt's and the check dialog's button once an update is ready and waits for the app to close.
        restart_to_update: "updateDialog.restartToUpdate" = "Restart to Update";
        download: "updateDialog.download" = "Download";
        later: "updateDialog.later" = "Later";
        whats_new: "updateDialog.whatsNew" = "What's New";
        releases_listed: "updateDialog.releasesListed" = "All releases are listed at";
        version_line: template "updateDialog.versionLine" = "v{running} - {version}";
        by_developer: template "updateDialog.byDeveloper" = "by {developer}";
        installing: "updateDialog.installing" = "Installing…";
        check_title: "updateDialog.checkTitle" = "Check for Updates";
        checking: "updateDialog.checking" = "Checking for updates…";
        up_to_date: template "updateDialog.upToDate" = "{app} {running} is the newest version.";
        failed: "updateDialog.failed" = "The update check failed.";
        check_again: "updateDialog.checkAgain" = "Check Again";
        check_now: "updateDialog.checkNow" = "Check Now";
        not_checked: "updateDialog.notChecked" = "Updates have not been checked yet.";
        cannot_auto_update: "updateDialog.cannotAutoUpdate" = "Cannot auto-update this installation type. Please download manually.";
        /// The check dialog's link to the release notes in its available state.
        release_notes: "updateDialog.releaseNotes" = "Release Notes";
        /// A ready update on Windows, which installs after the app exits ([`crate::updater::READY_TEXT`]).
        ready_after_exit: template "updateDialog.readyAfterExit" = "Update ready. Close {app} to install it.";
        /// A ready update on macOS and Linux, already in place ([`crate::updater::READY_TEXT`]).
        ready_installed: template "updateDialog.readyInstalled" = "The update is installed. Close {app} to finish; it restarts by itself.";
        managed: template "updateDialog.managed" = "{app} {running} follows the version chosen in the launcher, so it does not update itself.";
        updated: template "updateDialog.updated" = "{app} updated to {version}.";
        install_failed: template "updateDialog.installFailed" = "The update could not be installed: {reason}";
    }
}

/// What the user chose in the update prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UpdateChoice {
    /// Update and Restart: the default where the app can update itself. Once the update is ready the
    /// button reads Restart to Update and asks the app to close.
    UpdateAndRestart,
    /// Open the release page in the browser (the default elsewhere).
    Download,
    /// Later: close the prompt for this run.
    Later,
}

/// A PNG app icon as an RGBA image, or `None` when it does not decode or is over 1024 px a side.
pub fn icon_image(png: &[u8]) -> Option<ColorImage> {
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(ICON_MAX_SIDE);
    limits.max_image_height = Some(ICON_MAX_SIDE);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.to_rgba8();
    let size = [usize::try_from(rgba.width()).ok()?, usize::try_from(rgba.height()).ok()?];
    Some(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

/// The app's decoded icon, kept for the life of the dialog; `None` inside when it has none or it does not decode.
type IconCell = OnceLock<Option<ColorImage>>;

/// The app the dialogs are for: the installed host's name and the suite entry that carries its icon and
/// developer.
struct Subject {
    name: &'static str,
    app: Option<&'static App>,
}

impl Subject {
    fn of(env: &Env) -> Self {
        let info = env.view().host().map(|host| host.app().clone());
        Self { name: info.as_ref().map_or("", |info| info.name), app: info.and_then(|info| App::by_id(info.id)) }
    }

    /// The icon, the name, the developer and `version_line`, as the head of both dialogs. An app without a
    /// decodable icon shows its initial in the icon's place.
    fn head(&self, ui: &mut Ui, words: &UpdateWords, icon: &IconCell, version_line: &str) {
        let image = icon.get_or_init(|| self.app.and_then(|app| app.icon_png).and_then(icon_image)).as_ref();
        Row::new().show(ui, |ui, _| {
            match (image, self.app) {
                (Some(image), Some(app)) => {
                    Picture::new("", icon_key(app), &|| image.clone()).extent(PictureSize::Icon).show(ui);
                }
                _ => {
                    Text::new(self.name.chars().next().map_or_else(|| "A".to_owned(), String::from)).show(ui);
                }
            }
            Stack::new().show(ui, |ui, _| {
                Text::new(self.name).show(ui);
                if let Some(app) = self.app {
                    Text::new(words.by_developer.fill(&[("developer", app.developer)])).shade(Shade::Muted).show(ui);
                }
                Text::new(version_line.to_owned()).shade(Shade::Muted).show(ui);
            });
        });
    }
}

/// The picture key of `app`'s icon.
fn icon_key(app: &App) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    app.id.hash(&mut hasher);
    hasher.finish()
}

/// The spinner with the progress word, shown while an install runs.
fn busy(ui: &mut Ui, label: &Label) {
    Row::new().show(ui, |ui, _| {
        Spinner::new().show(ui);
        Text::new(label.clone()).show(ui);
    });
}

/// "Update available", on the startup queue at [`StartupStage::Update`](craft_ui::dialog::StartupStage::Update)
/// so it never overlaps the GPU warning or recovery, or opened by a manual check that found a release. The user
/// sees the app's icon and name, the developer, the new and running versions, the release notes link, and Update
/// and Restart / Download / Later in the platform order; while an install runs the install buttons are disabled
/// and a status line shows. Enter takes the default, Esc is Later. Update and Restart keeps the prompt open to show the install's progress
/// ([`take_update_request`](UpdatePromptDialog::take_update_request)). Later and Esc stay available while the install
/// runs. Result: [`UpdateChoice`].
#[derive(craft_ui::contract::Pattern)]
pub struct UpdatePromptDialog {
    id: DialogId,
    release: Release,
    running_version: String,
    self_updating: bool,
    status: Option<String>,
    installing: bool,
    /// The update is ready and waits for the app to close: the main button reads Restart to Update.
    ready: bool,
    open: bool,
    /// Update and Restart was picked; the dialog stays open to show the install's progress.
    requested: bool,
    icon: IconCell,
}

impl UpdatePromptDialog {
    /// A prompt for `release`; `self_updating` offers Update and Restart as the default.
    pub fn new(release: Release, running_version: impl Into<String>, self_updating: bool) -> Self {
        Self {
            id: dialog_id(PROMPT_KEY),
            release,
            running_version: running_version.into(),
            self_updating,
            status: None,
            installing: false,
            ready: false,
            open: true,
            requested: false,
            icon: IconCell::new(),
        }
    }

    /// Whether Update and Restart was picked since the last call. The dialog stays open on that pick, so the
    /// updater asks for it through `DialogHost::get_mut_by_id`.
    pub fn take_update_request(&mut self) -> bool {
        std::mem::take(&mut self.requested)
    }

    /// The install's progress line while [`installing`](UpdatePromptDialog::set_installing); the updater sets it
    /// through `DialogHost::get_mut_by_id`.
    pub fn set_status(&mut self, status: Option<String>) {
        self.status = status;
    }

    /// Disables the buttons while the installer runs.
    pub fn set_installing(&mut self, installing: bool) {
        self.installing = installing;
    }

    /// Shows the ready state: the main button reads Restart to Update and answers
    /// [`UpdateChoice::UpdateAndRestart`], which asks the app to close.
    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
    }

    /// Closes the prompt on the next pass with [`UpdateChoice::Later`], for a prompt the updater withdraws.
    pub fn withdraw(&mut self) {
        self.open = false;
    }

    /// The release the dialog offers.
    pub fn release(&self) -> &Release {
        &self.release
    }

    /// Shows one pass. The choices are the footer buttons' meanings, in the order of the dialog's `picked`.
    fn present(&mut self, ui: &mut Ui) -> (DialogOutput, Vec<UpdateChoice>) {
        let env = Env::of(ui);
        let words = env.words::<UpdateWords>();
        let subject = Subject::of(&env);
        let version = self.release.version_label();
        let message = words.available_message.fill(&[("app", subject.name), ("version", &version), ("running", &self.running_version)]);
        let version_line = words.version_line.fill(&[("running", &self.running_version), ("version", &version)]);
        let enabled = !self.installing;
        let mut choices = Vec::new();
        let mut buttons = Vec::new();
        if self.self_updating || self.ready {
            let label = if self.ready { words.restart_to_update.clone() } else { words.update_and_restart.clone() };
            choices.push(UpdateChoice::UpdateAndRestart);
            buttons.push(FooterButton::new(label, DialogRole::Accept).enabled(enabled).stays_open(true));
            choices.push(UpdateChoice::Download);
            buttons.push(FooterButton::new(words.download.clone(), DialogRole::Other).enabled(enabled));
        } else {
            choices.push(UpdateChoice::Download);
            buttons.push(FooterButton::new(words.download.clone(), DialogRole::Accept).enabled(enabled));
        }
        choices.push(UpdateChoice::Later);
        buttons.push(FooterButton::new(words.later.clone(), DialogRole::Cancel));
        // A blocked default button leaves the focus off the buttons: landing on another one would let Enter
        // answer with it.
        let initial = if enabled { Initial::Default } else { Initial::Keep };
        let (status, installing, release, icon) = (&self.status, self.installing, &self.release, &self.icon);
        let installing_label = words.installing.clone();
        let shown = Dialog::prompt(words.available_title.clone())
            .buttons(buttons)
            .initial_focus(initial)
            .close_on_enter(enabled)
            .opened(Bind::bound(&mut self.open))
            .show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    subject.head(ui, &words, icon, &version_line);
                    if let Some(status) = status {
                        Notice::new(Tone::Warn, status.clone()).show(ui);
                    }
                    if installing {
                        busy(ui, &installing_label);
                    } else {
                        Text::new(message.clone()).show(ui);
                    }
                    if safe_web_link(&release.html_url) {
                        Link::new(words.whats_new.clone()).url(release.html_url.clone()).show(ui);
                    }
                });
            });
        (shown.output, choices)
    }
}

impl StackDialog for UpdatePromptDialog {
    type Result = UpdateChoice;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<UpdateChoice>> {
        let (output, choices) = self.present(ui);
        if output.closed.is_none() {
            self.requested |= output.picked.and_then(|at| choices.get(at)) == Some(&UpdateChoice::UpdateAndRestart);
            return None;
        }
        Some(DialogResult::Ok(output.picked.and_then(|at| choices.get(at).copied()).unwrap_or(UpdateChoice::Later)))
    }
}

/// What the user chose in the check-result dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CheckChoice {
    /// Close the dialog.
    Close,
    /// Not-checked state only: run the first check.
    CheckNow,
    /// Not-checked, up-to-date, managed and failed states and the available state: run another check.
    CheckAgain,
    /// Ready state only: the app calls `Updater::restart_to_update`, which asks it to close.
    RestartToUpdate,
    /// Available state only: install the release and restart (the default where the install can
    /// update itself); the app calls `Updater::request_install`, which works for any release the updater knows of.
    Update,
    /// Available state only: open the release page in the browser (the default where the install
    /// can't update itself).
    Download,
}

/// The result of a manual "Check for Updates": "not checked yet" with Check Now before the first
/// check, a spinner while checking, "newest version" when up to date, the launcher's choice when
/// it manages the version, and the error when it failed, each with OK and Check Again; an update
/// installing (OK), or ready and waiting for the app to close (OK and Restart to Update). It also
/// shows what the last update came to at the first start after it: a failure with Check Again,
/// or "updated to" as a modeless notice. The app opens it when the user starts a check, or when
/// `Updater::status` is not idle after the check at start, and feeds [`set_status`](UpdateCheckDialog::set_status) each pass; an available release
/// closes it and opens an [`UpdatePromptDialog`] instead, unless the app calls
/// [`set_available`](UpdateCheckDialog::set_available): the dialog then shows the "update
/// available" state itself, with Update and Restart (the default), a Release Notes link and Check Again
/// (while [`set_installing`](UpdateCheckDialog::set_installing) the install buttons are disabled
/// and the progress line shows). Where the install can't update itself the dialog says so from
/// the start and Download replaces Update and Restart. Enter takes the default; Esc and the
/// close action close it. A host answers with [`set_choice`](UpdateCheckDialog::set_choice) and
/// `DialogHost::confirm_by_id`. Result: [`CheckChoice`].
#[derive(craft_ui::contract::Pattern)]
pub struct UpdateCheckDialog {
    id: DialogId,
    running_version: String,
    status: Status,
    available: Option<Release>,
    /// Whether this install can update itself: what the app set, else craft-update's detection for the host's
    /// app, made on the first pass.
    self_updating: Option<bool>,
    /// Whether the available release has a file this install can update from
    /// ([`self_update::release_installs`]), as of the last pass.
    release_installs: bool,
    installing: bool,
    message: Option<String>,
    /// What a programmatic answer gives; the state's default button's until set.
    chosen: Option<CheckChoice>,
    open: bool,
    icon: IconCell,
}

impl UpdateCheckDialog {
    /// A dialog in the checking state for `running_version`.
    pub fn new(running_version: impl Into<String>) -> Self {
        Self {
            id: dialog_id(CHECK_KEY),
            running_version: running_version.into(),
            status: Status::Checking,
            available: None,
            self_updating: None,
            release_installs: true,
            installing: false,
            message: None,
            chosen: None,
            open: true,
            icon: IconCell::new(),
        }
    }

    /// Sets whether the install can update itself, for an app that knows better than
    /// [`self_update::is_self_updating`] for the host's app (the dialog asks that when this isn't
    /// called). Without it Download replaces Update and Restart.
    #[must_use]
    pub fn with_self_updating(mut self, self_updating: bool) -> Self {
        self.self_updating = Some(self_updating);
        self
    }

    /// The updater's status this pass.
    pub fn set_status(&mut self, status: Status) {
        self.status = status;
    }

    /// Shows the "update available" state for `release`; `None` returns to the state of the
    /// [`status`](UpdateCheckDialog::set_status).
    pub fn set_available(&mut self, release: Option<Release>) {
        self.available = release;
    }

    /// Disables the install buttons while the installer runs (`Updater::is_installing`).
    pub fn set_installing(&mut self, installing: bool) {
        self.installing = installing;
    }

    /// The install's progress or error line in the available state (`Updater::status_message`).
    pub fn set_message(&mut self, message: Option<String>) {
        self.message = message;
    }

    /// The answer a host-side `DialogHost::confirm_by_id` gives, as pressing that button would;
    /// the state's default button until set. A choice the state doesn't offer, and an install
    /// choice while installing, give no answer.
    pub fn set_choice(&mut self, choice: CheckChoice) {
        self.chosen = Some(choice);
    }

    /// The release page the available state links to, when it is a safe web link.
    fn notes_url(&self) -> Option<&str> {
        self.available.as_ref().map(|release| release.html_url.as_str()).filter(|url| safe_web_link(url))
    }

    /// The choices the current state offers with their roles, the default first.
    fn offered(&self, self_updating: bool) -> Vec<(CheckChoice, DialogRole)> {
        if self.available.is_some() {
            let install = if self_updating { CheckChoice::Update } else { CheckChoice::Download };
            return vec![(install, DialogRole::Accept), (CheckChoice::CheckAgain, DialogRole::Other)];
        }
        match self.status {
            Status::Idle => vec![(CheckChoice::CheckNow, DialogRole::Accept)],
            Status::Checking | Status::Installing | Status::Updated(_) => vec![(CheckChoice::Close, DialogRole::Accept)],
            Status::Ready => vec![(CheckChoice::Close, DialogRole::Accept), (CheckChoice::RestartToUpdate, DialogRole::Other)],
            Status::UpToDate | Status::Managed | Status::Failed(_) | Status::InstallFailed(_) => {
                vec![(CheckChoice::Close, DialogRole::Accept), (CheckChoice::CheckAgain, DialogRole::Other)]
            }
        }
    }

    /// Whether `choice` can be answered now: the available state's buttons wait for a running install.
    fn answerable(&self, choice: CheckChoice) -> bool {
        !(self.installing && self.available.is_some() && matches!(choice, CheckChoice::Update | CheckChoice::Download | CheckChoice::CheckAgain))
    }

    /// Shows one pass. The choices are the footer buttons' meanings, in the order of the dialog's `picked`.
    fn present(&mut self, ui: &mut Ui) -> (DialogOutput, Vec<CheckChoice>) {
        let env = Env::of(ui);
        let words = env.words::<UpdateWords>();
        let ok = env.words::<CommonWords>().ok.clone();
        let subject = Subject::of(&env);
        let self_updating = *self.self_updating.get_or_insert_with(|| subject.app.is_some_and(self_update::is_self_updating));
        self.release_installs = match (subject.app, &self.available) {
            (Some(app), Some(release)) => self_update::release_installs(app, release),
            _ => true,
        };
        let self_updating = self_updating && self.release_installs;
        let offered = self.offered(self_updating);
        let choices: Vec<CheckChoice> = offered.iter().map(|(choice, _)| *choice).collect();
        let buttons: Vec<FooterButton> = offered
            .iter()
            .map(|(choice, role)| {
                let label: Label = match choice {
                    CheckChoice::Close => ok.clone(),
                    CheckChoice::CheckNow => words.check_now.clone(),
                    CheckChoice::CheckAgain => words.check_again.clone(),
                    CheckChoice::Update => words.update_and_restart.clone(),
                    CheckChoice::Download => words.download.clone(),
                    CheckChoice::RestartToUpdate => words.restart_to_update.clone(),
                };
                FooterButton::new(label, *role).enabled(self.answerable(*choice))
            })
            .collect();
        let enter = offered.first().is_some_and(|(choice, _)| self.answerable(*choice));
        let initial = if enter { Initial::Default } else { Initial::Keep };
        let notes = self.notes_url().map(str::to_owned);
        let version = self.available.as_ref().map(Release::version_label);
        let (status, running, message, installing, icon) = (&self.status, &self.running_version, &self.message, self.installing, &self.icon);
        let installing_label = words.installing.clone();
        // What the last update came to is a notice that leaves the app usable behind it.
        let dialog = Dialog::prompt(words.check_title.clone());
        let dialog = if matches!(status, Status::Updated(_)) { dialog.modeless() } else { dialog };
        let shown = dialog.buttons(buttons).initial_focus(initial).close_on_enter(enter).opened(Bind::bound(&mut self.open)).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                if let Some(version) = &version {
                    let version_line = words.version_line.fill(&[("running", running), ("version", version)]);
                    subject.head(ui, &words, icon, &version_line);
                    if !self_updating {
                        Notice::new(Tone::Warn, words.cannot_auto_update.clone()).show(ui);
                    }
                    if let Some(message) = message {
                        Notice::new(Tone::Warn, message.clone()).show(ui);
                    }
                    if installing {
                        busy(ui, &installing_label);
                    } else {
                        let text = words.available_message.fill(&[("app", subject.name), ("version", version), ("running", running)]);
                        Text::new(text).show(ui);
                    }
                    if let Some(url) = &notes {
                        Link::new(words.release_notes.clone()).url(url.clone()).show(ui);
                    }
                    return;
                }
                subject.head(ui, &words, icon, &format!("v{running}"));
                match status {
                    Status::Checking => busy(ui, &words.checking.clone()),
                    Status::UpToDate => {
                        Text::new(words.up_to_date.fill(&[("app", subject.name), ("running", running)])).show(ui);
                    }
                    Status::Failed(error) => {
                        Notice::new(Tone::Bad, error.clone()).title(words.failed.clone()).show(ui);
                        if let Some(app) = subject.app {
                            let url = app.releases_url();
                            Row::new().wrap(true).show(ui, |ui, _| {
                                Text::new(words.releases_listed.clone()).shade(Shade::Muted).show(ui);
                                Link::new(url.clone()).url(url.clone()).show(ui);
                            });
                        }
                    }
                    Status::Idle => {
                        Text::new(words.not_checked.clone()).show(ui);
                    }
                    Status::Installing => busy(ui, &installing_label),
                    Status::Ready => {
                        let ready = if cfg!(windows) { &words.ready_after_exit } else { &words.ready_installed };
                        Text::new(ready.fill(&[("app", subject.name)])).show(ui);
                    }
                    Status::Managed => {
                        Text::new(words.managed.fill(&[("app", subject.name), ("running", running)])).shade(Shade::Muted).show(ui);
                    }
                    Status::Updated(version) => {
                        Text::new(words.updated.fill(&[("app", subject.name), ("version", version)])).show(ui);
                    }
                    Status::InstallFailed(reason) => {
                        Notice::new(Tone::Bad, words.install_failed.fill(&[("reason", reason)])).show(ui);
                    }
                }
            });
        });
        (shown.output, choices)
    }
}

impl StackDialog for UpdateCheckDialog {
    type Result = CheckChoice;

    fn id(&self) -> DialogId {
        self.id
    }

    /// Answers as pressing the chosen button would. A dialog not yet drawn and not told by
    /// [`with_self_updating`](UpdateCheckDialog::with_self_updating) answers as an install that cannot update itself.
    fn confirm(&mut self, _: &Context) -> Option<CheckChoice> {
        let offered = self.offered(self.self_updating.unwrap_or(false) && self.release_installs);
        let choice = self.chosen.unwrap_or(offered.first()?.0);
        (offered.iter().any(|(offered, _)| *offered == choice) && self.answerable(choice)).then_some(choice)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<CheckChoice>> {
        let (output, choices) = self.present(ui);
        output.closed?;
        Some(DialogResult::Ok(output.picked.and_then(|at| choices.get(at).copied()).unwrap_or(CheckChoice::Close)))
    }
}
