//! The update flow of an egui app in one place: a check at start or on request, the startup prompt,
//! and the self-update worker. An app creates one [`Updater`], calls [`Updater::tick`] and
//! [`Updater::run`] each frame before it runs its `DialogHost`, and acts on the [`Action`] it returns,
//! so every app gets the same handling of failures, dropped workers, repaints and quitting.
//!
//! An app the launcher keeps on another version (see [`crate::pins`]) offers nothing itself: the
//! check reads the shared choice first, and a requested check reports [`Status::Managed`] while
//! [`Updater::managed`] says which choice it is.
//!
//! An update ends with the app closing. On Windows the update installs only after the app has
//! exited, so the updater says it is ready rather than installed, and the first start afterwards
//! reports what came of it: [`Status::Updated`] or [`Status::InstallFailed`].

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SendError, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, PoisonError};

use craft_ui::dialog::{DialogHost, StartupStage, dialog_id};
use craft_ui::runtime::frame::request_repaint_after;

use crate::pins::{Choice, Pins};
use crate::release::Release;
use crate::self_update::{Outcome, Restart};
use crate::stack_dialogs::{UpdateChoice, UpdatePromptDialog};
use crate::translate::{Note, Translate, english};
use crate::{App, Http, Paths, Platform, Stop, check_for_update, self_update};

/// Asks for the newest release (blocking; it runs on its own thread). Tests stand in for the network
/// with one; without it the check asks GitHub.
pub type CheckSource = Arc<dyn Fn() -> Result<Option<Release>, String> + Send + Sync>;

/// What a requested check came to, for a window that answers Help ▸ Check for Updates, and what
/// the last update came to, at the first start after it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Status {
    /// Nothing to report.
    Idle,
    /// A requested check is running.
    Checking,
    /// A requested check found nothing newer.
    UpToDate,
    /// A requested check failed.
    Failed(String),
    /// A requested check found an update downloading and installing; the startup prompt shows
    /// its progress.
    Installing,
    /// A requested check found an update ready and waiting for the app to close
    /// ([`READY_TEXT`]). [`Updater::restart_to_update`] asks the app to close.
    Ready,
    /// A requested check found that the launcher keeps this app on a version of its choosing
    /// ([`Updater::managed`]), so it does not update itself.
    Managed,
    /// This start follows an update that installed this version.
    Updated(String),
    /// The last update could not be installed, for this reason.
    InstallFailed(String),
}

/// What [`Status::Ready`] says: on Windows the update installs after the app exits; elsewhere it
/// is already in place and takes effect when the app restarts.
pub const READY_TEXT: &str =
    if cfg!(windows) { "Update ready. Close {app} to install it." } else { "The update is installed. Close {app} to finish; it restarts by itself." };

/// What the app should do after [`Updater::run`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Action {
    /// Nothing for the app to do this frame.
    None,
    /// Open this page in the browser.
    OpenUrl(String),
    /// Close the app through its normal close path (which asks about unsaved work): an update is
    /// ready and a helper installs it and restarts the app once the app has exited. This is a
    /// request: the user may cancel the close, and the update then stays ready, with
    /// [`Status::Ready`] for a requested check and Restart to Update in the prompt. The helper
    /// starts only at the exit: when the app drops the [`Updater`] as it exits, or
    /// calls [`Updater::start_pending_update`] in `on_exit`, which an app that ends the process
    /// there without dropping the `Updater` must do. An installation still in progress when
    /// such an app ends the process is lost, since its worker thread ends with the process.
    Close,
}

type Checked = Result<Option<Release>, String>;

/// The seconds between passes while a check or an installation runs; the poll is what picks up a finished worker.
const POLL_SECONDS: f64 = 0.1;

/// The line under the prompt's text: the installer's own words, or one of the updater's, translated each pass.
#[derive(Debug)]
enum Message {
    Installer(String),
    Note(Note),
}

/// The update prompt the updater holds for the app's dialog host.
#[derive(Debug)]
struct Offer {
    release: Release,
    /// The prompt is on the host's startup queue or open.
    shown: bool,
    message: Option<Message>,
    /// The update is ready and waits for the app to close: the main button reads Restart to
    /// Update.
    ready: bool,
}

impl Offer {
    fn new(release: Release) -> Self {
        Self { release, shown: false, message: None, ready: false }
    }

    fn text(&self, translate: Translate) -> Option<String> {
        self.message.as_ref().map(|message| match message {
            Message::Installer(text) => text.clone(),
            Message::Note(note) => note.translated(translate),
        })
    }
}

/// Why an installation failed: the installer's own words, shown as they are, or the updater's,
/// shown in the interface language.
#[derive(Debug)]
enum Failure {
    Installer(String),
    Updater(Note),
}

impl Failure {
    /// The worker panicked.
    fn panicked() -> Self {
        Self::Updater(Note::new("The update failed unexpectedly.", &[]))
    }

    /// The worker went away without an answer.
    fn dropped() -> Self {
        Self::Updater(Note::new("The update stopped unexpectedly.", &[]))
    }

    /// The reason in English, for the log and for the record the next start reads.
    fn reason(&self) -> String {
        match self {
            Self::Installer(e) => e.clone(),
            Self::Updater(note) => note.translated(english),
        }
    }
}

/// Logs a failed update and records it under `paths`, so that the next start reports it
/// ([`Status::InstallFailed`]). Used at the exit, where no prompt can show the reason.
fn record(app: &App, paths: Option<&Paths>, reason: &str) {
    log::warn!("{} self-update failed: {reason}", app.name);
    if let Some(paths) = paths {
        self_update::record_failure(paths, reason);
    }
}

impl From<String> for Failure {
    fn from(e: String) -> Self {
        Self::Installer(e)
    }
}

impl From<&str> for Failure {
    fn from(e: &str) -> Self {
        Self::Installer(e.to_owned())
    }
}

/// One app's update flow: the check, the startup prompt and the self-update worker. See the
/// module docs.
pub struct Updater {
    app: &'static App,
    version: String,
    source: Option<CheckSource>,
    status: Status,
    /// The check running (or queued behind it) was asked for by the user.
    requested: bool,
    started: bool,
    check: Option<Receiver<Checked>>,
    install: Option<(Receiver<Result<Restart, Failure>>, Release)>,
    prompt: Option<Offer>,
    /// A prompt the updater dropped is still on the host; it is withdrawn when it shows.
    stale: bool,
    /// The newest release a check found or the app offered; [`Updater::request_install`] works
    /// from it after the prompt closed.
    known: Option<Release>,
    /// A finished installation waiting for the app to exit: what still has to start then, and
    /// the release it installs.
    restart: Option<(Restart, Release)>,
    /// An update is ready and waits for the app to exit; nothing starts a second one.
    installed: bool,
    /// [`Self::run`] returns [`Action::Close`] at its next pass that may close: once when an
    /// installation finishes, and again for each [`Self::restart_to_update`].
    close_wanted: bool,
    /// The folders that hold the outcome records ([`self_update::take_outcome`]) in place of the
    /// app's own (tests).
    paths: Option<Paths>,
    /// Why the last update could not be installed, until a prompt that offers the update again
    /// shows it.
    failed_reason: Option<String>,
    /// Set when the app exits: the `Updater` is dropped or [`Updater::start_pending_update`] is
    /// called. An install worker sends its outcome while holding it, and starts a finished
    /// installation's helper itself once it is set.
    exiting: Arc<Mutex<bool>>,
    /// The version-choice file to read in place of [`Pins::default_path`] (tests).
    pins_path: Option<PathBuf>,
    /// The launcher's choice for this app when it is not stable, as of the last check or offer.
    managed: Option<Choice>,
    /// Puts the prompt and its messages in the interface language ([`crate::translate`]).
    translate: Translate,
}

impl Updater {
    /// An updater for `app`, running `version`.
    pub fn new(app: &'static App, version: impl Into<String>) -> Self {
        let version = version.into();
        let version = self_update::current_version(app, &version).map_or(version, |v| v.to_string());
        Self {
            app,
            version,
            source: None,
            status: Status::Idle,
            requested: false,
            started: false,
            check: None,
            install: None,
            prompt: None,
            stale: false,
            known: None,
            restart: None,
            installed: false,
            close_wanted: false,
            paths: None,
            failed_reason: None,
            exiting: Arc::new(Mutex::new(false)),
            pins_path: None,
            managed: None,
            translate: english,
        }
    }

    /// Words the messages under the startup prompt with `translate`, in the app's interface language
    /// ([`crate::translate`]). Errors from the network stay as they came. The prompt's own words come from the
    /// app's `UpdateWords`.
    #[must_use]
    pub fn with_translator(mut self, translate: Translate) -> Self {
        self.translate = translate;
        self
    }

    /// Opens the prompt offering `release`. A failed update offered again says why the last
    /// attempt failed, once.
    fn open_offer(&mut self, release: Release) {
        let mut offer = Offer::new(release);
        if let Some(reason) = self.failed_reason.take() {
            offer.message = Some(Message::Note(failed_note(&reason)));
        }
        self.prompt = Some(offer);
    }

    /// [`READY_TEXT`], translated when the prompt shows it.
    fn installed_note(&self) -> Note {
        Note::new(READY_TEXT, &[("app", self.app.name)])
    }

    /// Drops the prompt; one already on the host is withdrawn on a later [`Updater::run`].
    fn drop_prompt(&mut self) {
        if self.prompt.take().is_some_and(|offer| offer.shown) {
            self.stale = true;
        }
    }

    /// Replaces the network with `source` (tests).
    pub fn with_source(mut self, source: CheckSource) -> Self {
        self.source = Some(source);
        self
    }

    /// Reads and writes the update's outcome records under `paths` instead of the app's own
    /// folders (tests).
    pub fn with_paths(mut self, paths: Paths) -> Self {
        self.paths = Some(paths);
        self
    }

    /// The folders of the outcome records: [`Self::with_paths`]'s, else the app's own. Unit tests
    /// never read or write the app's own records.
    fn outcome_paths(&self) -> Option<Paths> {
        self.paths.clone().or_else(|| if cfg!(test) { None } else { Paths::for_app(self.app).ok() })
    }

    /// What the last update came to, once: [`Status::Updated`] when it installed the version
    /// running now, [`Status::InstallFailed`] with the reason when it failed.
    fn read_outcome(&mut self) {
        let Some(paths) = self.outcome_paths() else { return };
        match self_update::take_outcome(&paths, &self.version) {
            Some(Outcome::Installed(version)) => self.status = Status::Updated(version.to_string()),
            Some(Outcome::Failed(reason)) => {
                log::warn!("{} self-update failed: {reason}", self.app.name);
                self.failed_reason = Some(reason.clone());
                self.status = Status::InstallFailed(reason);
            }
            None => {}
        }
    }

    /// Reads the version choices from `path` instead of the launcher's file (tests).
    pub fn with_pins_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.pins_path = Some(path.into());
        self
    }

    /// The launcher's choice for this app when it keeps it off stable releases (a pinned version,
    /// nightlies, or a choice this version cannot read), as of the last check. A window answering
    /// Help ▸ Check for Updates can say so instead of "newest version".
    pub fn managed(&self) -> Option<&Choice> {
        self.managed.as_ref()
    }

    /// Reads the launcher's choice for this app; remembers it when it is not stable.
    fn read_choice(&mut self) -> bool {
        let pins = match &self.pins_path {
            Some(path) => Pins::load(path),
            None => Pins::load_default(),
        };
        self.managed = Some(pins.choice(self.app.id)).filter(|c| *c != Choice::Stable);
        if let Some(choice) = &self.managed {
            log::info!("{} follows the launcher's choice ({choice:?}); no update offered here", self.app.name);
            // A prompt opened before the pin would still install the stable release. One that is
            // installing or finished stays: `tick` reports through it.
            if self.install.is_none() && !self.installed {
                self.drop_prompt();
            }
        }
        self.managed.is_some()
    }

    /// The check at start, once. Later calls do nothing. It first reads what the last update came
    /// to, once: [`Self::status`] is then [`Status::Updated`] or [`Status::InstallFailed`], and
    /// the app shows its [`UpdateCheckDialog`](crate::stack_dialogs::UpdateCheckDialog) while the
    /// status is not [`Status::Idle`]. The check itself stays quiet unless it finds a release.
    pub fn check_at_start(&mut self) {
        if !std::mem::replace(&mut self.started, true) {
            self.read_outcome();
            self.begin(false);
        }
    }

    /// Help ▸ Check for Updates: a check whose outcome [`Self::status`] reports. A check already
    /// running becomes a requested one. While an update installs or waits for the app to close,
    /// nothing starts and the status says so ([`Status::Installing`], [`Status::Ready`]).
    pub fn check_now(&mut self) {
        self.started = true;
        self.begin(true);
    }

    /// Asks the app to close so that a ready update installs: the next [`Self::run`] (or
    /// [`Self::take_close`]) that may close returns [`Action::Close`]. Restart to Update calls it
    /// after the user canceled an earlier close. No close is asked for while no update is ready.
    pub fn restart_to_update(&mut self) {
        self.close_wanted = true;
    }

    /// Opens the startup prompt for a release the app found itself. After a failed update the
    /// prompt says why it failed, like the one a check opens. Opening the prompt sets
    /// [`Self::status`] to [`Status::Idle`].
    pub fn offer(&mut self, release: Release) {
        self.known = Some(release.clone());
        if self.install.is_none() && !self.installed && !self.read_choice() {
            self.drop_prompt();
            self.open_offer(release);
            self.status = Status::Idle;
        }
    }

    /// What the last requested check came to.
    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Dismisses the outcome of a requested check. One still running carries on, and reports
    /// like the check at start (a prompt if there is a release, otherwise nothing).
    pub fn dismiss_status(&mut self) {
        self.requested = false;
        self.status = Status::Idle;
    }

    /// Whether an installation is running.
    pub fn is_installing(&self) -> bool {
        self.install.is_some()
    }

    /// The prompt's status line in the interface language: the installer's error, or one of the
    /// updater's own notes ("Save or close your open work first"). `None` when there is none or
    /// no prompt is open.
    pub fn status_message(&self) -> Option<String> {
        self.prompt.as_ref().and_then(|offer| offer.text(self.translate))
    }

    /// Whether the startup prompt is showing.
    pub fn prompt_open(&self) -> bool {
        self.prompt.is_some()
    }

    fn begin(&mut self, requested: bool) {
        if self.install.is_some() || self.installed {
            if requested {
                self.status = self.pending_status();
            }
            return;
        }
        if requested {
            self.requested = true;
            self.status = Status::Checking;
        }
        if self.check.is_some() {
            return;
        }
        // The file is a few bytes, read once per check; the launcher may have changed it since.
        if self.read_choice() {
            self.finish_check(Ok(None));
            return;
        }
        let (tx, rx) = channel();
        let (source, app, version) = (self.source.clone(), self.app, self.version.clone());
        let spawned = std::thread::Builder::new().name("update-check".into()).spawn(move || {
            let ask = || match source {
                Some(ask) => ask(),
                None => Http::new(&format!("{}/Update", app.name)).and_then(|http| check_for_update(app, &http, &version)),
            };
            let answer =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(ask)).unwrap_or_else(|_| Err("the update check failed unexpectedly".into()));
            let _ = tx.send(answer);
        });
        match spawned {
            Ok(_) => self.check = Some(rx),
            Err(e) => self.finish_check(Err(format!("couldn't start the update check ({e})"))),
        }
    }

    /// The answer to a requested check while an update installs or waits for the app to exit. A
    /// helper that already started while the app kept running may have failed: its recorded
    /// reason is the answer then, the update is on offer again, and an open prompt says why
    /// instead of offering Restart to Update.
    fn pending_status(&mut self) -> Status {
        if self.install.is_some() {
            return Status::Installing;
        }
        if self.restart.is_none()
            && let Some(reason) = self.outcome_paths().and_then(|paths| self_update::take_failure(&paths))
        {
            log::warn!("{} self-update failed: {reason}", self.app.name);
            let reason = reason.trim().to_owned();
            self.installed = false;
            self.close_wanted = false;
            match &mut self.prompt {
                Some(offer) => {
                    offer.ready = false;
                    offer.message = Some(Message::Note(failed_note(&reason)));
                }
                // The next prompt that offers the update says why.
                None => self.failed_reason = Some(reason.clone()),
            }
            return Status::InstallFailed(reason);
        }
        Status::Ready
    }

    /// Sets the outcome of a check. The check at start leaves the status as it is: nothing, or
    /// the outcome of the last update ([`Self::read_outcome`]).
    fn finish_check(&mut self, answer: Checked) {
        let requested = std::mem::take(&mut self.requested);
        // A check that was running when an installation started answers with the installation's
        // state: the update installing, ready, or the recorded helper failure.
        if self.install.is_some() || self.installed {
            match &answer {
                Ok(Some(release)) => log::info!("{} update check found {} while an update installs", self.app.name, release.tag),
                Ok(None) => log::info!("{} update check found nothing newer while an update installs", self.app.name),
                Err(e) => log::warn!("{} update check failed while an update installs: {e}", self.app.name),
            }
            if requested {
                self.status = self.pending_status();
            }
            return;
        }
        // The launcher may have pinned or unpinned the app while the check ran: whatever the
        // answer, an app it manages offers nothing and says so.
        if self.read_choice() {
            if let Err(e) = &answer {
                log::info!("{} update check failed ({e}), but the launcher manages its version", self.app.name);
            }
            if requested {
                self.status = Status::Managed;
            }
            return;
        }
        match answer {
            Ok(Some(release)) => {
                self.known = Some(release.clone());
                match &mut self.prompt {
                    None => self.open_offer(release),
                    // A prompt the app opened before (`offer`) says why the last update failed.
                    Some(offer) => {
                        if let Some(reason) = self.failed_reason.take() {
                            offer.message = Some(Message::Note(failed_note(&reason)));
                        }
                    }
                }
                self.status = Status::Idle;
            }
            Ok(None) if requested => self.status = Status::UpToDate,
            Ok(None) => {}
            Err(e) => {
                log::warn!("{} update check failed: {e}", self.app.name);
                if requested {
                    self.status = Status::Failed(e);
                }
            }
        }
    }

    /// Picks up a finished check or installation. Call once per frame, before [`Self::run`]. It
    /// asks for the next pass every [`POLL_SECONDS`] while either is running.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.check {
            match rx.try_recv() {
                Ok(answer) => {
                    self.check = None;
                    self.finish_check(answer);
                }
                Err(TryRecvError::Empty) => request_repaint_after(ctx, POLL_SECONDS),
                Err(TryRecvError::Disconnected) => {
                    self.check = None;
                    self.finish_check(Err("the update check stopped unexpectedly".into()));
                }
            }
        }
        if let Some((rx, _)) = &self.install {
            let outcome = match rx.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(TryRecvError::Empty) => {
                    request_repaint_after(ctx, POLL_SECONDS);
                    None
                }
                Err(TryRecvError::Disconnected) => Some(Err(Failure::dropped())),
            };
            if let Some(outcome) = outcome {
                let release = self.install.take().map(|(_, release)| release);
                match outcome {
                    Ok(restart) => {
                        self.installed = true;
                        self.restart = release.map(|release| (restart, release));
                        self.close_wanted = true;
                        let note = self.installed_note();
                        if let Some(offer) = &mut self.prompt {
                            offer.ready = true;
                            offer.message = Some(Message::Note(note));
                        }
                        if matches!(self.status, Status::Installing) {
                            self.status = Status::Ready;
                        }
                    }
                    Err(failure) => {
                        match &failure {
                            Failure::Installer(e) => log::warn!("{} self-update failed: {e}", self.app.name),
                            Failure::Updater(note) => log::warn!("{} self-update failed: {}", self.app.name, note.translated(english)),
                        }
                        // The prompt says why; a result window showing the installation closes.
                        if matches!(self.status, Status::Installing) {
                            self.status = Status::Idle;
                        }
                        // The prompt may have been dismissed meanwhile: bring it back with the reason.
                        if self.prompt.is_none()
                            && let Some(release) = release
                        {
                            self.prompt = Some(Offer::new(release));
                        }
                        if let Some(offer) = &mut self.prompt {
                            offer.message = Some(match failure {
                                Failure::Installer(e) => Message::Installer(e),
                                Failure::Updater(note) => Message::Note(note),
                            });
                        }
                    }
                }
            }
        }
    }

    /// Whether the app should close now so a ready update installs, without showing the startup
    /// prompt (a host that shows the progress in its own dialog calls this instead of
    /// [`run`](Self::run)). True once per finished install and once per
    /// [`restart_to_update`](Self::restart_to_update), and only with `can_start` true: while the
    /// app holds unsaved work the close waits and is asked again. The close is only asked for:
    /// the helper, which waits just a minute for the app to exit, starts at the exit itself
    /// ([`start_pending_update`](Self::start_pending_update)), so a close the user cancels leaves
    /// the update ready.
    pub fn take_close(&mut self, can_start: bool) -> bool {
        can_start && self.restart.is_some() && std::mem::take(&mut self.close_wanted)
    }

    /// Puts the startup prompt on `dialogs` at [`StartupStage::Update`] when one is due, feeds the open one the
    /// install's progress, and starts what the user chose. An installation only starts, and a ready update only
    /// asks the app to close, when `can_start` is true: an app passes `false` while it holds unsaved work. The app
    /// calls this before `DialogHost::run`.
    pub fn run(&mut self, dialogs: &mut DialogHost, can_start: bool) -> Action {
        // The close waits for the app to hold no unsaved work: the user may have edited while the
        // update installed.
        if self.take_close(can_start) {
            return Action::Close;
        }
        // Update and Restart leaves the prompt open to show the progress; the pick comes through the dialog.
        if self.prompt.is_some() && dialogs.get_mut_by_id::<UpdatePromptDialog>(prompt_id()).is_some_and(UpdatePromptDialog::take_update_request) {
            self.request_install(can_start);
        }
        let mut result = dialogs.take::<UpdatePromptDialog>();
        if self.stale {
            if let Some(dialog) = dialogs.get_mut_by_id::<UpdatePromptDialog>(prompt_id()) {
                dialog.withdraw();
            }
            self.stale = result.is_none() && self.stale;
            result = None;
        }
        let mut action = Action::None;
        if let (Some(result), Some(offer)) = (result, &mut self.prompt) {
            offer.shown = false;
            match result.ok().unwrap_or(UpdateChoice::Later) {
                UpdateChoice::UpdateAndRestart => {
                    self.request_install(can_start);
                }
                UpdateChoice::Download => {
                    action = download_action(self.app, &offer.release);
                    self.prompt = None;
                }
                UpdateChoice::Later => {
                    // An installation carries on and reports through `tick`.
                    self.prompt = None;
                }
            }
        }
        self.sync(dialogs);
        action
    }

    /// Enqueues the prompt when it is not on the host, else shows it the install's progress.
    fn sync(&mut self, dialogs: &mut DialogHost) {
        let Some(offer) = &self.prompt else { return };
        let (installing, message) = (self.install.is_some(), offer.text(self.translate));
        if offer.shown {
            if let Some(dialog) = dialogs.get_mut_by_id::<UpdatePromptDialog>(prompt_id()) {
                dialog.set_installing(installing);
                dialog.set_ready(offer.ready);
                dialog.set_status(message);
            }
        } else if !self.stale {
            let self_updating = self_update::is_self_updating(self.app) && self_update::release_installs(self.app, &offer.release);
            let mut dialog = UpdatePromptDialog::new(offer.release.clone(), self.version.clone(), self_updating);
            dialog.set_installing(installing);
            dialog.set_ready(offer.ready);
            dialog.set_status(message);
            dialogs.enqueue(StartupStage::Update, dialog);
            if let Some(offer) = &mut self.prompt {
                offer.shown = true;
            }
        }
    }

    /// Starts the helper of a ready update, so that the update installs once the app has exited.
    /// Call it once the app's exit is certain (a close nothing can cancel any more, in `on_exit`),
    /// whether or not the exit came through [`Action::Close`]; dropping the `Updater` calls it
    /// too. Later calls do nothing. An installation still running at the first call starts its
    /// helper itself when it finishes, as long as the process is still running then, and no
    /// longer reports through [`Updater::tick`]; an app that ends the process in `on_exit` loses
    /// an installation still in progress, since its worker thread ends with the process. A
    /// helper that cannot start, or an installation that failed and was not reported yet, is
    /// recorded, and the next start reports it ([`Status::InstallFailed`]).
    pub fn start_pending_update(&mut self) {
        // Held while the channel is read: a worker sent its outcome before, and it is read here,
        // or it sends after and starts its helper (or records its failure) itself.
        let exiting = Arc::clone(&self.exiting);
        let mut exiting = exiting.lock().unwrap_or_else(PoisonError::into_inner);
        *exiting = true;
        // An installation that finished since the last `tick`, or whose worker ended without a word.
        // One still running is let go: it now reports through `deliver` alone.
        if let Some((rx, release)) = self.install.take()
            && let Some(outcome) = match rx.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(Failure::dropped())),
            }
        {
            match outcome {
                Ok(restart) => {
                    self.restart = Some((restart, release));
                    self.installed = true;
                }
                Err(failure) => record(self.app, self.outcome_paths().as_ref(), &failure.reason()),
            }
        }
        if let Some((restart, _)) = self.restart.take()
            && let Err(e) = restart.start()
        {
            record(self.app, self.outcome_paths().as_ref(), &e);
        }
    }

    /// Starts installing the newest release the updater knows of, as the prompt's Update and
    /// Restart does: the one a check found or [`offer`](Self::offer) was given, whether or not
    /// the prompt is still open (a closed one is made again to hold the progress). Returns false
    /// when it knows of no release (or the launcher manages the version), and then nothing
    /// happens. With `can_start` false (the app holds unsaved work) nothing starts and
    /// [`status_message`](Self::status_message) asks the user to save first. Progress and failures
    /// show through [`is_installing`](Self::is_installing) and `status_message`, and a finished
    /// install ends in [`Action::Close`] from [`run`](Self::run).
    pub fn request_install(&mut self, can_start: bool) -> bool {
        if self.prompt.is_none()
            && let Some(release) = self.known.clone()
            && !self.read_choice()
        {
            self.prompt = Some(Offer::new(release));
        }
        if self.prompt.is_none() {
            return false;
        }
        if can_start {
            self.start_install();
        } else if let Some(offer) = &mut self.prompt {
            offer.message = Some(Message::Note(Note::new("Save or close your open work first; the update restarts the app.", &[])));
        }
        true
    }

    fn start_install(&mut self) {
        // The launcher may have pinned the app while the prompt was open; that closes it.
        if self.install.is_none() && !self.installed && self.read_choice() {
            return;
        }
        let installed_note = self.installed_note();
        let paths = self.outcome_paths();
        let Some(offer) = &mut self.prompt else { return };
        if self.installed {
            // Restart to update: the app is asked to close again.
            offer.ready = true;
            offer.message = Some(Message::Note(installed_note));
            if self.restart.is_some() {
                self.close_wanted = true;
            }
            return;
        }
        let Some(installed) = self_update::current(self.app) else {
            offer.message = Some(Message::Note(Note::new("Cannot auto-update this installation type. Please download manually.", &[])));
            return;
        };
        offer.message = None;
        self.failed_reason = None;
        let (tx, rx) = channel();
        let (app, version, release) = (self.app, self.version.clone(), offer.release.clone());
        let worker_release = release.clone();
        let exiting = handoff(&mut self.exiting);
        let spawned = std::thread::Builder::new().name("self-update".into()).spawn(move || {
            let run = || install(app, &installed, worker_release, &version).map_err(Failure::Installer);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).unwrap_or_else(|_| Err(Failure::panicked()));
            deliver(app, &tx, &exiting, paths.as_ref(), outcome);
        });
        match spawned {
            Ok(_) => self.install = Some((rx, release)),
            Err(e) => {
                offer.message = Some(Message::Note(Note::new("Couldn't start the update ({error})", &[("error", &e.to_string())])));
            }
        }
    }
}

impl Drop for Updater {
    /// The app is exiting: an installed update that is still waiting starts its helper, which
    /// waits for this process to exit before it installs.
    fn drop(&mut self) {
        self.start_pending_update();
    }
}

/// The flag a new install worker sends its outcome under (the `Updater`'s `exiting`). After
/// [`Updater::start_pending_update`] set it while the app kept running (a helper that then failed
/// puts the update on offer again), a new flag replaces it, so that the new worker reports
/// through `tick` instead of starting its helper at once. A worker let go before keeps the old one.
fn handoff(exiting: &mut Arc<Mutex<bool>>) -> Arc<Mutex<bool>> {
    if *exiting.lock().unwrap_or_else(PoisonError::into_inner) {
        *exiting = Arc::new(Mutex::new(false));
    }
    Arc::clone(exiting)
}

/// Sends the install worker's outcome to the [`Updater`]. When the app is exiting (the `Updater`
/// was dropped or [`Updater::start_pending_update`] was called), the worker starts a finished
/// installation's helper itself, since nothing will start it at a close. A helper that cannot
/// start, or a failed installation, is then recorded under `paths` for the next start.
fn deliver(app: &App, tx: &Sender<Result<Restart, Failure>>, exiting: &Mutex<bool>, paths: Option<&Paths>, outcome: Result<Restart, Failure>) {
    let gone = exiting.lock().unwrap_or_else(PoisonError::into_inner);
    let unsent = if *gone { Some(outcome) } else { tx.send(outcome).err().map(|SendError(outcome)| outcome) };
    drop(gone);
    match unsent {
        Some(Ok(restart)) => {
            if let Err(e) = restart.start() {
                record(app, paths, &e);
            }
        }
        Some(Err(failure)) => record(app, paths, &failure.reason()),
        None => {}
    }
}

/// "The update could not be installed: …", for a prompt that offers a failed update again.
fn failed_note(reason: &str) -> Note {
    Note::new("The update could not be installed: {reason}", &[("reason", reason)])
}

/// The id of the update prompt on the host.
fn prompt_id() -> craft_ui::dialog::DialogId {
    dialog_id("updateDialog.prompt")
}

#[cfg(feature = "screen-testing")]
impl Updater {
    /// Waits for the running check's answer on its channel (its thread always sends one, or ends
    /// and closes the channel), then hands it to `tick` as a pass would (tests).
    pub fn settle_for_tests(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.check.take() else { return };
        let (tx, answer) = channel();
        if let Ok(checked) = rx.recv() {
            let _ = tx.send(checked);
        }
        // Without an answer `tx` is dropped here, and `tick` sees a check that stopped.
        drop(tx);
        self.check = Some(answer);
        self.tick(ctx);
    }
}

fn install(app: &'static App, installed: &self_update::Install, mut release: Release, version: &str) -> Result<Restart, String> {
    let http = Http::new(&format!("{}/Update", app.name)).map_err(|e| format!("could not initialize HTTP: {e}"))?;
    if release.assets.is_empty()
        && let Some(full) = check_for_update(app, &http, version).map_err(|e| format!("could not fetch release assets: {e}"))?
    {
        release = full;
    }
    let paths = Paths::for_app(app).map_err(|e| format!("could not find paths: {e}"))?;
    let platform = Platform::current().ok_or("unsupported platform for self update")?;
    self_update::prepare_update(app, installed, &release, platform, &http, &paths, &Stop::default(), &mut |_| {}, &|| Ok(()), Some(version))
}

/// Download in the prompt: the release page, when it is a safe web link (an app may build its
/// `Release` from its own feed or cache, so the link isn't always one GitHub's parser checked).
/// A link that fails the check falls back to the app's releases page: the prompt has closed, so
/// Download must still open something.
pub fn download_action(app: &App, release: &Release) -> Action {
    let safe = crate::release::safe_web_link;
    if safe(&release.html_url) {
        return Action::OpenUrl(release.html_url.clone());
    }
    let releases = app.releases_url();
    if safe(&releases) { Action::OpenUrl(releases) } else { Action::None }
}

#[cfg(test)]
mod tests;
